//! M5 自动更新链路：「被动过的更新包必须验签失败」的自动化证据。
//!
//! 自包含：夹具（一对测试密钥的公钥 + 103 字节假安装包 + 预先算好的签名）全部内联在本文件里；
//! 不读仓库外密钥（~/.tauri/* 与本测试无关）、不联网；主用例不依赖构建产物。
//!
//! ## 这条测试复刻的是谁
//! 「pnpm tauri signer」只有 sign / generate，**没有 verify** —— 验签发生在 Rust 侧。
//! tauri-plugin-updater 2.11.0 的 src/updater.rs::verify_signature()（源码 1524-1534 行）逐行如下：
//!
//!     pub_key_decoded = base64_to_string(pub_key)            // tauri.conf.json 的 plugins.updater.pubkey
//!     public_key      = PublicKey::decode(&pub_key_decoded)
//!     sig_decoded     = base64_to_string(release_signature)   // latest.json 的 platforms.*.signature
//!     signature       = Signature::decode(&sig_decoded)
//!     public_key.verify(data, &signature, true)               // allow_legacy = true
//!
//! 所以 pubkey 与 signature 都是「整份 minisign 文本的 base64」，真正的密码学校验由 minisign-verify 完成。
//! 本测试用与该插件同源的 minisign-verify = "0.2"（插件自己的 Cargo.toml 写的也是 minisign-verify = "0.2"）
//! 逐行复刻上面这条调用序列 —— 验的是产品链路本身，不是另写一套验签。
//!
//! ## 断言清单（每条都要能看出「为什么」失败）
//! 1. 原包 + 原签名 + 原公钥 → **通过**：先证明夹具与链路真的在验签，而不是恒定报错；
//! 2. **改包**（1 字节）+ 原签名 + 原公钥 → InvalidSignature（Ed25519 不匹配）。key id 未动，
//!    因此失败原因只能是「内容对不上签名」，排除「换密钥」的解释；
//! 3. **改签名体**（签名最后 1 字节）+ 原包 + 原公钥 → InvalidSignature，key id 仍不变；
//! 4. **改 trusted comment**（时间戳加一位）+ 原包 + 原公钥 → InvalidSignature：
//!    minisign 的 global signature 覆盖 trusted comment，时间戳造假同样过不去；
//! 5. 原包 + 原签名 + **另一把密钥的公钥** → UnexpectedKeyId：明确是「签名不是这把公钥签的」，
//!    而不是「签名坏了」。
//!
//! ## 不验
//! HTTP 下载、latest.json 拉取、安装器执行、前端弹窗 —— 那些要真实 Release 与 AppHandle。
//! 另有两个依赖本地构建产物的用例（真实安装包 / latest.json）：产物不存在时**明确跳过并打印**，
//! 不假装通过。

// 测试里断言失败就该炸：显式放行 panic 系列 lint（工作区默认 deny）。
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use minisign_verify::{Error as MinisignError, PublicKey, Signature};

/// 测试密钥对 #1 的公钥（tauri signer generate 产出的 .pub 原文，正是 tauri 的 pubkey 配置格式）。
/// 生成命令：pnpm tauri signer generate -w <临时目录>/test1.key -p fixture-pw
/// **私钥不落库**：本测试只需要「公钥 + 预生成签名」，不需要私钥。
const FIXTURE_PUBKEY_B64: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEJFREZGMTAxNDZFNzgzNTUKUldSVmcrZEdBZkhmdmltM3dFV3RMQkZMMEFDbGUwdUxveGhnZm40SW9GdFlOTGFwbEFDSzJqakgK";

/// 假安装包对应的预生成签名，即 .sig 文件的内容
/// （pnpm tauri signer sign -f <临时目录>/test1.key -p fixture-pw <假安装包> 产出）。
const FIXTURE_SIGNATURE_B64: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSVmcrZEdBZkhmdnBaSE4zTllnKzlsN0pHWkpEek4vUjdrRU9YMWFnaUxkQkFKN3ZDVy9mQVJPOW5pR0hpTVJCU2NuQ09UbElyMjNFNFdzMUE2Rk1pdTZUZUFJMUZ6QUFJPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzg5NjEyNTkwCWZpbGU6QXVkaW9MaW5rXzAuMS4wX3g2NC1zZXR1cC5leGUKMzFZNDZldjhTcjlMaTNyeGh2WFFIVldua3MzYUVPL2NSdjNucFhaT0VucllBVGkvMGR5WDcrckxvNm94RHRMM0JGclprdllFcnkyeUNMN09MbjVOQ1E9PQo=";

/// 测试密钥对 #2 的公钥：只用于「公钥不对」那一条，证明 UnexpectedKeyId 来自密钥而不是内容。
const FIXTURE_OTHER_PUBKEY_B64: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE1NjI3RUQ1NDRDOTMxNkIKUldSck1jbEUxWDVpRlEyQnBieG9UcTlITEF1a1NtK0c2dlN0RUh0RVB4U28rSVJZUmdUamJaNzIK";

/// 假安装包的明文前缀（其后是 0..64 的递增字节，合计 103 字节）。
const FIXTURE_PAYLOAD_PREFIX: &[u8] = b"AudioLink fake installer payload v0.1.0";

/// 与签名时逐字节一致的那 103 字节。
fn fixture_payload() -> Vec<u8> {
    let mut payload = FIXTURE_PAYLOAD_PREFIX.to_vec();
    payload.extend(0u8..64);
    payload
}

/// base64_to_string() 的等价物：base64 → UTF-8 文本；解不出文本就当作「编码非法」。
fn decode_b64_text(input: &str) -> Result<String, MinisignError> {
    let bytes = BASE64
        .decode(input)
        .map_err(|_| MinisignError::InvalidEncoding)?;
    String::from_utf8(bytes).map_err(|_| MinisignError::InvalidEncoding)
}

/// 逐行复刻 tauri-plugin-updater 的 verify_signature()：两步 base64 → 两次 decode → verify。
fn verify_update(
    payload: &[u8],
    signature_b64: &str,
    pubkey_b64: &str,
) -> Result<(), MinisignError> {
    let pubkey_text = decode_b64_text(pubkey_b64)?;
    let public_key = PublicKey::decode(&pubkey_text)?;

    let signature_text = decode_b64_text(signature_b64)?;
    let signature = Signature::decode(&signature_text)?;

    // 与插件一致：allow_legacy = true（老版本 minisign 的未预哈希签名也要能验）。
    public_key.verify(payload, &signature, true)
}

/// 失败的成因：把 minisign 的错误变体映射成「人话」，断言里必须能读出是哪一类不匹配。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rejection {
    /// 签名与内容（或其 trusted comment）对不上。
    SignatureMismatch,
    /// 签名根本不是这把公钥签的。
    KeyMismatch,
}

impl Rejection {
    fn explain(self) -> &'static str {
        match self {
            Self::SignatureMismatch => "签名与内容不匹配（Ed25519 校验失败）",
            Self::KeyMismatch => "签名不是这把公钥签的（minisign key id 不匹配）",
        }
    }
}

/// 断言「必须被拒绝」，并把实际错误核对到具体变体 —— 不接受笼统的 is_err()。
fn assert_rejected(result: Result<(), MinisignError>, expected: Rejection, scenario: &str) {
    match result {
        Ok(()) => unreachable!("{scenario}：篡改后的输入竟然验签通过 —— 安全断言被击穿"),
        Err(actual) => {
            let matched = matches!(
                (expected, &actual),
                (
                    Rejection::SignatureMismatch,
                    MinisignError::InvalidSignature
                ) | (Rejection::KeyMismatch, MinisignError::UnexpectedKeyId)
            );
            assert!(
                matched,
                "{scenario}：期望 {}，实际 {actual:?}（{actual}）",
                expected.explain()
            );
        }
    }
}

/// 翻掉第 index 个字节的最高位 —— 模拟被替换/损坏的更新包。
fn flip_byte(data: &[u8], index: usize) -> Vec<u8> {
    let mut tampered = data.to_vec();
    tampered[index] ^= 0x80;
    tampered
}

/// 篡改签名体：只动 74 字节里签名本体的最后 1 字节，key id（前 10 字节）保持原样。
fn tamper_signature_body(signature_b64: &str) -> String {
    let text = decode_b64_text(signature_b64).expect("夹具签名必须是 base64 文本");
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines.len() >= 4,
        "minisign 签名文本应有 4 行，实际 {} 行",
        lines.len()
    );

    let mut body = BASE64.decode(lines[1]).expect("签名字段应是 base64");
    assert_eq!(body.len(), 74, "minisign 签名字段应为 2 + 8 + 64 字节");

    let last = body.len() - 1;
    body[last] ^= 0x01;

    let tampered_text = format!(
        "{}\n{}\n{}\n{}\n",
        lines[0],
        BASE64.encode(&body),
        lines[2],
        lines[3]
    );
    BASE64.encode(tampered_text)
}

/// 篡改 trusted comment：时间戳加一位（签名体一个字节都不动）。
fn tamper_trusted_comment(signature_b64: &str) -> String {
    let text = decode_b64_text(signature_b64).expect("夹具签名必须是 base64 文本");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    assert!(
        lines.len() >= 4,
        "minisign 签名文本应有 4 行，实际 {} 行",
        lines.len()
    );

    let tampered = lines[2].replace("timestamp:", "timestamp:1");
    assert_ne!(tampered, lines[2], "trusted comment 里应当有 timestamp");

    lines[2] = tampered;
    BASE64.encode(format!(
        "{}\n{}\n{}\n{}\n",
        lines[0], lines[1], lines[2], lines[3]
    ))
}

// ---------------------------------------------------------------- 四条核心断言

/// ① 原包 + 原签名 + 原公钥 → 通过。这条是其余三条的对照组。
#[test]
fn untampered_update_passes_signature_check() {
    let payload = fixture_payload();
    assert_eq!(payload.len(), 103, "夹具假安装包必须是 103 字节");

    verify_update(&payload, FIXTURE_SIGNATURE_B64, FIXTURE_PUBKEY_B64)
        .expect("原包 + 原签名 + 原公钥必须验签通过（不通过说明夹具或调用链本身有问题）");
}

/// ② 改包（1 字节）+ 原签名 → 必须失败，且原因必须是「签名与内容不匹配」。
#[test]
fn tampered_payload_is_rejected() {
    let payload = fixture_payload();
    // 只动最后一个字节：包长不变、key id 不变，唯一变化的只有内容。
    let tampered = flip_byte(&payload, payload.len() - 1);
    assert_ne!(
        tampered, payload,
        "篡改必须真的改掉字节，否则这条测试是假的"
    );
    assert_eq!(tampered.len(), payload.len(), "篡改不应改变长度");

    assert_rejected(
        verify_update(&tampered, FIXTURE_SIGNATURE_B64, FIXTURE_PUBKEY_B64),
        Rejection::SignatureMismatch,
        "篡改包 + 原签名",
    );

    // 同一份签名对原包仍然有效 —— 证明失败确实来自「包被改」，而不是签名本身坏了。
    verify_update(&payload, FIXTURE_SIGNATURE_B64, FIXTURE_PUBKEY_B64)
        .expect("原包 + 原签名在同一个测试里仍应通过");
}

/// ③ 原包 + 改签名体 → 必须失败，原因必须是「签名与内容不匹配」。
#[test]
fn tampered_signature_body_is_rejected() {
    let tampered = tamper_signature_body(FIXTURE_SIGNATURE_B64);
    assert_ne!(
        tampered, FIXTURE_SIGNATURE_B64,
        "篡改后的签名必须与原签名不同"
    );

    assert_rejected(
        verify_update(&fixture_payload(), &tampered, FIXTURE_PUBKEY_B64),
        Rejection::SignatureMismatch,
        "原包 + 改签名体",
    );
}

/// ③-补 原包 + 改 trusted comment（时间戳）→ 必须同样失败。
#[test]
fn tampered_trusted_comment_is_rejected() {
    let tampered = tamper_trusted_comment(FIXTURE_SIGNATURE_B64);
    assert_ne!(
        tampered, FIXTURE_SIGNATURE_B64,
        "篡改后的签名必须与原签名不同"
    );

    assert_rejected(
        verify_update(&fixture_payload(), &tampered, FIXTURE_PUBKEY_B64),
        Rejection::SignatureMismatch,
        "原包 + 改 trusted comment",
    );
}

/// ④ 原包 + 原签名 + 另一把密钥的公钥 → 必须失败，原因必须是「公钥不匹配」。
#[test]
fn wrong_public_key_is_rejected() {
    assert_ne!(
        FIXTURE_OTHER_PUBKEY_B64, FIXTURE_PUBKEY_B64,
        "两把公钥必须不同，否则这条测试是假的"
    );

    assert_rejected(
        verify_update(
            &fixture_payload(),
            FIXTURE_SIGNATURE_B64,
            FIXTURE_OTHER_PUBKEY_B64,
        ),
        Rejection::KeyMismatch,
        "原包 + 原签名 + 错误公钥",
    );
}

// ---------------------------------------------------------------- 仓库内配置 / 真实产物（可跳过）

/// 仓库根目录（本文件在 desktop/src-tauri/tests/ 下）。
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .to_path_buf()
}

/// 从 tauri.conf.json 取 updater 公钥 —— 与产品运行时读的是同一份配置。
fn configured_pubkey() -> Option<String> {
    let conf =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"))
            .ok()?;
    let json: serde_json::Value = serde_json::from_str(&conf).ok()?;
    json.get("plugins")?
        .get("updater")?
        .get("pubkey")?
        .as_str()
        .map(str::to_string)
}

/// 目录下第一个以 suffix 结尾的文件（按名字排序，结果稳定）。
fn first_with_suffix(dir: &Path, suffix: &str) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(suffix))
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

/// 配置里的公钥必须是一把能被 minisign 解析的公钥 —— 否则更新永远装不上。
#[test]
fn configured_pubkey_parses_as_minisign_key() {
    let pubkey_b64 = configured_pubkey().expect("tauri.conf.json 里必须配 plugins.updater.pubkey");
    let text = decode_b64_text(&pubkey_b64).expect("配置里的 pubkey 必须是 base64 文本");
    let key = PublicKey::decode(&text).expect("配置里的 pubkey 必须能被 minisign 解析");
    assert!(
        key.untrusted_comment()
            .is_some_and(|comment| comment.contains("minisign public key")),
        "配置里的 pubkey 应当是一把 minisign 公钥，实际注释为 {:?}",
        key.untrusted_comment()
    );
}

/// 真实发布产物（若本地已构建）必须验签通过；被篡改的真实包则必须失败。
///
/// 产物不存在时跳过：这条依赖 target/ 下的构建输出，未构建的机器上不该红。
#[test]
fn real_release_artifact_verifies_when_present() {
    let nsis_dir = repo_root().join("target/x86_64-pc-windows-msvc/release/bundle/nsis");
    let Some(installer) = first_with_suffix(&nsis_dir, "-setup.exe") else {
        eprintln!(
            "跳过：没有本地构建产物 {}（先跑 pnpm tauri:build 才能验真实安装包）",
            nsis_dir.display()
        );
        return;
    };
    let sig_path = PathBuf::from(format!("{}.sig", installer.display()));
    let Ok(signature) = std::fs::read_to_string(&sig_path) else {
        eprintln!(
            "跳过：没有签名文件 {}（打包时缺 TAURI_SIGNING_PRIVATE_KEY）",
            sig_path.display()
        );
        return;
    };
    let pubkey_b64 = configured_pubkey().expect("tauri.conf.json 里必须配 plugins.updater.pubkey");

    let bytes = std::fs::read(&installer).expect("真实安装包应当可读");
    let signature_b64 = signature.trim();

    verify_update(&bytes, signature_b64, &pubkey_b64).expect(
        "真实安装包 + 真实 .sig + tauri.conf.json 公钥必须验签通过（这是「配了签名但更新装不上」的护栏）",
    );

    // 真实包被改 1 字节 → 必须失败。
    let tampered = flip_byte(&bytes, bytes.len() - 1);
    assert_rejected(
        verify_update(&tampered, signature_b64, &pubkey_b64),
        Rejection::SignatureMismatch,
        "真实安装包被篡改",
    );
}

/// tools/tauri-latest-json.ps1 把 .sig 文件内容写进 latest.json 的 signature 字段；
/// updater 会对该字段再做一次 base64 解码，所以两者必须逐字符一致（trim 后）。
#[test]
fn latest_json_signature_matches_sig_file_when_present() {
    let nsis_dir = repo_root().join("target/x86_64-pc-windows-msvc/release/bundle/nsis");
    let Some(installer) = first_with_suffix(&nsis_dir, "-setup.exe") else {
        eprintln!("跳过：没有本地构建产物，无法比对 latest.json 与 .sig");
        return;
    };
    let sig_path = PathBuf::from(format!("{}.sig", installer.display()));
    let Ok(sig_text) = std::fs::read_to_string(&sig_path) else {
        eprintln!("跳过：没有签名文件 {}", sig_path.display());
        return;
    };

    let manifest_path = repo_root().join("target/evidence/release/latest.json");
    let Ok(manifest) = std::fs::read_to_string(&manifest_path) else {
        eprintln!(
            "跳过：没有生成清单 {}（先跑 tools/tauri-latest-json.ps1）",
            manifest_path.display()
        );
        return;
    };
    let json: serde_json::Value =
        serde_json::from_str(&manifest).expect("latest.json 必须是合法 JSON");
    let signature = json
        .get("platforms")
        .and_then(|platforms| platforms.get("windows-x86_64"))
        .and_then(|platform| platform.get("signature"))
        .and_then(|value| value.as_str())
        .expect("latest.json 的 platforms.windows-x86_64.signature 必须存在");

    assert_eq!(
        signature.trim(),
        sig_text.trim(),
        "latest.json 的 signature 必须与 .sig 文件内容一致；否则 updater 会解不出签名而拒绝更新"
    );

    // 该字段要能被 base64 解回 minisign 文本、并解析成签名，才可能被 updater 接受。
    let signature_text =
        decode_b64_text(signature.trim()).expect("latest.json 的 signature 必须是 base64 文本");
    Signature::decode(&signature_text)
        .expect("latest.json 的 signature 必须能解码成 minisign 签名");
}
