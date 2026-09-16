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
use audiolink_types::{Caps, DEFAULT_QUIC_PORT, ErrorCode, NodeId, StreamStats};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

use crate::audio_bridge::{
    PcmFeed, PcmPull, capture_factory, playout_factory, share_feed, share_pull,
};
use crate::error::FfiError;

/// 工作线程数：QUIC 驱动 + 编解码会话任务都要跑，1 个线程会被网络 IO 阻塞拖死。
const WORKER_THREADS: usize = 4;

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
/// - `playout`：接收方向的内核 PCM 出口（Android 传 `AudioLinkService`）；`null` = 本机不接收；
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

        let mut engine_config = EngineConfig::new(
            config.node_name.clone(),
            PathBuf::from(config.data_dir.clone()),
        );
        if config.listen_port != 0 {
            engine_config.listen.set_port(config.listen_port);
        }
        engine_config.playout = playout.map(|feed| playout_factory(share_feed(feed)));
        engine_config.capture = capture.map(|pull| capture_factory(share_pull(pull)));
        let engine = Engine::start(engine_config)
            .await
            .map_err(|error| FfiError::from_audio_link(&error))?;
        let status = local_status_of(&engine);
        // 接受循环和会话都由 Engine 跟踪；shutdown 会等它们全部退出。
        let _accept = engine.spawn_accept_loop();
        *lock(&ENGINE)? = Some(EngineHandle { config, engine });
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

/// 停止推流（保留连接与信任）。
#[uniffi::export(async_runtime = "tokio")]
pub async fn stop_send() -> Result<(), FfiError> {
    let engine = require_engine("stopSend")?;
    let peer = current_peer(&engine)?;
    on_runtime(async move { engine.stop_send(peer).await }).await
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
