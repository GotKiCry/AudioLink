//! M5 · 端到端：**从本地 HTTP 托管取回清单与安装包，再验签**。
//!
//! 由 tools/updater-local-e2e.ps1 驱动：它在 127.0.0.1 上起一个静态托管（把 latest.json 与安装包
//! 放进同一个目录），再设置下面两个环境变量调起本测试。未设置时**明确跳过**，
//! 所以常规的全量 cargo test 不会因此变红。
//!
//! 环境变量：
//! * AUDIOLINK_UPDATER_E2E_BASE —— 形如 http://127.0.0.1:8321
//! * AUDIOLINK_UPDATER_E2E_DIR  —— 托管目录（含 latest.json 与它 url 指向的安装包）
//!
//! ## 这条测试补的是哪一半
//! tools/check-update.mjs 已经把「清单里的签名 ↔ 本地那个安装包」验过了（离线自洽）。
//! 它没覆盖的两件事在这里：
//! 1. 客户端**真的发 HTTP 请求**去取（按 latest.json 里那个 releases/latest/download 形状的 URL 取回字节）；
//! 2. 取回的字节必须能被**配置公钥**验签通过，且**改 1 字节必须被拒绝**。
//!
//! 验签调用序列与 tests/updater_signature.rs 同源（复刻 tauri-plugin-updater 的 verify_signature：
//! base64(pubkey) → PublicKey::decode → base64(signature) → Signature::decode → verify(data, sig, true)）。
//! 这里刻意自带一份 helper、不跨文件共享：两个测试目标各自独立可跑，改一个不会牵连另一个。

// 测试里断言失败就该炸：显式放行 panic 系列 lint（工作区默认 deny）。
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use minisign_verify::{Error as MinisignError, PublicKey, Signature};

/// base64 → UTF-8 文本（对应插件的 base64_to_string）。
fn decode_b64_text(input: &str) -> Result<String, MinisignError> {
    let bytes = BASE64
        .decode(input)
        .map_err(|_| MinisignError::InvalidEncoding)?;
    String::from_utf8(bytes).map_err(|_| MinisignError::InvalidEncoding)
}

/// 复刻 tauri-plugin-updater 的 verify_signature()（allow_legacy = true）。
fn verify_update(
    payload: &[u8],
    signature_b64: &str,
    pubkey_b64: &str,
) -> Result<(), MinisignError> {
    let pubkey_text = decode_b64_text(pubkey_b64)?;
    let public_key = PublicKey::decode(&pubkey_text)?;

    let signature_text = decode_b64_text(signature_b64)?;
    let signature = Signature::decode(&signature_text)?;

    public_key.verify(payload, &signature, true)
}

/// 从 tauri.conf.json 取 updater 公钥 —— 与产品运行时读的是同一份配置。
fn configured_pubkey() -> String {
    let conf =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"))
            .expect("tauri.conf.json 应当可读");
    let json: serde_json::Value =
        serde_json::from_str(&conf).expect("tauri.conf.json 应是合法 JSON");
    json.get("plugins")
        .and_then(|plugins| plugins.get("updater"))
        .and_then(|updater| updater.get("pubkey"))
        .and_then(|value| value.as_str())
        .expect("tauri.conf.json 里必须配 plugins.updater.pubkey")
        .to_string()
}

/// 发一个最小 HTTP/1.0 GET 并把响应体取回来。
///
/// 为什么手写而不是用 reqwest：本测试不引新依赖（write scope 只允许动 tests/）。
/// HTTP/1.0 + Connection: close 的语义足够简单，服务端会带 Content-Length 并关闭连接。
fn http_get(base: &str, path: &str) -> Vec<u8> {
    let authority = base
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    let mut stream = TcpStream::connect(authority).expect("应当能连上本地托管服务");
    let request = format!("GET {path} HTTP/1.0\r\nHost: {authority}\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .expect("应当能把请求写出去");

    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("应当能读完响应体");

    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP 响应应当有头/体分隔的空行");
    let head = String::from_utf8_lossy(&response[..header_end]).to_string();
    let status_line = head.lines().next().unwrap_or_default().to_string();
    assert!(
        status_line.contains(" 200"),
        "GET {path} 应当返回 200，实际首行：{status_line}"
    );

    let content_length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length:"))
        .map(|value| {
            value
                .trim()
                .parse::<usize>()
                .expect("Content-Length 应当是数字")
        })
        .expect("响应必须带 Content-Length（客户端要靠它确认没被截断）");
    let body = response[header_end + 4..].to_vec();
    assert_eq!(
        body.len(),
        content_length,
        "GET {path} 的响应体长度应与 Content-Length 一致"
    );
    body
}

/// 失败的成因（与 updater_signature.rs 同一套判别）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rejection {
    SignatureMismatch,
}

/// 断言必须被拒绝，并核对到具体错误变体 —— 不接受笼统的 is_err()。
fn assert_rejected(result: Result<(), MinisignError>, expected: Rejection, scenario: &str) {
    match result {
        Ok(()) => unreachable!("{scenario}：改动后的输入竟然验签通过 —— 断言被击穿"),
        Err(actual) => {
            let matched = matches!(
                (expected, &actual),
                (
                    Rejection::SignatureMismatch,
                    MinisignError::InvalidSignature
                )
            );
            assert!(
                matched,
                "{scenario}：期望签名与内容不匹配，实际 {actual:?}（{actual}）"
            );
        }
    }
}

/// 端到端：清单走 HTTP 取、安装包走 HTTP 取，验签通过；包改 1 字节必须被拒绝。
#[test]
fn hosted_release_end_to_end_from_local_http() {
    let (Ok(base), Ok(dir)) = (
        std::env::var("AUDIOLINK_UPDATER_E2E_BASE"),
        std::env::var("AUDIOLINK_UPDATER_E2E_DIR"),
    ) else {
        eprintln!(
            "跳过：未设置 AUDIOLINK_UPDATER_E2E_BASE / AUDIOLINK_UPDATER_E2E_DIR（由 tools/updater-local-e2e.ps1 设置）"
        );
        return;
    };

    // 1) 客户端要读的清单：走 HTTP 取，落到内存里解析（不从磁盘直接读，避免「测的是本地文件」）。
    let manifest_bytes = http_get(&base, "/latest.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).expect("HTTP 取回的 latest.json 应是合法 JSON");
    let entry = &manifest["platforms"]["windows-x86_64"];
    let signature = entry["signature"]
        .as_str()
        .expect("清单的 platforms.windows-x86_64.signature 必须存在");
    let url = entry["url"]
        .as_str()
        .expect("清单的 platforms.windows-x86_64.url 必须存在");
    let file_name = url.rsplit('/').next().expect("清单的 url 应当带文件名");
    let request_path = format!("/releases/latest/download/{file_name}");

    // 2) 按清单里那个 URL 的形状把安装包取回来。
    let downloaded = http_get(&base, &request_path);
    let hosted = std::fs::read(Path::new(&dir).join(file_name)).expect("托管目录里应当有安装包");
    assert_eq!(
        downloaded.len(),
        hosted.len(),
        "HTTP 取回的字节数应与托管文件一致"
    );
    assert_eq!(downloaded, hosted, "HTTP 取回的字节应与托管文件逐字节一致");
    println!(
        "E2E-HOSTED: downloaded bytes={} from {request_path}",
        downloaded.len()
    );

    // 3) 正例：配置公钥 + 清单签名 + 取回的字节 → 必须验签通过。
    let pubkey = configured_pubkey();
    verify_update(&downloaded, signature, &pubkey)
        .expect("本地托管的发布包 + 清单签名 + 配置公钥必须验签通过");
    println!(
        "E2E-HOSTED: positive ok bytes={} version={}",
        downloaded.len(),
        manifest["version"].as_str().unwrap_or_default()
    );

    // 4) 负例：把取回的字节改 1 字节，必须被拒绝。
    let mut tampered = downloaded.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x80;
    assert_ne!(tampered, downloaded, "篡改必须真的改掉字节");
    assert_rejected(
        verify_update(&tampered, signature, &pubkey),
        Rejection::SignatureMismatch,
        "从本地 HTTP 取回的包被改 1 字节",
    );
    println!("E2E-HOSTED: negative rejected InvalidSignature");
}
