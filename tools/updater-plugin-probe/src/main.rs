//! M5 · 用**真插件**跑一次「检查 → 下载 → 验签」：本地托管的静态站点当更新源。
//!
//! ## 为什么需要它
//! tools/updater-local-e2e.ps1 的 S1–S6 都是本机的，但它们**没有驱动真插件** ——
//! 验签是拿 tauri-plugin-updater 的调用序列复刻出来的。真正的插件入口
//! （UpdaterExt::updater() / Update::download()）需要 AppHandle（实现自 Manager），
//! 而集成测试里构造它要开 tauri 的 test feature。
//!
//! 这个 crate 把那个缺口补上：它不是 AudioLink workspace 的成员（自带 [workspace]），
//! 所以「只为测试存在」的 tauri/test feature 不会污染产品依赖图。
//!
//! ## 它做的事
//! 1. 用 tauri 的 mock runtime 建一个 AppHandle；
//! 2. 往 app config 里注入 updater 插件配置：endpoints 指向本地 127.0.0.1 的托管、
//!    pubkey 用**产品配置里那把**、并打开 dangerousInsecureTransportProtocol
//!    （插件对任何非 https 端点都会返回 InsecureTransportProtocol，localhost 没有豁免 ——
//!     所以本地 http 托管必须显式开这个开关；产品配置里绝不能有它）；
//! 3. 真调 app.updater().check() → 真读清单；再真调 update.download() → 真下载 + 真验签；
//! 4. 负例：--expect-error 时要求 download() **必须失败**（把托管包改 1 字节就能造出来）。
//!
//! ## 用法
//!     updater-plugin-probe --base http://127.0.0.1:8321 [--endpoint /latest.json]
//!                          [--pubkey <base64>] [--expect-error] [--expect-insecure-rejected]
//!                          [--require-reject]
//!
//! 退出码：0 = 断言全过；非 0 = 有断言失败（panic）。

use std::path::PathBuf;

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri_plugin_updater::UpdaterExt;

/// 产品配置里的 updater 公钥（与运行时读的是同一份 tauri.conf.json）。
fn configured_pubkey() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("desktop/src-tauri/tauri.conf.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读不到 {}：{error}", path.display()));
    let json: serde_json::Value =
        serde_json::from_str(&text).expect("tauri.conf.json 应是合法 JSON");
    json.get("plugins")
        .and_then(|plugins| plugins.get("updater"))
        .and_then(|updater| updater.get("pubkey"))
        .and_then(|value| value.as_str())
        .expect("tauri.conf.json 里必须配 plugins.updater.pubkey")
        .to_string()
}

struct Args {
    base: String,
    endpoint: String,
    pubkey: String,
    expect_error: bool,
    expect_insecure_rejected: bool,
    /// 与 expect_insecure_rejected 合用：要求端点**真的被拒**（release 构建下的行为）。
    require_reject: bool,
}

fn parse_args() -> Args {
    let mut base: Option<String> = None;
    let mut endpoint = String::from("/latest.json");
    let mut pubkey: Option<String> = None;
    let mut expect_error = false;
    let mut expect_insecure_rejected = false;
    let mut require_reject = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--base" => base = args.next(),
            "--endpoint" => endpoint = args.next().expect("--endpoint 需要一个值"),
            "--pubkey" => pubkey = args.next(),
            "--expect-error" => expect_error = true,
            "--expect-insecure-rejected" => expect_insecure_rejected = true,
            "--require-reject" => require_reject = true,
            other => panic!("不认识的参数：{other}"),
        }
    }

    Args {
        base: base.expect("必须给 --base（例如 http://127.0.0.1:8321）"),
        endpoint,
        pubkey: pubkey.unwrap_or_else(configured_pubkey),
        expect_error,
        expect_insecure_rejected,
        require_reject,
    }
}

#[tokio::main]
async fn main() {
    let args = parse_args();
    let url = format!("{}{}", args.base.trim_end_matches('/'), args.endpoint);

    // 注入插件配置：endpoints / pubkey / （本地 http 必需的）危险开关。
    // 注意 config_mut() 是 tauri 的公开 API，改的是**这个进程内**的 mock config，
    // 不碰仓库里的 tauri.conf.json。
    let mut context = mock_context(noop_assets());
    let mut plugin_config = serde_json::json!({
        "endpoints": [url],
        "pubkey": args.pubkey,
    });
    if !args.expect_insecure_rejected {
        plugin_config["dangerousInsecureTransportProtocol"] = serde_json::Value::Bool(true);
    }
    context
        .config_mut()
        .plugins
        .0
        .insert("updater".to_string(), plugin_config);

    let built = mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context);

    if args.expect_insecure_rejected {
        // 不打开危险开关时，http:// 端点会被怎么处理？**取决于构建 profile** ——
        // tauri-plugin-updater 的 config.rs 里那个 return Err(InsecureTransportProtocol)
        // 在 #[cfg(not(debug_assertions))] 里：
        //   * debug（dev）构建：只往 stderr 打一条 [WARNING]，仍然接受；
        //   * release 构建：直接拒绝。
        // 所以这里按 profile 分别断言：默认（debug）要求「接受但警告」，
        // 加 --require-reject（release 下跑）要求「真的被拒」。
        match built {
            Ok(_) if args.require_reject => {
                panic!("要求被拒，但 http 端点被接受了（release 构建下应当返回 InsecureTransportProtocol）")
            }
            Ok(_) => {
                println!("PROBE: insecure endpoint allowed but warned (debug build)");
            }
            Err(error) => {
                // 实际错误形状（实测）：failed to initialize plugin `updater`:
                // Error deserializing 'plugins.updater' within your Tauri configuration:
                // The configured updater endpoint must use a secure protocol like `https`.
                let text = error.to_string();
                assert!(
                    text.contains("secure protocol") || text.contains("InsecureTransportProtocol"),
                    "期望因不安全传输协议被拒，实际错误：{text}"
                );
                println!("PROBE: insecure endpoint rejected as expected: {text}");
            }
        }
        return;
    }

    let app = built.expect("mock app 应当能构造（插件配置注入失败？）");
    let updater = app.updater().expect("应当能构造 Updater");
    println!("PROBE: updater built, endpoint={url}");

    let update = updater
        .check()
        .await
        .expect("check() 应当成功（本地托管可达且清单合法）")
        .expect("清单版本应当比当前版本新（否则拿不到 Update）");
    println!("PROBE: check ok remote_version={}", update.version);

    let downloaded = update.download(|_chunk, _total| {}, || {}).await;
    match downloaded {
        Ok(bytes) => {
            assert!(
                !args.expect_error,
                "期望 download() 因验签失败被拒，实际却成功了（{} 字节）",
                bytes.len()
            );
            assert!(!bytes.is_empty(), "下载结果不应为空");
            println!("PROBE: download ok bytes={}", bytes.len());
            println!("PROBE: plugin end-to-end ok");
        }
        Err(error) => {
            assert!(
                args.expect_error,
                "期望 download() 成功，实际失败：{error:?}（{error}）"
            );
            let debug = format!("{error:?}");
            assert!(
                debug.contains("Minisign"),
                "期望失败原因是验签（Minisign），实际：{debug}"
            );
            // 只打一条场景无关的标记行：调用方再按错误原文核对「为什么」失败
            // （篡改包 → signature verification failed；错公钥 → different key）。
            println!("PROBE: download rejected as expected: {debug}");
        }
    }
}
