//! 契约 §8 的导出面：把 `audiolink-engine` 的 API 翻成 UniFFI 类型
//!
//! ```text
//! Kotlin                      audiolink-ffi                        audiolink-engine
//! engineStart(config, …) ──►  runtime.spawn(Engine::start)   ──►  Engine::start(cfg)
//! connect(addr)          ──►  runtime.spawn(Engine::connect) ──►  Engine::connect(addr)
//! peers() / telemetry()  ──►  直接读 Engine 的同步快照          ──►  Engine::peers()/telemetry()
//! displayedPin()         ──►  直接读当前会话的 PIN 快照        ──►  Engine::displayed_pin()
//! ```
//!
//! # 线程模型（Android 上最容易踩的一脚）
//!
//! 引擎的每个异步调用都**派发到本 crate 自建的多线程 tokio 运行时**上执行，FFI 的 async 函数
//! 只做「派发 + 等 oneshot」。理由有两层：
//!
//! 1. UniFFI 0.29 的 `async_runtime = "tokio"` 只是把导出的 future 包一层 `async_compat::Compat`：
//!    async-compat 0.2.6 的 crate 文档写得很直白 —— 没有现成运行时上下文时会
//!    「*a new single-threaded runtime will be created on demand*」，而且
//!    「*the future is not polled by the tokio runtime*」（它只设线程局部变量，poll 发生在调用线程上）。
//!    把 QUIC 驱动 + 编解码 + 全部会话任务押在那个**单线程**兜底运行时上，音频链路的抖动不可控。
//! 2. 契约要求「**不要把 tokio 运行时绑到 UI 线程**」：Kotlin 在 UI 线程 `await` 这些函数时，
//!    真正的活儿在 `audiolink-rt-*` 工作线程上跑，UI 线程只是挂起等回调。
//!
//! # 全局态
//!
//! 一个进程一个引擎。`ENGINE` 持有当前句柄；PIN 与对端列表都读取引擎的同步快照，
//! 不再另建广播事件缓存，避免丢事件后永久缺失 PIN 或旧引擎的事件污染重启状态。
//!
//! 运行时**永不释放**（进程级）：`tokio::runtime::Runtime` 若在 async 上下文里被 drop 会 panic
//! （"Cannot drop a runtime in a context where blocking is not allowed"），而 FFI 的 async 函数
//! 恰好总在 async 上下文里。Android 进程的生命周期就是它的生命周期，engineStop 只停引擎。

use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use audiolink_engine::{Engine, EngineConfig, PeerStatus};
use audiolink_types::{Capabilities, Caps, DEFAULT_QUIC_PORT, ErrorCode, NodeId, StreamStats};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

use crate::audio_bridge::{
    PcmFeed, PcmPull, capture_factory, playout_factory, share_feed, share_pull,
};
use crate::error::FfiError;

/// 工作线程数：QUIC 驱动 + 编解码会话任务都要跑，1 个线程会被网络 IO 阻塞拖死。
const WORKER_THREADS: usize = 4;

/// 按设备调音量时的渐变时长（ms）。
///
/// 为什么固定 200：桌面端前端调的就是 `setPeerGain(idShort, gain, 200)` —— 「同一件事在两端的
/// 听感」不该因为参数默认值不同而分叉。协议里 `SET_MUTE` 帧自带 50 ms（只做 0/1 切换、要更
/// 干脆，见 engine `runtime.rs` 的处理），两者服务的场景不同，不要互相抄。
const DEFAULT_PEER_GAIN_RAMP_MS: u32 = 200;

/// 进程级多线程运行时（见模块文档「线程模型」）。**故意不实现 Drop 路径**。
static RUNTIME: Mutex<Option<Arc<Runtime>>> = Mutex::new(None);

/// 当前引擎（`None` = 未启动）。
static ENGINE: Mutex<Option<EngineHandle>> = Mutex::new(None);

/// 启停串行：等待锁时取消 = 未执行；派发后由运行时持锁到操作真正结束。
static LIFECYCLE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---------------------------------------------------------------------------
// 对外类型（Kotlin 侧的名字由 UniFFI 转成 camelCase）
// ---------------------------------------------------------------------------

/// `engineStart` 的入参。
///
/// 编码参数 / 播放环深度刻意**不暴露**：M1 冻结为 engine 的默认值（20 ms / 160 kbps / VBR / 48 kHz），
/// 开成旋钮只会让「零重采样」与延迟预算失去唯一解。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EngineStartConfig {
    /// 用户可见节点名（仅展示，不参与身份判定）。
    pub node_name: String,
    /// 身份材料（`cert.pem` / `key.pem`）与信任库（`trust.json`）的落地目录。
    pub data_dir: String,
    /// QUIC 监听端口；`0` = 默认 `58290`。
    pub listen_port: u16,
    /// **平台侧额外声明**的能力位（§13 的协商位图，位名与语义见 `audiolink_types::Capabilities`）。
    ///
    /// 语义是「**这台设备能**做什么」，**不是**「此刻正在用什么」—— 与 `CAPTURE` / `PLAYOUT` 同口径
    /// （纯接收端「不需要」CAPTURE，但它并不是「不能」采集）。所以发送源关着也照旧声明：
    /// 对端据此知道「这台设备具备内录 / 麦克风」，真正用不用得上由后续的授权流程与用户选择决定。
    ///
    /// `0`（缺省）＝ 与加上这个字段之前**逐位一致**，也就是内核默认的 `Capabilities::CURRENT`；
    /// 非零值与内核默认取**并集**，因此**不可能**抹掉必需位（`REQUIRED` 只有 `OPUS`）——
    /// 若允许调用方覆盖整张位图，一个只写 `MICROPHONE` 的调用方就会让双方缺必需位、直接拒连。
    // UniFFI 的 default 表达式不接受带后缀的字面量（`0u32` 会报 "integer literals with suffix
    // not supported here"），类型由字段本身决定。
    #[uniffi(default = 0)]
    pub capabilities: u32,
}

/// 本机状态（`localStatus()`）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LocalStatus {
    /// 指纹短码（16 hex 字符）—— UI 展示用。
    pub id_short: String,
    /// 完整指纹（64 hex 字符）—— 信任判定的唯一依据。
    pub id_hex: String,
    /// 节点展示名。
    pub name: String,
    /// 实际监听的 `ip:port`。
    pub addr: String,
    /// `win` / `android` / `unknown`。
    pub platform: String,
    /// 能力位原始值。
    pub caps_bits: u16,
    /// 本机能否推流（= 配了采集回调）。
    pub can_send: bool,
    /// 本机能否接收（= 配了播放回调）。
    pub can_receive: bool,
    /// 协议版本（`0x0201`）。
    pub proto_version: u16,
}

/// 一个对端的快照。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PeerView {
    /// 指纹短码。
    pub id_short: String,
    /// 完整指纹。
    pub id_hex: String,
    /// 对端展示名（对端自报，**不参与信任判定**）。
    pub name: String,
    /// 对端地址 `ip:port`。
    pub addr: String,
    /// `idle` / `handshaking` / `streaming` / `degraded` / `reconnecting` / `failed`
    /// （与桌面端契约 §6 的 `state` 字面量同一套）。
    pub state: String,
    /// 是否已在信任库中。
    pub trusted: bool,
    /// 该对端的遥测快照（`peers` 字段恒为 1）。
    pub telemetry: TelemetryView,
}

/// 遥测快照（`telemetry()`；字段尽可能对齐桌面契约 §6 的 `TelemetryView`）。
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct TelemetryView {
    /// 这份快照覆盖的对端数（`telemetry()` 是全部，`PeerView.telemetry` 恒为 1）。
    pub peers: u32,
    /// 流 ID（未推流时为 0）。
    pub stream_id: u32,
    /// QUIC 平滑 RTT（µs）。
    pub rtt_us: u32,
    /// 到达间隔抖动 P50（µs）。
    pub jitter_us: u32,
    /// 到达间隔抖动 P95（µs）。
    pub jitter_p95_us: u32,
    /// 丢包率（百分比，`0.0`–`100.0`）。
    pub loss_pct: f64,
    /// 实际编码码率（bps）。
    pub bitrate_bps: u32,
    /// 帧长（ms）。
    pub frame_ms: u8,
    /// 声道数。
    pub channels: u8,
    /// 播放环水位（µs）。
    pub buffer_level_us: u32,
    /// 累计欠载次数。
    pub underruns: u32,
    /// 丢包隐藏次数（M1 只计数，不做掩盖）。
    pub plc_count: u32,
    /// 迟到丢弃包数。
    pub late_drops: u32,
    /// 估算端到端延迟（µs）。
    pub e2e_latency_us: u32,
}

impl TelemetryView {
    fn from_stats(stats: &StreamStats, peers: u32) -> Self {
        Self {
            peers,
            stream_id: stats.stream_id,
            rtt_us: stats.rtt_us,
            jitter_us: stats.jitter_us,
            jitter_p95_us: stats.jitter_p95_us,
            loss_pct: f64::from(stats.loss_pct_x100) / 100.0,
            bitrate_bps: stats.bitrate_bps,
            frame_ms: stats.codec.frame_ms,
            channels: stats.codec.channels,
            buffer_level_us: stats.buffer_level_us,
            underruns: stats.underruns,
            plc_count: stats.plc_count,
            late_drops: stats.late_drops,
            e2e_latency_us: stats.e2e_latency_us,
        }
    }
}

// ---------------------------------------------------------------------------
// 全局态
// ---------------------------------------------------------------------------

/// 引擎句柄 + 重启所需的配置。
struct EngineHandle {
    config: EngineStartConfig,
    engine: Arc<Engine>,
    advertiser: Option<audiolink_discovery::Advertiser>,
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, FfiError> {
    mutex
        .lock()
        .map_err(|_| FfiError::invalid_argument("internal FFI state lock is poisoned"))
}

/// 取（必要时创建）进程级运行时。
///
/// **在 `Mutex` 里做「检查 + 创建」**，不只是为了防数据竞争：建两个运行时再把多余的那个丢掉，
/// 等于在 async 上下文里 drop 一个 `Runtime` —— 那是 panic 路径（模块文档已说明）。锁内检查保证
/// 任何时刻只会建出一个。
fn runtime() -> Result<Arc<Runtime>, FfiError> {
    let mut guard = lock(&RUNTIME)?;
    if let Some(existing) = guard.as_ref() {
        return Ok(Arc::clone(existing));
    }
    let built = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(WORKER_THREADS)
        .thread_name("audiolink-rt")
        .enable_all()
        .build()
        .map_err(|error| {
            FfiError::invalid_argument(format!("failed to create the engine runtime: {error}"))
        })?;
    let created = Arc::new(built);
    *guard = Some(Arc::clone(&created));
    Ok(created)
}

/// 在引擎运行时上跑一个返回 `AudioLinkError` 的 future，把结果搬回调用方。
///
/// 外层 future 只 `await` 一个 `oneshot`：它不依赖任何运行时上下文，所以无论 UniFFI 在哪个
/// 线程上 poll（Kotlin 协程线程 / UI 线程）都成立。
async fn on_runtime<T, F>(future: F) -> Result<T, FfiError>
where
    T: Send + 'static,
    F: Future<Output = Result<T, audiolink_types::AudioLinkError>> + Send + 'static,
{
    let runtime = runtime()?;
    let (sender, receiver) = oneshot::channel();
    runtime.spawn(async move {
        // 调用方可能已经走了（Kotlin 协程取消）：发送失败即可，不 panic、不重试。
        let _ = sender.send(future.await);
    });
    match receiver.await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(FfiError::from_audio_link(&error)),
        Err(_) => Err(FfiError::invalid_argument(
            "engine task ended before replying (runtime shut down?)",
        )),
    }
}

/// 生命周期操作不随 Kotlin 等待协程取消而遗留半启动/半停止的引擎。
async fn on_lifecycle<T: Send + 'static>(
    operation: impl Future<Output = Result<T, FfiError>> + Send + 'static,
) -> Result<T, FfiError> {
    let runtime = runtime()?;
    let guard = LIFECYCLE.lock().await;
    let (sender, receiver) = oneshot::channel();
    runtime.spawn(async move {
        let _guard = guard;
        let _ = sender.send(operation.await);
    });
    receiver
        .await
        .map_err(|_| FfiError::invalid_argument("engine lifecycle task ended before replying"))?
}

fn require_engine(operation: &str) -> Result<Arc<Engine>, FfiError> {
    let guard = lock(&ENGINE)?;
    match guard.as_ref() {
        Some(handle) => Ok(Arc::clone(&handle.engine)),
        None => Err(FfiError::engine_not_started(operation)),
    }
}

/// M1 的「当前对端」：优先取正在推流的，否则取第一个。
///
/// 契约 §8 里 `startSend()` / `stopSend()` / `submitPin(pin)` 都不带对端参数 —— 这是**单对端 UI**
/// 的语义（Android 端 M1 一次只连一台 PC）。多对端选择器属后续里程碑。
fn current_peer(engine: &Engine) -> Result<NodeId, FfiError> {
    let peers = engine.peers();
    peers
        .iter()
        .find(|status| status.state.is_streaming())
        .or_else(|| peers.first())
        .map(|status| status.id)
        .ok_or_else(|| {
            FfiError::from_code(
                ErrorCode::NotPaired,
                "no peer session; call connect() first",
            )
        })
}

fn parse_addr(text: &str) -> Result<SocketAddr, FfiError> {
    let trimmed = text.trim();
    if let Ok(addr) = trimmed.parse::<SocketAddr>() {
        return Ok(addr);
    }
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_QUIC_PORT));
    }
    Err(FfiError::invalid_argument(format!(
        "invalid address {trimmed:?}; expected \"192.168.1.5\" or \"192.168.1.5:{DEFAULT_QUIC_PORT}\""
    )))
}

/// 壳侧给的设备标识：完整指纹或短码。两者的来源都是 [`peers()`] 返回的 [`PeerView`]。
enum PeerRef {
    /// 64 hex 完整指纹：不查表就能确定身份。
    Full(NodeId),
    /// 16 hex 短码：必须去会话表里查，且要求唯一命中。
    Short(String),
}

/// 把壳侧给的标识切成「完整指纹 / 短码」两种形态。
///
/// 先试完整指纹是因为它**无歧义**（`NodeId::from_hex` 只接受 64 hex）；短码是 64 位截断，
/// 理论上会撞（概率极低，但后果见 [`pick_short`]）。
fn parse_peer_ref(text: &str) -> Result<PeerRef, FfiError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(FfiError::invalid_argument(
            "peer id is empty; pass PeerView.idHex (or idShort) from peers()",
        ));
    }
    match NodeId::from_hex(trimmed) {
        Some(id) => Ok(PeerRef::Full(id)),
        None => Ok(PeerRef::Short(trimmed.to_lowercase())),
    }
}

/// 短码 → 唯一对端。**纯函数**（不碰引擎），所以「0 条 / 1 条 / 多条」三种结局都能单测。
///
/// 多条命中必须报错而不是取第一条：短码撞车时取第一条的后果是「点了 A 的停止，B 的流掉了」——
/// 这类错在真机上极难复现（要两台设备前 8 字节相同），也最难向用户解释。
fn pick_short(short: &str, known: &[NodeId]) -> Result<NodeId, FfiError> {
    let mut hits = known.iter().filter(|id| id.short_matches(short));
    match (hits.next(), hits.next()) {
        (Some(only), None) => Ok(*only),
        (None, _) => Err(FfiError::from_code(
            ErrorCode::NotPaired,
            format!("no session matches peer id {short}; call peers() and pass its idHex"),
        )),
        (Some(_), Some(_)) => Err(FfiError::invalid_argument(format!(
            "peer short id {short} matches more than one session; pass the full idHex from peers()"
        ))),
    }
}

/// 设备标识 → [`NodeId`]：完整指纹直接采信，短码在**会话表**里查。
///
/// # 为什么两种形态都收（「壳侧不做映射」的落点）
///
/// [`peers()`] 返回的每个 [`PeerView`] 同时带 `idHex` 与 `idShort`，都是壳侧**已经在手**的东西：
/// 界面上显示的是短码，列表去重/持久化更可能用完整指纹。任选其一直接回传即可 ——
/// 壳侧不必自己把短码换算成指纹（那正是「各端各写一份映射、迟早分叉」的来源）。
///
/// # 为什么只查会话表（不查信任库）
///
/// 本模块导出的按设备动作（停流 / 调音量）都是**对活会话**的动作：一台只在信任库里、
/// 没有会话的设备没有任何可停的流。删除配对记录是另一件事，桌面端有独立入口覆盖它。
fn resolve_peer(engine: &Engine, peer_id: &str) -> Result<NodeId, FfiError> {
    match parse_peer_ref(peer_id)? {
        PeerRef::Full(id) => Ok(id),
        PeerRef::Short(short) => {
            let known: Vec<NodeId> = engine.peers().into_iter().map(|status| status.id).collect();
            pick_short(&short, &known)
        }
    }
}

fn local_status_of(engine: &Engine) -> LocalStatus {
    let info = engine.info();
    LocalStatus {
        id_short: info.id.short(),
        id_hex: info.id.to_hex(),
        name: info.name.clone(),
        addr: engine.local_addr().to_string(),
        platform: info.platform.as_str().to_string(),
        caps_bits: info.caps.bits(),
        can_send: info.caps.contains(Caps::CAN_SEND),
        can_receive: info.caps.contains(Caps::CAN_RECEIVE),
        proto_version: info.proto_version,
    }
}

fn peer_view(status: PeerStatus) -> PeerView {
    PeerView {
        id_short: status.id.short(),
        id_hex: status.id.to_hex(),
        name: status.name.clone(),
        addr: status.addr.to_string(),
        state: status.state.name().to_string(),
        trusted: status.trusted,
        telemetry: TelemetryView::from_stats(&status.stats, 1),
    }
}

/// 连接成功是「对端已在会话表里」，所以查不到就是异常状态 —— 如实报错，而不是编一个 `state` 出来。
fn peer_view_for(engine: &Engine, id: NodeId) -> Result<PeerView, FfiError> {
    engine
        .peers()
        .into_iter()
        .find(|status| status.id == id)
        .map(peer_view)
        .ok_or_else(|| {
            FfiError::from_code(
                ErrorCode::BadRequest,
                format!("peer {} vanished right after the handshake", id.short()),
            )
        })
}

// ---------------------------------------------------------------------------
// 导出面（契约 §8）
// ---------------------------------------------------------------------------

/// 启动引擎并开始监听入站连接。
///
/// - `playout`：播放方向的内核 PCM 出口（Android 传 `AudioLinkService`）；`null` = 本机不接收；
/// - `capture`：发送方向的 PCM 入口；`null` = 本机不推流（M1 Android 即为 `null`）；
/// - 两个回调都传 `null` 时能力位为 0，UI 可以据此置灰按钮（能力位只由「有没有工厂」决定）。
///
/// 幂等：同一份 `config` 重复调用返回当前状态（Android 前台服务会被系统反复拉起，
/// 调用方的真实意图是「确保在跑」）；配置不同则返回 `1009 BUSY`，不做静默替换。
#[uniffi::export(async_runtime = "tokio")]
pub async fn engine_start(
    config: EngineStartConfig,
    playout: Option<Box<dyn PcmFeed>>,
    capture: Option<Box<dyn PcmPull>>,
) -> Result<LocalStatus, FfiError> {
    on_lifecycle(async move {
        {
            let guard = lock(&ENGINE)?;
            if let Some(handle) = guard.as_ref() {
                if handle.config == config {
                    return Ok(local_status_of(&handle.engine));
                }
                return Err(FfiError::from_code(
                    ErrorCode::Busy,
                    "engine is already running with a different config; call engineStop() first",
                ));
            }
        }

        // §13 能力：调用方（Android 侧）通过 `config.capabilities` **追加**它确实具备的平台能力位
        // （内录 / 麦克风），内核自身的能力（`Capabilities::CURRENT`）由内核说了算。
        //
        // 为什么用并集而不是直接赋值：不变量是「必需位（REQUIRED = OPUS）永远在」。直接赋值等于
        // 把整张位图的正确性交给调用方 —— 只写一个采集位的调用方会让双方缺必需位、**直接拒连**。
        let mut engine_config = EngineConfig::new(
            config.node_name.clone(),
            PathBuf::from(config.data_dir.clone()),
        );
        if config.listen_port != 0 {
            engine_config.listen.set_port(config.listen_port);
        }
        engine_config.capabilities = Capabilities::CURRENT | config.capabilities;
        engine_config.playout = playout.map(|feed| playout_factory(share_feed(feed)));
        engine_config.capture = capture.map(|pull| capture_factory(share_pull(pull)));
        let engine = Engine::start(engine_config)
            .await
            .map_err(|error| FfiError::from_audio_link(&error))?;
        let status = local_status_of(&engine);
        // 接受循环和会话都由 Engine 跟踪；shutdown 会等它们全部退出。
        let _accept = engine.spawn_accept_loop();
        let info = engine.info();
        let advertiser = if info.caps.contains(Caps::CAN_SEND) {
            let beacon = audiolink_proto::discovery::DiscoveryBeacon::new(
                info.id.short(),
                info.name,
                info.platform,
                info.caps,
                engine.local_addr().port(),
            );
            audiolink_discovery::Advertiser::start(beacon)
                .map_err(|error| {
                    tracing::warn!(%error, "LAN advertisement failed to start");
                })
                .ok()
        } else {
            None
        };
        *lock(&ENGINE)? = Some(EngineHandle {
            config,
            engine,
            advertiser,
        });
        Ok(status)
    })
    .await
}

/// 停止引擎（幂等）。未启动时返回 `Ok`：调用方的意图是「确保停下来」。
#[uniffi::export(async_runtime = "tokio")]
pub async fn engine_stop() -> Result<(), FfiError> {
    on_lifecycle(async {
        let taken = lock(&ENGINE)?.take();
        // 同步查询立即看到未启动；生命周期锁继续拦住新启动，直到旧资源全部释放。
        if let Some(handle) = taken {
            drop(handle.advertiser);
            handle.engine.shutdown().await;
        }
        Ok(())
    })
    .await
}

/// 主动连接对端（FR-17 手工 IP）。成功返回对端快照。
///
/// 对端要求 PIN 配对时返回 `1002 NOT_PAIRED`，但**这不是连接失败**：QUIC 握手与会话已经在，
/// UI 提示用户输入 PIN 后调用 [`submit_pin`] 即可继续同一条连接（见 engine `connect()` 的文档）。
#[uniffi::export(async_runtime = "tokio")]
pub async fn connect(addr: String) -> Result<PeerView, FfiError> {
    let engine = require_engine("connect")?;
    let socket = parse_addr(&addr)?;
    let id = on_runtime(async move { engine.connect(socket).await }).await?;
    let engine = require_engine("connect")?;
    peer_view_for(&engine, id)
}

/// 开始向当前对端推流（发送方向）。
#[uniffi::export(async_runtime = "tokio")]
pub async fn start_send() -> Result<(), FfiError> {
    let engine = require_engine("startSend")?;
    let peer = current_peer(&engine)?;
    on_runtime(async move { engine.start_send(peer).await }).await
}

/// 停止推流（保留连接与信任）—— **单对端语义**：停内核选中的那位 [`current_peer`]。
///
/// 多设备界面上请用 [`stop_send_to`]（显式点名对端）；本函数保留是给单对端调用方
/// （Android 现有代码）的兼容入口，两者**是同一个内核动作**，不是两套实现。
#[uniffi::export(async_runtime = "tokio")]
pub async fn stop_send() -> Result<(), FfiError> {
    let engine = require_engine("stopSend")?;
    let peer = current_peer(&engine)?;
    on_runtime(async move { engine.stop_send(peer).await }).await
}

/// 按设备停这一路（显式指定对端，`PeerView.idHex` 或 `idShort`）。
///
/// 与无参 [`stop_send`] 的区别**只有「停谁」**：无参版停 [`current_peer`]（单对端 UI 的兜底），
/// 本函数停调用方点名的那一台 —— 多设备界面上的「停止」按钮用这个。
///
/// 语义按**本机在链路的哪一侧**分成两种（内核只有一条实现，效果由此决定）：
/// - 本机是**发送端**：停本机采集并向对端发 `CLOSE_STREAM`，对端停止播放这一路；
/// - 本机是**接收端**：本机没有采集可停，实际效果是**请对端停发这一路**。
///
/// 两种都**保留连接与信任** —— 要彻底结束这条会话（仍然保留信任）用 [`disconnect_peer`]；
/// 要连信任一起撤、逼它重新配对是 [`Engine::revoke_trust`]（FFI 侧本里程碑不提供）。
#[uniffi::export(async_runtime = "tokio")]
pub async fn stop_send_to(peer_id: String) -> Result<(), FfiError> {
    let engine = require_engine("stopSendTo")?;
    let peer = resolve_peer(&engine, &peer_id)?;
    on_runtime(async move { engine.stop_send(peer).await }).await
}

/// 按设备调音量（发送方 → 对端播放侧；§4.1 的 `SET_GAIN`）。
///
/// `gain` 取值 0.0–2.0（1.0 = 原声）。NaN / 负数 / 超上限由**引擎边界**拒绝并返回人话原因，
/// 这里不重复校验 —— 判据只留一处，错误文案才能与桌面端逐字一致。
///
/// 渐变时长固定 [`DEFAULT_PEER_GAIN_RAMP_MS`]：壳侧的滑块拖动自带频次控制，
/// 多开一个参数只会让两端各自发明一套取值。
#[uniffi::export(async_runtime = "tokio")]
pub async fn set_peer_gain(peer_id: String, gain: f32) -> Result<(), FfiError> {
    let engine = require_engine("setPeerGain")?;
    let peer = resolve_peer(&engine, &peer_id)?;
    on_runtime(async move {
        engine
            .set_peer_gain(peer, gain, DEFAULT_PEER_GAIN_RAMP_MS)
            .await
    })
    .await
}

/// 设置**本机**对该设备的本地播放增益（FR-12：接收端每路独立音量）。
///
/// 与 [`set_peer_gain`] 方向相反、互不覆盖：
/// - [`set_peer_gain`] 是**发送方向**（让对端调它播放本机音频的音量，走网络）；
/// - 本函数只影响**本机混音**，一个字节都不发出去。
///
/// 最终音量 = **本地 × 对端下发**（相乘）—— 对端照常能调，本机再叠一层。
///
/// `gain` 取值 0.0–2.0（1.0 = 原声），非法值当场拒绝。**本地静音就是 `gain = 0.0`**：
/// 内核只维护「增益」一份状态，「取消静音时回到多少」由壳侧自己记 —— 它才是持有 UI 状态的一侧。
///
/// 设备当前没在收音频（本机是发送端 / 还没开流）也**照样成功**：用户设的是「这台设备的音量」，
/// 值会在下次开流时自然生效。
#[uniffi::export(async_runtime = "tokio")]
pub async fn set_local_peer_gain(peer_id: String, gain: f32) -> Result<(), FfiError> {
    let engine = require_engine("setLocalPeerGain")?;
    let peer = resolve_peer(&engine, &peer_id)?;
    on_runtime(async move { engine.set_local_peer_gain(peer, gain).map(|_| ()) }).await
}

/// 读回本机对该设备的本地增益（**千分点**；`null` = 用户没设过，等价 1.0）。
///
/// 为什么给千分点整数而不是浮点：内核内部一律用整数比较（浮点会让「有没有变化」不可复现，
/// 见 engine `gain.rs` 的说明），而界面上的滑块本来就是 0–2000 的离散值 —— 转成浮点再转回来
/// 只会引入误差。`None` 与「设成了 1000」是两件事：界面据此决定要不要打「已调整」标记。
#[uniffi::export]
pub fn local_peer_gain(peer_id: String) -> Result<Option<u32>, FfiError> {
    let engine = require_engine("localPeerGain")?;
    let peer = resolve_peer(&engine, &peer_id)?;
    Ok(engine.local_peer_gain(peer))
}

/// 断开与某台设备的会话（**保留信任**）。
///
/// 与 [`Engine::disconnect`] 同语义：只结束这条会话，信任库**不读不写不落盘** —— 该设备下次
/// 连进来仍然免交互直连（§8 白名单命中）。要「连信任一起撤、逼它重新配对」是另一件事
/// （引擎侧 [`Engine::revoke_trust`]；FFI 侧本里程碑不提供）。
///
/// 返回是否真的断了一条会话：`false` = 本来就没有（幂等，调用方不必先查 `peers()`）。
///
/// **别承诺做不到的事**：断开只让对端看到「链路丢失」，若对端是发起方且正在推流，
/// 它会按自己的 FR-27 逻辑重拨回来（本机信任库还留着它，握手直接过）。
#[uniffi::export(async_runtime = "tokio")]
pub async fn disconnect_peer(peer_id: String) -> Result<bool, FfiError> {
    let engine = require_engine("disconnectPeer")?;
    let peer = resolve_peer(&engine, &peer_id)?;
    on_runtime(async move { engine.disconnect(peer).await }).await
}

/// 提交对端屏幕上显示的 6 位 PIN（本机是发起端时用）。
#[uniffi::export(async_runtime = "tokio")]
pub async fn submit_pin(pin: String) -> Result<(), FfiError> {
    let engine = require_engine("submitPin")?;
    let peer = match engine.pending_pin_peer() {
        Some(peer) => peer,
        None => current_peer(&engine)?,
    };
    on_runtime(async move { engine.submit_pin(peer, &pin).await }).await
}

/// 本机状态。
#[uniffi::export]
pub fn local_status() -> Result<LocalStatus, FfiError> {
    let engine = require_engine("localStatus")?;
    Ok(local_status_of(&engine))
}

/// 已连接对端列表。
#[uniffi::export]
pub fn peers() -> Result<Vec<PeerView>, FfiError> {
    let engine = require_engine("peers")?;
    Ok(engine.peers().into_iter().map(peer_view).collect())
}

/// 遥测快照：当前对端的指标 + 对端总数。
#[uniffi::export]
pub fn telemetry() -> Result<TelemetryView, FfiError> {
    let engine = require_engine("telemetry")?;
    let peers = engine.peers();
    let stats = peers
        .iter()
        .find(|status| status.state.is_streaming())
        .or_else(|| peers.first())
        .and_then(|status| engine.telemetry(status.id))
        .unwrap_or_default();
    Ok(TelemetryView::from_stats(
        &stats,
        u32::try_from(peers.len()).unwrap_or(u32::MAX),
    ))
}

/// 本机（接收端）当前展示给用户的配对 PIN；没有在配对时返回 `None`。
#[uniffi::export]
pub fn displayed_pin() -> Result<Option<String>, FfiError> {
    // 保持未启动时返回 None 的既有 FFI 语义。锁住句柄直到查询结束，避免读到旧引擎。
    let guard = lock(&ENGINE)?;
    Ok(guard
        .as_ref()
        .and_then(|handle| handle.engine.displayed_pin()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::audio_bridge::PcmFeed;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 短码解析的三种结局：唯一命中 / 查无此设备 / 短码撞车。
    ///
    /// 改坏（撞车时取第一条）→ 用户在 A 上点「停止」，掉的是 B 的流。这类错在真机上要求
    /// 「两台设备前 8 字节相同」，几乎无法复现，也最难向用户解释。
    #[test]
    fn short_peer_ids_require_a_unique_match() {
        let first = NodeId::from_bytes([0x11; NodeId::LEN]);
        let second = NodeId::from_bytes([0x22; NodeId::LEN]);
        assert_eq!(
            pick_short(&first.short(), &[first, second]).ok(),
            Some(first)
        );
        assert_eq!(
            pick_short(&second.short(), &[first, second]).ok(),
            Some(second)
        );
        // 会话表里没有的短码：报「没有会话」，并提示去 peers() 拿 idHex。
        let absent = NodeId::from_bytes([0x33; NodeId::LEN]);
        assert!(pick_short(&absent.short(), &[first, second]).is_err());
        assert!(pick_short(&first.short(), &[]).is_err());

        // 短码相同、但完整指纹不同（只差最后一个字节）→ 必须报错，绝不猜。
        let mut twin_bytes = [0x11; NodeId::LEN];
        twin_bytes[NodeId::LEN - 1] = 0x99;
        let twin = NodeId::from_bytes(twin_bytes);
        assert_eq!(twin.short(), first.short(), "构造前提：两台短码相同");
        assert_ne!(twin, first);
        assert!(
            pick_short(&first.short(), &[first, twin]).is_err(),
            "短码撞车时不能猜一台"
        );
        // 完整指纹不查表也能定身份（撞车只影响短码路径）。
        assert_eq!(
            parse_peer_ref(&first.to_hex())
                .ok()
                .map(|r| matches!(r, PeerRef::Full(x) if x == first)),
            Some(true)
        );
    }

    /// 标识解析：大小写/首尾空白要容忍（壳侧的值多半来自复制粘贴或列表控件），空串要有人话原因。
    #[test]
    fn peer_id_parsing_is_forgiving_but_explicit() {
        let id = NodeId::from_bytes([0xAB; NodeId::LEN]);
        assert!(
            matches!(parse_peer_ref(&format!("  {}  ", id.to_hex())), Ok(PeerRef::Full(x)) if x == id)
        );
        // 大写十六进制照样能认（短码比较本来就是大小写不敏感）。
        assert!(matches!(
            parse_peer_ref(&id.short().to_uppercase()),
            Ok(PeerRef::Short(_))
        ));
        // 短码形态不在解析阶段失败：它要等会话表来判「有没有 / 唯一不唯一」。
        assert!(matches!(parse_peer_ref(&id.short()), Ok(PeerRef::Short(_))));
        // 空串：直接拒绝，并说清该传什么。
        assert!(parse_peer_ref("   ").is_err());
    }

    #[tokio::test]
    async fn lifecycle_cancellation_preserves_dispatched_work_and_skips_queued_work() {
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let finished = Arc::new(AtomicUsize::new(0));
        let done = Arc::clone(&finished);
        let first = tokio::spawn(on_lifecycle(async move {
            entered_tx.send(()).unwrap();
            release_rx.await.unwrap();
            done.store(1, Ordering::SeqCst);
            Ok(())
        }));
        entered_rx.await.unwrap();
        first.abort(); // 派发后的操作继续持有生命周期锁。
        let _ = first.await;

        let should_not_run = Arc::new(AtomicUsize::new(0));
        let marker = Arc::clone(&should_not_run);
        let mut queued = Box::pin(on_lifecycle(async move {
            marker.store(1, Ordering::SeqCst);
            Ok(())
        }));
        // 真实 poll 一次以确认已经排队；取消尚未获锁的请求，不应执行其操作。
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(queued.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        drop(queued);
        release_tx.send(()).unwrap();
        on_lifecycle(async { Ok(()) }).await.unwrap();
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert_eq!(should_not_run.load(Ordering::SeqCst), 0);
    }

    #[derive(Default)]
    struct CountingFeed {
        calls: AtomicUsize,
    }

    impl PcmFeed for CountingFeed {
        fn feed_pcm(&self, samples: Vec<f32>, _frames: i32) -> i32 {
            self.calls.fetch_add(1, Ordering::Relaxed);
            (samples.len() / 2) as i32
        }
    }

    fn config(dir: &std::path::Path) -> EngineStartConfig {
        EngineStartConfig {
            node_name: "ffi-test".to_string(),
            data_dir: dir.to_string_lossy().to_string(),
            listen_port: free_udp_port(),
            // 这条用例不关心平台能力位：显式写 0 = 与加这个字段之前逐位一致。
            capabilities: 0,
        }
    }

    /// 挑一个当前空闲的 UDP 端口（QUIC 跑在 UDP 上）。
    ///
    /// **用例里不能用 `listen_port: 0`**：那会落到默认端口 58290，而它随时可能被桌面端外壳或
    /// `audiolink-tools` 的延迟探针占着（或正在被另一个 teammate 的 `cargo test --workspace` 占用），
    /// 于是这条用例会以「地址被占用」随机变红 —— 那种红是环境噪声，不是缺陷信号。
    fn free_udp_port() -> u16 {
        std::net::UdpSocket::bind("127.0.0.1:0")
            .and_then(|socket| socket.local_addr())
            .map(|addr| addr.port())
            .unwrap_or(0)
    }

    /// 引擎生命周期：未启动 → 启动 → 幂等 → 配置冲突 → 停止 → 幂等。
    ///
    /// 这是**唯一**触碰全局引擎状态的用例（`cargo test` 的用例是并行线程，全局态不该被两个用例抢）。
    #[tokio::test]
    async fn 引擎生命周期与能力位() {
        // 1) 未启动时同步与异步导出面都必须给出 1008 而不是 panic
        let error = local_status().unwrap_err();
        assert_eq!(error.code(), ErrorCode::BadRequest.as_u16());
        assert!(
            error.context().contains("localStatus"),
            "{}",
            error.context()
        );
        let error = connect("127.0.0.1:1".to_string()).await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::BadRequest.as_u16());

        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());

        // 2) 不带任何音频回调 → 能力位为 0（UI 据此置灰）
        let silent = engine_start(cfg.clone(), None, None).await.unwrap();
        assert_eq!(silent.name, "ffi-test");
        assert_eq!(silent.id_short.len(), 16);
        assert_eq!(silent.id_hex.len(), 64);
        assert_eq!(
            silent.platform,
            if cfg!(windows) { "win" } else { "android" }
        );
        assert_eq!(silent.proto_version, audiolink_types::PROTO_VERSION);
        assert!(!silent.can_send && !silent.can_receive, "{silent:?}");
        assert!(silent.addr.parse::<SocketAddr>().is_ok(), "{}", silent.addr);

        // 3) 幂等：同配置重复启动返回同一身份
        let again = engine_start(cfg.clone(), None, None).await.unwrap();
        assert_eq!(again.id_hex, silent.id_hex);

        // 4) 配置不同 → 1009 BUSY，不做静默替换
        let other = EngineStartConfig {
            node_name: "other".to_string(),
            ..cfg.clone()
        };
        let error = engine_start(other, None, None).await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::Busy.as_u16(), "{error}");

        // 5) 停止 → 幂等
        engine_stop().await.unwrap();
        engine_stop().await.unwrap();
        assert_eq!(
            local_status().unwrap_err().code(),
            ErrorCode::BadRequest.as_u16()
        );

        // 6) 带播放回调重启：能力位必须随工厂变化（这是「唯一诚实的来源」那条纪律的端到端验证）
        let wired = engine_start(cfg.clone(), Some(Box::new(CountingFeed::default())), None)
            .await
            .unwrap();
        assert!(
            wired.can_receive && !wired.can_send,
            "配了播放回调就必须报 CAN_RECEIVE: {wired:?}"
        );
        assert_eq!(
            wired.caps_bits & Caps::CAN_RECEIVE.bits(),
            Caps::CAN_RECEIVE.bits()
        );

        // 7) 对端表空 → peers() 空、telemetry() 零值、displayedPin() 无
        assert!(peers().unwrap().is_empty());
        let view = telemetry().unwrap();
        assert_eq!(view.peers, 0);
        assert_eq!(view.rtt_us, 0);
        assert_eq!(view.loss_pct, 0.0);
        assert!(displayed_pin().unwrap().is_none());

        // 8) 没有对端时 startSend/stopSend 报 1002（不是 panic、也不是假成功）
        assert_eq!(
            start_send().await.unwrap_err().code(),
            ErrorCode::NotPaired.as_u16()
        );
        assert_eq!(
            stop_send().await.unwrap_err().code(),
            ErrorCode::NotPaired.as_u16()
        );
        assert_eq!(
            submit_pin("123456".to_string()).await.unwrap_err().code(),
            ErrorCode::NotPaired.as_u16()
        );

        engine_stop().await.unwrap();
    }

    #[test]
    fn 地址解析接受裸_ip_与_带端口() {
        let bare = parse_addr(" 192.168.1.5 ").unwrap();
        assert_eq!(bare.port(), DEFAULT_QUIC_PORT);
        let full = parse_addr("192.168.1.5:1234").unwrap();
        assert_eq!(full.port(), 1234);

        let error = parse_addr("not-an-ip").unwrap_err();
        assert_eq!(error.code(), ErrorCode::BadRequest.as_u16());
        assert!(error.context().contains("not-an-ip"), "{error}");
    }

    #[test]
    fn 遥测换算把百分比还原成小数() {
        let stats = StreamStats {
            loss_pct_x100: 250,
            rtt_us: 1_500,
            codec: audiolink_types::CodecStats {
                frame_ms: 20,
                channels: 2,
                complexity: 5,
            },
            ..StreamStats::default()
        };
        let view = TelemetryView::from_stats(&stats, 1);
        assert!((view.loss_pct - 2.5).abs() < f64::EPSILON);
        assert_eq!(view.frame_ms, 20);
        assert_eq!(view.channels, 2);
        assert_eq!(view.peers, 1);
    }
}
