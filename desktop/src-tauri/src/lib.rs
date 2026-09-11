//! AudioLink 桌面端外壳（Tauri 2）
//!
//! 职责边界（见 `docs/02-architecture.md`）：
//! * **这里是外壳**：托盘、单实例、开机自启、窗口状态、自动更新、日志、把内核能力暴露给前端；
//! * **不做音频与网络**：那是 `audiolink-engine` 的事，本层只做编排与 UI 适配。
//!
//! 铁律：UI 通过 `command` / `event` 通信，遥测类事件按 500 ms 批量推送（避免高频 IPC）。

// 启动期初始化失败无法恢复，允许直接终止（非实时路径，与内核的 no-panic 纪律不冲突）。
#![allow(clippy::expect_used)]

use tauri::Manager;

/// 版本号单一来源：Cargo workspace（CI 校验三处一致，见 tools/check-version.ps1）
#[tauri::command]
fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// 前端查询本机身份指纹（配对/白名单 UI 使用）。
/// TODO(M1)：接入 `audiolink-identity`，返回 SHA-256 指纹短码（16 hex）。
#[tauri::command]
fn local_fingerprint() -> String {
    "0000000000000000".to_string()
}

pub fn run() {
    tauri::Builder::default()
        // 单实例：第二次启动时唤出已有窗口（旧版靠 Mutex + 命名管道手写）
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_autostart::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_log::Builder::default().build())
        .invoke_handler(tauri::generate_handler![version, local_fingerprint])
        .setup(|app| {
            // TODO(M5)：TrayIconBuilder 托盘菜单（连接/断开/静音/显示主窗口/退出）
            // TODO(M5)：启动后自动连接上次设备（FR-31，可关）
            // TODO(M1)：启动 audiolink-engine，注册遥测事件转发到前端
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running AudioLink desktop");
}
