//! AudioLink 桌面端外壳（Tauri 2）
//!
//! 职责边界（见 `docs/02-architecture.md`）：
//! * **这里是外壳**：把内核能力翻译成前端可用的 command / event、托盘、单实例、开机自启、
//!   窗口状态、自动更新、日志；
//! * **不做音频与网络**：那是 `audiolink-engine` 的事，本层只做编排与 UI 适配。
//!
//! 铁律：UI 只通过 `command` / `event` 通信；遥测类事件按 500 ms 批量推送
//! （避免高频 IPC，架构 §4 + 契约 §6）。
//!
//! 模块划分（重要）：
//! * [`view`]  —— 前端视图形状（契约 §6 冻结，含序列化形状的钉死测试）；
//! * [`error`] —— 前端可读的失败原因 `{code, message, context}`；
//! * [`engine_bridge`] —— **外壳 ⇄ `audiolink-engine` 的唯一接缝**：
//!   引擎启动、事件翻译、状态映射、错误人话化全在那里；本层不认识引擎类型。
//!
//! 本文件只做三件事：声明 command、注册 handler、装配插件。**不要在这里写业务逻辑。**

// 启动期初始化失败无法恢复，允许直接终止（非实时路径，与内核的 no-panic 纪律不冲突）。
#![allow(clippy::expect_used)]

mod capture;
mod engine_bridge;
mod error;
mod view;

use tauri::{Manager, State};

use engine_bridge::EngineBridge;
use error::CommandError;
use view::{
    AlignmentView, CaptureDeviceView, GroupView, LocalStatus, PeerView, StartSendResult,
    SubmitPinResult, TelemetryRow, TelemetryView,
};

#[tauri::command]
async fn list_capture_devices() -> Result<Vec<CaptureDeviceView>, CommandError> {
    capture::list_devices().await
}

#[tauri::command]
async fn active_capture_device(
    bridge: State<'_, EngineBridge>,
) -> Result<Option<CaptureDeviceView>, CommandError> {
    bridge.active_capture_device().await
}

/// 版本号单一来源：Cargo workspace（CI 校验三处一致，见 tools/check-version.ps1）
#[tauri::command]
fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// 本机身份：指纹短码 / 名字 / 端点 / 平台（契约 §6 `local_status`）。
///
/// 为什么和其它查询一样返回 `Result`：真实引擎侧这些查询**会失败**
/// （如 `Engine::local_addr()` 返回 `Result`），前端因此统一走 try/catch 一条路径。
#[tauri::command]
async fn local_status(bridge: State<'_, EngineBridge>) -> Result<LocalStatus, CommandError> {
    bridge.local_status().await
}

/// 已连接/已登记对端列表（契约 §6 `list_peers`）。
#[tauri::command]
async fn list_peers(bridge: State<'_, EngineBridge>) -> Result<Vec<PeerView>, CommandError> {
    bridge.list_peers().await
}

/// 手工 IP 连接（契约 §6 `connect`；需求 FR-17：发现被 AP 隔离时的兜底入口）。
///
/// 返回的是"已登记 + 握手中"的对端卡片；配对/失败由事件异步推给 UI。
#[tauri::command]
async fn connect(bridge: State<'_, EngineBridge>, addr: String) -> Result<PeerView, CommandError> {
    bridge.connect(&addr).await
}

/// 开始推流（契约 §6 `start_send`）。
///
/// **注意**：契约把入参定为 `{ id_short }`（snake_case），与其它命令的 camelCase 不同 ——
/// Tauri 2 的 command 参数默认按 camelCase 解析，所以这里必须显式 `rename_all = "snake_case"`。
#[tauri::command(rename_all = "snake_case")]
async fn start_send(
    bridge: State<'_, EngineBridge>,
    id_short: String,
    capture_device_id: Option<String>,
) -> Result<StartSendResult, CommandError> {
    bridge.start_send(&id_short, capture_device_id).await
}

/// 停止推流（契约 §6 `stop_send`，无入参，返回 `null`）。幂等。
#[tauri::command]
async fn stop_send(bridge: State<'_, EngineBridge>) -> Result<(), CommandError> {
    bridge.stop_send().await
}

/// 提交 6 位配对码（`submit_pin`）。
///
/// **入参比契约 §6 多了一个 `id_short`**：引擎的 `submit_pin(peer, pin)` 必须知道是哪条会话
/// （可能同时有多个对端要找配对，而 `pin` 本身没有任何归属信息）。契约文档那一行需要同步更新 ——
/// 已在交接里点名。这个命令会**等**到引擎给出配对结论（`PairCompleted`）才返回，不只是"已提交"。
#[tauri::command(rename_all = "snake_case")]
async fn submit_pin(
    bridge: State<'_, EngineBridge>,
    id_short: String,
    pin: String,
) -> Result<SubmitPinResult, CommandError> {
    bridge.submit_pin(&id_short, &pin).await
}

/// 遥测快照（契约 §6 `telemetry`）；持续刷新请订阅 `audiolink://telemetry`。
#[tauri::command]
async fn telemetry(bridge: State<'_, EngineBridge>) -> Result<TelemetryView, CommandError> {
    bridge.telemetry().await
}

/// 导出遥测历史为 CSV（M2 的「日志导出」；契约 §6 把它列在「不做」里，本轮解除）。
///
/// 入参是前端累积的采样点（形状 = `TelemetryView` + `atUnixMs`），返回落盘路径。
#[tauri::command(rename_all = "snake_case")]
fn export_telemetry(
    bridge: State<'_, EngineBridge>,
    rows: Vec<TelemetryRow>,
) -> Result<String, CommandError> {
    bridge.export_telemetry(&rows)
}

/// 同步组列表（M3 交付物 4）。
#[tauri::command]
async fn list_groups(bridge: State<'_, EngineBridge>) -> Result<Vec<GroupView>, CommandError> {
    bridge.list_groups().await
}

/// 建组：把点名的对端（指纹短码）组成临时同步组，返回组 ID。
#[tauri::command(rename_all = "snake_case")]
async fn create_group(
    bridge: State<'_, EngineBridge>,
    id_shorts: Vec<String>,
    lead_ms: u32,
) -> Result<u32, CommandError> {
    bridge.create_group(&id_shorts, lead_ms).await
}

/// 成员动态加入。
#[tauri::command(rename_all = "snake_case")]
async fn join_group(
    bridge: State<'_, EngineBridge>,
    id_short: String,
    group_id: u32,
) -> Result<(), CommandError> {
    bridge.join_group(&id_short, group_id).await
}

/// 成员退出。
#[tauri::command(rename_all = "snake_case")]
async fn leave_group(
    bridge: State<'_, EngineBridge>,
    id_short: String,
    group_id: u32,
) -> Result<(), CommandError> {
    bridge.leave_group(&id_short, group_id).await
}

/// §4.1 调对端音量（0.0–2.0；ramp_ms 是渐变时长）。
#[tauri::command(rename_all = "snake_case")]
async fn set_peer_gain(
    bridge: State<'_, EngineBridge>,
    id_short: String,
    gain: f32,
    ramp_ms: u32,
) -> Result<(), CommandError> {
    bridge.set_peer_gain(&id_short, gain, ramp_ms).await
}

/// M4：多源对齐快照（各路样本编号 + 当前跨度）。
#[tauri::command]
async fn alignment(bridge: State<'_, EngineBridge>) -> Result<AlignmentView, CommandError> {
    bridge.alignment().await
}

/// M4：广播共同时间基准（接收端 → 各发送端），返回发出的会话数。
#[tauri::command(rename_all = "snake_case")]
async fn broadcast_epoch(
    bridge: State<'_, EngineBridge>,
    lead_ms: u32,
) -> Result<u32, CommandError> {
    bridge.broadcast_epoch(lead_ms).await
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
        .invoke_handler(tauri::generate_handler![
            version,
            local_status,
            list_peers,
            list_capture_devices,
            active_capture_device,
            connect,
            start_send,
            stop_send,
            submit_pin,
            telemetry,
            export_telemetry,
            list_groups,
            create_group,
            join_group,
            leave_group,
            set_peer_gain,
            alignment,
            broadcast_epoch
        ])
        .setup(|app| {
            // 桥接层必须在窗口加载**之前**就位：前端一挂载就会 invoke，拿不到 State 会直接报错。
            // 引擎真正的启动是异步的（`Engine::start`），命令层会等到它就绪（见 engine_bridge.rs）。
            let _managed = app.manage(EngineBridge::new(app.handle().clone()));

            // TODO(M5)：TrayIconBuilder 托盘菜单（连接/断开/静音/显示主窗口/退出）
            // TODO(M5)：启动后自动连接上次设备（FR-31，可关）
            // TODO(M5)：退出时调 `Engine::shutdown()` 优雅关闭会话并给对方发 BYE
            //           （需要换成 Builder::build + RunEvent::ExitRequested，属托盘/退出行为那一批）
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running AudioLink desktop");
}
