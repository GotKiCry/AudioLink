//! 引擎运行时：把 `net`（QUIC）与 `audio`（采集/编解码/播放）拼成一条会说话的链路
//!
//! 对外 API 见 `docs/11-m1-contract.md` §5。
//!
//! # 线程模型（`docs/02-architecture.md` §4 的落地）
//!
//! ```text
//! ┌─ 采集线程 ──────────────┐   ┌─ 会话任务（tokio）───────────────┐   ┌─ 播放线程 ───────────┐
//! │ 拥有 CaptureSource      │   │ 拥有 Connection + ControlChannel  │   │ 拥有 PlayoutSink      │
//! │ 组帧 → Opus 编码         │──►│ 收控制帧 → 握手/状态机             │──►│ 一帧一写 sink         │
//! │ 编码帧走 tokio mpsc      │   │ 收数据报 → Opus 解码               │   │ 超时 = 欠载，补静音    │
//! └─────────────────────────┘   └──────────────────────────────────┘   └───────────────────────┘
//! ```
//!
//! **为什么采集与播放必须各在自己的 OS 线程里、且只能由工厂函数创建**：
//! `CaptureSource` / `PlayoutSink` 都是 `!Send`（WASAPI 的 COM 对象线程绑定，
//! 见 `audiolink-audio::source` 的文档）。所以引擎不能持有对象再搬给别的线程，
//! 只能持有**工厂**（`Fn() -> Result<Box<dyn ...>>`），由目标线程自己去建。
//! 这是 ADR-010「内核不持有平台音频线程」的直接推论，不是实现细节。
//!
//! # M1 刻意不做的事（属 M2/M3，做了就是范围蔓延）
//!
//! - **不做抖动缓冲**：播放侧一到即播，队列深度由发送帧率自然形成。代价是网络抖动直接变成欠载，
//!   而这恰好是 M2 要解决的对象 —— M1 先把它**量出来**（`underruns` / `late_drops`）。
//! - **不做时钟同步**：`clock_offset_us` / `drift_ppm` 恒为 0（§6 的估计算法在 `audiolink-net` 里，
//!   接线属 M3）；`buffer_level_us` 是「队列里的帧数 × 帧长」的换算值。
//! - **不做 FEC / 双发 / NACK / 自适应码率**：数据报丢了就丢，只计数。
//!
//! # 实时纪律
//!
//! 采集线程与播放线程的**稳态路径**里没有 `unwrap` / `expect` / 日志 / 堆分配；
//! 错误一律上报并触发状态机迁移，绝不静默停止（架构 §4）。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use audiolink_audio::{
    AudioError, CaptureSource, CodecConfig, FrameChunker, OpusDecoder, OpusEncoder, PlayoutSink,
};
use audiolink_identity::{IdentityError, NodeIdentity, TrustEntry, TrustStore};
use audiolink_net::{AudioLinkEndpoint, Connection, ControlChannel, EndpointConfig, NetError};
use audiolink_proto::{AudioDatagram, AudioDatagramHeader};
use audiolink_types::{
    AudioLinkError, Caps, DATAGRAM_MAX_LEN, DEFAULT_QUIC_PORT, ErrorCode, NodeId, NodeInfo,
    Platform, Ptype, StreamStats,
};
use crossbeam_channel::{Receiver, Sender};
use tokio::sync::{broadcast, mpsc};

use crate::dispatch::{ControlRequest, DispatchStats, dispatch_into};
use crate::format_guard::require_unified_format;
use crate::handshake::{Handshake, HandshakeEvent, HandshakeStep, Outgoing, Role};
use crate::measure::MeasurementTap;
use crate::payload::{
    CloseStreamPayload, CodecPref, OpenStreamAckPayload, OpenStreamPayload, SourceKind,
};
use crate::session::{SessionEvent, SessionMachine, SessionState};
use crate::telemetry::TelemetryAggregator;

/// 采集源工厂：**在采集线程内**调用；返回值全程不跨线程（`CaptureSource` 是 `!Send`）。
///
/// 用 `Arc<dyn Fn>` 而不是 `Box<dyn Fn>`：每路流都要自己建一个采集源，
/// 而 `Box<dyn Fn>` 不可克隆 —— 那样第二个会话就没法再建源了。
pub type CaptureFactory =
    Arc<dyn Fn() -> Result<Box<dyn CaptureSource>, AudioError> + Send + Sync + 'static>;

/// 播放输出工厂：**在播放线程内**调用（理由同上）。
pub type PlayoutFactory =
    Arc<dyn Fn() -> Result<Box<dyn PlayoutSink>, AudioError> + Send + Sync + 'static>;

/// 播放队列深度（帧数）。满了丢最旧 —— 与「宁可丢一帧，也不延迟出声」同一条原则（§7 第 1 条）。
const PLAYBACK_QUEUE_FRAMES: usize = 16;

/// 编码帧队列深度（帧数）。采集线程比网络快时，满队列意味着积压 —— 丢最旧而不是阻塞采集。
const ENCODE_QUEUE_FRAMES: usize = 8;

/// 播放**起步前**要攒够的帧数（20 ms 帧下 = 40 ms）。
///
/// 一帧都不攒就开播，任何一次到达抖动都会立刻变成欠载 —— 欠载率会变成 100% 量级，
/// 于是「M1 的欠载数字」既不能反映链路质量，也不能作为 M2 的基线。
/// 这是 M1 的**最小**抖动吸收量；完整的自适应抖动缓冲（15–60 ms 动态深度）属 M2。
const PRIME_FRAMES: usize = 2;

/// 引擎配置。
pub struct EngineConfig {
    /// 用户可见的节点名（仅展示，不参与身份判定）。
    pub node_name: String,
    /// 身份材料目录（`cert.pem` / `key.pem`）。
    pub identity_dir: std::path::PathBuf,
    /// 信任库路径（`trust.json`）。
    pub trust_store_path: std::path::PathBuf,
    /// QUIC 监听地址（`0.0.0.0:DEFAULT_QUIC_PORT` 即全网卡）。
    pub listen: std::net::SocketAddr,
    /// 编码参数（默认 20 ms / 160 kbps / VBR / 48 kHz）。
    pub codec: CodecConfig,
    /// 采集源工厂；纯接收端为 `None`。
    pub capture: Option<CaptureFactory>,
    /// 播放输出工厂；纯发送端为 `None`。
    pub playout: Option<PlayoutFactory>,
    /// 端到端测量探针；仅本机验收使用（见 [`MeasurementTap`] 的文档 —— 跨机不适用）。
    pub measurement: Option<Arc<MeasurementTap>>,
}

impl EngineConfig {
    /// 常用默认值（身份与信任库都落在 `dir` 下，监听全网卡默认端口）。
    pub fn new(node_name: impl Into<String>, dir: impl Into<std::path::PathBuf>) -> Self {
        let dir = dir.into();
        Self {
            node_name: node_name.into(),
            trust_store_path: dir.join("trust.json"),
            identity_dir: dir,
            listen: std::net::SocketAddr::from(([0, 0, 0, 0], DEFAULT_QUIC_PORT)),
            codec: CodecConfig::m1_default(),
            capture: None,
            playout: None,
            measurement: None,
        }
    }
}

/// 对端会话快照（UI 与验收报告的数据源）。
#[derive(Debug, Clone)]
pub struct PeerStatus {
    /// 对端身份（证书指纹）—— 信任判定的唯一依据。
    pub id: NodeId,
    /// 对端展示名（对端自报，**不参与信任判定**）。
    pub name: String,
    /// 对端地址。
    pub addr: std::net::SocketAddr,
    /// 会话状态。
    pub state: SessionState,
    /// 是否已在信任库中。
    pub trusted: bool,
    /// 遥测快照。
    pub stats: StreamStats,
}

/// 引擎事件（UI 订阅；遥测类由外壳按 500 ms 节流转发，见架构 §4）。
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// 对端状态变化（连接 / 状态迁移）。
    PeerUpdated(Box<PeerStatus>),
    /// 对端断开。
    PeerDisconnected {
        /// 对端身份。
        id: NodeId,
        /// 断开原因。
        reason: String,
    },
    /// 本机是接收端：请把 PIN 显示给用户（对端要照着念）。
    DisplayPin {
        /// 申请配对的对端。
        from: NodeId,
        /// 对端展示名。
        name: String,
        /// 6 位码。
        pin: String,
        /// 剩余尝试次数。
        remaining_attempts: u8,
    },
    /// 本机是发起端：需要用户输入对端屏幕上显示的 PIN。
    PinNeeded {
        /// 对端身份。
        id: NodeId,
        /// 对端展示名。
        name: String,
    },
    /// 配对结果。
    PairCompleted {
        /// 对端身份。
        id: NodeId,
        /// 是否成功。
        ok: bool,
        /// 详情（失败时含剩余次数）。
        reason: String,
    },
    /// 遥测（约 1 Hz 产出）。
    Telemetry(Box<StreamStats>),
    /// 会话级错误（不致命；致命路径走 `PeerDisconnected`）。
    Error {
        /// §11 错误码。
        code: u16,
        /// 详情。
        context: String,
    },
}

/// 应用 → 会话任务的指令。
#[derive(Debug, Clone)]
enum SessionCommand {
    /// 向对端开流（发送方向）。
    StartSend,
    /// 关闭当前流（保留连接）。
    CloseStream {
        /// 关闭原因。
        reason: String,
    },
    /// 提交 PIN（发起端）。
    SubmitPin(String),
    /// 关闭整个会话。
    Shutdown,
}

/// 会话表项：控制面与应用共享的句柄。
struct PeerSession {
    id: NodeId,
    name: Mutex<String>,
    addr: std::net::SocketAddr,
    machine: Mutex<SessionMachine>,
    telemetry: Arc<Mutex<TelemetryAggregator>>,
    commands: mpsc::Sender<SessionCommand>,
    trusted: AtomicBool,
    state: Mutex<SessionState>,
}

impl PeerSession {
    fn snapshot(&self) -> PeerStatus {
        PeerStatus {
            id: self.id,
            name: self
                .name
                .lock()
                .map(|n| n.clone())
                .unwrap_or_else(|_| "unknown".into()),
            addr: self.addr,
            state: self
                .state
                .lock()
                .map(|s| *s)
                .unwrap_or(SessionState::Failed),
            trusted: self.trusted.load(Ordering::Relaxed),
            stats: self
                .telemetry
                .lock()
                .map(|t| t.snapshot())
                .unwrap_or_default(),
        }
    }

    fn set_state(&self, state: SessionState) {
        if let Ok(mut current) = self.state.lock() {
            *current = state;
        }
    }

    /// 更新对端展示名（来自 `HELLO.node_info.name`；**仅展示**，不参与信任判定）。
    fn set_name(&self, name: &str) {
        if let Ok(mut current) = self.name.lock() {
            *current = name.to_string();
        }
    }

    fn apply(&self, event: SessionEvent) {
        if let Ok(mut machine) = self.machine.lock() {
            let _ = machine.apply(event);
        }
    }
}

/// 引擎内部共享状态。
struct Inner {
    local: NodeInfo,
    identity: NodeIdentity,
    trust: Mutex<TrustStore>,
    endpoint: AudioLinkEndpoint,
    config: EngineConfig,
    peers: Mutex<HashMap<NodeId, Arc<PeerSession>>>,
    events: broadcast::Sender<EngineEvent>,
    shutdown: AtomicBool,
    listen_addr: std::net::SocketAddr,
}

/// AudioLink 引擎：一个进程一个实例，管理全部对端会话。
pub struct Engine {
    inner: Arc<Inner>,
}

impl Engine {
    /// 启动引擎：加载/生成本机身份与信任库，绑定 QUIC 端点。
    ///
    /// **不**自动开始监听 —— 调用方需显式 [`Engine::spawn_accept_loop`]。
    /// 让「谁在听」这件事在代码里可见，而不是藏在 `start` 的副作用里。
    pub async fn start(config: EngineConfig) -> Result<Arc<Self>, AudioLinkError> {
        let identity = NodeIdentity::load_or_create(&config.identity_dir, &config.node_name)
            .map_err(|e| identity_error(&e))?;
        let trust = TrustStore::load(&config.trust_store_path).map_err(|e| identity_error(&e))?;

        let endpoint = AudioLinkEndpoint::bind(EndpointConfig {
            bind: config.listen,
            cert_der: identity.cert_der().to_vec(),
            key_der_pkcs8: identity.key_der_pkcs8().to_vec(),
            idle_timeout_ms: 10_000,
            keep_alive_ms: 3_000,
        })
        .await
        .map_err(|e| net_error(&e))?;

        let listen_addr = endpoint.local_addr().map_err(|e| net_error(&e))?;

        // 能力位只由「有没有对应工厂」决定 —— 这是唯一诚实的来源，
        // 否则 UI 会告诉用户「可以收」，而实际没有播放路径。
        let mut caps = Caps::NONE;
        if config.capture.is_some() {
            caps |= Caps::CAN_SEND;
        }
        if config.playout.is_some() {
            caps |= Caps::CAN_RECEIVE;
        }
        let platform = if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Android
        };
        let local = NodeInfo::new(identity.id(), config.node_name.clone(), platform, caps);

        let (events, _) = broadcast::channel(256);

        Ok(Arc::new(Self {
            inner: Arc::new(Inner {
                local,
                identity,
                trust: Mutex::new(trust),
                endpoint,
                config,
                peers: Mutex::new(HashMap::new()),
                events,
                shutdown: AtomicBool::new(false),
                listen_addr,
            }),
        }))
    }

    /// 本机节点描述。
    pub fn info(&self) -> NodeInfo {
        self.inner.local.clone()
    }

    /// QUIC 实际监听地址。
    pub fn local_addr(&self) -> std::net::SocketAddr {
        self.inner.listen_addr
    }

    /// 订阅引擎事件。
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    /// 已连接对端快照。
    pub fn peers(&self) -> Vec<PeerStatus> {
        self.inner
            .peers
            .lock()
            .map(|peers| peers.values().map(|p| p.snapshot()).collect())
            .unwrap_or_default()
    }

    /// 指定对端的遥测。
    pub fn telemetry(&self, peer: NodeId) -> Option<StreamStats> {
        let peers = self.inner.peers.lock().ok()?;
        let telemetry = peers.get(&peer)?.telemetry.lock().ok()?;
        Some(telemetry.snapshot())
    }

    /// 启动入站接受循环。
    pub fn spawn_accept_loop(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            while !inner.shutdown.load(Ordering::Relaxed) {
                match inner.endpoint.accept().await {
                    Ok(connection) => {
                        let inner = Arc::clone(&inner);
                        let addr = connection.remote_addr();
                        let peer_id = match connection.peer_id() {
                            Ok(peer_id) => peer_id,
                            Err(error) => {
                                tracing::warn!("inbound without cert: {}", error.context());
                                continue;
                            }
                        };
                        let trusted = is_trusted(&inner, peer_id);
                        let (session, commands) = create_session(&inner, peer_id, addr, trusted);
                        tokio::spawn(async move {
                            run_session(
                                inner,
                                connection,
                                session,
                                commands,
                                Role::Responder,
                                None,
                            )
                            .await;
                        });
                    }
                    Err(error) => {
                        if inner.shutdown.load(Ordering::Relaxed) {
                            return;
                        }
                        tracing::warn!("accept failed: {}", error.context());
                    }
                }
            }
        })
    }

    /// 主动连接（`docs/03-protocol.md` §5）：QUIC + 握手 + （必要时）PIN 配对。
    ///
    /// 成功返回对端身份。
    ///
    /// 对端要求 PIN 配对时本方法返回 `1002 NOT_PAIRED`，但**会话与命令通道已经建立**：
    /// UI 收到 [`EngineEvent::PinNeeded`] 后调 [`Engine::submit_pin`] 即可继续同一条连接。
    /// 这是刻意的 —— 把「需要 PIN」当成连接失败会让 UI 只能整条重连，白白丢掉已完成的
    /// QUIC 握手与 HELLO 交换。
    pub async fn connect(
        self: &Arc<Self>,
        addr: std::net::SocketAddr,
    ) -> Result<NodeId, AudioLinkError> {
        let connection = self
            .inner
            .endpoint
            .connect(addr, "audiolink")
            .await
            .map_err(|e| net_error(&e))?;
        let peer_id = connection.peer_id().map_err(|e| net_error(&e))?;

        // 同一个地址重复 connect 必须挡住：会话表按 `NodeId` 键，新会话会**覆盖**旧表项，
        // 而旧会话任务仍在后台跑着收数据报 —— 那条会话从此再也无法被 UI 触达（关不掉、看不到），
        // 变成一条静默泄漏的后台任务。宁可明确回 1009 BUSY，让 UI 提示「已在连接中」。
        let duplicate = self
            .inner
            .peers
            .lock()
            .map(|peers| peers.values().any(|session| session.addr == addr))
            .unwrap_or(false);
        if duplicate {
            connection.close(0, "duplicate connect");
            return Err(AudioLinkError::busy(
                "a session to this address already exists",
            ));
        }

        let trusted = is_trusted(&self.inner, peer_id);
        let (session, commands) = create_session(&self.inner, peer_id, addr, trusted);

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(run_session(
            Arc::clone(&self.inner),
            connection,
            session,
            commands,
            Role::Initiator,
            Some(ready_tx),
        ));

        // 外层再设一道超时：会话任务自己也有握手死线，但任务若在建立控制流时卡住，
        // 这道兜底能保证 `connect()` 一定会返回，而不是让 UI 永久转圈。
        match tokio::time::timeout(Duration::from_secs(15), ready_rx).await {
            Ok(Ok(Ok(()))) => Ok(peer_id),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(_)) => Err(AudioLinkError::bad_request(
                "session task ended before the handshake finished",
            )),
            Err(_) => Err(AudioLinkError::bad_request("connect timed out")),
        }
    }

    /// 向已连接对端开流（发送方向）。
    pub async fn start_send(&self, peer: NodeId) -> Result<(), AudioLinkError> {
        self.send_command(peer, SessionCommand::StartSend).await
    }

    /// 关闭与对端的流（保留连接与信任）。
    pub async fn stop_send(&self, peer: NodeId) -> Result<(), AudioLinkError> {
        self.send_command(
            peer,
            SessionCommand::CloseStream {
                reason: "user stop".to_string(),
            },
        )
        .await
    }

    /// 提交对端显示的 PIN（本机是发起端时）。
    pub async fn submit_pin(&self, peer: NodeId, pin: &str) -> Result<(), AudioLinkError> {
        self.send_command(peer, SessionCommand::SubmitPin(pin.to_string()))
            .await
    }

    /// 关闭引擎（幂等）。
    pub async fn shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::Relaxed);

        let sessions: Vec<Arc<PeerSession>> = self
            .inner
            .peers
            .lock()
            .map(|peers| peers.values().cloned().collect())
            .unwrap_or_default();
        for session in sessions {
            let _ = session.commands.try_send(SessionCommand::Shutdown);
        }

        self.inner.endpoint.close(0, "engine shutdown");
    }

    async fn send_command(
        &self,
        peer: NodeId,
        command: SessionCommand,
    ) -> Result<(), AudioLinkError> {
        let sender = {
            let peers = self
                .inner
                .peers
                .lock()
                .map_err(|_| AudioLinkError::bad_request("peer table poisoned"))?;
            peers
                .get(&peer)
                .ok_or_else(|| AudioLinkError::not_paired("unknown peer"))?
                .commands
                .clone()
        };

        sender
            .send(command)
            .await
            .map_err(|_| AudioLinkError::bad_request("session task is gone"))
    }
}

// ---------------------------------------------------------------------------
// 错误映射（契约 §2.3：在 engine 边界收敛为 AudioLinkError）
// ---------------------------------------------------------------------------

fn net_error(error: &NetError) -> AudioLinkError {
    AudioLinkError::owned(error.code(), error.context().to_string())
}

fn audio_error(error: &AudioError) -> AudioLinkError {
    AudioLinkError::owned(error.code(), error.context().to_string())
}

fn identity_error(error: &IdentityError) -> AudioLinkError {
    AudioLinkError::owned(error.code(), error.context().to_string())
}

// ---------------------------------------------------------------------------
// 会话驱动
// ---------------------------------------------------------------------------

/// 会话任务：**握手阶段 + 会话阶段**合成一个任务。
///
/// # 为什么不能把握手单独做成一个函数
///
/// 握手期间用户可能被要求输入 PIN（[`EngineEvent::PinNeeded`]）。如果握手跑在一个
/// 独立函数里、命令通道在握手**之后**才建立，那么用户在 PIN 弹窗里敲的那 6 位数字
/// 就无处可去 —— UI 只能整条重连，白白丢掉已完成的 QUIC 握手与 `HELLO` 交换。
///
/// 所以会话任务从第一帧 `HELLO` 起就同时监听控制流与命令通道；
/// [`Engine::connect`] 通过 `ready` 一次性拿到「握手成了没有」的结果。
async fn run_session(
    inner: Arc<Inner>,
    connection: Connection,
    session: Arc<PeerSession>,
    mut commands: mpsc::Receiver<SessionCommand>,
    role: Role,
    ready: Option<tokio::sync::oneshot::Sender<Result<(), AudioLinkError>>>,
) {
    let peer_id = session.id;
    let mut ready = ready;
    let codec = inner.config.codec;
    let frame_ms = u64::from(codec.frame_ms.max(1));

    let peer_cert = match connection.peer_cert_der() {
        Ok(cert) => cert,
        Err(error) => {
            finish_ready(&mut ready, Err(net_error(&error)));
            drop_session(&inner, &session, "no peer certificate");
            return;
        }
    };
    let mut control = match connection.open_control().await {
        Ok(control) => control,
        Err(error) => {
            finish_ready(&mut ready, Err(net_error(&error)));
            drop_session(&inner, &session, "control stream failed");
            return;
        }
    };

    let trusted_before = session.trusted.load(Ordering::Relaxed);
    let mut handshake = Handshake::new(role, inner.local.clone(), peer_id, trusted_before);
    let mut queue: VecDeque<Outgoing> = VecDeque::new();
    enqueue(&mut queue, handshake.start());
    if let Err(error) = flush_control(&mut control, &mut queue).await {
        finish_ready(&mut ready, Err(error.clone()));
        drop_session(&inner, &session, error.context());
        return;
    }

    // ---------------------------------------------------------------
    // 阶段一：握手与配对
    // ---------------------------------------------------------------
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(SessionCommand::SubmitPin(pin)) => {
                        let step = handshake.on_pin_input(&pin);
                        enqueue(&mut queue, step);
                        if let Err(error) = flush_control(&mut control, &mut queue).await {
                            finish_ready(&mut ready, Err(error));
                            drop_session(&inner, &session, "control stream failed");
                            return;
                        }
                    }
                    Some(SessionCommand::Shutdown) | None => {
                        finish_ready(&mut ready, Err(AudioLinkError::bad_request("cancelled by local side")));
                        drop_session(&inner, &session, "shutdown during handshake");
                        return;
                    }
                    _ => {}
                }
            }

            message = control.recv() => {
                let message = match message {
                    Ok(message) => message,
                    Err(error) => {
                        let error = net_error(&error);
                        finish_ready(&mut ready, Err(error.clone()));
                        drop_session(&inner, &session, error.context());
                        return;
                    }
                };

                let mut stats = DispatchStats::default();
                let (request, _) = dispatch_into(message.op, &message.payload, &mut stats);
                let Some(request) = request else {
                    continue; // §1.1：未知 / 载荷非法的帧忽略并计数，不断流
                };

                let step = handshake.on_control(&request, &inner.identity, &peer_cert, Instant::now());
                enqueue(&mut queue, step.clone());
                if let Err(error) = flush_control(&mut control, &mut queue).await {
                    finish_ready(&mut ready, Err(error));
                    drop_session(&inner, &session, "control stream failed");
                    return;
                }

                match step.event {
                    HandshakeEvent::Established { peer, persist } => {
                        if persist {
                            if let Err(error) = remember_peer(&inner, &peer) {
                                report_error(&inner, &session, &error);
                            }
                            session.trusted.store(true, Ordering::Relaxed);
                        }
                        session.set_name(&peer.name);
                        mark_streaming(&inner, &session);
                        let _ = inner.events.send(EngineEvent::PairCompleted {
                            id: peer_id,
                            ok: true,
                            reason: if persist { "paired".into() } else { "already trusted".into() },
                        });
                        finish_ready(&mut ready, Ok(()));
                        break;
                    }
                    HandshakeEvent::Rejected { error } => {
                        finish_ready(&mut ready, Err(error.clone()));
                        report_peer_gone(&inner, &session, &error);
                        drop_session(&inner, &session, error.context());
                        return;
                    }
                    HandshakeEvent::NeedPin { peer } => {
                        session.set_name(&peer.name);
                        let _ = inner.events.send(EngineEvent::PinNeeded {
                            id: peer_id,
                            name: peer.name,
                        });
                        // 「需要 PIN」不是失败：会话仍活着，UI 输入后会走上面的 SubmitPin 分支。
                        finish_ready(
                            &mut ready,
                            Err(AudioLinkError::not_paired("peer requires pin pairing")),
                        );
                    }
                    HandshakeEvent::DisplayPin { pin, remaining_attempts } => {
                        let _ = inner.events.send(EngineEvent::DisplayPin {
                            from: peer_id,
                            name: session
                                .name
                                .lock()
                                .map(|n| n.clone())
                                .unwrap_or_else(|_| "unknown".into()),
                            pin,
                            remaining_attempts,
                        });
                    }
                    HandshakeEvent::PinRejected { reason } => {
                        let _ = inner.events.send(EngineEvent::PairCompleted {
                            id: peer_id,
                            ok: false,
                            reason,
                        });
                    }
                    _ => {}
                }
            }

            _ = tokio::time::sleep_until(deadline) => {
                let error = AudioLinkError::bad_request("handshake timed out");
                finish_ready(&mut ready, Err(error));
                drop_session(&inner, &session, "handshake timed out");
                return;
            }
        }
    }

    // ---------------------------------------------------------------
    // 阶段二：会话（控制面 + 音频收发 + 应用指令）
    // ---------------------------------------------------------------
    let mut decoder = match OpusDecoder::new(codec) {
        Ok(decoder) => decoder,
        Err(error) => {
            report_error(&inner, &session, &audio_error(&error));
            drop_session(&inner, &session, "decoder init failed");
            return;
        }
    };

    let mut rx_buf = vec![0u8; DATAGRAM_MAX_LEN];
    let mut pcm = vec![0f32; codec.interleaved_frame() * 2];
    let mut dispatch_stats = DispatchStats::default();

    let mut expected_seq: Option<u32> = None;
    let mut last_datagram_at: Option<Instant> = None;
    let mut playback: Option<PlayoutHandle> = None;
    let mut next_stream_id: u32 = 1;

    let mut capture: Option<CaptureHandle> = None;
    let mut stream_id: Option<u32> = None;
    let mut epoch_id: u64 = 0;

    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break; };
                match command {
                    SessionCommand::StartSend => {
                        match start_send_pipeline(&inner, &session, &codec) {
                            Ok((handle, epoch)) => {
                                capture = Some(handle);
                                epoch_id = epoch;
                                stream_id = Some(1);
                                let _ = send_control(
                                    &mut control,
                                    &ControlRequest::OpenStream(open_stream_payload(&codec)),
                                ).await;
                            }
                            Err(error) => report_error(&inner, &session, &error),
                        }
                    }
                    SessionCommand::CloseStream { reason } => {
                        if let Some(id) = stream_id.take() {
                            let _ = send_control(
                                &mut control,
                                &ControlRequest::CloseStream(CloseStreamPayload {
                                    stream_id: id,
                                    reason,
                                }),
                            ).await;
                        }
                        stop_capture(&mut capture);
                    }
                    SessionCommand::SubmitPin(pin) => {
                        let _ = send_control(
                            &mut control,
                            &ControlRequest::PairSubmit(crate::payload::PairSubmitPayload { pin }),
                        ).await;
                    }
                    SessionCommand::Shutdown => break,
                }
            }

            message = control.recv() => {
                let message = match message {
                    Ok(message) => message,
                    Err(error) => {
                        report_peer_gone(&inner, &session, &net_error(&error));
                        break;
                    }
                };
                let (request, _) = dispatch_into(message.op, &message.payload, &mut dispatch_stats);
                let Some(request) = request else { continue; };
                handle_control(
                    &inner,
                    &session,
                    &mut control,
                    request,
                    &mut playback,
                    &mut capture,
                    &mut stream_id,
                    &mut next_stream_id,
                    &codec,
                ).await;
            }

            datagram = connection.read_datagram_into(&mut rx_buf) => {
                match datagram {
                    Ok(len) => {
                        let Some(slice) = rx_buf.get(..len) else { continue; };
                        let Ok(datagram) = AudioDatagram::decode(slice) else {
                            continue; // §1.1：非法数据报忽略并计数，不断流
                        };
                        if datagram.header.ptype != Ptype::Audio {
                            continue; // M1 不处理 FEC / NACK / 时钟探测（属 M2/M3）
                        }

                        // 到达间隔抖动：|实际间隔 − 标称帧长|（§10 的 jitter 口径）。
                        // 必须真的量出来 —— 一个恒为 0 的 jitter 会让报告读起来像「网络完美」，
                        // 而那只是「没测」的另一种写法。P50 决定缓冲该多深，P95 决定最坏会多深。
                        let now = Instant::now();
                        if let Some(previous) = last_datagram_at.replace(now) {
                            let interval_us =
                                u32::try_from(now.saturating_duration_since(previous).as_micros())
                                    .unwrap_or(u32::MAX);
                            let nominal_us = u32::try_from(frame_ms).unwrap_or(20) * 1_000;
                            if let Ok(mut telemetry) = session.telemetry.lock() {
                                telemetry.record_jitter(interval_us.abs_diff(nominal_us));
                            }
                        }

                        receive_audio(
                            &session,
                            &mut decoder,
                            &mut pcm,
                            &playback,
                            datagram.header.seq,
                            datagram.payload,
                            &mut expected_seq,
                        );
                    }
                    Err(error) => {
                        report_peer_gone(&inner, &session, &net_error(&error));
                        break;
                    }
                }
            }

            frame = next_encoded(&mut capture) => {
                let Some(frame) = frame else { continue; };
                let Some(id) = stream_id else { continue; };

                if let Some(tap) = inner.config.measurement.as_ref() {
                    tap.record_sealed(frame.seq, frame.sealed_at);
                }

                if let Err(error) = transmit_audio(
                    &connection,
                    id,
                    epoch_id,
                    &frame,
                    &session,
                ).await {
                    report_error(&inner, &session, &error);
                }
            }

            _ = ticker.tick() => {
                let snapshot = match session.telemetry.lock() {
                    Ok(mut telemetry) => {
                        let rtt = u32::try_from(connection.rtt_us()).unwrap_or(u32::MAX);
                        telemetry.set_rtt_us(rtt);
                        // 水位 = 待播队列深度 × 帧长（口径见 `PlayoutHandle::depth_frames`）。
                        if let Some(handle) = playback.as_ref() {
                            let depth = u32::try_from(handle.depth_frames()).unwrap_or(u32::MAX);
                            telemetry.set_buffer_level_us(
                                depth.saturating_mul(codec.frame_ms).saturating_mul(1_000),
                            );
                        }
                        telemetry.roll();
                        telemetry.snapshot()
                    }
                    Err(_) => StreamStats::default(),
                };

                // §10：`STREAM_STATS` 是**双方互发**的 1 Hz 上报，不是「只发本地事件」。
                // 漏掉这一条发送的后果很具体：推流端的面板永远看不到 e2e 延迟、播放环水位、
                // 欠载次数 —— 而那三个量**只有接收侧才量得到**。
                if let Err(error) = send_control(&mut control, &ControlRequest::StreamStats(snapshot)).await {
                    report_error(&inner, &session, &error);
                }

                let _ = inner.events.send(EngineEvent::Telemetry(Box::new(snapshot)));
            }
        }
    }

    let _ = frame_ms; // 帧长已在 codec 里体现，这里只用于文档可读性
    stop_capture(&mut capture);
    stop_playout(&mut playback);
    drop_session(&inner, &session, "session closed");
}

/// 向 `connect()` 回报握手结果（只回报一次）。
fn finish_ready(
    ready: &mut Option<tokio::sync::oneshot::Sender<Result<(), AudioLinkError>>>,
    outcome: Result<(), AudioLinkError>,
) {
    if let Some(sender) = ready.take() {
        let _ = sender.send(outcome);
    }
}

/// 从会话表里摘掉，并广播断开。
fn drop_session(inner: &Arc<Inner>, session: &Arc<PeerSession>, reason: &str) {
    if let Ok(mut peers) = inner.peers.lock() {
        peers.remove(&session.id);
    }
    let _ = inner.events.send(EngineEvent::PeerDisconnected {
        id: session.id,
        reason: reason.to_string(),
    });
}

fn open_stream_payload(codec: &CodecConfig) -> OpenStreamPayload {
    OpenStreamPayload {
        session_id: 1,
        source: SourceKind::SystemLoopback,
        codec_prefs: vec![codec_pref(codec)],
        target_rate_kbps: u32::try_from(codec.bitrate_bps / 1_000).unwrap_or(160),
        channels: 2,
        group: None,
    }
}

/// 建会话表项。对端展示名在收到 `HELLO` 之前只能用**短指纹**占位 ——
/// 展示名是对端自报的，在拿到证书指纹之前任何名字都不可信。
fn create_session(
    inner: &Arc<Inner>,
    peer_id: NodeId,
    addr: std::net::SocketAddr,
    trusted: bool,
) -> (Arc<PeerSession>, mpsc::Receiver<SessionCommand>) {
    let (commands, command_rx) = mpsc::channel(16);
    let session = Arc::new(PeerSession {
        id: peer_id,
        name: Mutex::new(peer_id.short()),
        addr,
        machine: Mutex::new(SessionMachine::new()),
        telemetry: Arc::new(Mutex::new(TelemetryAggregator::new(
            1,
            inner.config.codec.telemetry(),
        ))),
        commands,
        trusted: AtomicBool::new(trusted),
        state: Mutex::new(SessionState::Handshaking),
    });

    if let Ok(mut peers) = inner.peers.lock() {
        peers.insert(peer_id, Arc::clone(&session));
    }
    (session, command_rx)
}

fn mark_streaming(inner: &Arc<Inner>, session: &Arc<PeerSession>) {
    session.apply(SessionEvent::HandshakeOk);
    session.set_state(SessionState::Streaming);
    let _ = inner
        .events
        .send(EngineEvent::PeerUpdated(Box::new(session.snapshot())));
}

fn is_trusted(inner: &Arc<Inner>, id: NodeId) -> bool {
    inner
        .trust
        .lock()
        .map(|trust| trust.is_trusted(id))
        .unwrap_or(false)
}

fn remember_peer(inner: &Arc<Inner>, peer: &NodeInfo) -> Result<(), AudioLinkError> {
    let mut trust = inner
        .trust
        .lock()
        .map_err(|_| AudioLinkError::bad_request("trust store poisoned"))?;
    trust
        .trust(TrustEntry {
            id: peer.id,
            name: peer.name.clone(),
            platform: peer.platform,
            paired_at_unix: unix_now(),
        })
        .map_err(|e| identity_error(&e))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn enqueue(queue: &mut VecDeque<Outgoing>, step: HandshakeStep) {
    queue.extend(step.send);
}

async fn flush_control(
    control: &mut ControlChannel,
    queue: &mut VecDeque<Outgoing>,
) -> Result<(), AudioLinkError> {
    while let Some(outgoing) = queue.pop_front() {
        let body = outgoing.request.encode_body()?;
        control
            .send(outgoing.request.op(), outgoing.request_id, &body)
            .await
            .map_err(|e| net_error(&e))?;
    }
    Ok(())
}

/// 采集管线：建立采集线程并启动编码。
///
/// 返回 `(句柄, epoch_id)`；调用方拿到句柄后再发 `OPEN_STREAM` 并把 ACK 里的 `stream_id` 填进发送路径。
///
/// 失败时**什么都不会启动**：采集源的「零重采样」断言与编码器初始化都在线程内完成、
/// 通过 `ready` 一次性回报，所以「推流已开始」这件事不会在失败时被谎报给你。
fn start_send_pipeline(
    inner: &Arc<Inner>,
    session: &Arc<PeerSession>,
    codec: &CodecConfig,
) -> Result<(CaptureHandle, u64), AudioLinkError> {
    let Some(factory) = inner.config.capture.as_ref() else {
        return Err(AudioLinkError::cap_unsupported(
            "this node has no capture source configured",
        ));
    };

    let handle = spawn_capture_thread(
        factory,
        *codec,
        Arc::clone(&session.telemetry),
        inner.config.measurement.clone(),
        new_stop_flag(),
    )?;

    Ok((handle, random_u64()))
}

/// 采集线程句柄。
struct CaptureHandle {
    frames: mpsc::Receiver<EncodedFrame>,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

/// 采集线程产出的一个已编码帧。
struct EncodedFrame {
    seq: u32,
    sample_index: u32,
    payload: Vec<u8>,
    sealed_at: Instant,
}

/// 播放线程要消费的一帧 PCM（48 kHz / f32 / 2ch 交错）。
struct PlaybackFrame {
    seq: u32,
    samples: Vec<f32>,
}

/// 播放管线句柄：发送端（会话任务）+ 停止标志 + 线程句柄。
struct PlayoutHandle {
    frames: Sender<PlaybackFrame>,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl PlayoutHandle {
    /// 待播队列深度（帧数）。
    ///
    /// # 口径说明（别把它读成「播放器里积压了多少」）
    ///
    /// 这是**已解码、还没被播放线程取走**的帧数 —— 也就是 M1 的抖动缓冲本身。
    /// 时钟驱动的播放线程每拍取走一帧，所以它在 1 Hz 采样下通常只有 0–1 帧。
    /// 曾经试过改用 `PlayoutSink::buffered_frames()`（播放器内部积压），但那是个**实现自定义**的量：
    /// 真实 WASAPI sink 与合成 sink 的语义不同，同一份遥测在两端会给出不可比的数字。
    /// 与其发布一个语义漂移的指标，不如发布一个定义精确的。
    fn depth_frames(&self) -> usize {
        self.frames.len()
    }

    /// 非阻塞投递一帧；返回 `false` 表示队列已满（调用方负责计 `late_drops`）。
    fn try_push(&self, frame: PlaybackFrame) -> bool {
        self.frames.try_send(frame).is_ok()
    }
}

fn stop_capture(handle: &mut Option<CaptureHandle>) {
    if let Some(handle) = handle.take() {
        handle.stop.store(true, Ordering::Relaxed);
        if let Some(join) = handle.join {
            let _ = join.join();
        }
    }
}

/// 停掉播放线程：先置停止标志，再**丢弃发送端**。
///
/// 丢弃发送端是必须的：`recv_timeout` 只在通道断开时返回 `Disconnected`，
/// 否则最坏要等一个帧长才轮到下一轮循环 —— 置标志位本身不会唤醒它。
///
/// 刻意**不 join**：会话任务是 async 的，在这里阻塞等线程退出会把整个 tokio worker 卡住，
/// 而同 worker 上还跑着别的会话。线程自己会在一个帧长内退出，句柄丢弃即分离。
fn stop_playout(handle: &mut Option<PlayoutHandle>) {
    if let Some(mut handle) = handle.take() {
        handle.stop.store(true, Ordering::Relaxed);
        // 用 `drop` 显式表达「这里断开通道，是为了唤醒 recv_timeout」，而不是顺手丢掉。
        drop(handle.frames);
        let _ = handle.join.take();
    }
}

/// 取下一帧编码结果；没有采集管线时永久挂起（`select!` 需要一个永不就绪的分支）。
async fn next_encoded(handle: &mut Option<CaptureHandle>) -> Option<EncodedFrame> {
    match handle {
        Some(handle) => handle.frames.recv().await,
        None => std::future::pending().await,
    }
}

/// 采集线程：**在本线程内**创建 `CaptureSource`（`!Send`，谁用谁建）。
fn spawn_capture_thread(
    factory: &CaptureFactory,
    codec: CodecConfig,
    telemetry: Arc<Mutex<TelemetryAggregator>>,
    tap: Option<Arc<MeasurementTap>>,
    stop: Arc<AtomicBool>,
) -> Result<CaptureHandle, AudioLinkError> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), AudioLinkError>>();
    let (frame_tx, frame_rx) = mpsc::channel::<EncodedFrame>(ENCODE_QUEUE_FRAMES);
    let thread_stop = Arc::clone(&stop);
    // 先把工厂的 `Arc` 克隆出来再进线程：闭包里借用 `&Arc` 会让引用逃逸出函数体，
    // 而线程闭包要求 `'static`。
    let factory = Arc::clone(factory);

    let join = std::thread::Builder::new()
        .name("audiolink-capture".to_string())
        .spawn(move || {
            capture_main(
                factory.as_ref(),
                codec,
                telemetry,
                tap,
                thread_stop,
                frame_tx,
                ready_tx,
            );
        })
        .map_err(|_| AudioLinkError::bad_request("failed to spawn capture thread"))?;

    // 等采集源建好并过了「零重采样」断言再返回 —— 否则调用方会以为推流已经开始，
    // 而失败只留在一条日志里（旧版「异常即永久静音」的复发路径）。
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(CaptureHandle {
            frames: frame_rx,
            stop,
            join: Some(join),
        }),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(AudioLinkError::bad_request(
            "capture source did not report readiness in time",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn capture_main(
    factory: &(dyn Fn() -> Result<Box<dyn CaptureSource>, AudioError> + Send + Sync),
    codec: CodecConfig,
    telemetry: Arc<Mutex<TelemetryAggregator>>,
    tap: Option<Arc<MeasurementTap>>,
    stop: Arc<AtomicBool>,
    frame_tx: mpsc::Sender<EncodedFrame>,
    ready_tx: std::sync::mpsc::Sender<Result<(), AudioLinkError>>,
) {
    let mut capture = match factory() {
        Ok(capture) => capture,
        Err(error) => {
            let _ = ready_tx.send(Err(audio_error(&error)));
            return;
        }
    };

    // 「零重采样」硬闸：不达标就**拒绝推流**，绝不静默 SRC（NFR-13）。
    if let Err(error) =
        require_unified_format("capture", capture.backend_name(), capture.device_format())
    {
        let _ = ready_tx.send(Err(error));
        return;
    }

    let mut chunker = match FrameChunker::new(codec.frame_ms, ENCODE_QUEUE_FRAMES * 2) {
        Ok(chunker) => chunker,
        Err(error) => {
            let _ = ready_tx.send(Err(audio_error(&error)));
            return;
        }
    };
    let mut encoder = match OpusEncoder::new(codec) {
        Ok(encoder) => encoder,
        Err(error) => {
            let _ = ready_tx.send(Err(audio_error(&error)));
            return;
        }
    };

    let _ = ready_tx.send(Ok(()));

    let frame_samples = u32::try_from(codec.frame_samples()).unwrap_or(960);
    let mut samples: Vec<f32> = Vec::with_capacity(codec.interleaved_frame() * 2);
    // 编码输出缓冲：取协议的最大载荷（§3），这样「编码器写不下」这件事永远不会先于
    // 「超过数据报预算」发生 —— 前者是本地缓冲不够（实现 bug），后者才是要报给用户的配置问题。
    let mut opus_buf = vec![0u8; audiolink_types::DATAGRAM_MAX_PAYLOAD];
    let mut seq = 0u32;
    let mut sample_index = 0u32;

    while !stop.load(Ordering::Relaxed) {
        match capture.read(&mut samples, Duration::from_millis(200)) {
            Ok(Some(_packet)) => {
                chunker.push(&samples);
            }
            Ok(None) => {
                // 空闲端点零数据不是错误（坑清单 #12）：计数即可，绝不报错断流。
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.set_buffer_level_us(0);
                }
            }
            Err(error) => {
                // 采集源失效：计数并结束线程。引擎侧由会话状态机迁移到 Failed（架构 §4 铁律 2）。
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.record_late_drop();
                }
                let _ = error;
                break;
            }
        }

        while let Some(frame) = chunker.next_frame() {
            let sealed_at = Instant::now();
            let Ok(written) = encoder.encode_into(frame, &mut opus_buf) else {
                break;
            };
            if written == 0 {
                continue; // DTX 静默帧：不占序号
            }

            let packet = EncodedFrame {
                seq,
                sample_index,
                payload: opus_buf[..written].to_vec(),
                sealed_at,
            };

            if tap.is_none() {
                // 没装探针时不需要封口时刻，但序号仍要推进（对端按 seq 判丢包）。
            }

            // 队列满 = 编码比网络快：丢最旧（与「不阻塞采集」同一条纪律）。
            if frame_tx.try_send(packet).is_err()
                && let Ok(mut telemetry) = telemetry.lock()
            {
                telemetry.record_late_drop();
            }

            seq = seq.wrapping_add(1);
            sample_index = sample_index.wrapping_add(frame_samples);
        }
    }

    capture.stop();
}

/// 发送一个音频数据报。
async fn transmit_audio(
    connection: &Connection,
    stream_id: u32,
    epoch_id: u64,
    frame: &EncodedFrame,
    session: &Arc<PeerSession>,
) -> Result<(), AudioLinkError> {
    let budget = connection.max_audio_payload();

    // 单帧超过预算说明码率档与 MTU 不匹配（架构 §12 技术债：应用层分片属 M2+）。
    // 明确拒绝并把这一帧丢掉，而不是发出去让对端解不出来 —— 后者会表现成「对端一直没声音」，
    // 而这里至少会留下一条带具体字节数的错误。
    if frame.payload.len() > budget {
        return Err(AudioLinkError::owned(
            ErrorCode::CodecUnsupported,
            format!(
                "opus frame of {} B exceeds the {} B datagram budget; \
                 lower the bitrate or shorten the frame",
                frame.payload.len(),
                budget
            ),
        ));
    }

    let header = AudioDatagramHeader {
        version: audiolink_types::PROTO_MAJOR,
        ptype: Ptype::Audio,
        flags: audiolink_types::Flags::NONE,
        stream_id,
        seq: frame.seq,
        sample_index: frame.sample_index,
        epoch_id,
    };
    let bytes = AudioDatagram {
        header,
        payload: &frame.payload,
    }
    .encode_to_vec()?;

    connection
        .send_datagram(&bytes)
        .await
        .map_err(|e| net_error(&e))?;

    if let Ok(mut telemetry) = session.telemetry.lock() {
        telemetry.record_expected();
        telemetry.record_received(frame.payload.len());
    }
    Ok(())
}

/// 收到一个音频数据报：解码 → 推给播放线程。
///
/// # 收费口径（为什么这里每一处计数都不能省）
///
/// 接收侧是**唯一**能看见「真实丢包」的地方：`expected = received + lost`，
/// 分母必须把丢掉的包也算进去，否则丢包率会系统性偏低，而 M2 的自适应码率就建在这个数字上。
#[allow(clippy::too_many_arguments)] // 参数都是「本函数做完这件事所需的最小输入」，拆包只会把耦合藏起来
fn receive_audio(
    session: &Arc<PeerSession>,
    decoder: &mut OpusDecoder,
    pcm: &mut [f32],
    playback: &Option<PlayoutHandle>,
    seq: u32,
    payload: &[u8],
    expected_seq: &mut Option<u32>,
) {
    // 序号跳跃 = 中间那些包在网络里丢了（QUIC 数据报不重传）。
    if let Some(expected) = *expected_seq {
        let gap = seq.wrapping_sub(expected);
        // `gap == 0` 是重传/重复；`gap` 很大说明是会话重启后序号回绕，两种都不计丢包。
        if gap > 0
            && gap < 1_000
            && let Ok(mut telemetry) = session.telemetry.lock()
        {
            for _ in 0..gap {
                telemetry.record_expected();
            }
            telemetry.record_lost(gap);
        }
    }
    *expected_seq = Some(seq.wrapping_add(1));

    if let Ok(mut telemetry) = session.telemetry.lock() {
        telemetry.record_expected();
        telemetry.record_received(payload.len());
    }

    // CELT-only 没有真 PLC（ADR-003 注记）：`decode_into` 失败就等于这一帧没了，
    // 自建掩盖属 M2。M1 只如实计数，不假装补出了声音。
    let Ok(decoded) = decoder.decode_into(payload, pcm) else {
        if let Ok(mut telemetry) = session.telemetry.lock() {
            telemetry.record_plc();
        }
        return;
    };

    let Some(handle) = playback.as_ref() else {
        return; // 对端在开流协商前就发了音频：忽略
    };

    // `decoded` 是**每声道**样本数（坑清单 #13），转成交错样本数要乘声道数。
    let interleaved = decoded.saturating_mul(2);
    let Some(frame) = pcm.get(..interleaved) else {
        return;
    };

    // 队列满 = 播放跟不上：丢这一帧（§7 第 1 条：宁可丢一帧，也不延迟出声）。
    if !handle.try_push(PlaybackFrame {
        seq,
        samples: frame.to_vec(),
    }) && let Ok(mut telemetry) = session.telemetry.lock()
    {
        telemetry.record_late_drop();
    }
}

// ---------------------------------------------------------------------------
// 控制面
// ---------------------------------------------------------------------------

/// 处理一条会话期控制命令。
///
/// 参数是「会话当前的可变状态」—— 每个 `&mut` 都代表一条生命周期横跨整轮会话的资源，
/// 打包成一个结构体只是把同样的可变借用藏进字段里，不会降低耦合。
#[allow(clippy::too_many_arguments)]
async fn handle_control(
    inner: &Arc<Inner>,
    session: &Arc<PeerSession>,
    control: &mut ControlChannel,
    request: ControlRequest,
    playback: &mut Option<PlayoutHandle>,
    capture: &mut Option<CaptureHandle>,
    stream_id: &mut Option<u32>,
    next_stream_id: &mut u32,
    codec: &CodecConfig,
) {
    match request {
        ControlRequest::OpenStream(open) => {
            // 接收端：开播放线程并把流 ID 回给发送端。
            if playback.is_some() {
                // M1 一个对端只接一路流；多路混音属 M4。明确拒绝而不是默默覆盖 ——
                // 默默覆盖会让第一路流永远收不到音频，而发送端还以为一切正常。
                let _ = send_control(
                    control,
                    &ControlRequest::CloseStream(CloseStreamPayload {
                        stream_id: stream_id.unwrap_or(0),
                        reason: "M1 supports a single stream per peer".to_string(),
                    }),
                )
                .await;
                return;
            }

            let id = *next_stream_id;
            *next_stream_id = next_stream_id.saturating_add(1);
            *stream_id = Some(id);

            match spawn_playout_thread(inner, session, codec) {
                Ok(handle) => *playback = Some(handle),
                Err(error) => {
                    report_error(inner, session, &error);
                    return;
                }
            }

            let ack = ControlRequest::OpenStreamAck(OpenStreamAckPayload {
                session_id: open.session_id,
                stream_id: id,
                codec_chosen: codec_pref(codec),
                epoch_id: 1,
                epoch_local_us: 0,
            });
            let _ = send_control(control, &ack).await;
        }

        ControlRequest::OpenStreamAck(ack) => {
            // 发送端：协商完成。推流在 `start_send` 里已经起来了，这里只把流 ID 对齐到对端给的编号。
            *stream_id = Some(ack.stream_id);
            let _ = inner
                .events
                .send(EngineEvent::PeerUpdated(Box::new(session.snapshot())));
        }

        ControlRequest::CloseStream(_) => {
            stop_capture(capture);
            stop_playout(playback);
            *stream_id = None;
        }

        ControlRequest::Bye(_) => {
            session.apply(SessionEvent::LinkLost);
            session.set_state(SessionState::Reconnecting);
        }

        ControlRequest::StreamStats(stats) => {
            // 对端遥测：M1 只透传给 UI，不做自适应决策（M2）。
            let _ = inner.events.send(EngineEvent::Telemetry(Box::new(stats)));
        }

        ControlRequest::Error(payload) => {
            let error = AudioLinkError::owned(
                audiolink_types::ErrorCode::from_u16(payload.code)
                    .unwrap_or(audiolink_types::ErrorCode::BadRequest),
                payload.message,
            );
            report_error(inner, session, &error);
        }

        // SetGain / SetMute / ClockResult / Ping / Pong / 及其它 M2/M3 命令：
        // M1 尚无对应效果，**明确不做**而不是假装接受 ——
        // 假装接受了 `SET_GAIN`，用户会以为音量真的变了，然后来查「为什么调音量没用」。
        _ => {}
    }
}

async fn send_control(
    control: &mut ControlChannel,
    request: &ControlRequest,
) -> Result<(), AudioLinkError> {
    let body = request.encode_body()?;
    control
        .send(request.op(), 0, &body)
        .await
        .map_err(|e| net_error(&e))
}

fn report_error(inner: &Arc<Inner>, session: &Arc<PeerSession>, error: &AudioLinkError) {
    session.apply(SessionEvent::LinkDegraded);
    let _ = inner.events.send(EngineEvent::Error {
        code: error.code().as_u16(),
        context: error.context().to_string(),
    });
}

fn report_peer_gone(inner: &Arc<Inner>, session: &Arc<PeerSession>, error: &AudioLinkError) {
    session.apply(SessionEvent::LinkLost);
    session.set_state(SessionState::Failed);
    let _ = inner.events.send(EngineEvent::PeerDisconnected {
        id: session.id,
        reason: format!("{}: {}", error.code().as_u16(), error.context()),
    });
}

// ---------------------------------------------------------------------------
// 播放线程
// ---------------------------------------------------------------------------

/// 播放线程：**在本线程内**创建 `PlayoutSink`（`!Send`）。
fn spawn_playout_thread(
    inner: &Arc<Inner>,
    session: &Arc<PeerSession>,
    codec: &CodecConfig,
) -> Result<PlayoutHandle, AudioLinkError> {
    let Some(factory) = inner.config.playout.as_ref() else {
        return Err(AudioLinkError::cap_unsupported(
            "this node has no playout sink configured",
        ));
    };

    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), AudioLinkError>>();
    let (frame_tx, frame_rx) = crossbeam_channel::bounded(PLAYBACK_QUEUE_FRAMES);
    let stop = new_stop_flag();
    let thread_stop = Arc::clone(&stop);

    let factory = Arc::clone(factory);
    let telemetry = Arc::clone(&session.telemetry);
    let tap = inner.config.measurement.clone();
    let frame_ms = u64::from(codec.frame_ms.max(1));
    let pcm = codec.interleaved_frame();

    let join = std::thread::Builder::new()
        .name("audiolink-playout".to_string())
        .spawn(move || {
            playout_main(
                factory.as_ref(),
                frame_ms,
                pcm,
                telemetry,
                tap,
                thread_stop,
                frame_rx,
                ready_tx,
            );
        })
        .map_err(|_| AudioLinkError::bad_request("failed to spawn playout thread"))?;

    // 与采集侧同理：等播放器建好并过了「零重采样」断言再返回 ——
    // 否则发送端会以为对端已经在放音，而失败只留在一条日志里。
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(PlayoutHandle {
            frames: frame_tx,
            stop,
            join: Some(join),
        }),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(AudioLinkError::bad_request(
            "playout sink did not report readiness in time",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn playout_main(
    factory: &(dyn Fn() -> Result<Box<dyn PlayoutSink>, AudioError> + Send + Sync),
    frame_ms: u64,
    pcm_len: usize,
    telemetry: Arc<Mutex<TelemetryAggregator>>,
    tap: Option<Arc<MeasurementTap>>,
    stop: Arc<AtomicBool>,
    frames: Receiver<PlaybackFrame>,
    ready_tx: std::sync::mpsc::Sender<Result<(), AudioLinkError>>,
) {
    let mut sink = match factory() {
        Ok(sink) => sink,
        Err(error) => {
            let _ = ready_tx.send(Err(audio_error(&error)));
            return;
        }
    };

    // 「零重采样」硬闸（NFR-13）：播放侧同样不得静默 SRC。
    if let Err(error) = require_unified_format("playout", sink.backend_name(), sink.device_format())
    {
        let _ = ready_tx.send(Err(error));
        return;
    }

    let _ = ready_tx.send(Ok(()));

    let period = Duration::from_millis(frame_ms);
    let silence = vec![0f32; pcm_len];
    let mut primed = false;
    let mut next_write = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        // 睡到下一个提交时刻。节奏必须由**本地时钟**决定，数据到没到只影响
        // 「这一拍写什么」，不影响「什么时候写」—— 否则每一个网络到达抖动都会直接
        // 变成出声时间抖动，§7 的「由 epoch 驱动而非到达时间驱动」就落空了。
        let now = Instant::now();
        if next_write > now {
            std::thread::sleep(next_write - now);
        }
        next_write += period;
        // 落后太多（进程被抢占 / 系统挂起）就重新对齐，避免连续追赶式提交。
        if next_write < Instant::now() {
            next_write = Instant::now() + period;
        }

        // 起步攒帧：不足 `PRIME_FRAMES` 就继续等。这一拍**不补静音也不算欠载** ——
        // 还没开始播，谈不上「欠」；把攒帧期算成欠载会让欠载率失去意义。
        if !primed {
            if frames.len() >= PRIME_FRAMES {
                primed = true;
            } else {
                continue;
            }
        }

        match frames.try_recv() {
            Ok(frame) => {
                if sink.write(&frame.samples).is_err() {
                    break;
                }
                if let Some(tap) = tap.as_ref() {
                    tap.record_played(frame.seq, Instant::now());
                }
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {
                // 欠载（§7 第 4 条）：补齐静音并计数。**不**静默忽略 ——
                // 「能听出卡顿」与「遥测显示欠载」必须是同一件事。
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.record_underrun();
                }
                if sink.write(&silence).is_err() {
                    break;
                }
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => break,
        }
    }

    sink.stop();
}

fn codec_pref(config: &CodecConfig) -> CodecPref {
    CodecPref::Opus {
        frame_ms: u8::try_from(config.frame_ms).unwrap_or(20),
        bitrate_kbps: u16::try_from(config.bitrate_bps / 1_000).unwrap_or(160),
        fec: config.use_inband_fec,
        vbr: !config.use_cbr,
    }
}

fn random_u64() -> u64 {
    use rand::Rng;
    rand::rng().random()
}

/// 每次创建音频线程时发一枚**独立的**停止标志。
///
/// 刻意与会话/引擎的生命周期解耦：关掉一路流不该让整台引擎停下来，
/// 也不该让同进程里的另一路流受影响。谁创建线程，谁负责置位（`stop_capture` / `stop_playback`）。
fn new_stop_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
