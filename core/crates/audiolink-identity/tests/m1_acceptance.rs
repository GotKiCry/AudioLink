//! `docs/11-m1-contract.md` §4 的验收用例（黑盒：只用 crate 的公开 API，不碰内部实现）。
//!
//! 覆盖契约 §4 列出的 6 组：
//! 1. `load_or_create` 两次 → 同一 `NodeId`（持久化生效）且两个 PEM 文件确实存在；
//! 2. `NodeId` == 用 `sha2` **独立现算**的 SHA-256(cert DER)（不调被测代码自证）；
//! 3. 签名往返（A 签 B 验）+ 换 nonce / 换 peer / 篡改签名 1 bit 三种失败；
//! 4. `TrustStore` 落盘→重载、`revoke`、损坏 JSON → `Err`；
//! 5. `PinGate` 消耗 / 递减 / 锁定 / 过期；
//! 6. 错误路径返回 `Err` 且不 panic。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// 形状自检把完整函数指针类型直接写出来（含 5 参数的 verify_challenge）——
// 「类型复杂」正是那一段的**目的**，拆成 type 别名反而看不出契约长什么样。
#![allow(clippy::type_complexity)]

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use audiolink_identity::{
    CERT_FILE_NAME, CERT_SUBJECT_ALT_NAME, IdentityError, KEY_FILE_NAME, NodeIdentity, PIN_LOCKOUT,
    PIN_MAX_ATTEMPTS, PIN_TTL, PairRejection, PinGate, TrustEntry, TrustStore,
};
use audiolink_types::{NodeId, Platform};
use sha2::{Digest, Sha256};

/// 契约 §4 的形状自检：把每个签名写成函数指针，任何形状漂移都会在这里**编译失败**。
#[test]
fn 契约形状自检() {
    // `impl AsRef<Path>` 参数不能用 turbofish 固定，用带注解的非捕获闭包把形状钉死。
    let _: fn(&Path, &str) -> Result<NodeIdentity, IdentityError> =
        |dir: &Path, name: &str| NodeIdentity::load_or_create(dir, name);
    let _: fn(&str, &str, &str) -> Result<NodeIdentity, IdentityError> = NodeIdentity::from_pem;
    let _: fn(&NodeIdentity) -> NodeId = NodeIdentity::id;
    let _: for<'a> fn(&'a NodeIdentity) -> &'a [u8] = NodeIdentity::cert_der;
    let _: for<'a> fn(&'a NodeIdentity) -> &'a [u8] = NodeIdentity::key_der_pkcs8;
    let _: for<'a> fn(&'a NodeIdentity) -> &'a str = NodeIdentity::node_name;
    let _: for<'a> fn(&'a NodeIdentity) -> &'a str = NodeIdentity::cert_pem;
    let _: fn(&NodeIdentity, &[u8; 32], NodeId) -> Result<Vec<u8>, IdentityError> =
        NodeIdentity::sign_challenge;
    let _: fn(&[u8], &[u8; 32], NodeId, NodeId, &[u8]) -> Result<(), IdentityError> =
        NodeIdentity::verify_challenge;

    let _: fn(&Path) -> Result<TrustStore, IdentityError> = |path: &Path| TrustStore::load(path);
    let _: fn(&TrustStore, NodeId) -> bool = TrustStore::is_trusted;
    let _: fn(&mut TrustStore, TrustEntry) -> Result<(), IdentityError> = TrustStore::trust;
    let _: fn(&mut TrustStore, NodeId) -> Result<bool, IdentityError> = TrustStore::revoke;
    let _: for<'a> fn(&'a TrustStore) -> &'a [TrustEntry] = TrustStore::entries;

    let _: fn(Instant) -> PinGate = PinGate::new;
    let _: for<'a> fn(&'a PinGate) -> &'a str = PinGate::pin;
    let _: fn(&PinGate) -> Instant = PinGate::expires_at;
    let _: fn(&PinGate) -> u8 = PinGate::remaining_attempts;
    let _: fn(&mut PinGate, &str, Instant) -> Result<(), PairRejection> = PinGate::verify;
    let _: fn(&PinGate, Instant) -> Option<Duration> = PinGate::lockout_remaining;

    let _: Duration = PIN_TTL;
    let _: u8 = PIN_MAX_ATTEMPTS;
    let _: Duration = PIN_LOCKOUT;
    let _ = PairRejection::Expired;
    let _ = PairRejection::WrongPin { remaining: 0 };
    let _ = PairRejection::Locked {
        retry_after: Duration::ZERO,
    };

    let entry = TrustEntry {
        id: NodeId::from_bytes([0; NodeId::LEN]),
        name: String::new(),
        platform: Platform::Unknown,
        paired_at_unix: 0,
    };
    assert_eq!(entry.paired_at_unix, 0);
}

/// 回归：中文（非 ASCII）设备名必须能建出身份。
///
/// 曾经的实现把展示名塞进证书 SAN，而 SAN 是 `dNSName`（IA5String，仅 ASCII），于是
/// `load_or_create(dir, "我的手机")` 直接报 `Invalid IA5String` —— 本项目设备名天然是中文。
/// 现在 SAN 固定为常量 `audiolink`，展示名只留在内存/信任库里。
#[test]
fn 中文设备名也能建身份且不进证书() {
    let dir = tempfile::tempdir().unwrap();
    let name = "我的手机🎧 · 客厅 PC";

    let identity = NodeIdentity::load_or_create(dir.path(), name).unwrap();
    assert_eq!(identity.node_name(), name);
    // SAN 必须是固定的 ASCII 常量（与 net 的 server_name 对齐）……
    assert!(
        identity
            .cert_der()
            .windows(CERT_SUBJECT_ALT_NAME.len())
            .any(|window| window == CERT_SUBJECT_ALT_NAME.as_bytes()),
        "证书里找不到固定的 SAN 常量"
    );
    // ……而展示名一个字节都不该出现在证书里。
    assert!(
        !identity
            .cert_der()
            .windows(name.len())
            .any(|window| window == name.as_bytes()),
        "展示名被写进了证书"
    );

    // 重新加载：指纹不变（展示名不参与身份）。
    let again = NodeIdentity::load_or_create(dir.path(), "换了个名字").unwrap();
    assert_eq!(again.id(), identity.id(), "改名字不该换身份");
}

/// 用例 1：`load_or_create` 两次得到**同一** `NodeId`，且 `cert.pem` / `key.pem` 已落盘。
#[test]
fn 用例1_两次加载同一指纹且_pem_落盘() {
    let dir = tempfile::tempdir().unwrap();

    let first = NodeIdentity::load_or_create(dir.path(), "验收-持久化").unwrap();
    let cert_path = dir.path().join(CERT_FILE_NAME);
    let key_path = dir.path().join(KEY_FILE_NAME);
    assert!(cert_path.exists(), "cert.pem 未落盘");
    assert!(key_path.exists(), "key.pem 未落盘");
    assert!(first.cert_pem().contains("BEGIN CERTIFICATE"));
    assert!(!first.key_der_pkcs8().is_empty());
    assert_eq!(first.node_name(), "验收-持久化");

    let second = NodeIdentity::load_or_create(dir.path(), "验收-持久化").unwrap();
    assert_eq!(
        first.id(),
        second.id(),
        "第二次加载的指纹变了，持久化没生效"
    );
    assert_eq!(first.cert_der(), second.cert_der());
    // 加载路径不许改写已有身份文件。
    assert_eq!(
        fs::read_to_string(&cert_path).unwrap(),
        first.cert_pem(),
        "加载过程改写了 cert.pem"
    );
    assert!(
        !fs::read_to_string(&key_path).unwrap().is_empty(),
        "key.pem 是空的"
    );
}

/// 用例 2：`NodeId` == 独立现算的 SHA-256(cert DER)（用 `sha2`，不调被测代码）。
#[test]
fn 用例2_指纹等于独立现算的_sha256() {
    let dir = tempfile::tempdir().unwrap();
    let identity = NodeIdentity::load_or_create(dir.path(), "验收-指纹").unwrap();

    let expected = Sha256::digest(identity.cert_der());
    assert_eq!(identity.id().as_bytes().as_slice(), expected.as_slice());
    // 顺带确认 hex 形式也是同一份字节（展示层不得引入第二种指纹）。
    assert_eq!(
        identity.id().to_hex(),
        audiolink_types::hex_encode(expected.as_slice())
    );
    assert!(!identity.id().is_zero());
}

/// 用例 3：A 签 B 验通过；换 nonce / 换 peer / 篡改签名 1 bit → 全部失败。
#[test]
fn 用例3_签名往返与三种篡改都必须失败() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let a = NodeIdentity::load_or_create(dir_a.path(), "A").unwrap();
    let b = NodeIdentity::load_or_create(dir_b.path(), "B").unwrap();
    let nonce = [0x5a; 32];

    // 正向：A 签（绑定 A、B 双方指纹），B 用 A 的证书验。
    let signature = a.sign_challenge(&nonce, b.id()).unwrap();
    NodeIdentity::verify_challenge(a.cert_der(), &nonce, b.id(), a.id(), &signature)
        .expect("A 签 B 验应通过");
    // 反向也必须成立（B 签 A 验），避免报文顺序写成「单向碰巧对」。
    let signature_back = b.sign_challenge(&nonce, a.id()).unwrap();
    NodeIdentity::verify_challenge(b.cert_der(), &nonce, a.id(), b.id(), &signature_back)
        .expect("B 签 A 验应通过");

    // 篡改 1：换 nonce。
    let other_nonce = [0x5b; 32];
    assert!(
        NodeIdentity::verify_challenge(a.cert_der(), &other_nonce, b.id(), a.id(), &signature)
            .is_err(),
        "换了 nonce 竟然验过了"
    );

    // 篡改 2：换 peer（local/peer 对调 → 报文里的双方指纹顺序变了）。
    assert!(
        NodeIdentity::verify_challenge(a.cert_der(), &nonce, a.id(), b.id(), &signature).is_err(),
        "换了 peer 竟然验过了"
    );

    // 篡改 3：签名末位翻 1 bit（保持 DER 结构合法，纯粹把签名值改坏）。
    let mut tampered = signature.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x01;
    let outcome = NodeIdentity::verify_challenge(a.cert_der(), &nonce, b.id(), a.id(), &tampered);
    // §11：验签失败必须报 1004（可能是中间人）。
    assert_eq!(
        outcome.unwrap_err().code(),
        audiolink_types::ErrorCode::AuthFailed,
        "篡改签名应报 AUTH_FAILED"
    );

    // 边界：空签名 / 垃圾签名只能是 Err，不许 panic。
    assert!(NodeIdentity::verify_challenge(a.cert_der(), &nonce, b.id(), a.id(), &[]).is_err());
    assert!(
        NodeIdentity::verify_challenge(a.cert_der(), &nonce, b.id(), a.id(), &[0xff; 70]).is_err()
    );
}

/// 用例 4：信任库落盘 → 重载 `is_trusted` 保持；`revoke` 后为 false；损坏 JSON → `Err`。
#[test]
fn 用例4_信任库落盘重载撤销与损坏报错() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trust.json");

    // 不存在 = 空库（不是错误）。
    let mut store = TrustStore::load(&path).unwrap();
    assert!(store.entries().is_empty());

    let phone = NodeId::from_bytes([0x11; NodeId::LEN]);
    let desktop = NodeId::from_bytes([0x22; NodeId::LEN]);
    store
        .trust(TrustEntry {
            id: phone,
            name: "我的手机".to_owned(),
            platform: Platform::Android,
            paired_at_unix: 1_760_000_000,
        })
        .unwrap();
    store
        .trust(TrustEntry {
            id: desktop,
            name: "办公室 PC".to_owned(),
            platform: Platform::Windows,
            paired_at_unix: 1_760_000_001,
        })
        .unwrap();

    // 重载：信任保持，字段完整。
    let mut reloaded = TrustStore::load(&path).unwrap();
    assert!(reloaded.is_trusted(phone), "重载后 phone 不再被信任");
    assert!(reloaded.is_trusted(desktop));
    assert!(!reloaded.is_trusted(NodeId::from_bytes([0x33; NodeId::LEN])));
    assert_eq!(reloaded.entries().len(), 2);

    // 撤销：返回值 + 重载后依然不信任。
    assert!(reloaded.revoke(phone).unwrap());
    assert!(!reloaded.is_trusted(phone));
    assert!(!reloaded.revoke(phone).unwrap(), "重复撤销应返回 false");
    let after_revoke = TrustStore::load(&path).unwrap();
    assert!(!after_revoke.is_trusted(phone));
    assert!(after_revoke.is_trusted(desktop));

    // 损坏 JSON：必须 Err —— 静默重置等于悄悄清空信任边界。
    fs::write(&path, "{ \"version\": 1, \"entries\": [ {\"id\": ").unwrap();
    let broken = TrustStore::load(&path);
    assert!(
        matches!(broken, Err(IdentityError::TrustStore { .. })),
        "损坏的信任库没报错：{broken:?}"
    );
    // 报错之后文件还在原地（没有被"重置"成空库）。
    assert!(fs::read_to_string(&path).unwrap().contains("entries"));
}

/// 用例 5：正确 PIN 通过且二次通过被拒；错误 PIN 递减；第 5 次失败进入锁定；过期返回 `Expired`。
#[test]
fn 用例5_pin_门禁的消耗递减锁定与过期() {
    let t0 = Instant::now();

    // --- 消耗（不可重放）---
    let mut gate = PinGate::new(t0);
    let pin = gate.pin().to_owned();
    assert_eq!(pin.len(), 6, "PIN 不是 6 位：{pin}");
    assert!(pin.bytes().all(|byte| byte.is_ascii_digit()));
    assert_eq!(gate.expires_at(), t0 + PIN_TTL);
    assert_eq!(gate.remaining_attempts(), PIN_MAX_ATTEMPTS);
    assert_eq!(gate.lockout_remaining(t0), None);

    assert!(gate.verify(&pin, t0).is_ok(), "正确 PIN 应通过");
    assert_eq!(
        gate.verify(&pin, t0 + Duration::from_secs(1)),
        Err(PairRejection::Expired),
        "已消耗的 PIN 竟能二次通过（可重放）"
    );
    // 紧接着用错误 PIN 也不行：消耗状态不会因为一次失败而回退。
    assert_eq!(
        gate.verify(&wrong_pin_of(&pin), t0 + Duration::from_secs(2)),
        Err(PairRejection::Expired)
    );

    // --- 递减 + 锁定 ---
    let mut gate = PinGate::new(t0);
    let pin = gate.pin().to_owned();
    let wrong = wrong_pin_of(&pin);
    for expected_remaining in (1..PIN_MAX_ATTEMPTS).rev() {
        assert_eq!(
            gate.verify(&wrong, t0),
            Err(PairRejection::WrongPin {
                remaining: expected_remaining
            })
        );
        assert_eq!(gate.remaining_attempts(), expected_remaining);
    }
    // 第 5 次失败 → 锁定 5 分钟。
    assert_eq!(
        gate.verify(&wrong, t0),
        Err(PairRejection::Locked {
            retry_after: PIN_LOCKOUT
        })
    );
    assert_eq!(gate.remaining_attempts(), 0);
    assert_eq!(gate.lockout_remaining(t0), Some(PIN_LOCKOUT));
    assert_eq!(
        gate.lockout_remaining(t0 + Duration::from_secs(299)),
        Some(Duration::from_secs(1))
    );
    assert_eq!(gate.lockout_remaining(t0 + PIN_LOCKOUT), None);
    // 锁定期间即使提交正确 PIN 也不放行，且返回真实剩余时长。
    assert_eq!(
        gate.verify(&pin, t0 + Duration::from_secs(10)),
        Err(PairRejection::Locked {
            retry_after: PIN_LOCKOUT - Duration::from_secs(10)
        })
    );

    // --- 过期 ---
    let mut gate = PinGate::new(t0);
    let pin = gate.pin().to_owned();
    // 59 s 仍有效（有效期是 60 s）。
    assert!(gate.verify(&pin, t0 + Duration::from_secs(59)).is_ok());

    let mut gate = PinGate::new(t0);
    let pin = gate.pin().to_owned();
    assert_eq!(
        gate.verify(&pin, t0 + PIN_TTL),
        Err(PairRejection::Expired),
        "到期时刻应已过期"
    );
    assert_eq!(
        gate.verify(&pin, t0 + Duration::from_secs(600)),
        Err(PairRejection::Expired)
    );

    // 长度不对的提交只能是 WrongPin，不许 panic。
    let mut gate = PinGate::new(t0);
    assert!(matches!(
        gate.verify("1", t0),
        Err(PairRejection::WrongPin { .. })
    ));
    assert!(matches!(
        gate.verify("", t0),
        Err(PairRejection::WrongPin { .. })
    ));
    assert!(matches!(
        gate.verify("1234567", t0),
        Err(PairRejection::WrongPin { .. })
    ));
}

/// 用例 6：落盘 / 读取的错误路径（路径被文件占住、文件位置被目录占住、内容损坏）→ `Err` 且不 panic。
#[test]
fn 用例6_错误路径返回_err_且不_panic() {
    let dir = tempfile::tempdir().unwrap();

    // (a) 身份目录的位置被一个普通文件占住。
    let occupied = dir.path().join("occupied-by-file");
    fs::write(&occupied, b"do-not-touch").unwrap();
    assert!(NodeIdentity::load_or_create(&occupied, "x").is_err());
    assert_eq!(
        fs::read(&occupied).unwrap(),
        b"do-not-touch",
        "失败路径改动了占位文件"
    );

    // (b) cert.pem 的位置被一个目录占住。
    let partial = tempfile::tempdir().unwrap();
    fs::create_dir(partial.path().join(CERT_FILE_NAME)).unwrap();
    fs::write(partial.path().join(KEY_FILE_NAME), b"not a pem").unwrap();
    assert!(NodeIdentity::load_or_create(partial.path(), "x").is_err());

    // (c) 信任库路径指向一个目录。
    let as_dir = dir.path().join("trust-as-dir");
    fs::create_dir(&as_dir).unwrap();
    assert!(TrustStore::load(&as_dir).is_err());

    // (d) 落盘目标不可达：trust.json 的父路径是个文件。
    let parent_is_file = dir.path().join("file-not-dir");
    fs::write(&parent_is_file, b"x").unwrap();
    let mut store = TrustStore::load(parent_is_file.join("trust.json")).unwrap();
    let outcome = store.trust(TrustEntry {
        id: NodeId::from_bytes([0x44; NodeId::LEN]),
        name: "写不进去".to_owned(),
        platform: Platform::Windows,
        paired_at_unix: 0,
    });
    assert!(outcome.is_err(), "落盘到不可达路径竟然成功了");

    // (e) 证书 PEM 内容损坏 / 私钥 PEM 内容损坏。
    let garbage = tempfile::tempdir().unwrap();
    fs::write(
        garbage.path().join(CERT_FILE_NAME),
        b"-----BEGIN CERTIFICATE-----\nnot base64!!\n-----END CERTIFICATE-----\n",
    )
    .unwrap();
    fs::write(garbage.path().join(KEY_FILE_NAME), b"garbage, not pem").unwrap();
    assert!(NodeIdentity::load_or_create(garbage.path(), "x").is_err());

    // (f) 空节点名（PEM 完全合法）也必须是 Err，不能悄悄生成一个无名身份。
    let named = tempfile::tempdir().unwrap();
    let good = NodeIdentity::load_or_create(named.path(), "ok").unwrap();
    let key_pem = fs::read_to_string(named.path().join(KEY_FILE_NAME)).unwrap();
    assert!(
        NodeIdentity::from_pem(good.cert_pem(), &key_pem, "   ").is_err(),
        "空节点名被接受了"
    );
    // (g) 以上每条都走到了断言之后 —— 没有 panic 本身就是证据。
}

/// 造一个与 `pin` 不同的 6 位数字串。
fn wrong_pin_of(pin: &str) -> String {
    if pin == "000000" {
        "000001".to_owned()
    } else {
        "000000".to_owned()
    }
}
