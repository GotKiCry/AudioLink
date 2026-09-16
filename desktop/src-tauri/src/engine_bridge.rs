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
//! 1. **`connect` 返回 `1002 NOT_PAIRED` 不是失败**：会话与命令通道仍然活着
//!    （见 `Engine::connect` 的文档）。这里把它翻成"请对方念配对码"，并**刷新对端列表**
//!    让卡片先出现；PIN 由 `EngineEvent::PinNeeded` / `DisplayPin` 转成前端事件。
//! 2. **引擎的 `Streaming` ≠ 契约的 `streaming`**：引擎在握手完成时就把会话置为
//!    `Streaming`（`runtime.rs::mark_streaming`），而契约的 `streaming` 对用户意味着"正在推流"。
//!    只有外壳知道本机有没有在推流（`start_send`/`stop_send` 不产生引擎状态迁移），
//!    所以用 `Cache::sending` 打标：**引擎 Streaming + 未推流 → 视图 `idle`**
//!    （卡片不消失，按钮回到"开始推流"）。
//! 3. **引擎有 6 个会话状态，契约 §6 只有 5 个**（缺 `reconnecting`）→ 归入 `degraded`
//!    （"琥珀 / 重连中"语义最近）。这是契约待补的一处，已在汇报里点名。
//! 4. **`submit_pin` 必须等结果**：`Engine::submit_pin` 只是把命令投给会话任务并立刻返回，
//!    真正的判定以 `EngineEvent::PairCompleted` 回来；因此这里先订阅、再提交、再等事件。
//! 5. **遥测源是引擎的 1 Hz**：`StreamStats` 没有分位数，契约却要 `e2eP50Us/e2eP95Us`，
//!    所以外壳按对端维护滑动窗口自己算（见 `aggregate`）。这一层只保证"不快于 500 ms 一次"。

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use audiolink_engine::{Engine, EngineConfig, EngineEvent, PeerStatus, SessionState};
use audiolink_types::{AudioLinkError, DEFAULT_QUIC_PORT, ErrorCode, NodeId, StreamStats};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{broadcast, watch};

use crate::capture::{self, SharedCapture};
use crate::error::CommandError;
use crate::view::{
    CaptureDeviceView, LocalStatus, PairRequiredPayload, PeerState, PeerView, StartSendResult,
    SubmitPinResult, TelemetryView,
};

// ---------------------------------------------------------------------------
// 契约常量（`docs/11-m1-contract.md` §6，冻结：名字不许改）
// ---------------------------------------------------------------------------

/// 对端列表变化事件，载荷 `PeerView[]`。
pub const EVENT_PEER: &str = "audiolink://peer";
/// 遥测事件，载荷 `TelemetryView`；**不快于 500 ms 一次**。
pub const EVENT_TELEMETRY: &str = "audiolink://telemetry";
/// 配对请求事件，载荷 `{ idShort, name, pin }`。
///
/// 两个方向共用这一个事件（契约只冻结了一个名字）：
/// * `pin` 非空 → **本机是接收端**：把码亮给用户，让对方照着输入（`EngineEvent::DisplayPin`）；
/// * `pin` 为空 → **本机是发起端**：请用户输入对方屏幕上显示的码（`EngineEvent::PinNeeded`）。
pub const EVENT_PAIR_REQUIRED: &str = "audiolink://pair-required";

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

/// 等配对结果（`PairCompleted`）的上限。引擎握手自己有死线，这里只做兜底，
/// 保证 UI 不会永远转圈。
const PAIR_OUTCOME_TIMEOUT: Duration = Duration::from_secs(15);

/// 对端上报遥测的新鲜度上限。超过它就不再采信 —— 链路断了却继续显示 3 秒前的"漂亮数字"
/// 比显示"没有数据"更糟。
const PEER_REPORT_TTL: Duration = Duration::from_secs(3);

/// WASAPI 缓冲请求值（ms）。20 ms 是本仓工具链的既有取值：
/// 共享模式下小于一个周期的请求会被抬到 1056 帧 = 22 ms（`wasapi/mod.rs` 的实测表），
/// 所以填 20 与填 1 的实际效果一样，填 20 至少让意图可读。
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
    /// 串行化开始/停止：更换端点与开流必须属于同一次操作。
    send_operation: tokio::sync::Mutex<()>,
}

impl EngineBridge {
    /// 建立接缝并**异步**启动引擎（启动结果通过 watch 通道广播给命令层）。
    pub fn new(app: AppHandle) -> Self {
        let (state_tx, state_rx) = watch::channel(EngineState::Starting);
        let cache = Arc::new(Mutex::new(Cache::default()));
        let capture = SharedCapture::default();
        // 发送端交给启动任务持有：它的生命周期就是应用的生命周期，
        // 命令层只在"还没就绪"时才会去等这个通道。
        tauri::async_runtime::spawn(boot(
            app.clone(),
            state_tx,
            Arc::clone(&cache),
            Arc::clone(&capture),
        ));
        Self {
            app,
            state_rx,
            cache,
            capture,
            send_operation: tokio::sync::Mutex::new(()),
        }
    }

    /// 本机身份（`local_status`）。
    pub async fn local_status(&self) -> Result<LocalStatus, CommandError> {
        let engine = self.engine().await?;
        let info = engine.info();
        Ok(LocalStatus {
            // 短码口径由内核唯一确定（`NodeId::short()` = 指纹前 8 字节的 hex），
            // 外壳**不自己截字符串**：否则 UI 上的码会与协议/发现报文里的码悄悄分叉。
            id_short: info.id.short(),
            name: info.name.clone(),
            addr: engine.local_addr().to_string(),
            platform: info.platform.as_str().to_string(),
        })
    }

    /// 已连接/已登记对端（`list_peers`）。
    pub async fn list_peers(&self) -> Result<Vec<PeerView>, CommandError> {
        let engine = self.engine().await?;
        let (peers, _) = refresh(&engine, &self.cache);
        Ok(peers)
    }

    /// 手工 IP 连接（`connect`，FR-17）。
    ///
    /// 返回"已登记 + 正在握手/配对"的对端视图；配对请求与后续状态由事件推进。
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
                let (peers, _) = refresh(&engine, &self.cache);
                emit_peer(&self.app, &self.cache, &peers);
                find_view(&peers, peer_id).ok_or_else(|| {
                    CommandError::bad_request(
                        "连接成功了，但对端没出现在会话表里",
                        format!("connect: peer={}", peer_id.short()),
                    )
                })
            }
            Err(error) if error.code() == ErrorCode::NotPaired => {
                // 刻意不当成失败：会话与控制通道还活着（`Engine::connect` 文档）。
                // 刷新列表让卡片出现；前端收到 1002 后应引导用户去输配对码，而不是重连。
                let (peers, _) = refresh(&engine, &self.cache);
                emit_peer(&self.app, &self.cache, &peers);
                Err(engine_error_with_message(
                    "connect",
                    &error,
                    "对方要求配对：请输入对方屏幕上显示的 6 位配对码",
                ))
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

    /// 提交 6 位配对码（`submit_pin`）。
    ///
    /// **等真实结果**：引擎的 `submit_pin` 是"投递命令"，判定走 `PairCompleted` 事件。
    /// 契约要求返回 `{ ok, reason }`，所以这里订阅事件再等 —— 否则 UI 只能显示"已提交"，
    /// 而用户真正要知道的是"配对成不成、还能试几次"。
    pub async fn submit_pin(
        &self,
        id_short: &str,
        pin: &str,
    ) -> Result<SubmitPinResult, CommandError> {
        let engine = self.engine().await?;
        let peer = resolve_peer(&engine, id_short)?;

        // 先订阅再投递：广播通道不回放历史，反过来做会漏掉秒回的结果。
        let mut events = engine.subscribe();
        engine
            .submit_pin(peer, pin)
            .await
            .map_err(|error| engine_error("submit_pin", &error))?;

        let deadline = Instant::now() + PAIR_OUTCOME_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(timeout_result());
            }
            match tokio::time::timeout(remaining, events.recv()).await {
                Err(_) => return Ok(timeout_result()),
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Ok(SubmitPinResult {
                        ok: false,
                        reason: "引擎事件通道已关闭，请重启应用".to_string(),
                    });
                }
                // 事件积压只会发生在 UI 不消费时；PIN 判定必须继续等。
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Ok(EngineEvent::PairCompleted { id, ok, reason })) if id == peer => {
                    let (peers, _) = refresh(&engine, &self.cache);
                    emit_peer(&self.app, &self.cache, &peers);
                    return Ok(SubmitPinResult {
                        ok,
                        // 成功后不需要解释；失败原因由引擎给出
                        // （已是中文整句，含剩余尝试次数 / 锁定剩余秒数）。
                        reason: if ok { String::new() } else { reason },
                    });
                }
                Ok(Ok(EngineEvent::PeerDisconnected { id, reason })) if id == peer => {
                    let (peers, _) = refresh(&engine, &self.cache);
                    emit_peer(&self.app, &self.cache, &peers);
                    return Ok(SubmitPinResult {
                        ok: false,
                        reason: format!("连接已断开（{reason}），请重新连接"),
                    });
                }
                // 其余事件与本命令无关，继续等（不发散：PIN 结果只有这一条路）。
                Ok(Ok(_)) => continue,
            }
        }
    }

    /// 当前遥测快照（`telemetry`）。
    pub async fn telemetry(&self) -> Result<TelemetryView, CommandError> {
        let engine = self.engine().await?;
        let (_, view) = refresh(&engine, &self.cache);
        Ok(view)
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

/// 等配对结果超时。
fn timeout_result() -> SubmitPinResult {
    SubmitPinResult {
        ok: false,
        reason: "等对方确认超时，请重试".to_string(),
    }
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
                // 遥测：可能是本机 1 Hz 采样，也可能是对端 `STREAM_STATS` 的透传。
                // 只有 e2e > 0 的那种才是"接收侧测得的量"，留作本机推流时的展示来源。
                EngineEvent::Telemetry(stats) => {
                    if stats.e2e_latency_us > 0 {
                        lock(&cache).peer_report = Some((*stats, Instant::now()));
                    }
                    let (peers, view) = refresh(&engine, &cache);
                    emit_peer(&app, &cache, &peers);
                    emit_telemetry(&app, &cache, &view);
                }
                // 本机是发起端：请用户输入对端屏幕上的码（pin 留空表达这一点）。
                EngineEvent::PinNeeded { id, name } => emit(
                    &app,
                    EVENT_PAIR_REQUIRED,
                    &PairRequiredPayload {
                        id_short: id.short(),
                        name,
                        pin: String::new(),
                    },
                ),
                // 本机是接收端：把码亮给用户（对方要照着输入）。
                EngineEvent::DisplayPin {
                    from, name, pin, ..
                } => emit(
                    &app,
                    EVENT_PAIR_REQUIRED,
                    &PairRequiredPayload {
                        id_short: from.short(),
                        name,
                        pin,
                    },
                ),
                // 配对结果的"等待"在 `submit_pin` 里做；这里只同步一次状态。
                EngineEvent::PairCompleted { .. } => {
                    let (peers, _) = refresh(&engine, &cache);
                    emit_peer(&app, &cache, &peers);
                }
                // §8 自适应码率生效：目前只记日志 —— 面板上的可视化属于 M2 的「遥测面板」那一项。
                EngineEvent::CodecAdapted {
                    from_bps,
                    to_bps,
                    reason,
                } => {
                    tracing::info!(from_bps, to_bps, reason = %reason, "自适应码率变更");
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

/// 引擎配置：身份/信任库落盘位置 + QUIC 监听 + 音频工厂。
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
    // 身份材料在 `<config>/identity/`，信任库在 `<config>/trust.json`（架构 §9 的分工）。
    config.trust_store_path = dir.join("trust.json");
    config.listen = SocketAddr::from(([0, 0, 0, 0], DEFAULT_QUIC_PORT));

    // 音频工厂：`CaptureSource` / `PlayoutSink` 都是 `!Send`（WASAPI 对象绑线程），
    // 所以**不能**在这里把对象建好 —— 只能交出"怎么建"，由音频线程自己调用。
    #[cfg(windows)]
    {
        config.capture = Some(capture::factory(capture));
        config.playout = Some(playout_factory());
    }

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
    /// 最近一条**由对端上报**的遥测。
    ///
    /// 为什么要它：端到端延迟与播放水位是**接收侧**才量得到的量 —— 本机推流时自己的
    /// `StreamStats.e2e_latency_us` 恒为 0，面板要显示就必须用对端报回来的那一份。
    /// 数据来源是引擎把对端的 `STREAM_STATS` 帧透传成的 `EngineEvent::Telemetry`
    /// （见 `runtime.rs` 的 `ControlRequest::StreamStats` 分支）。
    ///
    /// 已知局限：该事件**不带对端 id**，所以这里只有一个槽位 —— 多对端同时上报时无法归属
    /// （见汇报里的契约缺口）。M1 单路推流下够用。
    peer_report: Option<(StreamStats, Instant)>,
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
            peer_report: None,
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

    let peers: Vec<PeerView> = statuses
        .iter()
        .map(|status| peer_view(status, sending))
        .collect();
    let peer_report = guard.peer_report;
    let telemetry = aggregate(&statuses, sending, &mut guard.e2e_windows, peer_report);

    guard.peers = peers.clone();
    guard.telemetry = telemetry.clone();
    (peers, telemetry)
}

/// `PeerStatus` → 契约视图。
fn peer_view(status: &PeerStatus, sending: Option<NodeId>) -> PeerView {
    PeerView {
        id_short: status.id.short(),
        name: status.name.clone(),
        addr: status.addr.to_string(),
        state: map_state(status.state, sending == Some(status.id)),
        trusted: status.trusted,
    }
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
/// 选谁做展示对象：优先当前推流目标；否则取 e2e 最差的一路。
/// 取最差而不是平均 —— 这个面板是"体检表"，平均值会把一路正在爆的链路藏起来。
///
/// 用哪份数字：本机是接收侧（`e2e_latency_us > 0`，说明这条流在本机测过）时用本机统计，
/// 否则用对端上报的那一份（推流方向上，水位/欠载/端到端只有接收侧量得到）。
fn aggregate(
    statuses: &[PeerStatus],
    sending: Option<NodeId>,
    windows: &mut HashMap<NodeId, VecDeque<u32>>,
    peer_report: Option<(StreamStats, Instant)>,
) -> TelemetryView {
    let peers = statuses.len() as u32;

    let focus = sending
        .and_then(|id| statuses.iter().find(|status| status.id == id))
        .or_else(|| {
            statuses
                .iter()
                .filter(|status| status.stats.e2e_latency_us > 0)
                .max_by_key(|status| status.stats.e2e_latency_us)
        });
    let Some(focus) = focus else {
        return TelemetryView::zeros(peers);
    };

    let local = &focus.stats;
    let reported = peer_report
        .filter(|(stats, at)| stats.e2e_latency_us > 0 && at.elapsed() < PEER_REPORT_TTL)
        .map(|(stats, _)| stats);
    let stats = if local.e2e_latency_us > 0 {
        local
    } else {
        reported.as_ref().unwrap_or(local)
    };

    let window = windows.entry(focus.id).or_default();
    // 只记"有流"的样本：空闲时的 0 会把分位数一路拖平，读起来像"网络突然变好了"。
    if stats.e2e_latency_us > 0 {
        window.push_back(stats.e2e_latency_us);
        while window.len() > E2E_WINDOW {
            let _dropped = window.pop_front();
        }
    }
    let mut sorted: Vec<u32> = window.iter().copied().collect();
    sorted.sort_unstable();
    let p50 = percentile(&sorted, 50).unwrap_or(stats.e2e_latency_us);
    let p95 = percentile(&sorted, 95).unwrap_or(stats.e2e_latency_us);

    TelemetryView {
        peers,
        rtt_us: u64::from(stats.rtt_us),
        jitter_us: u64::from(stats.jitter_us),
        // 内核用 ×100 的整数存百分比（避免浮点），契约要百分数 → 这里还原。
        loss_pct: f64::from(stats.loss_pct_x100) / 100.0,
        bitrate_bps: u64::from(stats.bitrate_bps),
        buffer_level_us: u64::from(stats.buffer_level_us),
        underruns: u64::from(stats.underruns),
        e2e_latency_us: u64::from(stats.e2e_latency_us),
        e2e_p50_us: u64::from(p50),
        e2e_p95_us: u64::from(p95),
    }
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
        SessionState::Handshaking => "正在配对",
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

/// 同上，但覆盖文案（用于 `1002` 这种"不是失败、而是要用户做点什么"的情形）。
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
        ErrorCode::NotPaired => "该设备还没配对：请输入对方屏幕上显示的 6 位配对码",
        ErrorCode::PairRejected => "配对码不正确或已过期，请重新输入",
        ErrorCode::AuthFailed => "安全校验未通过（对方证书签名不匹配），已拒绝对接",
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

#[cfg(test)]
// 测试里断言失败就该炸：显式放行 panic 系列 lint。
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use audiolink_types::StreamStats;

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
            ErrorCode::NotPaired,
            ErrorCode::PairRejected,
            ErrorCode::AuthFailed,
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

    /// 聚合的取景规则：优先推流目标；没有目标时取"最差一路"而不是平均。
    #[test]
    fn aggregate_prefers_send_target_and_worst_link() {
        let mut windows: HashMap<NodeId, VecDeque<u32>> = HashMap::new();
        let empty = aggregate(&[], None, &mut windows, None);
        assert_eq!(empty.peers, 0);
        assert_eq!(empty.e2e_latency_us, 0);

        let far = NodeId::from_bytes([2u8; 32]);
        let near = NodeId::from_bytes([1u8; 32]);
        let status = |id: NodeId, e2e: u32| PeerStatus {
            id,
            name: format!("peer-{}", id.short()),
            addr: "127.0.0.1:58290".parse().expect("addr"),
            state: SessionState::Streaming,
            trusted: true,
            stats: StreamStats {
                e2e_latency_us: e2e,
                ..Default::default()
            },
        };
        let peers = vec![status(near, 60_000), status(far, 200_000)];

        // 没有推流目标 → 取最差一路
        let worst = aggregate(&peers, None, &mut windows, None);
        assert_eq!(worst.peers, 2);
        assert_eq!(worst.e2e_latency_us, 200_000);

        // 指定推流目标 → 以目标为准（即使它不是最差的一路）
        let focused = aggregate(&peers, Some(near), &mut windows, None);
        assert_eq!(focused.e2e_latency_us, 60_000);

        // 窗口按对端分开累计：near 与 far 各 1 条，不会互相污染
        assert_eq!(windows.get(&near).map(|w| w.len()), Some(1));
        assert_eq!(windows.get(&far).map(|w| w.len()), Some(1));
    }

    /// 推流方向：本机 e2e 恒为 0（测量点在接收侧），面板必须改用对端上报的那一份；
    /// 报告过期就退回本机统计，绝不让过期数字继续冒充"当前值"。
    #[test]
    fn aggregate_uses_peer_report_when_local_cannot_measure() {
        let mut windows: HashMap<NodeId, VecDeque<u32>> = HashMap::new();
        let peer = NodeId::from_bytes([7u8; 32]);
        let sender_side = vec![PeerStatus {
            id: peer,
            name: "phone".to_string(),
            addr: "192.168.1.23:58290".parse().expect("addr"),
            state: SessionState::Streaming,
            trusted: true,
            // 发送侧：只量得到 RTT 与码率，e2e / 水位 / 欠载都是 0
            stats: StreamStats {
                rtt_us: 3_200,
                bitrate_bps: 160_000,
                ..Default::default()
            },
        }];

        // 没有对端上报 → 本机统计（e2e = 0，UI 会显示 "—"）
        let local_only = aggregate(&sender_side, Some(peer), &mut windows, None);
        assert_eq!(local_only.rtt_us, 3_200);
        assert_eq!(local_only.e2e_latency_us, 0);

        // 有新鲜的对端上报 → 用对端的（这才是接收侧测得的 62 ms）
        let report = StreamStats {
            e2e_latency_us: 62_000,
            buffer_level_us: 38_000,
            underruns: 1,
            ..Default::default()
        };
        let merged = aggregate(
            &sender_side,
            Some(peer),
            &mut windows,
            Some((report, Instant::now())),
        );
        assert_eq!(merged.e2e_latency_us, 62_000);
        assert_eq!(merged.buffer_level_us, 38_000);
        assert_eq!(merged.underruns, 1);
        // 分位数窗口也吃到了这条样本
        assert_eq!(merged.e2e_p50_us, 62_000);
        assert_eq!(merged.e2e_p95_us, 62_000);

        // 报告过期（超过 TTL）→ 不再采信，退回本机统计
        let stale = aggregate(
            &sender_side,
            Some(peer),
            &mut windows,
            Some((
                report,
                Instant::now() - PEER_REPORT_TTL - Duration::from_secs(1),
            )),
        );
        assert_eq!(stale.e2e_latency_us, 0);
    }

    /// 视图字段口径：短码必须是内核定义的 `NodeId::short()`（16 hex），
    /// 不能由外壳自己截字符串 —— `resolve_peer` 正是靠 `short_matches` 反查回 `NodeId` 的，
    /// 两边口径一旦分叉，"点按钮找不到设备"会变成常态。
    #[test]
    fn peer_view_uses_canonical_short_id() {
        let id = NodeId::from_bytes([0xab; 32]);
        let status = PeerStatus {
            id,
            name: "客厅 R1".to_string(),
            addr: "192.168.1.23:58290".parse().expect("addr"),
            state: SessionState::Streaming,
            trusted: true,
            stats: StreamStats::default(),
        };

        let view = peer_view(&status, Some(id));
        assert_eq!(view.id_short, id.short());
        assert_eq!(view.id_short.len(), 16);
        assert!(
            id.short_matches(&view.id_short),
            "短码必须能反查回同一个 NodeId"
        );
        assert_eq!(view.state, PeerState::Streaming);
        assert!(view.trusted);
        assert_eq!(view.addr, "192.168.1.23:58290");

        // 同一个引擎状态、但本机没在推流 → 呈现为 idle（卡片仍在，按钮回到"开始推流"）
        assert_eq!(peer_view(&status, None).state, PeerState::Idle);
    }
}
