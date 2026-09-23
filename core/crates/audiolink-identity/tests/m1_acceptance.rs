//! `docs/11-m1-contract.md` §4 的验收用例（黑盒：只用 crate 的公开 API，不碰内部实现）。
//!
//! 覆盖契约 §4 里与身份相关的全部内容（配对 / 信任库 / PIN / 挑战应答四组用例随
//! `docs/71-remove-pairing.md` 一起删除）：
//! 1. `load_or_create` 两次 → 同一 `NodeId`（持久化生效）且两个 PEM 文件确实存在；
//! 2. `NodeId` == 用 `sha2` **独立现算**的 SHA-256(cert DER)（不调被测代码自证）；
//! 3. 错误路径返回 `Err` 且不 panic。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// 形状自检把完整函数指针类型直接写出来 —— 「类型复杂」正是那一段的**目的**，
// 拆成 type 别名反而看不出契约长什么样。
#![allow(clippy::type_complexity)]

use std::fs;
use std::path::Path;

use audiolink_identity::{
    CERT_FILE_NAME, CERT_SUBJECT_ALT_NAME, IdentityError, KEY_FILE_NAME, NodeIdentity,
};
use audiolink_types::NodeId;
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
}

/// 回归：中文（非 ASCII）设备名必须能建出身份。
///
/// 曾经的实现把展示名塞进证书 SAN，而 SAN 是 `dNSName`（IA5String，仅 ASCII），于是
/// `load_or_create(dir, "我的手机")` 直接报 `Invalid IA5String` —— 本项目设备名天然是中文。
/// 现在 SAN 固定为常量 `audiolink`，展示名只留在内存里。
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

/// 用例 3：落盘 / 读取的错误路径（路径被文件占住、文件位置被目录占住、内容损坏）→ `Err` 且不 panic。
#[test]
fn 用例3_错误路径返回_err_且不_panic() {
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

    // (c) 证书 PEM 内容损坏 / 私钥 PEM 内容损坏。
    let garbage = tempfile::tempdir().unwrap();
    fs::write(
        garbage.path().join(CERT_FILE_NAME),
        b"-----BEGIN CERTIFICATE-----\nnot base64!!\n-----END CERTIFICATE-----\n",
    )
    .unwrap();
    fs::write(garbage.path().join(KEY_FILE_NAME), b"garbage, not pem").unwrap();
    assert!(NodeIdentity::load_or_create(garbage.path(), "x").is_err());

    // (d) 空节点名（PEM 完全合法）也必须是 Err，不能悄悄生成一个无名身份。
    let named = tempfile::tempdir().unwrap();
    let good = NodeIdentity::load_or_create(named.path(), "ok").unwrap();
    let key_pem = fs::read_to_string(named.path().join(KEY_FILE_NAME)).unwrap();
    assert!(
        NodeIdentity::from_pem(good.cert_pem(), &key_pem, "   ").is_err(),
        "空节点名被接受了"
    );
    // (e) 以上每条都走到了断言之后 —— 没有 panic 本身就是证据。
}
