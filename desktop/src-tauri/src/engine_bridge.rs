//! 外壳 ⇄ `audiolink-engine` 的**唯一接缝**（真实实现，M1）。
//!
//! ## 这一层负责什么
//! 外壳不做音频、不做网络；它只做三件固定的事：
//! 1. **把引擎事件翻译成前端事件**（契约 §6 冻结的三个名字，遥测不快于 500 ms 一次）；
//! 2. **把引擎查询翻译成契约视图**（`PeerView` / `TelemetryView` / `LocalStatus`）；
//! 3. **把引擎错误翻译成人话**（`{code, message, context}`，UI 规格 §4「错误必说人话」）。
//!
//! 前端不认识引擎：它只认 `lib.rs` 里的 command 与这里的三个事件名。
//!
//! ## 与引擎的接缝语义（读代码前先看这几条，都是刻意的）
//! 1. **`connect` 成功即握手完成**：无认证之后没有 PIN 提交或挑战应答要推进
//!    （见 `Engine::connect` 的文档），这里把返回的 `NodeId` 翻成 `PeerView`，
//!    并**刷新对端列表**让卡片立刻出现。
//! 2. **引擎的 `Streaming` ≠ 契约的 `streaming`**：引擎在握手完成时就把会话置为
//!    `Streaming`（`runtime.rs::mark_streaming`），而契约的 `streaming` 对用户意味着"正在推流"。
//!    只有外壳知道本机有没有在推流（`start_send`/`stop_send` 不产生引擎状态迁移），
//!    所以用 `Cache::sending` 打标：**引擎 Streaming + 未推流 → 视图 `idle`**
//!    （卡片不消失，按钮回到"开始推流"）。
//! 3. **引擎有 6 个会话状态，契约 §6 只有 5 个**（缺 `reconnecting`）→ 归入 `degraded`
//!    （"琥珀 / 重连中"语义最近）。这是契约待补的一处，已在汇报里点名。
//! 4. **遥测源是引擎的 1 Hz**：`StreamStats` 没有分位数，契约却要 `e2eP50Us/e2eP95Us`，
//!    所以外壳按对端维护滑动窗口自己算（见 `aggregate`）。这一层只保证"不快于 500 ms 一次"。

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use audiolink_engine::{
    Engine, EngineConfig, EngineEvent, GroupSnapshot, PeerCapabilities, PeerStatus, SessionState,
};
use audiolink_types::{
    AudioLinkError, Capabilities, ClockQuality, DEFAULT_QUIC_PORT, ErrorCode, NodeId, StreamStats,
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_store::StoreExt as _;

use std::path::PathBuf;

/// 外壳设置文件名（`tauri-plugin-store` 自己管目录）。
const SETTINGS_FILE: &str = "settings.json";
/// 「启动时自动连接上次设备」开关。
const KEY_AUTO_CONNECT: &str = "auto_connect";
/// 界面语言偏好（`zh-CN` / `en-US`）；键缺失 = 用户还没选过（前端跟随系统语言）。
const KEY_LOCALE: &str = "locale";
/// 上一次成功连接的地址。
const KEY_LAST_PEER: &str = "last_peer";
/// 「有人接入时自动开始推流」开关。
const KEY_AUTO_BROADCAST: &str = "auto_broadcast";
// 「低延迟档」的读写**不在本文件**：键常量 `KEY_LOW_LATENCY`（= 契约名 `lowLatency`）
// 与读写函数都在 settings.rs 里，engine_config 直接调 `crate::settings::read_low_latency(app)`。
// 为什么不在这里再抄一份键名：一个键两处字面量，改一处漏一处就会变成"开关存了但引擎读不到"
// 这种两边都不报错的静默故障 —— 少一份副本就少一个能踩的坑。

/// [`KEY_AUTO_BROADCAST`] 的默认值：**开**。
///
/// 与 [`KEY_AUTO_CONNECT`] 的「键缺失 = 关」刻意相反，因为两者的默认体验不是一回事：
/// 自动重连是「开机去连一台用户上次连过的机器」—— 用户此刻什么都没做，主动去连别人
/// 必须显式授权；自动推流是「别人连上我之后，我照他已经在走的那条路出声」—— 用户已经把
/// 设备连进来了，期待的就是「连上就有声」，还要再点一次「开始推流」属于多余门槛。
/// 这是产品要求的默认行为，不是历史兼容。
const DEFAULT_AUTO_BROADCAST: bool = true;

/// 开关取值：`settings.json` 里的原始值 → 布尔。
///
/// **键缺失即视为开**（理由见 [`DEFAULT_AUTO_BROADCAST`]）。类型不对也回默认值：
/// settings.json 是可以被手改的，把 `"auto_broadcast": "no"` 当成「用户想关」是猜，
/// 按默认值处理至少是可预期的行为。
fn auto_broadcast_from(stored: Option<&serde_json::Value>) -> bool {
    stored
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(DEFAULT_AUTO_BROADCAST)
}

/// 第三方声明的文件名（打包资源与仓库内生成路径同名）。
pub const NOTICES_FILE: &str = "THIRD-PARTY-NOTICES.md";
use tokio::sync::{broadcast, watch};

use crate::capture::{self, SharedCapture};
use crate::error::CommandError;
use crate::settings::AutoConnectPolicy;
use crate::view::{
    AlignmentView, CaptureDeviceView, GroupMemberView, GroupView, LocalStatus, NoticesView,
    PeerCapabilitiesView, PeerQualityView, PeerState, PeerView, StartSendResult, TelemetryRow,
    TelemetryView, alignment_view, notices_view, render_telemetry_csv,
};

// ---------------------------------------------------------------------------
// 契约常量（`docs/11-m1-contract.md` §6，冻结：名字不许改）
// ---------------------------------------------------------------------------

/// 对端列表变化事件，载荷 `PeerView[]`。
pub const EVENT_PEER: &str = "audiolink://peer";
/// 遥测事件，载荷 `TelemetryView`；**不快于 500 ms 一次**。
pub const EVENT_TELEMETRY: &str = "audiolink://telemetry";

/// §7 同步组变化（建组 / 成员加入退出）：前端收到就去拉一次最新的组列表。
pub const EVENT_GROUPS: &str = "audiolink://groups";

// ---------------------------------------------------------------------------
// 节奏与阈值
// ---------------------------------------------------------------------------

/// 遥测事件的最小间隔。契约 §6「500 ms 节流」+ 架构 §4「遥测聚合每 500 ms」。
///
/// 注意方向：这是**上限保护**（不允许多于 2 次/秒），不是"必须每 500 ms 推一次" ——
/// 真正的采样节奏由引擎决定（1 Hz）。没有新数据时通道是安静的。
const TELEMETRY_THROTTLE: Duration = Duration::from_millis(500);

/// 端到端延迟滑动窗口长度（样本数）。1 Hz 采样 → 300 条 ≈ 最近 5 分钟，
/// 与 UI 规格 §2.4「曲线保留最近 5 分钟」同一口径（本轮只出数字，不出曲线）。
const E2E_WINDOW: usize = 300;

/// 等引擎启动的上限：启动要读身份材料 + 绑 QUIC 端口，正常在毫秒级。
const ENGINE_READY_TIMEOUT: Duration = Duration::from_secs(10);

/// 对端上报遥测的新鲜度上限。超过它就不再采信 —— 链路断了却继续显示 3 秒前的"漂亮数字"
/// 比显示"没有数据"更糟。
const PEER_REPORT_TTL: Duration = Duration::from_secs(3);

/// WASAPI 缓冲请求值（ms）。20 ms 是本仓工具链的既有取值：
/// 共享模式下小于一个周期的请求会被抬到 1056 帧 = 22 ms（`wasapi/mod.rs` 的实测表），
/// 所以填 20 与填 1 的实际效果一样，填 20 至少让意图可读。
///
/// **这个值与低延迟档（M6）刻意不联动**：低延迟档压的是「Opus 帧长 + 播放水位」，
/// 而 Windows 共享模式 WASAPI 的下限由系统抬到约 22 ms —— 填 10 换不来更低的实际缓冲，
/// 只会让"我压过了"这件事在代码里名不副实。真要把播放端延迟压到 10 ms 量级，
/// 需要的是独占模式（exclusive）+ 事件驱动，那是另一件事（见 `docs/09-benchmark-notes.md`
/// 的实测对比），不在低延迟档的范围内。
#[cfg(windows)]
const WASAPI_BUFFER_MS: u32 = 20;

// ---------------------------------------------------------------------------
// 对外门面
// ---------------------------------------------------------------------------

/// 引擎的生命周期状态。
///
/// 为什么要有它：`Engine::start` 是异步的，而 Tauri 的 `setup` 是同步的。
/// 前端一挂载就会 `invoke`，此时引擎可能还没就绪 —— 命令在这里等一会儿，
/// 而不是甩一个"引擎没起来"给用户看。
enum EngineState {
    Starting,
    Ready(Arc<Engine>),
    Failed {
        code: u16,
        message: String,
        context: String,
    },
}

impl Clone for EngineState {
    fn clone(&self) -> Self {
        match self {
            Self::Starting => Self::Starting,
            Self::Ready(engine) => Self::Ready(Arc::clone(engine)),
            Self::Failed {
                code,
                message,
                context,
            } => Self::Failed {
                code: *code,
                message: message.clone(),
                context: context.clone(),
            },
        }
    }
}

/// 外壳看到的"引擎"：命令层只与它对话。
pub struct EngineBridge {
    app: AppHandle,
    state_rx: watch::Receiver<EngineState>,
    cache: Arc<Mutex<Cache>>,
    capture: SharedCapture,
    discovery: Arc<Mutex<Option<crate::discovery::LanDiscovery>>>,
    /// 串行化开始/停止：更换端点与开流必须属于同一次操作。
    send_operation: tokio::sync::Mutex<()>,
}

impl EngineBridge {
    /// 建立接缝并**异步**启动引擎（启动结果通过 watch 通道广播给命令层）。
    pub fn new(app: AppHandle) -> Self {
        let (state_tx, state_rx) = watch::channel(EngineState::Starting);
        let cache = Arc::new(Mutex::new(Cache::default()));
        let capture = SharedCapture::default();
        let discovery = Arc::new(Mutex::new(None));
        // 发送端交给启动任务持有：它的生命周期就是应用的生命周期，
        // 命令层只在"还没就绪"时才会去等这个通道。
        tauri::async_runtime::spawn(boot(
            app.clone(),
            state_tx,
            Arc::clone(&cache),
            Arc::clone(&capture),
            Arc::clone(&discovery),
        ));
        Self {
            app,
            state_rx,
            cache,
            capture,
            discovery,
            send_operation: tokio::sync::Mutex::new(()),
        }
    }

    pub async fn discovered_hosts(
        &self,
        refresh: bool,
    ) -> Result<Vec<audiolink_discovery::DiscoveredHost>, CommandError> {
        let _engine = self.engine().await?;
        lock(&self.discovery)
            .as_mut()
            .ok_or_else(|| CommandError::busy("正在准备局域网发现", "discovery not ready"))?
            .hosts(refresh)
    }

    /// 本机身份（`local_status`）。
    pub async fn local_status(&self) -> Result<LocalStatus, CommandError> {
        let engine = self.engine().await?;
        let info = engine.info();
        // 两个地址口径都要给出去，且**不能混用**：`addr` 是监听地址（诊断用，通配绑定下恒为
        // `0.0.0.0:58290`）；`lanAddrs/displayAddr` 才是用户能填进门牌号的东西。
        // 只印监听地址的后果实测过：用户抄 `0.0.0.0:58290` 到手机，必然连不上。
        let lan_addrs: Vec<String> = engine
            .lan_addrs()
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
        let display_addr = lan_addrs.first().cloned();
        Ok(LocalStatus {
            // 短码口径由内核唯一确定（`NodeId::short()` = 指纹前 8 字节的 hex），
            // 外壳**不自己截字符串**：否则 UI 上的码会与协议/发现报文里的码悄悄分叉。
            id_short: info.id.short(),
            name: info.name.clone(),
            addr: engine.local_addr().to_string(),
            lan_addrs,
            display_addr,
            platform: info.platform.as_str().to_string(),
        })
    }

    /// 已连接/已登记对端（`list_peers`）。
    pub async fn list_peers(&self) -> Result<Vec<PeerView>, CommandError> {
        let engine = self.engine().await?;
        let (peers, _) = refresh(&engine, &self.cache);
        Ok(peers)
    }

    /// 读设置（读不到就用默认值：设置坏了不该让应用起不来）。
    fn policy(&self) -> AutoConnectPolicy {
        let Ok(store) = self.app.store(SETTINGS_FILE) else {
            return AutoConnectPolicy::default();
        };
        AutoConnectPolicy {
            enabled: store
                .get(KEY_AUTO_CONNECT)
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            last_peer: store
                .get(KEY_LAST_PEER)
                .and_then(|value| value.as_str().map(str::to_string)),
        }
    }

    /// 记下「上次**成功**连接的地址」—— 失败的不记（免得开机就自动去连一个连不上的东西）。
    fn remember_peer(&self, addr: std::net::SocketAddr) {
        let Ok(store) = self.app.store(SETTINGS_FILE) else {
            return;
        };
        store.set(KEY_LAST_PEER, serde_json::Value::String(addr.to_string()));
        let _ = store.save();
    }

    /// 手工 IP 连接（`connect`，FR-17）。
    ///
    /// 返回"已登记 + 已建立"的对端视图；后续状态变化由 `audiolink://peer` 事件推进。
    pub async fn connect(&self, raw_addr: &str) -> Result<PeerView, CommandError> {
        let engine = self.engine().await?;
        let addr = parse_endpoint(raw_addr)?;

        // 去重（外壳侧把关）：引擎允许对同一地址重复 connect（会新建会话并覆盖会话表项），
        // 那既浪费一条 QUIC 连接，又让 UI 出现两张指向同一台设备的卡片。
        if let Some(existing) = engine.peers().iter().find(|peer| peer.addr == addr) {
            return Err(CommandError::busy(
                format!("{addr} 已经在列表里了（{}）", state_text(existing)),
                format!("connect: addr={addr} state={}", existing.state.name()),
            ));
        }

        match engine.connect(addr).await {
            Ok(peer_id) => {
                // 只有成功才记：失败也记的话，开机自动重连会去连一个已知连不上的地址。
                self.remember_peer(addr);
                let (peers, _) = refresh(&engine, &self.cache);
                emit_peer(&self.app, &self.cache, &peers);
                find_view(&peers, peer_id).ok_or_else(|| {
                    CommandError::bad_request(
                        "连接成功了，但对端没出现在会话表里",
                        format!("connect: peer={}", peer_id.short()),
                    )
                })
            }
            Err(error) => {
                let (peers, _) = refresh(&engine, &self.cache);
                emit_peer(&self.app, &self.cache, &peers);
                Err(engine_error("connect", &error))
            }
        }
    }

    /// 开始推流（`start_send`）。
    pub async fn start_send(
        &self,
        id_short: &str,
        capture_device_id: Option<String>,
    ) -> Result<StartSendResult, CommandError> {
        let _operation = self.send_operation.lock().await;
        let engine = self.engine().await?;
        let peer = resolve_peer(&engine, id_short)?;

        // M1 单路推流：契约的 `stop_send` 不收参数，所以"推给谁"必须由外壳记住。
        if let Some(active) = lock(&self.cache).sending {
            return Err(CommandError::busy(
                if active == peer {
                    "已经在给这个设备推流了"
                } else {
                    "已有正在进行的推流，请先停止"
                },
                format!(
                    "start_send: active={} target={}",
                    active.short(),
                    peer.short()
                ),
            ));
        }

        let devices = capture::list_devices().await?;
        capture::validate_selection(&devices, capture_device_id.as_deref())?;
        {
            let mut selection = lock(&self.capture);
            selection.requested_id = capture_device_id;
            selection.opened = None;
        }
        if let Err(error) = engine.start_send(peer).await {
            lock(&self.capture).opened = None;
            return Err(match error.code() {
                ErrorCode::CaptureLost => engine_error_with_message(
                    "start_send",
                    &error,
                    "无法打开所选输出设备，请检查设备连接，刷新后重试",
                ),
                ErrorCode::CapUnsupported => engine_error_with_message(
                    "start_send",
                    &error,
                    "所选输出设备的格式不受支持，请检查 Windows 声音设置或选择其他设备",
                ),
                _ => engine_error("start_send", &error),
            });
        }

        lock(&self.cache).sending = Some(peer);
        let (peers, _) = refresh(&engine, &self.cache);
        emit_peer(&self.app, &self.cache, &peers);

        // 契约 §6 要求返回 stream_id，而 `Engine::start_send` 返回 `()`。
        // 取真实值：会话遥测里的 stream_id（发送侧 M1 固定为 1，见 `runtime.rs` 的
        // `stream_id = Some(1)`）。0 表示引擎还没开始记账，UI 不使用这个值。
        let stream_id = engine
            .telemetry(peer)
            .map(|stats| stats.stream_id)
            .unwrap_or(0);
        Ok(StartSendResult { stream_id })
    }

    /// 停止推流（`stop_send`）。幂等：没有在推流时也返回成功。
    pub async fn stop_send(&self) -> Result<(), CommandError> {
        let _operation = self.send_operation.lock().await;
        let engine = self.engine().await?;
        let Some(peer) = lock(&self.cache).sending else {
            return Ok(());
        };
        // 先清标记：即使引擎侧报错（比如会话已经没了），UI 也不该卡在"推流中"。
        lock(&self.cache).sending = None;
        lock(&self.capture).opened = None;
        let result = engine
            .stop_send(peer)
            .await
            .map_err(|error| engine_error("stop_send", &error));
        let (peers, _) = refresh(&engine, &self.cache);
        emit_peer(&self.app, &self.cache, &peers);
        result
    }

    /// 返回实际打开的端点；默认设备改变时不把正在采集的端点改写成新的默认设备。
    pub async fn active_capture_device(&self) -> Result<Option<CaptureDeviceView>, CommandError> {
        let engine = self.engine().await?;
        refresh(&engine, &self.cache);
        if lock(&self.cache).sending.is_none() {
            return Ok(None);
        }
        Ok(lock(&self.capture).opened.clone())
    }

    /// 当前遥测快照（`telemetry`）。
    pub async fn telemetry(&self) -> Result<TelemetryView, CommandError> {
        let engine = self.engine().await?;
        let (_, view) = refresh(&engine, &self.cache);
        Ok(view)
    }

    /// 把前端累积的遥测历史导出成 CSV，返回落盘路径。
    ///
    /// **不弹文件对话框**（那要引入 dialog 插件，且无头/CI 下没法用）：目录固定在用户目录下，
    /// 路径由本命令返回、UI 原样显示。导出本身不依赖引擎状态 —— 会话已经结束也允许导历史。
    /// §4.1：调对端音量（发送方 →）。非法值在引擎边界就被拒绝，UI 会收到人话原因。
    pub async fn set_peer_gain(
        &self,
        id_short: &str,
        gain: f32,
        ramp_ms: u32,
    ) -> Result<(), CommandError> {
        let engine = self.engine().await?;
        let peer = resolve_peer(&engine, id_short)?;
        engine
            .set_peer_gain(peer, gain, ramp_ms)
            .await
            .map_err(|error| engine_error("set_peer_gain", &error))
    }

    /// M4：多源对齐快照 —— 各路「最近一帧编号 ↔ 到达时刻」与当前跨度。
    ///
    /// 与 `broadcast_epoch` 是本机接收端侧的一对：一个负责让两路对齐，一个负责**显示**它对齐了没有。
    pub async fn alignment(&self) -> Result<AlignmentView, CommandError> {
        let engine = self.engine().await?;
        Ok(alignment_view(&engine.stream_axes(), engine.monotonic_ms()))
    }

    /// M5：界面语言偏好（`zh-CN` / `en-US`）；`None` = 用户还没选过，前端跟随系统语言。
    pub async fn locale(&self) -> Result<Option<String>, CommandError> {
        let Ok(store) = self.app.store(SETTINGS_FILE) else {
            return Ok(None);
        };
        Ok(store
            .get(KEY_LOCALE)
            .and_then(|value| value.as_str().map(str::to_string)))
    }

    /// M5：保存界面语言偏好。
    ///
    /// 只接受受支持的两个标记：`settings.json` 是可以被手改的，写进来一个 `ja-JP` 不该让界面
    /// 变成「查不到的键都显示键名」—— 拒掉非法值，前端会回落到跟随系统语言。
    pub async fn set_locale(&self, tag: String) -> Result<(), CommandError> {
        if !matches!(tag.as_str(), "zh-CN" | "en-US") {
            return Err(CommandError::busy(
                format!("不支持的语言标记：{tag}"),
                "set_locale",
            ));
        }
        let store = self
            .app
            .store(SETTINGS_FILE)
            .map_err(|error| CommandError::busy(format!("打开设置失败：{error}"), "set_locale"))?;
        store.set(KEY_LOCALE, serde_json::Value::String(tag));
        store
            .save()
            .map_err(|error| CommandError::busy(format!("保存设置失败：{error}"), "set_locale"))?;
        Ok(())
    }

    /// M5：自动重连的当前设置（开关 + 上次设备）。
    pub async fn auto_connect_state(&self) -> Result<AutoConnectPolicy, CommandError> {
        Ok(self.policy())
    }

    /// M5：开关「启动时自动连接上次设备」。
    pub async fn set_auto_connect(&self, enabled: bool) -> Result<(), CommandError> {
        let store = self.app.store(SETTINGS_FILE).map_err(|error| {
            CommandError::busy(format!("打开设置失败：{error}"), "set_auto_connect")
        })?;
        store.set(KEY_AUTO_CONNECT, serde_json::Value::Bool(enabled));
        store.save().map_err(|error| {
            CommandError::busy(format!("保存设置失败：{error}"), "set_auto_connect")
        })?;
        Ok(())
    }

    /// M5：自动推流开关的当前值（`auto_broadcast`，**默认开**）。
    ///
    /// 只读设置、不碰引擎：设置面板要在引擎就绪**之前**就能显示当前状态。
    /// 返回**裸布尔**（契约如此）：前端只关心开/关，再包一层对象没有信息量。
    pub async fn auto_broadcast_state(&self) -> Result<bool, CommandError> {
        let Ok(store) = self.app.store(SETTINGS_FILE) else {
            // 设置读不出来（目录不可写等）不是错误：回默认值，界面照常可用。
            return Ok(DEFAULT_AUTO_BROADCAST);
        };
        Ok(auto_broadcast_from(store.get(KEY_AUTO_BROADCAST).as_ref()))
    }

    /// M5：开关「有人接入时自动开始推流」。
    ///
    /// 只写设置，**不触碰任何正在跑的会话**：用户关掉的是「以后自动」，不是「现在别出声」。
    /// 关掉之后要不要立刻掐断在推的流，是调用方的取舍，不在本命令里替用户决定。
    pub async fn set_auto_broadcast(&self, enabled: bool) -> Result<(), CommandError> {
        let store = self.app.store(SETTINGS_FILE).map_err(|error| {
            CommandError::busy(format!("打开设置失败：{error}"), "set_auto_broadcast")
        })?;
        store.set(KEY_AUTO_BROADCAST, serde_json::Value::Bool(enabled));
        store.save().map_err(|error| {
            CommandError::busy(format!("保存设置失败：{error}"), "set_auto_broadcast")
        })?;
        Ok(())
    }

    /// M6：低延迟档的当前值（`lowLatency`，**默认关** = 20 ms 帧的标准档）。
    ///
    /// 只读设置、不碰引擎：设置面板要在引擎就绪**之前**就能显示当前状态（同 `auto_broadcast_state`）。
    /// 返回**裸布尔**：前端只关心开/关，再包一层对象没有信息量。
    pub async fn low_latency_state(&self) -> Result<bool, CommandError> {
        Ok(crate::settings::read_low_latency(&self.app))
    }

    /// M6：开关「低延迟档」。
    ///
    /// 只写设置，**不重启引擎**：档位是 `EngineConfig.codec` 的一部分，只在 `Engine::start`
    /// 读一次；已经跑着的会话（可能正在推流/接收）不会因为这一行改变 —— 这是与 `set_auto_broadcast`
    /// 完全一致的处置（那个也刻意不掐断在推的流）。
    ///
    /// 为什么不做「重启引擎让它立刻生效」：重启会掐断所有会话，而对端此刻可能正在听 ——
    /// 用户点的是一个设置开关，不该有掐断音频的副作用。UI 负责把「下次启动生效」说清楚。
    pub async fn set_low_latency(&self, enabled: bool) -> Result<(), CommandError> {
        crate::settings::write_low_latency(&self.app, enabled)
    }

    /// M5：启动时试一次自动重连。
    ///
    /// 返回连上的对端；没开、没记录或连不上都返回 `None` —— **不报错**：
    /// 开机自动重连失败不该弹一条错误横幅（用户什么都没点）。前端据此给一句轻提示即可。
    pub async fn try_auto_connect(&self) -> Result<Option<PeerView>, CommandError> {
        let Some(addr) = self.policy().target().map(str::to_string) else {
            return Ok(None);
        };
        match self.connect(&addr).await {
            Ok(view) => Ok(Some(view)),
            Err(_) => Ok(None),
        }
    }

    /// M5：退出前的优雅收尾 —— 给对方发 `BYE`，而不是让对端等 QUIC 空闲超时。
    ///
    /// 为什么值得单独一条命令：这类「退出时才走」的路径最容易一直没人管，
    /// 而它的代价落在**对端**身上（对方要多等一次超时才把会话判掉）。
    /// 内部带 3 s 上限：退出路径不能因为对端没响应就卡住用户。
    pub async fn shutdown_engine(&self) -> Result<(), CommandError> {
        lock(&self.discovery).take();
        let engine = self.engine().await?;
        match tokio::time::timeout(std::time::Duration::from_secs(3), engine.shutdown()).await {
            Ok(()) => Ok(()),
            Err(_) => Err(CommandError::busy(
                "退出时等待会话收尾超时（3 秒），仍会退出",
                "shutdown_engine",
            )),
        }
    }

    /// M5：第三方组件声明 —— 让用户**看得见**，而不是只躺在仓库里。
    ///
    /// 两处依次找：打包后的**资源目录**（安装版在这里）、再退回仓库内的生成路径（开发版）。
    /// 两处都没有时**不报错**：返回「怎么生成」的提示，界面照常显示 ——
    /// 空面板只会让人以为软件坏了。
    pub async fn third_party_notices(&self) -> Result<NoticesView, CommandError> {
        let candidates = [
            (
                "资源目录".to_string(),
                self.app
                    .path()
                    .resource_dir()
                    .ok()
                    .map(|dir| dir.join(NOTICES_FILE)),
            ),
            (
                "仓库（开发）".to_string(),
                Some(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../../docs/compliance")
                        .join(NOTICES_FILE),
                ),
            ),
        ];
        let found = candidates.into_iter().find_map(|(label, path)| {
            let path = path?;
            let text = std::fs::read_to_string(&path).ok()?;
            Some((format!("{label}：{}", path.display()), text))
        });
        Ok(notices_view(found))
    }

    /// M4：本机作为接收端，把所有发送端共用的时间原点广播出去。
    ///
    /// 返回成功发出的会话数 —— 调用方（UI）据此告诉用户「广播了几路」或「还没有对端」。
    /// 没有已连接对端时引擎会返回错误，UI 直接显示人话原因。
    pub async fn broadcast_epoch(&self, lead_ms: u32) -> Result<u32, CommandError> {
        let engine = self.engine().await?;
        engine
            .broadcast_epoch(lead_ms)
            .await
            .map_err(|error| engine_error("broadcast_epoch", &error))
    }

    /// 同步组列表（§7 / M3 交付物 4）。
    pub async fn list_groups(&self) -> Result<Vec<GroupView>, CommandError> {
        let engine = self.engine().await?;
        Ok(engine.groups().iter().map(group_view_of).collect())
    }

    /// 建组：把点名的对端组成一个临时同步组，返回组 ID。
    pub async fn create_group(
        &self,
        id_shorts: &[String],
        lead_ms: u32,
    ) -> Result<u32, CommandError> {
        let engine = self.engine().await?;
        let mut members = Vec::with_capacity(id_shorts.len());
        for id_short in id_shorts {
            members.push(resolve_peer(&engine, id_short)?);
        }
        engine
            .create_group(&members, lead_ms)
            .await
            .map_err(|error| engine_error("create_group", &error))
    }

    /// 成员动态加入（运行中的其它成员不受影响）。
    pub async fn join_group(&self, id_short: &str, group_id: u32) -> Result<(), CommandError> {
        let engine = self.engine().await?;
        let peer = resolve_peer(&engine, id_short)?;
        engine
            .join_group(peer, group_id)
            .await
            .map_err(|error| engine_error("join_group", &error))
    }

    /// 成员退出（组空了引擎会自己把条目清掉）。
    pub async fn leave_group(&self, id_short: &str, group_id: u32) -> Result<(), CommandError> {
        let engine = self.engine().await?;
        let peer = resolve_peer(&engine, id_short)?;
        engine
            .leave_group(peer, group_id)
            .await
            .map_err(|error| engine_error("leave_group", &error))
    }

    pub fn export_telemetry(&self, rows: &[TelemetryRow]) -> Result<String, CommandError> {
        let dir = telemetry_export_dir();
        let path = write_telemetry_csv(&dir, rows).map_err(|error| {
            CommandError::bad_request(
                format!("无法写入导出文件：{error}"),
                format!("dir={}", dir.display()),
            )
        })?;
        Ok(path.display().to_string())
    }

    /// 等到引擎就绪；未就绪时给出**可读**的原因（启动失败的原码与上下文原样带上，便于排查）。
    async fn engine(&self) -> Result<Arc<Engine>, CommandError> {
        let mut rx = self.state_rx.clone();
        let deadline = Instant::now() + ENGINE_READY_TIMEOUT;
        loop {
            // 先把当前值 clone 出来（`watch::Ref` 借住 `rx`，留在 match 里就没法再 await `changed()`）
            let current = rx.borrow_and_update().clone();
            match current {
                EngineState::Ready(engine) => return Ok(engine),
                EngineState::Failed {
                    code,
                    message,
                    context,
                } => {
                    return Err(CommandError {
                        code,
                        message,
                        context,
                    });
                }
                EngineState::Starting => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(starting_error());
                    }
                    match tokio::time::timeout(remaining, rx.changed()).await {
                        Ok(Ok(())) => continue,
                        Ok(Err(_)) => {
                            return Err(CommandError::busy(
                                "引擎状态通道已关闭，请重启应用",
                                "engine: watch channel closed",
                            ));
                        }
                        Err(_) => return Err(starting_error()),
                    }
                }
            }
        }
    }
}

/// 引擎还没就绪（仍在启动）。
fn starting_error() -> CommandError {
    CommandError::busy("引擎还在启动，请稍候再试", "engine: still starting")
}

// ---------------------------------------------------------------------------
// 启动与事件翻译
// ---------------------------------------------------------------------------

/// 启动引擎 → 开始监听 → 订阅事件 → 常驻翻译循环。
///
/// 启动失败不是"世界末日"：它被记进 `EngineState::Failed`，
/// 之后每条命令都以同一个真实原因（原错误码 + 上下文）失败，
/// 用户看到的是人话而不是"未知错误"。
async fn boot(
    app: AppHandle,
    state_tx: watch::Sender<EngineState>,
    cache: Arc<Mutex<Cache>>,
    capture: SharedCapture,
    discovery: Arc<Mutex<Option<crate::discovery::LanDiscovery>>>,
) {
    let config = match engine_config(&app, capture) {
        Ok(config) => config,
        Err(error) => {
            let _ = state_tx.send(EngineState::Failed {
                code: error.code,
                message: error.message,
                context: error.context,
            });
            return;
        }
    };

    let engine = match Engine::start(config).await {
        Ok(engine) => engine,
        Err(error) => {
            let mapped = engine_error("engine_start", &error);
            tracing::error!(code = mapped.code, context = %mapped.context, "引擎启动失败");
            let _ = state_tx.send(EngineState::Failed {
                code: mapped.code,
                message: "引擎启动失败".to_string(),
                context: mapped.context,
            });
            return;
        }
    };

    // 入站监听：手机主动连过来时的入口（接收方向）。
    let _accept = engine.spawn_accept_loop();
    *lock(&discovery) = Some(crate::discovery::LanDiscovery::start(&engine));
    tracing::info!(
        addr = %engine.local_addr(),
        id = %engine.info().id.short(),
        "engine ready"
    );

    // 先把引擎交给命令层，再推一次初始快照（前端可能已经拉过一次空列表了）。
    let _ = state_tx.send(EngineState::Ready(Arc::clone(&engine)));
    let (peers, _) = refresh(&engine, &cache);
    emit_peer(&app, &cache, &peers);

    run_event_loop(app, engine, cache).await;
}

/// 引擎事件 → 前端事件的常驻翻译循环。
async fn run_event_loop(app: AppHandle, engine: Arc<Engine>, cache: Arc<Mutex<Cache>>) {
    let mut events = engine.subscribe();
    loop {
        match events.recv().await {
            Ok(event) => match event {
                // 对端增删改：以 `peers()` 为准重算视图（会话表的权威快照就是它）。
                EngineEvent::PeerUpdated(_) | EngineEvent::PeerDisconnected { .. } => {
                    let (peers, _) = refresh(&engine, &cache);
                    emit_peer(&app, &cache, &peers);
                }
                // 事件只用于触发刷新；从按 NodeId 隔离的存储取值，避免多设备反馈串线。
                EngineEvent::Telemetry(_) => {
                    let (peers, view) = refresh(&engine, &cache);
                    emit_peer(&app, &cache, &peers);
                    emit_telemetry(&app, &cache, &view);
                }
                // §8 自适应码率生效：目前只记日志 —— 面板上的可视化属于 M2 的「遥测面板」那一项。
                EngineEvent::CodecAdapted {
                    from_bps,
                    to_bps,
                    reason,
                } => {
                    tracing::info!(from_bps, to_bps, reason = %reason, "自适应码率变更");
                }
                // §7 同步组变化（建组 / 成员加入退出）：这里只记日志 ——
                // UI 侧「勾选成组」的成组面板属于 M3 的同步组那一项。
                EngineEvent::GroupUpdated {
                    group_id,
                    epoch_id,
                    members,
                } => {
                    tracing::info!(group_id, epoch_id, members, "§7 同步组已更新");
                    // 事件驱动刷新：UI 不必轮询（payload 只带「变了」这件事，明细由 list_groups 拉）。
                    let _ = app.emit(EVENT_GROUPS, (group_id, epoch_id, members));
                }
                // §7 预约播放生效（接收端按 epoch 排播）：这里只记日志 ——
                // 时间线由内核发出，UI 侧的同步质量展示属于 M3 的同步组面板那一项。
                EngineEvent::PlayoutScheduled {
                    epoch_id,
                    target_local_us,
                    wait_us,
                } => {
                    tracing::info!(
                        epoch_id,
                        target_local_us,
                        wait_us,
                        "§7 预约播放：接收端已按 epoch 排播"
                    );
                }
                // M2：「长断音」（一段连续静音）刚结束。事件总线是广播式的、有容量上限，
                // 所以引擎只在段结束时发一次；进行中的静音由遥测快照的 current_ms 回答。
                // 面板上的可视化属于后续，这里先留时间线（与 CodecAdapted / PlayoutScheduled 同处置）。
                EngineEvent::SilenceDetected { peer, segment } => {
                    tracing::warn!(
                        peer = %peer.short(),
                        duration_ms = segment.duration_ms,
                        content_ms = segment.content_ms,
                        synthetic_ms = segment.synthetic_ms,
                        "播放输出出现连续静音（长断音）"
                    );
                }
                // 契约 §6 只冻结了三个前端事件，没有"错误"事件；
                // 命令路径的错误已由返回值承载，这里记录即可（架构 §11：要么处理要么上报）。
                EngineEvent::Error { code, context } => {
                    tracing::warn!(code, context, "引擎上报了会话级错误");
                }
            },
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                // 积压说明消费端跟不上；视图靠下一次刷新自愈，不丢状态。
                tracing::warn!(skipped, "引擎事件积压，已跳过");
            }
            Err(broadcast::error::RecvError::Closed) => {
                tracing::info!("引擎事件通道已关闭，翻译循环退出");
                return;
            }
        }
    }
}

/// 引擎配置：身份落盘位置 + QUIC 监听 + 音频工厂。
fn engine_config(app: &AppHandle, capture: SharedCapture) -> Result<EngineConfig, CommandError> {
    // 路径口径：用 Tauri 的 `app_config_dir()`（Windows 下 = `%APPDATA%\<identifier>`）。
    // 架构 §9 写的是 `%APPDATA%\AudioLink\` —— 只差目录名；这里选 Tauri 口径，
    // 因为自己拼 `%APPDATA%` 在跨平台/沙箱下都会踩坑，且只影响落盘位置，不影响任何协议行为。
    let dir = app.path().app_config_dir().map_err(|error| {
        CommandError::bad_request(
            "无法确定应用的配置目录，身份材料无处安放",
            format!("engine_config: app_config_dir failed: {error}"),
        )
    })?;
    std::fs::create_dir_all(&dir).map_err(|error| {
        CommandError::bad_request(
            "无法创建应用配置目录（检查磁盘权限）",
            format!(
                "engine_config: create_dir_all({}) failed: {error}",
                dir.display()
            ),
        )
    })?;

    let identity_dir = dir.join("identity");
    let mut config = EngineConfig::new(node_name(), &identity_dir);
    // §13：桌面端跑在 Windows 上，系统内录（WASAPI loopback）是**已实现**的采集源，所以敢声明它。
    // 能力由外壳按平台声明、而不是内核写死 —— 内核并不知道自己跑在谁的机器上。
    config.capabilities =
        audiolink_types::Capabilities::CURRENT | audiolink_types::Capabilities::SYSTEM_LOOPBACK;
    // 身份材料在 `<config>/identity/`（架构 §9）。
    config.listen = SocketAddr::from(([0, 0, 0, 0], DEFAULT_QUIC_PORT));

    // 音频工厂：`CaptureSource` / `PlayoutSink` 都是 `!Send`（WASAPI 对象绑线程），
    // 所以**不能**在这里把对象建好 —— 只能交出"怎么建"，由音频线程自己调用。
    #[cfg(windows)]
    {
        config.capture = Some(capture::factory(capture));
        config.playout = Some(playout_factory());
    }

    // 传输档位（M6）：标准档 = 20 ms Opus 帧（与 M1 基线完全一致），
    // 低延迟档 = 10 ms 帧。这里**显式**给两种档位各赋一次值，而不是"关着就不动"：
    // `EngineConfig::new` 的默认值与 `m1_default()` 今天是同一个常量，但把默认值抄在
    // 两个地方，将来内核改基线时外壳会**静默**跟着变 —— 这里写死"标准档 = m1_default()"
    // 让「不开开关 = 行为与现状完全一致」成为可读的承诺，而不是默认值的副作用。
    //
    // 档位从 settings.json 现读（不是启动时快照）：`boot` 只在进程启动时调本函数一次，
    // 而用户改设置走的是 `set_low_latency`（只落盘）—— 要让下一次引擎启动拿到新值，
    // 读设置这件事就必须发生在 `engine_config` 里。两者共同构成"改完 → 下次启动生效"。
    config.codec = if crate::settings::read_low_latency(app) {
        audiolink_audio::CodecConfig::m1_low_delay_tight()
    } else {
        audiolink_audio::CodecConfig::m1_default()
    };
    // 本开关只决定**发送方向**帧长：接收方向自帧长联动落地后自动跟随对端
    // （接收端开流期读 `OPEN_STREAM.codec_prefs` 的帧长建解码链路，
    // `Engine::negotiated_frame_ms` 暴露协商结果，PeerCard 展示）。
    // 历史上「两端必须同档、混档会被当 PLC 掩盖且无任何报错」的坑已由内核协商根治；
    // 这里仍需如实选档 —— 它决定本机推流时 `codec_prefs` 报出去的帧长。

    Ok(config)
}

/// 播放输出工厂：默认渲染端点（收到对方音频时出声）。
#[cfg(windows)]
fn playout_factory() -> audiolink_engine::PlayoutFactory {
    use audiolink_audio::PlayoutSink;
    use audiolink_audio::wasapi::{DeviceSelector, RenderSink};

    Arc::new(|| {
        audiolink_audio::wasapi::init_thread_mta()?;
        let sink = RenderSink::open(&DeviceSelector::Default, WASAPI_BUFFER_MS)?;
        Ok(Box::new(sink) as Box<dyn PlayoutSink>)
    })
}

/// 节点名（对端可见的展示名，不参与信任判定）。
fn node_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "AudioLink Desktop".to_string())
}

// ---------------------------------------------------------------------------
// 视图缓存与聚合
// ---------------------------------------------------------------------------

/// 外壳侧的派生状态。
///
/// 为什么不每次都重新问引擎：`peers()` 给的是引擎快照，而**契约视图需要三样引擎没有的东西** ——
/// 状态映射（见模块文档第 2/3 条）、"本机是否在推流"、e2e 分位数窗口。
struct Cache {
    /// 最近一次算出的对端视图（`list_peers` 的返回源）。
    peers: Vec<PeerView>,
    /// 最近一次算出的遥测视图（`telemetry` 的返回源）。
    telemetry: TelemetryView,
    /// 本机当前推流目标：契约的 `stop_send` 不收参数，所以只有这里知道"推给谁"。
    sending: Option<NodeId>,
    /// 最近一次已推送到前端的对端快照，用于"只在变化时推事件"。
    last_emitted_peers: Option<Vec<PeerView>>,
    /// 遥测节流闸门。
    last_telemetry_emit: Option<Instant>,
    /// 每个对端的 e2e 滑动窗口（µs）。按对端分开存：换了展示对象不能混样本。
    e2e_windows: HashMap<NodeId, VecDeque<u32>>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            peers: Vec::new(),
            telemetry: TelemetryView::zeros(0),
            sending: None,
            last_emitted_peers: None,
            last_telemetry_emit: None,
            e2e_windows: HashMap::new(),
        }
    }
}

/// 重算视图并写入缓存。返回 `(对端视图, 遥测视图)`；**不**发事件（调用方决定推不推）。
///
/// 采样节奏说明：e2e 滑动窗口由调用方驱动 —— 事件路径是引擎的 1 Hz 遥测（主来源），
/// 命令路径（`list_peers` / `telemetry`）只在首屏水合时发生一次。
fn refresh(engine: &Engine, cache: &Mutex<Cache>) -> (Vec<PeerView>, TelemetryView) {
    let statuses = engine.peers();
    let mut guard = lock(cache);

    // 对端消失（断开/被摘除）→ 推流标记自动作废，否则 UI 会永远停在"推送中"。
    let sending = guard
        .sending
        .filter(|id| statuses.iter().any(|status| status.id == *id));
    guard.sending = sending;

    let reports: HashMap<NodeId, StreamStats> = statuses
        .iter()
        .filter_map(|status| {
            engine
                .fresh_peer_stats(status.id, PEER_REPORT_TTL)
                .map(|stats| (status.id, stats))
        })
        .collect();
    guard
        .e2e_windows
        .retain(|id, _| statuses.iter().any(|s| s.id == *id && active_audio(s)));
    let peers: Vec<PeerView> = statuses
        .iter()
        .map(|status| {
            // 本端作接收端时对端协商出的帧长；本端作发送端 / 未开流 → None（界面显示占位）。
            let mut view = peer_view(status, sending, engine.negotiated_frame_ms(status.id));
            if active_audio(status) {
                let receiver = receiver_stats(status, &reports);
                view.quality = Some(PeerQualityView {
                    rtt_us: (status.stats.rtt_us > 0).then_some(status.stats.rtt_us),
                    loss_pct_x100: receiver.map(|stats| stats.loss_pct_x100),
                    underruns: receiver.map(|stats| stats.underruns),
                    buffer_level_us: receiver.map(|stats| stats.buffer_level_us),
                });
            }
            view
        })
        .collect();
    let telemetry = aggregate(&statuses, sending, &mut guard.e2e_windows, &reports);

    guard.peers = peers.clone();
    guard.telemetry = telemetry.clone();
    (peers, telemetry)
}

/// `PeerStatus` → 契约视图。
///
/// `negotiated_frame_ms` 由调用方查好传进来（[`Engine::negotiated_frame_ms`]），
/// 而不是在这里拿 `&Engine`：**它不在 `PeerStatus` 里** —— 帧长联动是「按 peer 查询的
/// 会话级事实」，内核做成访问器就是不想让它污染 `PeerStatus` 的字面量。
/// 拆成入参之后，这个映射保持纯函数（测试不必去起一个真引擎）。
fn peer_view(
    status: &PeerStatus,
    sending: Option<NodeId>,
    negotiated_frame_ms: Option<u8>,
) -> PeerView {
    PeerView {
        id_short: status.id.short(),
        name: status.name.clone(),
        addr: status.addr.to_string(),
        state: map_state(status.state, sending == Some(status.id)),
        receiving: status.initiated_locally,
        capabilities: capabilities_view(status.capabilities),
        reconnects: status.reconnects,
        quality: None,
        negotiated_frame_ms,
    }
}

/// §13 能力协商结果 → 契约视图（把位图翻成人话，见 [`PeerCapabilitiesView`] 的说明）。
fn capabilities_view(caps: Option<PeerCapabilities>) -> Option<PeerCapabilitiesView> {
    let caps = caps?;
    // 位图 → 人话（给人看）与位图 → 键（给判断用）两条都出，界面因此不必解析中文。
    let names = |bits: u32| -> Vec<String> {
        Capabilities::ALL_KNOWN
            .iter()
            .filter(|bit| bits & **bit != 0)
            .map(|bit| Capabilities::name(*bit).to_string())
            .collect()
    };
    let keys = |bits: u32| -> Vec<String> {
        Capabilities::ALL_KNOWN
            .iter()
            .filter(|bit| bits & **bit != 0)
            .map(|bit| Capabilities::key(*bit).to_string())
            .collect()
    };
    let missing = caps.missing_on_peer();
    Some(PeerCapabilitiesView {
        local: Capabilities::describe(caps.local),
        peer: Capabilities::describe(caps.peer),
        agreed: Capabilities::describe(caps.agreed),
        missing_on_peer: names(missing),
        agreed_keys: keys(caps.agreed),
        missing_keys: keys(missing),
    })
}

/// 引擎会话状态 → 契约状态（5 个）。
///
/// * `Streaming` 只有在**本机确实在推流**时才呈现为 `streaming`（理由见模块文档第 2 条）；
/// * `Reconnecting` 契约里没有 → 归入 `degraded`（理由见模块文档第 3 条）。
fn map_state(state: SessionState, is_sending: bool) -> PeerState {
    match state {
        SessionState::Idle => PeerState::Idle,
        SessionState::Handshaking => PeerState::Handshaking,
        SessionState::Streaming => {
            if is_sending {
                PeerState::Streaming
            } else {
                PeerState::Idle
            }
        }
        SessionState::Degraded => PeerState::Degraded,
        SessionState::Reconnecting => PeerState::Degraded,
        SessionState::Failed => PeerState::Failed,
    }
}

/// 把对端快照聚合成**一个** `TelemetryView`（契约只有一个面板，不是每对端一份）。
///
/// 选谁做展示对象：优先当前推流目标；否则按丢包率、e2e、RTT 取最差一路。
/// 取最差而不是平均 —— 这个面板是"体检表"，平均值会把一路正在爆的链路藏起来。
///
/// 接收方向用本地数据，发送方向用对应设备的反馈；端到端测量不是遥测有效的前提。
fn aggregate(
    statuses: &[PeerStatus],
    sending: Option<NodeId>,
    windows: &mut HashMap<NodeId, VecDeque<u32>>,
    reports: &HashMap<NodeId, StreamStats>,
) -> TelemetryView {
    let peers = statuses.len() as u32;

    let focus = sending
        .and_then(|id| {
            statuses
                .iter()
                .find(|status| status.id == id && active_audio(status))
        })
        .or_else(|| {
            statuses
                .iter()
                .filter(|status| active_audio(status))
                .max_by_key(|status| {
                    let stats = receiver_stats(status, reports).unwrap_or(&status.stats);
                    (stats.loss_pct_x100, stats.e2e_latency_us, stats.rtt_us)
                })
        });
    let Some(focus) = focus else {
        return TelemetryView::zeros(peers);
    };

    let local = &focus.stats;
    let receiver = receiver_stats(focus, reports);
    let stats = receiver.unwrap_or(local);

    let window = windows.entry(focus.id).or_default();
    // 只记"有流"的样本：空闲时的 0 会把分位数一路拖平，读起来像"网络突然变好了"。
    if stats.e2e_latency_us > 0 {
        window.push_back(stats.e2e_latency_us);
        while window.len() > E2E_WINDOW {
            let _dropped = window.pop_front();
        }
    } else {
        window.clear();
    }
    let mut sorted: Vec<u32> = window.iter().copied().collect();
    sorted.sort_unstable();
    let p50 = percentile(&sorted, 50).unwrap_or(stats.e2e_latency_us);
    let p95 = percentile(&sorted, 95).unwrap_or(stats.e2e_latency_us);

    TelemetryView {
        peers,
        rtt_us: u64::from(local.rtt_us),
        jitter_us: u64::from(stats.jitter_us),
        // 内核用 ×100 的整数存百分比（避免浮点），契约要百分数 → 这里还原。
        loss_pct: f64::from(stats.loss_pct_x100) / 100.0,
        receiver_report: receiver.is_some(),
        bitrate_bps: u64::from(stats.bitrate_bps),
        buffer_level_us: u64::from(stats.buffer_level_us),
        underruns: u64::from(stats.underruns),
        e2e_latency_us: u64::from(stats.e2e_latency_us),
        e2e_p50_us: u64::from(p50),
        e2e_p95_us: u64::from(p95),
    }
}

fn active_audio(status: &PeerStatus) -> bool {
    matches!(
        status.state,
        SessionState::Streaming | SessionState::Degraded
    )
}

fn receiver_stats<'a>(
    status: &'a PeerStatus,
    reports: &'a HashMap<NodeId, StreamStats>,
) -> Option<&'a StreamStats> {
    let stats = if status.initiated_locally {
        Some(&status.stats)
    } else {
        reports.get(&status.id)
    }?;
    (stats.bitrate_bps > 0).then_some(stats)
}

/// 最近秩（nearest-rank）分位数：升序样本里的第 `⌈p·n/100⌉` 个（1-based）。
fn percentile(sorted: &[u32], p: u32) -> Option<u32> {
    if sorted.is_empty() {
        return None;
    }
    let n = sorted.len();
    let rank = (p as usize * n).div_ceil(100);
    let index = rank.saturating_sub(1).min(n - 1);
    sorted.get(index).copied()
}

// ---------------------------------------------------------------------------
// 事件推送
// ---------------------------------------------------------------------------

/// 对端列表**变化**才推（没有变化就不占用 IPC 通道）。
fn emit_peer(app: &AppHandle, cache: &Mutex<Cache>, peers: &[PeerView]) {
    let changed = {
        let mut guard = lock(cache);
        if guard.last_emitted_peers.as_deref() == Some(peers) {
            false
        } else {
            guard.last_emitted_peers = Some(peers.to_vec());
            true
        }
    };
    if changed {
        emit(app, EVENT_PEER, &peers);
    }
}

/// 遥测按 500 ms 闸门放行。
fn emit_telemetry(app: &AppHandle, cache: &Mutex<Cache>, view: &TelemetryView) {
    let open = {
        let mut guard = lock(cache);
        match guard.last_telemetry_emit {
            Some(last) if last.elapsed() < TELEMETRY_THROTTLE => false,
            _ => {
                guard.last_telemetry_emit = Some(Instant::now());
                true
            }
        }
    };
    if open {
        emit(app, EVENT_TELEMETRY, view);
    }
}

/// 推送失败只可能是"窗口还没建好/已关闭"，记一条 debug 即可
/// （不许静默：架构 §11「要么处理、要么上报」）。
fn emit<T: serde::Serialize + Clone>(app: &AppHandle, event: &str, payload: &T) {
    if let Err(error) = app.emit(event, payload) {
        tracing::debug!(event, error = %error, "前端事件推送失败（窗口未就绪或已关闭）");
    }
}

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

/// 取锁。故意不 `unwrap()`（工作区 lint `unwrap_used = deny`）：
/// 缓存状态没有必须维持的跨线程不变量，中毒后继续用最后一份状态比让 UI 整页崩掉更好。
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 短码 → 完整 `NodeId`。按引擎的会话表现查，**不自己维护映射表**：
/// 会话表才是权威，自己再存一份就有两张真相，必然会分叉。
/// 引擎的组快照 → 前端视图形状（epoch_id 字符串化，理由见 GroupView 的注释）。
fn group_view_of(snapshot: &GroupSnapshot) -> GroupView {
    GroupView {
        group_id: snapshot.group_id,
        // 十六进制：既避免 JS 的 53 位精度问题，又与内核日志/文档里的 epoch 写法一致
        epoch_id: format!("{:#x}", snapshot.epoch_id),
        lead_ms: snapshot.lead_ms,
        members: snapshot
            .members
            .iter()
            .map(|member| GroupMemberView {
                id_short: member.id.short(),
                quality: quality_name(member.quality).to_string(),
                offset_us: member.offset_us,
            })
            .collect(),
    }
}

/// §6.5 的质量分级 → 前端可读字符串（契约里 UI 只认这三个词）。
fn quality_name(quality: ClockQuality) -> &'static str {
    match quality {
        ClockQuality::Good => "good",
        ClockQuality::Fair => "fair",
        ClockQuality::Poor => "poor",
    }
}

fn resolve_peer(engine: &Engine, id_short: &str) -> Result<NodeId, CommandError> {
    let trimmed = id_short.trim();
    if trimmed.is_empty() {
        return Err(CommandError::bad_request(
            "缺少设备标识，请先刷新设备列表",
            "resolve_peer: id_short 为空",
        ));
    }
    engine
        .peers()
        .iter()
        .find(|peer| peer.id.short_matches(trimmed))
        .map(|peer| peer.id)
        .ok_or_else(|| {
            CommandError::bad_request(
                "找不到这个设备，可能已经断开，请重新连接",
                format!("resolve_peer: id_short={trimmed}"),
            )
        })
}

fn find_view(peers: &[PeerView], id: NodeId) -> Option<PeerView> {
    let short = id.short();
    peers.iter().find(|peer| peer.id_short == short).cloned()
}

/// 会话状态的中文短语（错误文案里用）。
fn state_text(peer: &PeerStatus) -> &'static str {
    match peer.state {
        SessionState::Idle => "空闲",
        SessionState::Handshaking => "正在连接",
        SessionState::Streaming => "已连接",
        SessionState::Degraded => "网络不稳",
        SessionState::Reconnecting => "正在重连",
        SessionState::Failed => "已断开",
    }
}

/// 解析用户输入：接受 `ip`、`ip:port`、`[v6]:port`。
///
/// 为什么放宽到"裸 IP"：用户从路由器/手机设置里抄到的就是一个 IP，
/// 让他手打 `:58290` 是没必要的负担；端口用协议默认值（`docs/03-protocol.md` §1）。
/// 主机名不解析：DNS 属发现层的活，M1 不做（也不该在 UI 缝里偷偷做）。
fn parse_endpoint(raw: &str) -> Result<SocketAddr, CommandError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CommandError::bad_request(
            "请先填写对方的 IP 地址",
            "connect: addr 为空",
        ));
    }
    if let Ok(socket_addr) = trimmed.parse::<SocketAddr>() {
        return Ok(socket_addr);
    }
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_QUIC_PORT));
    }
    Err(CommandError::bad_request(
        format!(
            "地址格式不正确：「{trimmed}」。请填 IP（M1 只支持 IP 直连），例如 192.168.1.23 或 192.168.1.23:{DEFAULT_QUIC_PORT}"
        ),
        format!("connect: addr={trimmed}"),
    ))
}

/// 引擎错误 → 前端可读错误（码原样保留，文案换成人话）。
fn engine_error(command: &str, error: &AudioLinkError) -> CommandError {
    engine_error_with_message(command, error, human_message(error.code()))
}

/// 同上，但覆盖文案（按错误码给出更具体的处置指引，见 `start_send` 的两个分支）。
fn engine_error_with_message(command: &str, error: &AudioLinkError, message: &str) -> CommandError {
    CommandError::new(
        error.code(),
        message,
        format!("{command}: {} ({})", error.code().name(), error.context()),
    )
}

/// 错误码 → 人话（UI 规格 §4：不允许出现"未知错误"）。
///
/// 覆盖式 `match` 是刻意的：内核加了新错误码，这里会**编译失败**，逼着补齐文案，
/// 而不是让用户看到一句英文的 `context`。
fn human_message(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::VersionMismatch => "对方版本过旧，请两台设备升级到同一版本",
        ErrorCode::NoPeer => "找不到这个设备，可能已经断开，请重新连接",
        ErrorCode::CapUnsupported => "对方不支持这个能力",
        ErrorCode::StreamLimit => "同时推流的路上限已满，请先停止一路",
        ErrorCode::CodecUnsupported => "双方没有共同支持的编码，无法建立音频流",
        ErrorCode::BadRequest => "这次操作没能完成，详情见下方上下文",
        ErrorCode::Busy => "上一步还没完成，请稍候再试",
        ErrorCode::PlayoutUnderrun | ErrorCode::SinkRebuild | ErrorCode::CaptureLost => {
            "音频设备出现问题，正在自动重建"
        }
        ErrorCode::RateLimited => "操作太频繁，请稍后再试",
    }
}

/// 把遥测 CSV 写进 `dir`（文件名带毫秒时间戳），返回落盘路径。
///
/// 与命令层分离的理由：命令要 `AppHandle`（没法单测），而「文件真的写出来了、内容对不对」
/// 恰恰是导出功能最该被测到的一半。
fn write_telemetry_csv(
    dir: &std::path::Path,
    rows: &[TelemetryRow],
) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("telemetry-{}.csv", unix_millis()));
    std::fs::write(&path, render_telemetry_csv(rows))?;
    Ok(path)
}

/// 遥测导出的目录：`<用户目录>\AudioLink`（取不到用户目录时退回当前目录下的 `audiolink-export`）。
fn telemetry_export_dir() -> std::path::PathBuf {
    let base = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("AudioLink")
}

/// 当前 Unix 毫秒（导出文件名用）。
fn unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
// 测试里断言失败就该炸：显式放行 panic 系列 lint。
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use audiolink_engine::GroupMember;

    /// 自动推流开关的默认值必须是**开**：只有明确写进 settings.json 的 `false` 才算「用户关了」。
    ///
    /// 改坏（例如照抄 auto_connect 的 `unwrap_or(false)`）→ 键缺失就变成关，
    /// 「连上就有声」这个默认体验静默消失，而这正是本任务要交付的东西。
    #[test]
    fn auto_broadcast_is_on_unless_explicitly_disabled() {
        // 键缺失（全新安装 / 用户从没碰过设置）→ 开。
        assert!(auto_broadcast_from(None));
        // 写进去什么就读回什么：`set_auto_broadcast` 存的正是这个 Bool。
        assert!(auto_broadcast_from(Some(&serde_json::json!(true))));
        assert!(!auto_broadcast_from(Some(&serde_json::json!(false))));
        // 手改 settings.json 写成非布尔值：按默认值（开）走，而不是猜用户想关。
        assert!(auto_broadcast_from(Some(&serde_json::json!("no"))));
        assert!(auto_broadcast_from(Some(&serde_json::json!(0))));
        assert!(auto_broadcast_from(Some(&serde_json::Value::Null)));
    }

    #[test]
    fn group_view_keeps_the_epoch_as_a_string_and_shortens_members() {
        let member = NodeId::from_bytes([0xAB; 32]);
        let snapshot = GroupSnapshot {
            group_id: 7,
            epoch_id: 0x1122_3344_5566_7788,
            lead_ms: 120,
            members: vec![GroupMember {
                id: member,
                quality: ClockQuality::Poor,
                offset_us: Some(-2_500),
            }],
        };
        let view = group_view_of(&snapshot);
        assert_eq!(view.group_id, 7);
        assert_eq!(
            view.epoch_id, "0x1122334455667788",
            "u64 必须以字符串出网，否则前端会丢精度"
        );
        assert_eq!(view.lead_ms, 120);
        assert_eq!(view.members.len(), 1);
        assert_eq!(
            view.members.first().map(|m| m.id_short.as_str()),
            Some(member.short().as_str())
        );
        // §7：质量随成员一起出网，UI 才有依据明示「该设备同步质量差」
        assert_eq!(
            view.members.first().map(|m| m.quality.as_str()),
            Some("poor")
        );
        assert_eq!(view.members.first().and_then(|m| m.offset_us), Some(-2_500));
    }
    use crate::view::TELEMETRY_CSV_HEADER;
    use audiolink_types::StreamStats;

    /// 导出：文件真的落盘、首行是表头、一行一个采样点。
    #[test]
    fn export_writes_a_csv_file_with_one_row_per_sample() {
        let dir =
            std::env::temp_dir().join(format!("audiolink-export-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let rows = [TelemetryRow::from_view(
            1_700_000_000_000,
            &TelemetryView::zeros(1),
        )];
        let path = write_telemetry_csv(&dir, &rows).expect("写导出文件");
        let text = std::fs::read_to_string(&path).expect("读回导出文件");

        assert!(text.starts_with(TELEMETRY_CSV_HEADER), "首行必须是表头");
        assert_eq!(text.lines().count(), 2, "表头 + 1 行采样点");
        assert!(
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("telemetry-")),
            "文件名要带时间戳：{}",
            path.display()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_endpoint_accepts_bare_ip_and_full_addr() {
        assert_eq!(
            parse_endpoint(" 192.168.1.23 ").expect("bare ip"),
            format!("192.168.1.23:{DEFAULT_QUIC_PORT}")
                .parse::<SocketAddr>()
                .expect("addr")
        );
        assert_eq!(
            parse_endpoint("192.168.1.23:6000")
                .expect("ip:port")
                .to_string(),
            "192.168.1.23:6000"
        );
        assert_eq!(
            parse_endpoint("[::1]:6000").expect("v6").to_string(),
            "[::1]:6000"
        );
        assert!(parse_endpoint("").is_err());
        assert!(parse_endpoint("客厅的电脑").is_err());
        // 主机名不在 M1 支持范围（引擎收的是 SocketAddr）→ 必须明确报错，
        // 不能"善意"地换成 127.0.0.1 或默认路由。
        assert!(parse_endpoint("desktop.local:58290").is_err());
    }

    /// 状态映射是契约与引擎之间最容易悄悄错位的地方：这里逐条钉死。
    #[test]
    fn session_state_maps_onto_contract_states() {
        // 未推流时，引擎的 Streaming 只是"会话已建立"，对用户不是"推送中"
        assert_eq!(map_state(SessionState::Streaming, false), PeerState::Idle);
        assert_eq!(
            map_state(SessionState::Streaming, true),
            PeerState::Streaming
        );
        assert_eq!(map_state(SessionState::Idle, true), PeerState::Idle);
        assert_eq!(
            map_state(SessionState::Handshaking, false),
            PeerState::Handshaking
        );
        assert_eq!(
            map_state(SessionState::Degraded, false),
            PeerState::Degraded
        );
        // 契约 §6 没有 reconnecting → 归入 degraded
        assert_eq!(
            map_state(SessionState::Reconnecting, false),
            PeerState::Degraded
        );
        assert_eq!(map_state(SessionState::Failed, false), PeerState::Failed);
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        assert_eq!(percentile(&[], 50), None);
        assert_eq!(percentile(&[7], 50), Some(7));
        assert_eq!(percentile(&[7], 95), Some(7));
        let samples: Vec<u32> = (1..=100).collect();
        assert_eq!(percentile(&samples, 50), Some(50));
        assert_eq!(percentile(&samples, 95), Some(95));
        assert_eq!(percentile(&samples, 100), Some(100));
        // 小样本：n=2 时 P50 取第 1 个、P95 取第 2 个（最近秩的定义）
        assert_eq!(percentile(&[10, 20], 50), Some(10));
        assert_eq!(percentile(&[10, 20], 95), Some(20));
    }

    /// 每个错误码都必须有人话文案：漏一个就等于把"未知错误"交回给用户。
    #[test]
    fn every_error_code_has_a_human_message() {
        let codes = [
            ErrorCode::VersionMismatch,
            ErrorCode::NoPeer,
            ErrorCode::CapUnsupported,
            ErrorCode::StreamLimit,
            ErrorCode::CodecUnsupported,
            ErrorCode::BadRequest,
            ErrorCode::Busy,
            ErrorCode::PlayoutUnderrun,
            ErrorCode::SinkRebuild,
            ErrorCode::CaptureLost,
            ErrorCode::RateLimited,
        ];
        for code in codes {
            assert!(!human_message(code).is_empty(), "code {code:?} 没有文案");
        }
    }

    fn quality_status(id: u8, receiving: bool, loss: u16) -> PeerStatus {
        PeerStatus {
            id: NodeId::from_bytes([id; 32]),
            initiated_locally: receiving,
            reconnects: 0,
            capabilities: None,
            name: format!("peer-{id}"),
            addr: "127.0.0.1:58290".parse().expect("addr"),
            state: SessionState::Streaming,
            stats: StreamStats {
                rtt_us: 12_000,
                bitrate_bps: 160_000,
                loss_pct_x100: loss,
                ..Default::default()
            },
        }
    }

    #[test]
    fn quality_does_not_require_end_to_end_measurement() {
        let statuses = [quality_status(1, true, 250)];
        let view = aggregate(&statuses, None, &mut HashMap::new(), &HashMap::new());
        assert!(view.receiver_report);
        assert_eq!(view.loss_pct, 2.5);
        assert_eq!(view.rtt_us, 12_000);
        assert_eq!(view.e2e_latency_us, 0);
    }

    #[test]
    fn sender_uses_the_matching_receivers_report_including_zero_loss() {
        let statuses = [quality_status(1, false, 0), quality_status(2, false, 0)];
        let reports = HashMap::from([
            (
                statuses[0].id,
                StreamStats {
                    loss_pct_x100: 375,
                    bitrate_bps: 160_000,
                    underruns: 4,
                    ..Default::default()
                },
            ),
            (
                statuses[1].id,
                StreamStats {
                    loss_pct_x100: 0,
                    bitrate_bps: 160_000,
                    ..Default::default()
                },
            ),
        ]);
        let mut windows = HashMap::new();
        let first = aggregate(&statuses, Some(statuses[0].id), &mut windows, &reports);
        assert_eq!(first.loss_pct, 3.75);
        assert_eq!(first.underruns, 4);
        let second = aggregate(&statuses, Some(statuses[1].id), &mut windows, &reports);
        assert_eq!(second.loss_pct, 0.0);
        assert!(second.receiver_report);
        let worst = aggregate(&statuses, None, &mut windows, &reports);
        assert_eq!(worst.loss_pct, 3.75);
        let missing = aggregate(
            &statuses,
            Some(statuses[0].id),
            &mut windows,
            &HashMap::new(),
        );
        assert!(!missing.receiver_report, "尚未上报或过期不能冒充零丢包");
        assert_eq!(missing.rtt_us, 12_000, "RTT 仍来自本机 QUIC 测量");
    }

    #[test]
    fn idle_peer_does_not_keep_previous_quality_or_latency_history() {
        let mut statuses = [quality_status(1, true, 0)];
        statuses[0].stats.e2e_latency_us = 62_000;
        let mut windows = HashMap::new();
        let measured = aggregate(&statuses, None, &mut windows, &HashMap::new());
        assert_eq!(measured.e2e_p50_us, 62_000);
        statuses[0].stats.e2e_latency_us = 0;
        let unmeasured = aggregate(&statuses, None, &mut windows, &HashMap::new());
        assert_eq!(unmeasured.e2e_p50_us, 0);
        statuses[0].state = SessionState::Idle;
        let idle = aggregate(
            &statuses,
            Some(statuses[0].id),
            &mut windows,
            &HashMap::new(),
        );
        assert!(!idle.receiver_report);
        assert_eq!(idle.bitrate_bps, 0);
    }

    /// 视图字段口径：短码必须是内核定义的 `NodeId::short()`（16 hex），
    /// 不能由外壳自己截字符串 —— `resolve_peer` 正是靠 `short_matches` 反查回 `NodeId` 的，
    /// 两边口径一旦分叉，"点按钮找不到设备"会变成常态。
    #[test]
    fn peer_view_uses_canonical_short_id() {
        let id = NodeId::from_bytes([0xab; 32]);
        let status = PeerStatus {
            initiated_locally: true,
            id,
            reconnects: 0,
            name: "客厅 R1".to_string(),
            addr: "192.168.1.23:58290".parse().expect("addr"),
            state: SessionState::Streaming,
            stats: StreamStats::default(),
            capabilities: None,
        };

        let view = peer_view(&status, Some(id), None);
        assert_eq!(view.id_short, id.short());
        assert!(view.receiving);
        assert_eq!(view.id_short.len(), 16);
        assert!(
            id.short_matches(&view.id_short),
            "短码必须能反查回同一个 NodeId"
        );
        assert_eq!(view.state, PeerState::Streaming);
        assert_eq!(view.addr, "192.168.1.23:58290");

        // 同一个引擎状态、但本机没在推流 → 呈现为 idle（卡片仍在，按钮回到"开始推流"）
        assert_eq!(peer_view(&status, None, None).state, PeerState::Idle);
    }

    /// 协商帧长照原样进视图：内核给什么界面就显示什么。
    ///
    /// 改坏：这里若"顺手填个本机档位兜底"（None → 20），界面就会在**没协商**时显示一个
    /// 看起来很正常的数字 —— 而帧长不一致的症状恰恰是"遥测全绿但声音发闷"，
    /// 一个假数字会把唯一的直接读数变成误导。
    #[test]
    fn peer_view_passes_negotiated_frame_ms_through_without_a_fallback() {
        let id = NodeId::from_bytes([0xcd; 32]);
        let status = PeerStatus {
            initiated_locally: false,
            id,
            reconnects: 0,
            name: "手机".to_string(),
            addr: "192.168.1.31:58290".parse().expect("addr"),
            state: SessionState::Streaming,
            stats: StreamStats::default(),
            capabilities: None,
        };

        assert_eq!(
            peer_view(&status, Some(id), Some(10)).negotiated_frame_ms,
            Some(10),
            "本端作接收端、对端报 10 ms → 视图必须如实报 10"
        );
        assert_eq!(
            peer_view(&status, Some(id), None).negotiated_frame_ms,
            None,
            "还没协商 / 本端作发送端 → 必须是 None（界面据此显示占位符，不能兜底成 20）"
        );
    }
}
