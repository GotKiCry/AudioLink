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
mod settings;
mod update;
mod view;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, State};

use engine_bridge::EngineBridge;
use error::CommandError;
use update::UpdateCheckView;
use view::{
    AlignmentView, CaptureDeviceView, GroupView, LocalStatus, NoticesView, PeerView,
    StartSendResult, SubmitPinResult, TelemetryRow, TelemetryView,
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

/// M5：自动重连的当前设置（开关 + 上次设备）。
#[tauri::command]
async fn auto_connect_state(
    bridge: State<'_, EngineBridge>,
) -> Result<settings::AutoConnectPolicy, CommandError> {
    bridge.auto_connect_state().await
}

/// M5：开关「启动时自动连接上次设备」。
#[tauri::command(rename_all = "snake_case")]
async fn set_auto_connect(
    bridge: State<'_, EngineBridge>,
    enabled: bool,
) -> Result<(), CommandError> {
    bridge.set_auto_connect(enabled).await
}

/// M5：界面语言偏好（null = 还没选过，前端跟随系统语言）。
#[tauri::command]
async fn locale(bridge: State<'_, EngineBridge>) -> Result<Option<String>, CommandError> {
    bridge.locale().await
}

/// M5：保存界面语言偏好（只接受 zh-CN / en-US）。
#[tauri::command]
async fn set_locale(bridge: State<'_, EngineBridge>, tag: String) -> Result<(), CommandError> {
    bridge.set_locale(tag).await
}

/// M5：启动时试一次自动重连（没开 / 没记录 / 连不上都返回 null，不报错）。
#[tauri::command]
async fn try_auto_connect(
    bridge: State<'_, EngineBridge>,
) -> Result<Option<PeerView>, CommandError> {
    bridge.try_auto_connect().await
}

/// M5：开机自启状态（FR-31 的一部分）。
#[tauri::command]
async fn autostart_enabled(app: tauri::AppHandle) -> Result<bool, CommandError> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|error| {
        CommandError::busy(format!("读取自启状态失败：{error}"), "autostart_enabled")
    })
}

/// M5：开关开机自启。
#[tauri::command]
async fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), CommandError> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let outcome = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    outcome.map_err(|error| CommandError::busy(format!("设置自启失败：{error}"), "set_autostart"))
}

/// M5：第三方组件声明（用户看得见的投放）。
#[tauri::command]
async fn third_party_notices(bridge: State<'_, EngineBridge>) -> Result<NoticesView, CommandError> {
    bridge.third_party_notices().await
}

/// M5：检查更新（**只由用户主动触发**）。
///
/// 为什么没有「启动后自动检查」：这是个音频工具，用户可能在推流/录音 —— 静默的更新流程
/// （检查、下载、安装、重启）会把正在进行的会话打断。边界与理由写在 `src/update.rs` 顶部。
#[tauri::command]
async fn check_update(app: tauri::AppHandle) -> Result<UpdateCheckView, CommandError> {
    update::check(&app).await
}

/// M5：下载并安装更新（**必须由用户确认**）。
///
/// `version` 是用户在上一步界面上看到并确认的那个版本号：安装前会再查一次更新源，
/// 对不上就报错让他重新确认 —— 用户授权的是「升级到 X」，不是「随便装个新的」。
#[tauri::command]
async fn install_update(app: tauri::AppHandle, version: String) -> Result<String, CommandError> {
    update::install(&app, &version).await
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
        // 自动更新（M5）：插件在这里装配，调用只走 src/update.rs 的 check_update / install_update。
        //
        // 注意 capabilities/default.json **没有**给任何 updater 权限：webview 侧不直接调
        // `plugin:updater|*`（前端只调外壳命令），所以「静默下载安装」在权限层就没有入口。
        // 而 `update.rs` 里的 `app.updater()` 会去读本插件登记的状态 —— 插件没装配会直接
        // panic 而不是静默失效，因此「注册了没有」这件事是硬的。
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            broadcast_epoch,
            third_party_notices,
            check_update,
            install_update,
            autostart_enabled,
            set_autostart,
            auto_connect_state,
            set_auto_connect,
            try_auto_connect,
            locale,
            set_locale
        ])
        .setup(|app| {
            // 桥接层必须在窗口加载**之前**就位：前端一挂载就会 invoke，拿不到 State 会直接报错。
            // 引擎真正的启动是异步的（`Engine::start`），命令层会等到它就绪（见 engine_bridge.rs）。
            let _managed = app.manage(EngineBridge::new(app.handle().clone()));

            // TODO(M5)：启动后自动连接上次设备（FR-31，可关）
            build_tray(app)?;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // 关掉窗口 ≠ 退出程序：这是个常驻的音频工具，窗口只是它的一个面。
            // 真退出在托盘菜单里（见 quit_app），而那里会让引擎先给对方发 BYE。
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running AudioLink desktop");
}

/// 托盘图标与菜单：显示主窗口 / 退出。
///
/// 为什么值得做：这是「后台常驻」的音频工具 —— 关掉窗口不该等于退出程序；
/// 而托盘是用户唯一能「叫回窗口」和「真正退出」的地方。
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("AudioLink")
        .menu(&menu)
        // 左键留给「唤出窗口」，菜单走右键（与常见桌面应用一致）。
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "quit" => quit_app(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    // 图标来自 bundle 的默认窗口图标；没有就只建一个无图标的托盘（不 panic）。
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

/// 唤出主窗口（可能被隐藏到托盘了）。
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 退出：**先让引擎优雅收尾**（给对方发 `BYE`），再退进程。
///
/// 这条路径最容易一直没人管，而它的代价落在对端身上 —— 对方要多等一次 QUIC 空闲
/// 超时才能把会话判掉。桥接层内部有 3 s 上限，所以最坏情况也不会卡住用户。
fn quit_app(app: &tauri::AppHandle) {
    if let Some(bridge) = app.try_state::<EngineBridge>() {
        // 托盘事件跑在主线程；用 Tauri 的 block_on 等这一步是安全的。
        if let Err(error) = tauri::async_runtime::block_on(bridge.shutdown_engine()) {
            eprintln!("audiolink: 退出前收尾未完成：{error}");
        }
    }
    app.exit(0);
}
