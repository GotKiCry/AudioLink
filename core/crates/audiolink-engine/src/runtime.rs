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
//! # 尚未落地的 M2/M3 能力
//!
//! - **抖动缓冲已完成第一阶段**：20--60 ms 自适应目标深度 + 一帧有界重排；
//!   预约播放、跨设备同步和基于 epoch 的绝对目标时刻仍属 M3。
//! - **不做预约播放 / 同步组**：§7 的 epoch 驱动排播属 M3 —— 那是「**用** offset 排播」，
//!   与「**算** offset」是两件事。§6 的时钟同步**已经接线**（见 [`crate::clock`]）：
//!   `CLOCK_PROBE` / `CLOCK_REPLY` 的收发节奏都在会话任务里，估计结果写进
//!   `StreamStats.clock_offset_us` / `drift_ppm`。`buffer_level_us` 是「队列里的帧数 × 帧长」的换算值。
//! - **冗余双发与 NACK 已落地，尚不做自适应码率**：副本都丢时按 §8.1 请求重传
//!   （重试 ≤ 5 次 / 间隔 10 ms / 窗口 1 s，且只在 RTT < 30 ms 时启用）；窗口内补不回来由 PCM 掩盖兜底。
//!
//! # 实时纪律
//!
//! 采集线程与播放线程的**稳态路径**里没有 `unwrap` / `expect` / 日志 / 堆分配；
//! 错误一律上报并触发状态机迁移，绝不静默停止（架构 §4）。

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{
    AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use audiolink_audio::mixer::{MixFormat, PcmMixer};
use audiolink_audio::{
    AudioError, CONCEAL_FADE_MS, CaptureSource, CodecConfig, FrameChunker, OpusDecoder,
    OpusEncoder, PcmConcealer, PlayoutSink, SampleStats,
};
use audiolink_identity::{IdentityError, NodeIdentity, PIN_TTL, TrustEntry, TrustStore};
use audiolink_net::{
    AudioLinkEndpoint, ClockEstimate, Connection, ControlChannel, EndpointConfig, NetError,
};
use audiolink_proto::{AudioDatagram, AudioDatagramHeader, NackList};
use audiolink_types::{
    AudioLinkError, Caps, ClockQuality, DATAGRAM_MAX_LEN, DEFAULT_QUIC_PORT, ErrorCode, Flags,
    NodeId, NodeInfo, Platform, Ptype, StreamStats,
};
use crossbeam_channel::{Receiver, Sender};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::task::TaskTracker;

use crate::adaptive::AdaptiveBitrate;
use crate::clock::{
    ClockProbeState, ClockProbeStats, STEADY_INTERVAL_MS, now_monotonic_us, publish_clock,
};
use crate::dispatch::{ControlRequest, DispatchStats, dispatch_into};
use crate::epoch::{EpochSchedule, PlayoutAction};
use crate::format_guard::require_unified_format;
use crate::gain::{GainState, gain_x1000_from_f32};
use crate::handshake::{Handshake, HandshakeEvent, HandshakeStep, Outgoing, Role};
use crate::measure::MeasurementTap;
use crate::payload::{
    CloseStreamPayload, CodecPref, GroupCreatePayload, GroupEpochPayload, GroupJoinPayload,
    GroupLeavePayload, OpenStreamAckPayload, OpenStreamPayload, SetGainPayload, SourceKind,
};
use crate::runtime::jitter::{
    AdaptiveJitterDepth, DEFAULT_TARGET_FRAMES, EncodedAudioPacket, MAX_TARGET_FRAMES,
    MIN_TARGET_FRAMES, PacketReorderBuffer, PlayoutDepthAction, PlayoutDepthState, ReorderBatch,
    publish_target,
};
use crate::runtime::nack::{
    MissingTracker, NACK_MAX_RTT_US, NACK_RETRANSMIT_GRACE, RetransmitBuffer,
};
use crate::runtime::sink_watchdog::SinkWatchdog;
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

/// §5 握手的**非配对阶段**死线：10 s（[`EngineConfig::handshake_timeout`] 的默认值）。
///
/// 对端要是连 `HELLO` / `AUTH_RESPONSE` 都发不全，就不值得占着一条会话 —— 半开连接必须能被回收。
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// 等人工输入 PIN 的窗口余量（秒）。
///
/// 作用是让「PIN 到期」与「连接被抽掉」**不在同一瞬间发生**：否则用户看到的会是
/// `1002 NOT_PAIRED（unknown peer）`，而不是「PIN 已过期」。前者会让人以为配对功能坏了。
const PIN_WAIT_MARGIN_SECS: u64 = 15;

/// 等人工输入 PIN 的窗口：§5 的 PIN 有效期 + 余量（[`EngineConfig::pin_wait_timeout`] 的默认值）。
///
/// # 为什么是 60 s 而不是 10 s（task-10 真机阻断项）
///
/// `docs/03-protocol.md` §5 规定 PIN **60 s 有效、最多 5 次尝试**，而「把手机屏幕上的 6 位数字
/// 读到 PC 上敲进去」是**唯一的真实配对流程**（FR-17 手工配对）。原来的 10 s 握手死线对这段
/// 人工流程必然超时 —— 真机现场实测：device-link 连上真机、手机上 PIN 显示出来之后 30–40 s
/// 才提交，拿到 `1002 NOT_PAIRED（unknown peer）`（会话已被死线回收，见 `target/evidence/`
/// 下 device-link 的真机记录）。
///
/// 所以：**进入配对等待时把死线顺延到本值**（从顺延那一刻起算），配对窗口的权威留在
/// [`audiolink_identity::PinGate`]（60 s 过期 / 5 次锁定 / 锁 5 min），引擎**不再另立一套语义**。
/// 非配对阶段仍按 [`DEFAULT_HANDSHAKE_TIMEOUT`] 回收 —— 安全底线没有被放宽。
pub const DEFAULT_PIN_WAIT_TIMEOUT: Duration =
    Duration::from_secs(PIN_TTL.as_secs() + PIN_WAIT_MARGIN_SECS);

/// QUIC 空闲超时的默认值（[`EngineConfig::idle_timeout`]）。
///
/// # 为什么是 30 s 而不是 10 s
///
/// `docs/05-roadmap.md` 的 M2 验收写的是「拔网 10 s 后 ≤ 3 s 恢复」。空闲超时一旦先于
/// 「拔网 + 恢复预算」结束，连接就被判死，而引擎**没有**重拨路径（会话循环直接进
/// `SessionState::Failed`，见 `report_peer_gone`）—— 插回网线也救不回来。
///
/// 旧值 10 s 正好卡在验收那条线上：2026-09-17 的实测（`tests/engine/network_outage.rs`）
/// 拔网 10 s 后 8 s 观测窗内**完全没有恢复**，两侧会话表里连对端都被移除了。
/// 30 s = 10 s（最长可容忍断网）+ 3 s（恢复预算）再留 2 倍余量，也给移动网络下的
/// 切换/弱信号留出喘息。
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// QUIC 保活探测间隔的默认值（[`EngineConfig::keep_alive`]）。
///
/// # 为什么是 1 s 而不是 3 s
///
/// 拔网期间没有任何包能出去；插回后链路要等**下一次保活探测**才被重新点亮，
/// 于是「恢复延迟」的上界就是保活间隔本身。旧值 3 s 时实测恢复延迟呈**双峰**
/// （597 / 3417 / 3310 ms，见 `tests/engine/network_outage.rs`），峰值直接压过验收的 3 s 预算。
/// 降到 1 s 后上界 ≈ 1 s，余量充足；代价是每个会话每秒一个几十字节的探测包。
pub const DEFAULT_KEEP_ALIVE: Duration = Duration::from_secs(1);

/// 断链重连的总预算（FR-27）：从收到重连请求算起，超过它即判 ReconnectFailed。
///
/// 20 s 的依据是 M2 的验收形状：**拔网 10 s** 要能回来，另外留 10 s 给「插回后第一次握手真正
/// 成功」的余量（移动网络切换时首次握手可能要多试几次）。
pub const DEFAULT_RECONNECT_BUDGET: Duration = Duration::from_secs(20);

/// 单次重连尝试的超时。**必须远小于总预算**：断网期间每次拨号都挂在握手等待上，一次尝试若吃掉
/// 十几秒，退避循环会退化成「只试一两次」，插回网线时正好卡在尝试中间 —— 那等于没有重连。
const RECONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(800);

/// 重连退避的起点与上限。
///
/// 上限不放大是为了守住 M2 的 3 s：最坏恢复 ≈ 一次尝试超时 + 一次退避 = 800 + 400 = **1.2 s**，
/// 还得留余量给「重连后再等第一帧音频」。上一版用 1.5 s + 1.0 s，最坏 2.5 s 已贴着验收线，
/// 叠上握手与抖动就会越线（队友在 docs/50 §2.7 算过这笔账）。
const RECONNECT_BACKOFF_MIN: Duration = Duration::from_millis(150);
const RECONNECT_BACKOFF_MAX: Duration = Duration::from_millis(400);

/// 会话「多久没听到对端任何包」就认定链路需要重建（FR-27 的触发阈值）。
///
/// **必须明显大于时钟探测的稳态间隔**（`STEADY_INTERVAL_MS` = 1 s，双方互发）—— 这个判据的适用对象
/// 是**发起方**，而发起方在单向推流里几乎收不到业务包，它的「心跳」就是对端那 1 s 一次的探测。
/// 第一版取 1 s，结果是**正常推流也会误报**：一发一收刚好贴着阈值，于是反复重建连接、音频永远不稳
/// （实测症状：恢复延迟 None，15 s 窗口内没声音，但两侧会话表都显示 Streaming）。
/// 3 s = 三次探测都没来，才认定对方真的不在了；相对 20 s 的重连预算也留足了触发余量。
const LINK_STALL_THRESHOLD_MS: u64 = 3_000;

/// 断链重连请求（FR-27）。
struct ReconnectRequest {
    /// 对端地址（重拨用）。
    addr: std::net::SocketAddr,
    /// 对端身份；新会话必须还是它，否则说明重拨连到了别的设备。
    peer: NodeId,
    /// 投递这个请求的那条会话。
    ///
    /// 带着它走是为了两件事：① 单飞门（`reconnect_in_flight`）挂在这条会话上，重连终局时
    /// 必须由**同一个对象**清位 —— 摘表之后从 `peers` 里已经找不到它了；
    /// ② 预算耗尽要落 `ReconnectFailed` 时也有对象可落。
    session: Arc<PeerSession>,
}

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
    /// §5 握手的**非配对阶段**死线；默认 [`DEFAULT_HANDSHAKE_TIMEOUT`]（10 s）。
    ///
    /// 测试可以调小它（真实 I/O 下 tokio 的时钟暂停不可靠，改常量比 pause/advance 更诚实）。
    pub handshake_timeout: Duration,
    /// 等人工输入 PIN 的窗口；默认 [`DEFAULT_PIN_WAIT_TIMEOUT`]（PIN 有效期 60 s + 15 s 余量）。
    ///
    /// 进入配对等待（发出 / 收到 `PAIR_REQUIRED`）后，握手死线顺延到本值**从此刻起算**
    /// —— 理由见 [`DEFAULT_PIN_WAIT_TIMEOUT`] 的文档。
    pub pin_wait_timeout: Duration,
    /// §13 能力协商：本端声明的能力位图；默认 [`audiolink_types::Capabilities::CURRENT`]。
    ///
    /// **为什么要可注入**：`CURRENT` 说的是「**内核**能做到什么」，而真实设备还有平台差异 ——
    /// Windows 有 WASAPI loopback（系统内录），Android 的内录尚未实现。平台侧在构造引擎时
    /// 声明自己的能力，能力协商才有意义；把它写死成常量，等于让所有设备都说自己一样。
    pub capabilities: u32,
    /// QUIC 空闲超时；默认 [`DEFAULT_IDLE_TIMEOUT`]（30 s）。
    ///
    /// **这个值决定「拔网多久还能自愈」** —— 它必须大于「最长可容忍断网 + 恢复预算」，
    /// 理由见 [`DEFAULT_IDLE_TIMEOUT`] 的文档与 `tests/engine/network_outage.rs` 的实测。
    /// `Duration::ZERO` = 关闭空闲超时（QUIC 层面永不因静默判死）。
    pub idle_timeout: Duration,
    /// QUIC 保活探测间隔；默认 [`DEFAULT_KEEP_ALIVE`]（1 s）。
    ///
    /// 拔网期间没有任何包能出去，插回后链路要等**下一次保活探测**才被重新点亮 ——
    /// 恢复延迟的上界就是它。`Duration::ZERO` = 关闭保活。
    pub keep_alive: Duration,
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
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
            pin_wait_timeout: DEFAULT_PIN_WAIT_TIMEOUT,
            capabilities: audiolink_types::Capabilities::CURRENT,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            keep_alive: DEFAULT_KEEP_ALIVE,
        }
    }

    /// 声明本端能力（见 [`EngineConfig::capabilities`]）。
    #[must_use]
    pub const fn with_capabilities(mut self, capabilities: u32) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// 覆盖链路层的两个时间参数（见 [`EngineConfig::idle_timeout`] / [`EngineConfig::keep_alive`]）。
    ///
    /// 成对覆盖：这两个值只有**一起**看才有意义 —— 空闲超时必须大于「最长可容忍断网」，
    /// 保活间隔决定插回网线后多久被重新点亮。
    #[must_use]
    pub const fn with_link_timeouts(
        mut self,
        idle_timeout: Duration,
        keep_alive: Duration,
    ) -> Self {
        self.idle_timeout = idle_timeout;
        self.keep_alive = keep_alive;
        self
    }
}

/// `Duration` → QUIC 的毫秒字段（0 = 关闭该机制，与 `EndpointConfig` 的语义一致）。
fn quic_ms(value: Duration) -> u32 {
    u32::try_from(value.as_millis()).unwrap_or(u32::MAX)
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
    /// §13 能力协商结果；`None` = 还没走完能力交换。
    pub capabilities: Option<PeerCapabilities>,
    /// 这条会话成功重连过几次（审计 §2.6-④ 的回执读数）。
    ///
    /// 为什么必须出现在**快照**里：`ReconnectOk` 只活在会话状态机里，而 UI 能读到的只有
    /// `Engine::peers()` 拿到的这份快照。计数不在这里暴露，"重连成功"在界面上就没有回执。
    pub reconnects: u64,
}

/// §13 能力协商结果（一个对端一份）。
///
/// 位图本身是 [`audiolink_types::Capabilities`] 的裸 `u32`；这里不解释语义，只保证
/// 「本端 / 对端 / 交集」三者**一起**到达界面 —— 少一个就说不清「为什么这个功能用不了」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCapabilities {
    /// 本端声明。
    pub local: u32,
    /// 对端声明。
    pub peer: u32,
    /// 双方交集（真正可用的能力）。
    pub agreed: u32,
}

impl PeerCapabilities {
    /// 对端**缺**的、而本端有的能力（界面据此置灰并说明原因）。
    #[must_use]
    pub const fn missing_on_peer(&self) -> u32 {
        self.local & !self.peer
    }
}

/// M4 观测：一路流「最近一帧的样本编号 ↔ 到达时刻」。
///
/// 为什么需要它：共同时间基准（`RECEIVER_EPOCH`）落地之后，「两路到底有没有对齐」需要一个**直接读数**。
/// 有了这一对值，就能用**同一个**「现在」推算两路各自的编号：编号已对齐时两个值应当相等（差 < 1 帧）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamAxis {
    /// 对端身份。
    pub peer: NodeId,
    /// 最近一帧的 `sample_index`；`None` = 本会话还没收到过音频数据报。
    pub sample_index: Option<u32>,
    /// 那一帧到达本端的时刻（本端单调时钟，毫秒）。
    pub at_ms: u32,
}

impl StreamAxis {
    /// 用**同一个**「现在」推算这一路当前的样本编号。
    ///
    /// 两路编号已经对齐到共同基准时，这个值应当相等（差 < 1 帧）；否则差值就是两路的时间轴错位。
    /// 采样率按 48 kHz（1 ms = 48 样本）—— 它要回答的是「差了几十毫秒，还是差不多零」。
    pub fn index_at(&self, now_ms: u32) -> Option<u32> {
        let index = self.sample_index?;
        let elapsed_ms = now_ms.saturating_sub(self.at_ms);
        Some(index.wrapping_add(elapsed_ms.saturating_mul(48)))
    }
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
    /// §8 自适应码率生效（降级 / 恢复）—— 验收「自适应生效」就看这条时间线。
    CodecAdapted {
        /// 变更前的目标码率（bps）。
        from_bps: i32,
        /// 变更后的目标码率（bps）。
        to_bps: i32,
        /// 触发原因（人类可读）。
        reason: String,
    },
    /// §7 同步组变化（建组 / 成员加入退出）：成员表与基准的可见来源。
    GroupUpdated {
        /// 组 ID。
        group_id: u32,
        /// 组基准标识。
        epoch_id: u64,
        /// 当前成员数。
        members: u32,
    },
    /// §7 预约播放生效（接收端按 epoch 排播）：组内同步的验收时间线就看它。
    PlayoutScheduled {
        /// 组基准标识。
        epoch_id: u64,
        /// 该帧首样本在本机时钟轴上的目标时刻（µs）。
        target_local_us: i64,
        /// 触发时的等待量（µs；0 = 正好赶上）。
        wait_us: u64,
    },
    /// 会话级错误（不致命；致命路径走 `PeerDisconnected`）。
    Error {
        /// §11 错误码。
        code: u16,
        /// 详情。
        context: String,
    },
}

/// §7 组管理帧（发送方 → 成员）。
#[derive(Debug, Clone)]
enum GroupFrame {
    Create(GroupCreatePayload),
    Join(GroupJoinPayload),
    Leave(GroupLeavePayload),
}

/// 应用 → 会话任务的指令。
#[derive(Debug)]
enum SessionCommand {
    /// 向对端开流（发送方向）。
    StartSend(oneshot::Sender<Result<(), AudioLinkError>>),
    /// 关闭当前流（保留连接）。
    CloseStream {
        /// 关闭原因。
        reason: String,
    },
    /// 提交 PIN（发起端）。
    SubmitPin(String),
    /// 关闭整个会话。
    Shutdown,
    /// §7：向对端广播组基准（发送方 → `GROUP_EPOCH`）。
    AnnounceGroupEpoch(GroupEpochPayload),
    /// M4：把**接收端**广播的共同基准转给对端（接收端 → 发送端，`RECEIVER_EPOCH`）。
    BroadcastReceiverEpoch(GroupEpochPayload),
    /// §4.1：向对端下发音量变更（发送方 →）。
    SetGain {
        /// 目标增益（0.0–2.0，与协议一致）。
        gain: f32,
        /// 渐变时长（ms）。
        ramp_ms: u32,
        /// 完成回执。
        reply: oneshot::Sender<Result<(), AudioLinkError>>,
    },
    /// §7：向对端发送组管理帧（`GROUP_CREATE` / `GROUP_JOIN` / `GROUP_LEAVE`）。
    SendGroupFrame(GroupFrame),
    /// §7：给接收侧设置（或清除）预约播放基准；`None` 表示回到本地游标排播。
    SchedulePlayout {
        /// 组基准；`None` = 关闭预约。
        schedule: Option<EpochSchedule>,
        /// 完成回执。
        reply: oneshot::Sender<Result<(), AudioLinkError>>,
    },
}

/// 会话表项：控制面与应用共享的句柄。
struct PeerSession {
    id: NodeId,
    name: Mutex<String>,
    addr: std::net::SocketAddr,
    machine: Mutex<SessionMachine>,
    telemetry: Arc<Mutex<TelemetryAggregator>>,
    /// §6 的时钟探测状态（估计器 + 未决探测表 + 计数 + RTT 环）。**每个对端一份**。
    clock: Mutex<ClockProbeState>,
    /// 对端最近一次 1 Hz `STREAM_STATS` 快照（**对端视角**）。按 `peer` 隔离，多对端不串流。
    peer_stats: Mutex<Option<StreamStats>>,
    commands: mpsc::Sender<SessionCommand>,
    trusted: AtomicBool,
    state: Mutex<SessionState>,
    /// 会话任务在控制帧写出之前更新；UI 查询不依赖可丢失的广播事件。
    pairing: Mutex<PairingState>,
    /// M4 观测：最近一帧的（到达毫秒 << 32 | 编号）。一次 64 位原子写，读者不会读到撕裂组合。
    rx_axis: Arc<AtomicU64>,
    /// FR-27：对端最近一次「有任何包到达」的本机单调时刻（µs）。0 = 还没听到过。
    ///
    /// 为什么需要它：拔网时 QUIC **不会报错** —— 它只是收不到回包，直到 idle_timeout（30 s）才把
    /// 连接判死。所以「链路是不是断了」不能等读写报错，必须自己数「多久没听到对端」。
    last_rx_us: AtomicU64,
    /// FR-27：本会话是否已有一条重连在飞（**单飞门**）。
    ///
    /// 重连请求至少有三个来源：两个链路错误出口 + 静默看门狗。两套 `reconnect_once` 并发时
    /// 会互相摘掉对方刚插进表里的会话，症状是「接上又断」—— 所以投递前先在这里抢一次，
    /// 终局（成功、失败、引擎关闭）时清位。
    reconnect_in_flight: AtomicBool,
    /// §13 能力协商结果（`None` = 还没协商完）。
    capabilities: Mutex<Option<PeerCapabilities>>,
    /// §7：**待应用**的组基准 —— 收到 `GROUP_EPOCH` 时若播放句柄还没建好（流没开），先存这里，
    /// `OpenStream` 建好句柄后立刻应用。
    ///
    /// 为什么必须有这个槽位（第 58 轮定位的真缺陷）：`GROUP_EPOCH` 常常**先于开流**到达，
    /// 而那一分支原先只在 `playback` 已存在时才 `set_schedule`，否则静默跳过 —— 排播从未生效，
    /// 接收端退回本地游标，组内同步形同虚设。
    pending_schedule: Mutex<Option<EpochSchedule>>,
}

#[derive(Default)]
struct PairingState {
    displayed: Option<(String, Instant)>,
    needs_pin: bool,
}

impl PeerSession {
    /// M4 观测：记下「最近一帧的编号 ↔ 到达时刻」。
    ///
    /// 距离「最后一次听到对端任何包」过去了多少毫秒。
    ///
    /// 一次都没听到过时返回 0：刚建好的会话不该被判成静默，否则重连会在握手期就自己触发。
    fn silent_for_ms(&self) -> u64 {
        let last = self.last_rx_us.load(Ordering::Relaxed);
        if last == 0 {
            return 0;
        }
        now_monotonic_us().saturating_sub(last) / 1_000
    }

    /// 打包成**一次** 64 位原子写（高 32 位 = 毫秒时刻，低 32 位 = 编号），于是读者不会读到
    /// 「新编号 + 旧时刻」这种撕裂组合；单次 relaxed 存储，实时路径不加锁、不分配。
    fn note_rx_axis(&self, sample_index: u32) {
        let at_ms = u32::try_from(now_monotonic_us() / 1000).unwrap_or(u32::MAX);
        self.rx_axis.store(
            u64::from(at_ms) << 32 | u64::from(sample_index),
            Ordering::Relaxed,
        );
    }

    /// 记下一次能力协商结果（§13）。
    fn set_capabilities(&self, local: u32, peer: u32, agreed: u32) {
        if let Ok(mut slot) = self.capabilities.lock() {
            *slot = Some(PeerCapabilities {
                local,
                peer,
                agreed,
            });
        }
    }

    fn update_pairing(&self, handshake: &Handshake, event: &HandshakeEvent) {
        if let Ok(mut pairing) = self.pairing.lock() {
            pairing.displayed = handshake
                .display_pin_state()
                .map(|(pin, expires_at)| (pin.to_string(), expires_at));
            match event {
                HandshakeEvent::NeedPin { .. } | HandshakeEvent::PinRejected { .. } => {
                    pairing.needs_pin = true;
                }
                HandshakeEvent::Established { .. } | HandshakeEvent::Rejected { .. } => {
                    pairing.needs_pin = false;
                }
                _ => {}
            }
        }
    }

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
            capabilities: self.capabilities.lock().map(|caps| *caps).unwrap_or(None),
            reconnects: self
                .machine
                .lock()
                .map(|machine| machine.reconnects())
                .unwrap_or(0),
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

    /// 状态机当前状态。
    ///
    /// 与 [PeerSession::set_state] 写的那个 `state` 快照字段**不是一回事**：这个由**被接受的
    /// 事件**驱动（计数也挂在迁移成功之后），快照字段只是给 UI 读的最近值。重连回执要走状态机，
    /// 所以得先看清它现在站在哪一格。
    fn machine_state(&self) -> SessionState {
        self.machine
            .lock()
            .map(|machine| machine.state())
            .unwrap_or(SessionState::Failed)
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
    /// 注册任务与关闭入口共用此锁，停止后不能再生成漏出清理范围的任务。
    tasks: Mutex<TaskTracker>,
    listen_addr: std::net::SocketAddr,
    /// §7 临时同步组的账本（发送侧写、查询用）：组 ID → 基准与成员。
    groups: Mutex<HashMap<u32, GroupState>>,
    /// M3 多会话：引擎级共享采集枢纽（第一个会话启流时创建，最后一个停止时摘掉）。
    capture_hub: Mutex<Option<Arc<CaptureHub>>>,
    /// M4 汇聚：引擎级混音器（第一个接收会话创建 sink，后续会话把自己的帧混进来）。
    ///
    /// FR-27：槽位里除了混音器还带着「谁在真正持有播放设备」—— 见 [`PlayoutMixSlot`]。
    /// 这个槽**从第一次建起永不置空**（这是 M4 多路汇聚的前提），所以「槽里有没有东西」
    /// 绝不能当作「我是不是 owner」的判据。
    playout_mixer: Mutex<Option<Arc<PlayoutMixSlot>>>,
    /// FR-27 断链重连请求：会话任务退出前把「这条会话说要重拨」交给监督任务。
    ///
    /// 为什么用通道而不是原地重连：会话任务的连接是**参数**，它一旦因链路错误退出就换不了连接；
    /// 而重拨 + 重新握手 + 重新开流需要对整个引擎操作。通道把这两件事分开。
    reconnect_tx: mpsc::UnboundedSender<ReconnectRequest>,
    /// FR-27：关闭通知。重连监督任务阻塞在 `recv()` 上，而引擎关闭时**没有人会 drop
    /// 发送端**（`Inner` 自己持着 `reconnect_tx`），`recv()` 便永不返回 —— 那个任务留在
    /// `TaskTracker` 里，`Engine::shutdown()` 的 `tasks.wait()` 就会一直等下去（实测：
    /// 验收测试在收尾处挂到超时，断言却已全绿）。
    shutdown_notify: tokio::sync::Notify,
}

impl Inner {
    fn spawn<T: Send + 'static>(
        &self,
        future: impl Future<Output = T> + Send + 'static,
    ) -> Result<tokio::task::JoinHandle<T>, AudioLinkError> {
        let tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        if self.shutdown.load(Ordering::Relaxed) {
            return Err(AudioLinkError::bad_request("engine is stopping or stopped"));
        }
        Ok(tasks.spawn(future))
    }
}

/// 一个临时同步组的账本条目（§7 / FR-22）。
#[derive(Debug, Clone)]
struct GroupState {
    epoch: EpochSchedule,
    lead_ms: u32,
    members: BTreeSet<NodeId>,
}

/// 组内一个成员及其同步质量（§6.5 / §7）。
///
/// 质量必须随成员表一起出去：§7 明写「若某成员 Poor 质量 → UI 必须明示该设备同步质量差」——
/// 而 UI 要判断这件事，只能靠这里给出的分级，不能自己去猜 RTT。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupMember {
    /// 成员（证书指纹）。
    pub id: NodeId,
    /// §6.5 的时钟质量分级；还没有时钟估计时按 Poor 处理。
    pub quality: ClockQuality,
    /// 时钟偏移估计（对端 − 本机，µs）；还没有估计时为 None。
    pub offset_us: Option<i64>,
}

/// 同步组快照（UI 与验收报告的数据源）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupSnapshot {
    /// 组 ID。
    pub group_id: u32,
    /// 组基准标识。
    pub epoch_id: u64,
    /// 预约提前量（ms）。
    pub lead_ms: u32,
    /// 成员及其同步质量（按指纹排序）。
    pub members: Vec<GroupMember>,
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
            idle_timeout_ms: quic_ms(config.idle_timeout),
            keep_alive_ms: quic_ms(config.keep_alive),
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

        let (reconnect_tx, reconnect_rx) = mpsc::unbounded_channel();

        let engine = Arc::new(Self {
            inner: Arc::new(Inner {
                local,
                identity,
                trust: Mutex::new(trust),
                endpoint,
                config,
                peers: Mutex::new(HashMap::new()),
                groups: Mutex::new(HashMap::new()),
                capture_hub: Mutex::new(None),
                playout_mixer: Mutex::new(None),
                events,
                shutdown: AtomicBool::new(false),
                tasks: Mutex::new(TaskTracker::new()),
                listen_addr,
                reconnect_tx,
                shutdown_notify: tokio::sync::Notify::new(),
            }),
        });
        engine.spawn_reconnect_supervisor(reconnect_rx);
        Ok(engine)
    }

    /// 断链重连监督任务（FR-27）：串行收请求，每条交给一个引擎任务去退避重拨。
    ///
    /// 持 `Weak` 而不是 `Arc`：这个任务跑在引擎自己的任务表里，强引用会构成自我引用环、让引擎
    /// 永远回收不掉。升级失败即代表引擎已释放，退出即可。
    ///
    /// 子任务走 `inner.spawn` 而不是裸 `tokio::spawn`：只有前者登记在 `TaskTracker` 里，
    /// 引擎关闭时才回收得到（队友在 docs/50 §2.9 指出上一版漏了这点）。
    fn spawn_reconnect_supervisor(
        self: &Arc<Self>,
        mut requests: mpsc::UnboundedReceiver<ReconnectRequest>,
    ) {
        let weak = Arc::downgrade(self);
        let _ = self.inner.spawn(async move {
            while let Some(engine) = weak.upgrade() {
                if engine.inner.shutdown.load(Ordering::Relaxed) {
                    break;
                }
                // **不能裸等 `recv()`**：引擎关闭时没有任何人会 drop 发送端（`Inner` 自己
                // 持着 `reconnect_tx`），`recv()` 永不返回 ⇒ 本任务留在 `TaskTracker` 里，
                // `Engine::shutdown()` 的 `tasks.wait()` 会一直等下去。所以同时等关闭通知。
                let request = tokio::select! {
                    request = requests.recv() => request,
                    () = engine.inner.shutdown_notify.notified() => break,
                };
                let Some(request) = request else { break };
                let _ = engine
                    .inner
                    .spawn(reconnect_once(Arc::clone(&engine), request));
            }
        });
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
    /// M3 多会话：一次给多台接收端同时开流（≥8 台也走这一条路径）。
    ///
    /// 采集是**引擎级共享**的：所有会话拿到的是同一串帧、同一套 seq / sample_index，
    /// 所以各接收端按同一 epoch 播放时的偏差不会被采集端偷偷加上去。
    /// 每台的结果各自返回 —— 一台失败不影响其它：调用方按设备逐条报错。
    pub async fn start_send_many(
        &self,
        peers: &[NodeId],
    ) -> Result<Vec<(NodeId, Result<(), AudioLinkError>)>, AudioLinkError> {
        let mut results = Vec::with_capacity(peers.len());
        for peer in peers {
            let (ready_tx, ready_rx) = oneshot::channel();
            let outcome = self
                .send_command(*peer, SessionCommand::StartSend(ready_tx))
                .await;
            let result = match outcome {
                Ok(()) => ready_rx
                    .await
                    .unwrap_or_else(|_| Err(AudioLinkError::bad_request("session task is gone"))),
                Err(error) => Err(error),
            };
            results.push((*peer, result));
        }
        Ok(results)
    }

    /// §7 同步组：创建临时组并把组基准广播给成员（发送方）。
    ///
    /// 返回组 ID。epoch_local_us 取本机单调时刻，成员各自换算到本机轴后排播 ——
    /// 这正是「组内 ±10 ms」的机制部分（偏差本身的测量用 tools/sync-measure）。
    pub async fn create_group(
        &self,
        members: &[NodeId],
        lead_ms: u32,
    ) -> Result<u32, AudioLinkError> {
        let unique: BTreeSet<NodeId> = members.iter().copied().collect();
        if unique.is_empty() {
            return Err(AudioLinkError::bad_request(
                "a group needs at least one member",
            ));
        }
        let group_id = (random_u64() as u32).max(1);
        let epoch_id = random_u64().max(1);
        let epoch_local_us = now_monotonic_us();
        let epoch = EpochSchedule::new(epoch_id, epoch_local_us, lead_ms);
        {
            let mut table = self
                .inner
                .groups
                .lock()
                .map_err(|_| AudioLinkError::bad_request("group table poisoned"))?;
            table.insert(
                group_id,
                GroupState {
                    epoch,
                    lead_ms,
                    members: unique.clone(),
                },
            );
        }
        let payload = GroupCreatePayload {
            group_id,
            epoch_id,
            epoch_local_us,
            lead_ms,
            members: unique.iter().copied().collect(),
        };
        for member in &unique {
            self.send_command(
                *member,
                SessionCommand::SendGroupFrame(GroupFrame::Create(payload.clone())),
            )
            .await?;
        }
        let _ = self.inner.events.send(EngineEvent::GroupUpdated {
            group_id,
            epoch_id,
            members: u32::try_from(unique.len()).unwrap_or(u32::MAX),
        });
        Ok(group_id)
    }

    /// §7 同步组：成员动态加入（运行中的其它成员不受影响）。
    ///
    /// 新成员除了收到 JOIN，还会补一条 GROUP_EPOCH —— 它需要组基准才能排播。
    pub async fn join_group(&self, member: NodeId, group_id: u32) -> Result<(), AudioLinkError> {
        // **整份组基准都要复用**，包括 `epoch_local_us`：它是「样本序号 0 在发送端时钟上的时刻」，
        // 新成员拿它 + 自己的样本序号才换算得出正确的目标时刻。
        //
        // 第 61 轮修的真缺陷：这里原本只复用 `epoch_id`，`epoch_local_us` 却取了**当前时刻** ——
        // 于是新成员的目标时刻 = now + sample_index/48000 + lead，而 `sample_index` 是流开始以来累计的
        // （加入时已经是个大数），目标被推到未来好几秒。实测第 3 台在 4 s 里只写出 0.32 s 的音频，
        // 而 A、B 各 4 s —— 「加入了、也排播了，但几乎不出声」。
        let (epoch_id, epoch_local_us, lead_ms, count) = {
            let mut table = self
                .inner
                .groups
                .lock()
                .map_err(|_| AudioLinkError::bad_request("group table poisoned"))?;
            let state = table
                .get_mut(&group_id)
                .ok_or_else(|| AudioLinkError::bad_request("unknown group"))?;
            state.members.insert(member);
            let count = u32::try_from(state.members.len()).unwrap_or(u32::MAX);
            (
                state.epoch.epoch_id,
                state.epoch.epoch_local_us,
                state.lead_ms,
                count,
            )
        };
        self.send_group_frame(
            group_id,
            GroupFrame::Join(GroupJoinPayload { group_id, member }),
        )
        .await?;
        self.send_command(
            member,
            SessionCommand::AnnounceGroupEpoch(GroupEpochPayload {
                epoch_id,
                epoch_local_us,
                lead_ms,
            }),
        )
        .await?;
        let _ = self.inner.events.send(EngineEvent::GroupUpdated {
            group_id,
            epoch_id,
            members: count,
        });
        Ok(())
    }

    /// §7 同步组：成员退出（其余成员继续）。组空了就把账本条目删掉。
    pub async fn leave_group(&self, member: NodeId, group_id: u32) -> Result<(), AudioLinkError> {
        self.send_group_frame(
            group_id,
            GroupFrame::Leave(GroupLeavePayload { group_id, member }),
        )
        .await?;
        let (epoch_id, count) = {
            let mut table = self
                .inner
                .groups
                .lock()
                .map_err(|_| AudioLinkError::bad_request("group table poisoned"))?;
            let state = table
                .get_mut(&group_id)
                .ok_or_else(|| AudioLinkError::bad_request("unknown group"))?;
            state.members.remove(&member);
            let epoch_id = state.epoch.epoch_id;
            let count = u32::try_from(state.members.len()).unwrap_or(u32::MAX);
            if state.members.is_empty() {
                table.remove(&group_id);
            }
            (epoch_id, count)
        };
        let _ = self.inner.events.send(EngineEvent::GroupUpdated {
            group_id,
            epoch_id,
            members: count,
        });
        Ok(())
    }

    /// M4：引擎级混音器的累计观测（没有混音器时返回 None）。
    ///
    /// FR-27 判据 ⑨：重连 n 次后 `sources` 不得单调增长 —— 停止与让位都要把源号还回去，
    /// 否则反复重连会撞上 FR-12 的 8 路上限。
    pub fn mixer_stats(&self) -> Option<audiolink_audio::mixer::MixSnapshot> {
        let slot = self.inner.playout_mixer.lock().ok()?;
        let slot = slot.as_ref()?;
        let guard = slot.mixer.lock().ok()?;
        Some(guard.snapshot())
    }

    /// 同步组快照（组 ID 升序）。
    ///
    /// 成员的同步质量取自**每个会话自己的**时钟估计：所以这里先放掉组表锁再去查会话表，
    /// 避免「组表 ⇄ 对端表」两把锁被同时持有（那是最容易锁死的地方）。
    pub fn groups(&self) -> Vec<GroupSnapshot> {
        let raw: Vec<(u32, u64, u32, Vec<NodeId>)> = self
            .inner
            .groups
            .lock()
            .map(|table| {
                table
                    .iter()
                    .map(|(group_id, state)| {
                        (
                            *group_id,
                            state.epoch.epoch_id,
                            state.lead_ms,
                            state.members.iter().copied().collect(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        let peers = self
            .inner
            .peers
            .lock()
            .map(|table| table.clone())
            .unwrap_or_default();

        let mut list: Vec<GroupSnapshot> = raw
            .into_iter()
            .map(|(group_id, epoch_id, lead_ms, members)| GroupSnapshot {
                group_id,
                epoch_id,
                lead_ms,
                members: members
                    .into_iter()
                    .map(|id| {
                        let estimate = peers.get(&id).and_then(clock_estimate_of);
                        GroupMember {
                            id,
                            quality: estimate.map_or(ClockQuality::Poor, |value| value.quality),
                            offset_us: estimate.map(|value| value.offset_us),
                        }
                    })
                    .collect(),
            })
            .collect();
        list.sort_by_key(|snapshot| snapshot.group_id);
        list
    }

    /// 把一帧组管理消息发给组内全部成员。
    async fn send_group_frame(
        &self,
        group_id: u32,
        frame: GroupFrame,
    ) -> Result<(), AudioLinkError> {
        let members = {
            let table = self
                .inner
                .groups
                .lock()
                .map_err(|_| AudioLinkError::bad_request("group table poisoned"))?;
            table
                .get(&group_id)
                .map(|state| state.members.iter().copied().collect::<Vec<_>>())
                .unwrap_or_default()
        };
        for member in members {
            self.send_command(member, SessionCommand::SendGroupFrame(frame.clone()))
                .await?;
        }
        Ok(())
    }

    /// §4.1：向对端下发音量变更（发送方 →）。
    ///
    /// 音量状态在**接收端的播放侧**、每会话一份 —— 所以多会话时每台可以各自调音量，
    /// 而采样时间轴仍然共享（M3 交付物 1 的「每会话独立音量」）。
    pub async fn set_peer_gain(
        &self,
        peer: NodeId,
        gain: f32,
        ramp_ms: u32,
    ) -> Result<(), AudioLinkError> {
        let (reply, wait) = oneshot::channel();
        self.send_command(
            peer,
            SessionCommand::SetGain {
                gain,
                ramp_ms,
                reply,
            },
        )
        .await?;
        wait.await
            .map_err(|_| AudioLinkError::bad_request("session task is gone"))?
    }

    /// §7 同步组：向某个对端广播组基准（发送方 → `GROUP_EPOCH`）。
    ///
    /// `epoch_local_us` 取本机 `now_monotonic_us()`；接收端用各自的时钟偏移换算到本机轴后排播，
    /// 因此两台接收端会在**同一个组时刻**起播 —— 这正是「组内 ±10 ms」的定义。
    pub async fn announce_group_epoch(
        &self,
        peer: NodeId,
        epoch_id: u64,
        lead_ms: u32,
    ) -> Result<(), AudioLinkError> {
        let payload = GroupEpochPayload {
            epoch_id,
            epoch_local_us: now_monotonic_us(),
            lead_ms,
        };
        self.send_command(peer, SessionCommand::AnnounceGroupEpoch(payload))
            .await
    }

    /// M4 共同基准的当前值（本端单调 µs）；`None` = 本机没有采集枢纽，`i64::MIN` = 有枢纽但还没收到广播。
    ///
    /// 为什么要把它暴露出来：`broadcast_epoch` 那条链路（接收端广播 → 各发送端对齐编号）此前
    /// **没有任何当场可读的观测点** —— 对齐与否只体现在事后混音音频的编号对不对得上，验收只能靠听。
    /// 有了它，回环测试可以直接断言「广播前两端都是未对齐、广播后两端是同一个原点」。
    pub fn capture_epoch_us(&self) -> Option<i64> {
        let hub = self.inner.capture_hub.lock().ok()?.clone()?;
        Some(hub.epoch_us_value())
    }

    /// M4 共同基准：本机作为**接收端**（混音方），把所有发送端共用的时间原点广播出去。
    ///
    /// 与 [`Engine::announce_group_epoch`]（发送端指定、接收端排播）是**反方向**的：
    /// 「同一发送端 → 多台接收端」用 `0x43` 对齐；「多台发送端 → 一台接收端混音」只能由接收端
    /// 广播基准（`0x44`），否则两个发送端各自从启流瞬间编号，接收端按编号混音就错着几十毫秒。
    ///
    /// `epoch_local_us` 取**本机**单调时刻加 `lead_ms` 的余量：各发送端要等这个时刻到点才开始编号，
    /// 所以提前量是它们的准备时间。返回成功发出的会话数。
    pub async fn broadcast_epoch(&self, lead_ms: u32) -> Result<u32, AudioLinkError> {
        let peers: Vec<NodeId> = {
            let table = self
                .inner
                .peers
                .lock()
                .map_err(|_| AudioLinkError::bad_request("peer table poisoned"))?;
            table.keys().copied().collect()
        };
        if peers.is_empty() {
            return Err(AudioLinkError::bad_request("no connected peer to align"));
        }
        let payload = GroupEpochPayload {
            epoch_id: random_u64().max(1),
            epoch_local_us: now_monotonic_us().saturating_add(u64::from(lead_ms) * 1000),
            lead_ms,
        };
        let mut sent = 0u32;
        for peer in peers {
            self.send_command(peer, SessionCommand::BroadcastReceiverEpoch(payload))
                .await?;
            sent = sent.saturating_add(1);
        }
        Ok(sent)
    }

    /// §7 预约播放：给某个接收侧会话设置组基准（`None` = 回到本地游标排播）。
    ///
    /// 发送端在 `GROUP_EPOCH` 里给出 epoch 与提前量后由上层调用；只有本机是接收端
    /// （配了播放输出）时才真正生效。生效后起播时刻不再由「队列攒够」决定，而由 epoch 决定；
    /// 首次排播会发一条 [`EngineEvent::PlayoutScheduled`] 作为验收时间线。
    pub async fn schedule_playout(
        &self,
        peer: NodeId,
        schedule: Option<EpochSchedule>,
    ) -> Result<(), AudioLinkError> {
        let (reply, wait) = oneshot::channel();
        self.send_command(peer, SessionCommand::SchedulePlayout { schedule, reply })
            .await?;
        wait.await
            .map_err(|_| AudioLinkError::bad_request("session task is gone"))?
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    /// 已连接对端快照。
    /// M4 观测：每路流「最近一帧的编号 ↔ 到达时刻」。
    ///
    /// 与 [`Engine::broadcast_epoch`] 配套：一个负责让两路对齐，一个负责**证明**它对齐了。
    pub fn stream_axes(&self) -> Vec<StreamAxis> {
        let Ok(table) = self.inner.peers.lock() else {
            return Vec::new();
        };
        let mut axes: Vec<StreamAxis> = table
            .values()
            .map(|session| {
                let packed = session.rx_axis.load(Ordering::Relaxed);
                StreamAxis {
                    peer: session.id,
                    sample_index: if packed == u64::MAX {
                        None
                    } else {
                        Some(packed as u32)
                    },
                    at_ms: (packed >> 32) as u32,
                }
            })
            .collect();
        axes.sort_by_key(|axis| axis.peer);
        axes
    }

    /// 本端单调时钟（毫秒）。给「用同一个 now 比较两路编号」用（见 [`StreamAxis::index_at`]）。
    pub fn monotonic_ms(&self) -> u32 {
        u32::try_from(now_monotonic_us() / 1000).unwrap_or(u32::MAX)
    }

    pub fn peers(&self) -> Vec<PeerStatus> {
        self.inner
            .peers
            .lock()
            .map(|peers| peers.values().map(|p| p.snapshot()).collect())
            .unwrap_or_default()
    }

    /// 当前有效的接收端 PIN；无需订阅事件。多条配对请求时优先显示最新的一条。
    /// 成功、锁定、断开或引擎停止后清除；到期判断使用 PinGate 原始失效时刻。
    pub fn displayed_pin(&self) -> Option<String> {
        self.displayed_pin_at(Instant::now())
    }

    fn displayed_pin_at(&self, now: Instant) -> Option<String> {
        if self.inner.shutdown.load(Ordering::Relaxed) {
            return None;
        }
        let peers = self.inner.peers.lock().ok()?;
        peers
            .values()
            .filter_map(|session| session.pairing.lock().ok()?.displayed.clone())
            .filter(|(_, expires_at)| now < *expires_at)
            .max_by_key(|(_, expires_at)| *expires_at)
            .map(|(pin, _)| pin)
    }

    /// 本机作为发起端正在等待输入 PIN 的对端；与连接同生命周期，无事件缓存。
    pub fn pending_pin_peer(&self) -> Option<NodeId> {
        if self.inner.shutdown.load(Ordering::Relaxed) {
            return None;
        }
        let peers = self.inner.peers.lock().ok()?;
        peers
            .values()
            .find_map(|session| session.pairing.lock().ok()?.needs_pin.then_some(session.id))
    }

    /// 指定对端的遥测（**本机视角**：本机聚合的 1 Hz 快照，含本机算出的时钟估计）。
    pub fn telemetry(&self, peer: NodeId) -> Option<StreamStats> {
        let peers = self.inner.peers.lock().ok()?;
        let telemetry = peers.get(&peer)?.telemetry.lock().ok()?;
        Some(telemetry.snapshot())
    }

    /// **降档主动丢帧**的累计拍数（本机视角，按 `peer` 隔离）；对端不存在 → `None`。
    ///
    /// 与 [`Self::telemetry`] 快照里的 `late_drops` 是两个口径（第 85 轮拆分）：
    /// `late_drops` = 帧到得太晚、来不及播（网络 / 调度质量问题）；本计数 = 抖动深度降档那一拍
    /// 控制器主动丢最旧帧换更低延迟（`PlayoutDepthAction::DropOldest`，设计上每次降档必现）。
    ///
    /// **为什么不在 `StreamStats` 里**：那是 1 Hz `STREAM_STATS` 的 postcard 载荷，字段顺序即
    /// wire 顺序，解码端拒绝尾随字节 —— 追加字段会打断旧版本节点（§13 兼容）。所以它是
    /// **进程内可读**的观测口径：同进程的 soak-runner 靠它把「主动丢帧」与「真迟到」分开报告。
    pub fn depth_drops(&self, peer: NodeId) -> Option<u32> {
        let peers = self.inner.peers.lock().ok()?;
        let telemetry = peers.get(&peer)?.telemetry.lock().ok()?;
        Some(telemetry.depth_drops())
    }

    /// §6 的当前时钟估计；`None` = 对端不存在，或**有效样本 < 8 尚未收敛**。
    ///
    /// 绝不返回「样本不足但看起来像真的」的偏移：调用方拿到 `None` 就该明确呈现「未收敛/未测」。
    /// 跨机端到端延迟靠它把本机时刻换算到对端时基：
    /// `e2e = 本地出声时刻 − (对端采集时刻 + offset_us)`（`docs/10-handoff.md` §4.2）。
    pub fn clock_estimate(&self, peer: NodeId) -> Option<ClockEstimate> {
        let peers = self.inner.peers.lock().ok()?;
        let clock = peers.get(&peer)?.clock.lock().ok()?;
        clock.estimate()
    }

    /// 对端最近一次 1 Hz `STREAM_STATS` 快照（**对端视角**；本机视角见 [`Engine::telemetry`]）。
    ///
    /// 从未收到过对端的 `STREAM_STATS` 时返回 `None` —— 消费方必须能区分「对端没发」与
    /// 「对端发了但值就是 0」，否则会把「没测到」呈现成「测到了 0」。
    ///
    /// 按 `peer` 隔离：每个会话各存自己那份最近快照（`EngineEvent::Telemetry` 不带对端 id，
    /// 多对端时它无法区分来源，所以存储必须按 peer 分开 —— 见契约 §5 的已知局限）。
    pub fn peer_stats(&self, peer: NodeId) -> Option<StreamStats> {
        let peers = self.inner.peers.lock().ok()?;
        let session = peers.get(&peer)?;
        *session.peer_stats.lock().ok()?
    }

    /// §6 探针收发计数 + RTT 分位数（对端不存在 → `None`；会话刚建立 → 全 0 / 分位数 `None`）。
    pub fn clock_probe_stats(&self, peer: NodeId) -> Option<ClockProbeStats> {
        let peers = self.inner.peers.lock().ok()?;
        let clock = peers.get(&peer)?.clock.lock().ok()?;
        Some(clock.stats())
    }

    /// 启动入站接受循环。
    pub fn spawn_accept_loop(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let inner = Arc::clone(&self.inner);
        self.inner
            .spawn(async move {
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
                            let (session, commands) =
                                create_session(&inner, peer_id, addr, trusted);
                            let _ = inner.spawn(run_session(
                                Arc::clone(&inner),
                                connection,
                                session,
                                commands,
                                Role::Responder,
                                None,
                            ));
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
            .unwrap_or_else(|_| tokio::spawn(async {}))
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
        let engine = Arc::clone(self);
        self.inner
            .spawn(async move { engine.connect_inner(addr).await })?
            .await
            .map_err(|_| AudioLinkError::bad_request("connect task ended unexpectedly"))?
    }

    async fn connect_inner(
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
        self.inner.spawn(run_session(
            Arc::clone(&self.inner),
            connection,
            session,
            commands,
            Role::Initiator,
            Some(ready_tx),
        ))?;

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
    /// 等采集设备与编码器初始化、OPEN_STREAM 写出后返回；不代表对端已开始播放。
    pub async fn start_send(&self, peer: NodeId) -> Result<(), AudioLinkError> {
        let (ready_tx, ready_rx) = oneshot::channel();
        self.send_command(peer, SessionCommand::StartSend(ready_tx))
            .await?;
        ready_rx.await.map_err(|_| {
            AudioLinkError::bad_request("session ended or is not ready to start capture")
        })?
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

    /// 关闭引擎（幂等），等待接受/连接/会话任务、音频线程和 UDP 套接字释放。
    /// 返回后同端口可立即重启，保留旧 Engine 句柄不会继续占用端口。
    pub async fn shutdown(&self) {
        let tasks = {
            let tasks = self.inner.tasks.lock().unwrap_or_else(|e| e.into_inner());
            self.inner.shutdown.store(true, Ordering::Relaxed);
            // 叫醒重连监督任务：它不在任务表里自己做退出判定，只等这个通知（见 Inner 的字段文档）。
            self.inner.shutdown_notify.notify_one();
            tasks.close();
            tasks.clone()
        };

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
        tasks.wait().await;
        if let Ok(mut peers) = self.inner.peers.lock() {
            peers.clear();
        }
        self.inner.endpoint.shutdown().await;
    }

    async fn send_command(
        &self,
        peer: NodeId,
        command: SessionCommand,
    ) -> Result<(), AudioLinkError> {
        if self.inner.shutdown.load(Ordering::Relaxed) {
            return Err(AudioLinkError::bad_request("engine is stopping or stopped"));
        }
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

    // **状态机的起点**：会话任务开始跑，就代表「连接已发起（本端是发起方）」或
    // 「入站连接已被接受（本端是应答方）」。
    //
    // 缺了这一步，`SessionMachine` 会永远停在 `Idle`：`mark_streaming` 里的 `HandshakeOk`
    // 从 `Idle` 是**非法迁移**（迁移表只认 `Handshaking → Streaming`），于是 `handshakes` /
    // `reconnects` / `degradations` 三个计数**全是死的**，UI 看到的「状态」只是
    // `set_state` 写的快照字段，与状态机彻底脱钩。
    //
    // 审计 §2.6-④ 的「重连没有回执」有一半就来自这里：`ReconnectOk` 只在 `Reconnecting` 上
    // 合法，而这条会话的状态机从来没能离开 `Idle`。
    session.apply(match &role {
        Role::Initiator => SessionEvent::ConnectRequested,
        Role::Responder => SessionEvent::AcceptedInbound,
    });

    let peer_cert = match connection.peer_cert_der() {
        Ok(cert) => cert,
        Err(error) => {
            finish_ready(&mut ready, Err(net_error(&error)));
            drop_session(&inner, &session, "no peer certificate");
            return;
        }
    };
    // §6：控制流 #0 的获取在**服务端**是 `accept_bi()` —— 对端必须先往 #0 写第一个字节
    // （QUIC 的 `open_bi()` 是惰性的，见 `audiolink-net::Connection::open_control` 的文档）。
    // 而纯数据报的测量客户端（`audiolink-tools` 的 `latency-probe`）**永远不会**开控制流。
    // 所以这一步绝不能挡在「应答时钟探测」前面：那样会话任务会卡在这里，一个探测都答不了，
    // 真机验收量「PC→手机网络 RTT」的路就断了。两者必须**并行等**。
    let mut opening = Box::pin(connection.open_control());

    let mut rx_buf = vec![0u8; DATAGRAM_MAX_LEN];
    // 握手死线：**非配对阶段**按 handshake_timeout（默认 10 s）回收半开连接；
    // 一旦进入「等人工输入 PIN」的窗口，就顺延到 pin_wait_timeout（默认 75 s），
    // 而且**从顺延那一刻起算** —— 配对等待不吃握手死线（真机 1002 的根因，
    // 理由见 DEFAULT_PIN_WAIT_TIMEOUT 的文档）。
    let mut deadline = tokio::time::Instant::now() + inner.config.handshake_timeout;
    // 握手死线到点、但这条连接已经在应答时钟探测时，死线不再收回连接（测量连接专用，见下）。
    let mut serving_probes = false;
    let mut probes_answered: u64 = 0;

    // ---- 阶段零：拿到控制流之前，只做一件事 —— 应答 §6 的时钟探测 ----
    let mut control = loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(SessionCommand::Shutdown) | None => {
                        finish_ready(&mut ready, Err(AudioLinkError::bad_request("cancelled by local side")));


                        drop_session(&inner, &session, "shutdown before the control stream");
                        return;
                    }
                    // 握手还没开始，PIN 无处可去（控制流起来之后才轮得到它）。
                    _ => {}
                }
            }

            opened = &mut opening => {
                match opened {
                    Ok(control) => break control,
                    Err(error) => {
                        let error = net_error(&error);
                        finish_ready(&mut ready, Err(error.clone()));
                        drop_session(&inner, &session, error.context());
                        return;
                    }
                }
            }

            // §6：应答 CLOCK_PROBE **不依赖 §5 会话状态** —— 阶段零（连控制流都还没有）就在回包。
            //
            // 为什么必须这样：`audiolink-tools` 的 `latency-probe`（M0 交付物、
            // `docs/05-roadmap.md` M1 验收点名的跨机测量手段）只用数据报、不跑 §5 握手。
            //
            // 安全口径（Lead 已认可）：这里不额外要求 §5 认证 —— TLS 握手已经保证「对端可达且能完成
            // 握手」，且 12 B 进 → 28 B 出（放大 2.33×，还必须先完成 TLS 才到得了这一层），不构成
            // 源地址伪造的放大面。§5 认证保护的是「能不能收音频」，不是「能不能问时间」。
            //
            // 一条连接**只能有一个数据报读者**，所以这里与阶段一/二共用同一个读循环 ——
            // 另起一个「时钟任务」会与音频路径抢数据报（表现为偶发丢帧，最难查的那种症状）。
            datagram = connection.read_datagram_into(&mut rx_buf) => {
                match answer_probe_datagram(&connection, &session, datagram, &rx_buf).await {
                    ProbeStep::Replied => probes_answered = probes_answered.saturating_add(1),
                    ProbeStep::Ignored => {} // §1.1：忽略并计数，不断流
                    ProbeStep::SendFailed(error) => report_error(&inner, &session, &error),
                    ProbeStep::LinkLost(error) => {
                        finish_ready(&mut ready, Err(error.clone()));
                        drop_session(&inner, &session, error.context());
                        return;
                    }
                }
            }

            _ = tokio::time::sleep_until(deadline), if !serving_probes => {
                if !handshake_deadline_expired(&inner, &session, &mut ready, probes_answered) {
                    return;
                }
                serving_probes = true;
            }
        }
    };

    let trusted_before = session.trusted.load(Ordering::Relaxed);
    // §13：握手带上**本机声明的**能力（默认是内核能力，平台侧可覆写，见 `EngineConfig::capabilities`）。
    let mut handshake = Handshake::new(role, inner.local.clone(), peer_id, trusted_before)
        .with_capabilities(inner.config.capabilities);
    let mut queue: VecDeque<Outgoing> = VecDeque::new();
    enqueue(&mut queue, handshake.start());
    if let Err(error) = flush_control(&mut control, &mut queue).await {
        finish_ready(&mut ready, Err(error.clone()));
        drop_session(&inner, &session, error.context());
        return;
    }

    // ---------------------------------------------------------------
    // 阶段一：握手与配对（期间继续应答 §6 的时钟探测）
    // ---------------------------------------------------------------
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
                session.update_pairing(&handshake, &step.event);
                if let Some(peer) = handshake.peer() {
                    session.set_name(&peer.name);
                }
                enqueue(&mut queue, step.clone());
                if let Err(error) = flush_control(&mut control, &mut queue).await {
                    finish_ready(&mut ready, Err(error));
                    drop_session(&inner, &session, "control stream failed");
                    return;
                }

                match step.event {
                    HandshakeEvent::Established {
                        peer,
                        persist,
                        local_caps,
                        peer_caps,
                        agreed_caps,
                    } => {
                        // §13：把能力协商结果落到会话上 —— 界面要能回答「这台对端能做什么」。
                        session.set_capabilities(local_caps, peer_caps, agreed_caps);
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
                        report_peer_gone(&inner, &session, &error, false);
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
                        // 从这一刻起是**人工时间**：死线顺延到 PIN 窗口（真机现场 30–40 s 才提交）。
                        arm_pin_wait(&mut deadline, inner.config.pin_wait_timeout);
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
                        // 本端是响应方：现在轮到**对端**的用户看屏幕输数字，同样是人工时间。
                        // 顺延只做这一处（PIN 输错时**不**重算）：PIN 能不能用、还能试几次，
                        // 权威判据在 PinGate（60 s / 5 次 / 锁 5 min），引擎不另立一套语义。
                        arm_pin_wait(&mut deadline, inner.config.pin_wait_timeout);
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

            // 握手期间继续应答 §6 的探测（理由与安全口径见阶段零的注释）。
            datagram = connection.read_datagram_into(&mut rx_buf) => {
                match answer_probe_datagram(&connection, &session, datagram, &rx_buf).await {
                    ProbeStep::Replied => probes_answered = probes_answered.saturating_add(1),
                    ProbeStep::Ignored => {}
                    ProbeStep::SendFailed(error) => report_error(&inner, &session, &error),
                    ProbeStep::LinkLost(error) => {
                        finish_ready(&mut ready, Err(error.clone()));
                        drop_session(&inner, &session, error.context());
                        return;
                    }
                }
            }

            _ = tokio::time::sleep_until(deadline), if !serving_probes => {
                if !handshake_deadline_expired(&inner, &session, &mut ready, probes_answered) {
                    return;
                }
                serving_probes = true;
            }
        }
    }

    // ---------------------------------------------------------------
    // 阶段二：会话（控制面 + 音频收发 + 应用指令）
    // ---------------------------------------------------------------
    let mut audio_receiver = match AudioReceiver::new(codec) {
        Ok(receiver) => receiver,
        Err(error) => {
            report_error(&inner, &session, &audio_error(&error));
            drop_session(&inner, &session, "decoder init failed");
            return;
        }
    };

    let mut dispatch_stats = DispatchStats::default();

    let mut last_datagram_at: Option<Instant> = None;
    let frame_period = Duration::from_millis(u64::from(codec.frame_ms.max(1)));
    let frame_us = codec.frame_ms.max(1).saturating_mul(1_000);
    let jitter_depth = Arc::new(AtomicUsize::new(DEFAULT_TARGET_FRAMES));
    let mut adaptive_jitter = AdaptiveJitterDepth::new();
    let mut arrival_jitter = SampleStats::new(256);
    let mut packet_reorder = PacketReorderBuffer::new(frame_period);
    let mut last_adaptive_underruns = 0u32;
    let mut playback: Option<PlayoutHandle> = None;
    let mut next_stream_id: u32 = 1;

    let mut capture: Option<CaptureHandle> = None;
    let mut pending_redundant: Option<EncodedFrame> = None;
    let mut stream_id: Option<u32> = None;
    // FR-27：静默触发只做一次 —— 重连是整条连接的重建，反复触发只会互相打断。
    let mut reconnect_started = false;
    let mut epoch_id: u64 = 0;

    // §8.1 辅助抗丢包：接收侧的缺失跟踪 + 发送侧的重传窗口。
    let mut missing = MissingTracker::new();
    let mut retransmit = RetransmitBuffer::new();

    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // §8 自适应码率的决策器（1 Hz）。时钟基准取会话循环的起始时刻：策略只需要"单调推进的秒数"。
    let session_started = Instant::now();
    let mut adaptive_bitrate = AdaptiveBitrate::new(codec.bitrate_bps);

    // §6：会话可通信后立刻开始探测（首次到期即发），此后按 probe_interval() 推进：
    // 前 50 次 100 ms（首连快速同步 ≈ 5 s 收敛），之后 1 Hz 持续采样。
    let mut probe_deadline = tokio::time::Instant::now();

    loop {
        let reorder_deadline = packet_reorder
            .next_deadline()
            .map(tokio::time::Instant::from_std);
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break; };
                match command {
                    SessionCommand::StartSend(ready) => {
                        if ready.is_closed() {
                            continue;
                        }
                        if capture.is_some() {
                            let _ = ready.send(Err(AudioLinkError::owned(
                                ErrorCode::Busy, "capture is already running".to_string(),
                            )));
                            continue;
                        }
                        match start_send_pipeline(&inner, &session, &codec) {
                            Ok((handle, epoch)) => {
                                capture = Some(handle);
                                pending_redundant = None;
                                epoch_id = epoch;
                                stream_id = Some(1);
                                // 新流开始：上一轮的洞与重传窗口都作废（§8.1 的 1 s 窗口跨流没有意义）。
                                missing.clear();
                                retransmit.clear();
                                let result = send_control(
                                    &mut control,
                                    &ControlRequest::OpenStream(open_stream_payload(&codec)),
                                ).await;
                                if let Err(error) = &result {
                                    stop_capture(&mut capture).await;
                                    stream_id = None;
                                    report_error(&inner, &session, error);
                                }
                                let _ = ready.send(result);
                            }
                            Err(error) => {
                                report_error(&inner, &session, &error);
                                let _ = ready.send(Err(error));
                            }
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
                        stop_capture(&mut capture).await;
                        pending_redundant = None;
                        missing.clear();
                        retransmit.clear();
                    }
                    SessionCommand::SubmitPin(pin) => {
                        let _ = send_control(
                            &mut control,
                            &ControlRequest::PairSubmit(crate::payload::PairSubmitPayload { pin }),
                        ).await;
                    }
                    SessionCommand::AnnounceGroupEpoch(payload) => {
                        // §7：epoch_local_us 由调用方取「本机单调时刻」，接收端各自换算到本机轴。
                        let request = ControlRequest::GroupEpoch(payload);
                        match send_control(&mut control, &request).await {
                            Ok(()) => tracing::info!(
                                epoch_id = payload.epoch_id,
                                lead_ms = payload.lead_ms,
                                "§7 已广播 GROUP_EPOCH"
                            ),
                            Err(error) => report_error(&inner, &session, &error),
                        }
                    }
                    SessionCommand::BroadcastReceiverEpoch(payload) => {
                        // M4：接收端 → 发送端。这里的 epoch_local_us 是**接收端**的本机时刻，
                        // 发送端收到后用自己的时钟偏移换算到本端轴，再把编号 0 点钉到它。
                        let request = ControlRequest::ReceiverEpoch(payload);
                        match send_control(&mut control, &request).await {
                            Ok(()) => tracing::info!(
                                epoch_id = payload.epoch_id,
                                lead_ms = payload.lead_ms,
                                "M4 已广播 RECEIVER_EPOCH（共同基准）"
                            ),
                            Err(error) => report_error(&inner, &session, &error),
                        }
                    }
                    SessionCommand::SetGain { gain, ramp_ms, reply } => {
                        // 参数在这里先验一次：非法值不该发到线上，更不该让对方以为生效了。
                        let result = if gain_x1000_from_f32(gain).is_err() {
                            Err(AudioLinkError::bad_request("invalid gain value"))
                        } else {
                            let payload = SetGainPayload {
                                stream_id: crate::payload::STREAM_ID_ALL,
                                gain,
                                ramp_ms: u16::try_from(ramp_ms).unwrap_or(u16::MAX),
                            };
                            send_control(&mut control, &ControlRequest::SetGain(payload)).await
                        };
                        let _ = reply.send(result);
                    }
                    SessionCommand::SendGroupFrame(frame) => {
                        let request = match frame {
                            GroupFrame::Create(payload) => ControlRequest::GroupCreate(payload),
                            GroupFrame::Join(payload) => ControlRequest::GroupJoin(payload),
                            GroupFrame::Leave(payload) => ControlRequest::GroupLeave(payload),
                        };
                        if let Err(error) = send_control(&mut control, &request).await {
                            report_error(&inner, &session, &error);
                        }
                    }
                    SessionCommand::SchedulePlayout { schedule, reply } => {
                        // 播放句柄还没建好时**暂存**而不是报错：显式 API 与组基准同一条原则 ——
                        // 「先建组、再推流」是正常用法，回 `cap_unsupported` 等于把正常用法说成不支持
                        // （第 58 轮定位的真缺陷）。
                        let result = match schedule {
                            Some(schedule) => {
                                let frame_ms_u32 = u32::try_from(frame_ms).unwrap_or(20);
                                if apply_schedule(&session, &playback, schedule, frame_ms_u32) {
                                    Ok(())
                                } else {
                                    stash_schedule(&session, schedule);
                                    Ok(())
                                }
                            }
                            None => {
                                clear_stashed_schedule(&session);
                                if let Some(handle) = playback.as_ref() {
                                    let offset_us = clock_estimate_of(&session)
                                        .map(|estimate| estimate.offset_us)
                                        .unwrap_or(0);
                                    let frame_samples =
                                        u32::try_from(frame_ms).unwrap_or(20).max(1) * 48;
                                    let _ = handle.set_schedule(None, offset_us, frame_samples);
                                }
                                Ok(())
                            }
                        };
                        let _ = reply.send(result);
                    }
                    SessionCommand::Shutdown => break,
                }
            }

            message = control.recv() => {
                let message = match message {
                    Ok(message) => message,
                    Err(error) => {
                        report_peer_gone(
                            &inner,
                            &session,
                            &net_error(&error),
                            matches!(&role, Role::Initiator) && stream_id.is_some(),
                        );
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
                    &jitter_depth,
                ).await;
            }

            datagram = connection.read_datagram_into(&mut rx_buf) => {
                match datagram {
                    Ok(len) => {
                        // FR-27：任何一个包到达都算「听到对端」—— 静默看门狗全靠这个时刻。
                        session
                            .last_rx_us
                            .store(now_monotonic_us(), Ordering::Relaxed);
                        let Some(slice) = rx_buf.get(..len) else { continue; };
                        let Ok(datagram) = AudioDatagram::decode(slice) else {
                            continue; // §1.1：非法数据报忽略并计数，不断流
                        };

                        // §6 的两种时钟数据报与音频共用**同一个读循环**（一条连接只有一个读者；
                        // 另起任务会把音频数据报抢走，表现为「偶发丢帧」这种最难查的症状）。
                        match datagram.header.ptype {
                            Ptype::Audio => {
                                // M4 观测：记下「最近一帧的编号 ↔ 到达时刻」。放在分派最前面，
                                // 因为这里才是「包真的到了」的时刻 —— 后面的重排窗口会等、也会丢。
                                session.note_rx_axis(datagram.header.sample_index);
                            }
                            Ptype::ClockProbe => {
                                match respond_clock_probe(&connection, &session, &datagram).await {
                                    Ok(_) => {}
                                    Err(error) => report_error(&inner, &session, &error),
                                }
                                continue;
                            }
                            Ptype::ClockReply => {
                                // t4 必须在**收到包的那一刻**取：等互斥量的时间不算网络延迟。
                                let t4_us = now_monotonic_us();
                                let outcome = match session.clock.lock() {
                                    Ok(mut clock) => clock.on_reply(&datagram, t4_us),
                                    Err(_) => continue,
                                };
                                match outcome {
                                    // 每记一个样本就把估计写进遥测：快速阶段 10 次/秒，
                                    // 只在 1 Hz 的 roll 里写会让面板看不到收敛过程。
                                    Ok(Some(_sample)) => publish_clock_to_telemetry(&session),
                                    Ok(None) => {} // 配不上的应答（已计入 unmatched）
                                    Err(_) => {}   // §1.1：非法载荷忽略并计数，不断流
                                }
                                continue;
                            }
                            Ptype::Nack => {
                                // §8.1 辅助重传：只用窗口内还留着的副本回应，缺的静默忽略（§1.1）。
                                let Ok(list) = NackList::decode(datagram.payload) else {
                                    continue; // 载荷非法：忽略并计数，不断流（§1.1）
                                };
                                let Some(id) = stream_id else { continue };
                                let now = Instant::now();
                                retransmit.expire(now);
                                for seq in list.seqs() {
                                    // 拷贝一次再 await：重传是低频路径，换来的是借用关系的清爽。
                                    let Some((sample_index, payload)) = retransmit
                                        .find(*seq)
                                        .map(|(index, bytes)| (index, bytes.to_vec()))
                                    else {
                                        continue; // 窗口外 / 从未发过这个序号
                                    };

                                    if let Err(error) = send_audio_frame(
                                        &connection,
                                        id,
                                        epoch_id,
                                        *seq,
                                        sample_index,
                                        &payload,
                                        Flags::NONE,
                                    )
                                    .await
                                    {
                                        report_error(&inner, &session, &error);
                                        break;
                                    }
                                }
                                continue;
                            }
                            // FEC / KEEPALIVE 的处理仍属后续 M2。
                            _ => continue,
                        }

                        let now = Instant::now();
                        // 到达（主包与冗余副本都算）即视为该序号已补上；新出现的洞据此建立（§8.1）。
                        missing.observe(datagram.header.seq, now);
                        // 有洞才开「等重传」窗口，而且必须先问两个问题：
                        //   1. 播放队列还有余量吗？等待期间队列只出不进，空着等就是硬静音；
                        //   2. 这点预算够重传回来吗？取「两个 RTT + 5 ms」，上限 30 ms（§8.1 的 RTT 门槛）。
                        // 两个问题任一为否 → 窗口为 0，按原策略交付，由 PCM 掩盖兜底。
                        let grace = if missing.pending() > 0
                            && playback
                                .as_ref()
                                .is_some_and(|handle| handle.depth_frames() >= 2)
                        {
                            let budget_us = connection.rtt_us().saturating_mul(2).saturating_add(5_000);
                            let cap_us = u64::try_from(NACK_RETRANSMIT_GRACE.as_micros()).unwrap_or(u64::MAX);
                            Duration::from_micros(budget_us.min(cap_us))
                        } else {
                            Duration::ZERO
                        };
                        packet_reorder.set_retransmit_grace(grace);
                        if let Ok(mut telemetry) = session.telemetry.lock() {
                            telemetry.record_received(datagram.payload.len());
                        }
                        packet_reorder.set_target_frames(
                            jitter_depth.load(Ordering::Relaxed),
                        );
                        if let Some(handle) = playback.as_ref() {
                            handle.note_stream_base(
                                datagram.header.seq,
                                datagram.header.sample_index,
                            );
                        }
                        let batch = packet_reorder.push(
                            datagram.header.seq,
                            datagram.payload.to_vec(),
                            now,
                        );

                        // 到达间隔抖动只按每个序号的首个有效副本计算。冗余副本通常与下一主包
                        // 背靠背到达，把它纳入会人为制造约 20 ms 的“抖动”。
                        if batch.accepted_new_seq
                            && let Some(previous) = last_datagram_at.replace(now)
                        {
                            let interval_us =
                                u32::try_from(now.saturating_duration_since(previous).as_micros())
                                    .unwrap_or(u32::MAX);
                            let nominal_us = u32::try_from(frame_ms).unwrap_or(20) * 1_000;
                            let jitter_us = interval_us.abs_diff(nominal_us);
                            arrival_jitter.push(jitter_us);
                            if let Ok(mut telemetry) = session.telemetry.lock() {
                                telemetry.record_jitter(jitter_us);
                            }
                        }
                        deliver_reordered_audio(
                            &session,
                            &mut audio_receiver,
                            &playback,
                            batch,
                        );

                        // §8.1：把洞变成 `NACK`。门控是**硬条件** —— RTT ≥ 30 ms 时重传只会
                        // 把延迟尖峰拉得更长，那时「双发 + PCM 掩盖」才是正确的兜底。
                        if connection.rtt_us() < NACK_MAX_RTT_US {
                            missing.expire(now);
                            let requests = missing.due_batch(now);

                            if !requests.is_empty()
                                && let Ok(list) = NackList::from_slice(&requests)
                            {
                                match list.to_datagram_bytes() {
                                    Ok(bytes) => {
                                        if let Err(error) = connection.send_datagram(&bytes).await {
                                            report_error(&inner, &session, &net_error(&error));
                                        } else if let Ok(mut telemetry) = session.telemetry.lock() {
                                            telemetry.record_nack();
                                        }
                                    }
                                    Err(error) => report_error(&inner, &session, &error),
                                }
                            }
                        }
                    }
                    Err(error) => {
                        report_peer_gone(
                            &inner,
                            &session,
                            &net_error(&error),
                            matches!(&role, Role::Initiator) && stream_id.is_some(),
                        );
                        break;
                    }
                }
            }

            _ = wait_for_optional_deadline(reorder_deadline) => {
                let batch = packet_reorder.flush_expired(Instant::now());
                deliver_reordered_audio(
                    &session,
                    &mut audio_receiver,
                    &playback,
                    batch,
                );
            }

            frame = next_encoded(&mut capture) => {
                let Some(frame) = frame else { continue; };
                let Some(id) = stream_id else { continue; };

                if let Some(tap) = inner.config.measurement.as_ref() {
                    tap.record_sealed(frame.seq, frame.sealed_at);
                }

                let sealed_at = Instant::now();
                let primary = transmit_audio(
                    &connection,
                    id,
                    epoch_id,
                    &frame,
                    &session,
                    AudioCopy::Primary,
                ).await;
                match primary {
                    Ok(()) => {
                        // §8.1：只有真发出去的帧才进重传窗口（对端 NACK 时按序号取回）。
                        retransmit.record(frame.seq, frame.sample_index, &frame.payload, sealed_at);
                    }
                    Err(error) => report_error(&inner, &session, &error),
                }
                if let Some(redundant) = pending_redundant.replace(frame)
                    && let Err(error) = transmit_audio(
                        &connection,
                        id,
                        epoch_id,
                        &redundant,
                        &session,
                        AudioCopy::Redundant,
                    ).await
                {
                    report_error(&inner, &session, &error);
                }
            }

            _ = tokio::time::sleep_until(probe_deadline) => {
                // §6 的探测：t1 取在**紧邻发送前**，否则本地排队时间会被算进网络单程里。
                let now_us = now_monotonic_us();
                let (probe, interval) = match session.clock.lock() {
                    Ok(mut clock) => {
                        // 先问间隔再组包：build_probe 会推进计数，顺序反了这一拍就提前进稳态。
                        let interval = clock.probe_interval();
                        (clock.build_probe(now_us), interval)
                    }
                    Err(_) => (
                        Err(AudioLinkError::bad_request("clock state poisoned")),
                        Duration::from_millis(STEADY_INTERVAL_MS),
                    ),
                };

                match probe {
                    Ok(bytes) => {
                        if let Err(error) = connection.send_datagram(&bytes).await {
                            report_error(&inner, &session, &net_error(&error));
                        }
                    }
                    // 编码失败只可能是实现缺陷（定长 12 B），仍按「显式报错 + 计数」处置。
                    Err(error) => report_error(&inner, &session, &error),
                }
                probe_deadline = tokio::time::Instant::now() + interval;
            }

            _ = ticker.tick() => {
                // FR-27 静默看门狗：拔网时 QUIC 不报错（它只是收不到回包，直到 idle_timeout 30 s
                // 才判连接死），所以「链路断了」必须由自己数「多久没听到对端」来发现 —— 否则重连
                // 只能等 idle_timeout，远迟于 M2 的 3 s 预算。
                // 只有**发起方且在推流**的会话才拨号，理由见 report_peer_gone 的文档。
                if !reconnect_started
                    && stream_id.is_some()
                    && matches!(&role, Role::Initiator)
                    && session.silent_for_ms() >= LINK_STALL_THRESHOLD_MS
                {
                    reconnect_started = true;
                    report_peer_gone(
                        &inner,
                        &session,
                        &AudioLinkError::bad_request("link went silent while streaming"),
                        true,
                    );
                }
                let estimate = clock_estimate_of(&session);
                let adaptive_jitter_p95 = arrival_jitter.summary().map(|summary| summary.p95);
                arrival_jitter.clear();
                let snapshot = match session.telemetry.lock() {
                    Ok(mut telemetry) => {
                        // rtt_us 只有一种口径：**QUIC 平滑 RTT**（`docs/03-protocol.md` §10 的定义），
                        // 收敛与否都一样。§6 的代表 RTT / quality / 分位数走 clock_estimate() 与
                        // clock_probe_stats() 两个正式出口，不往这个冻结字段里塞第二种统计口径（task-9）。
                        telemetry.set_rtt_us(u32::try_from(connection.rtt_us()).unwrap_or(u32::MAX));
                        publish_clock(&mut telemetry, estimate);
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

                let new_underruns = snapshot.underruns.saturating_sub(last_adaptive_underruns);
                last_adaptive_underruns = snapshot.underruns;
                let observed_target = jitter_depth.load(Ordering::Relaxed);
                adaptive_jitter.raise_to(observed_target);
                let desired_target = adaptive_jitter.observe(
                    adaptive_jitter_p95,
                    new_underruns,
                    frame_us,
                );
                let target_frames = publish_target(
                    &jitter_depth,
                    observed_target,
                    desired_target,
                );
                adaptive_jitter.raise_to(target_frames);
                packet_reorder.set_target_frames(target_frames);
                let batch = packet_reorder.flush_expired(Instant::now());
                deliver_reordered_audio(
                    &session,
                    &mut audio_receiver,
                    &playback,
                    batch,
                );

                // §8 自适应码率：判据必须用**对端**的遥测 —— 丢包与欠载只有接收侧看得见，
                // 它 1 Hz 用 `STREAM_STATS` 把数字送过来，这里是发送侧唯一能看到真实链路的地方。
                let peer_view = session.peer_stats.lock().ok().and_then(|slot| *slot);
                if let Some(peer_stats) = peer_view
                    && let Some(change) = adaptive_bitrate.observe(
                        session_started.elapsed(),
                        peer_stats.loss_pct_x100,
                        peer_stats.rtt_us,
                    )
                {
                    if let Some(handle) = capture.as_ref() {
                        handle
                            .target_bitrate_bps
                            .store(change.to_bps, Ordering::Relaxed);
                    }
                    let _ = inner.events.send(EngineEvent::CodecAdapted {
                        from_bps: change.from_bps,
                        to_bps: change.to_bps,
                        reason: change.reason.describe(),
                    });
                }

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

    stop_capture(&mut capture).await;
    stop_playout(&mut playback).await;
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
        // 重连可能已换成同一身份的新会话；旧任务退出不能摘掉新 PIN 或广播假断开。
        if !peers
            .get(&session.id)
            .is_some_and(|current| Arc::ptr_eq(current, session))
        {
            return;
        }
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
        clock: Mutex::new(ClockProbeState::new()),
        peer_stats: Mutex::new(None),
        commands,
        trusted: AtomicBool::new(trusted),
        state: Mutex::new(SessionState::Handshaking),
        pairing: Mutex::new(PairingState::default()),
        rx_axis: Arc::new(AtomicU64::new(u64::MAX)),
        last_rx_us: AtomicU64::new(0),
        reconnect_in_flight: AtomicBool::new(false),
        capabilities: Mutex::new(None),
        pending_schedule: Mutex::new(None),
    });

    let previous = inner
        .peers
        .lock()
        .ok()
        .and_then(|mut peers| peers.insert(peer_id, Arc::clone(&session)));
    if let Some(previous) = previous {
        let _ = inner.spawn(async move {
            let _ = previous.commands.send(SessionCommand::Shutdown).await;
        });
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
    // M3 多会话：采集是**引擎级共享**的，所以同一引擎内所有会话必须用同一帧长
    // （枢纽按引擎默认帧长切片，各会话编码器拿到的输入长度必须与它一致）。
    if codec.frame_ms != inner.config.codec.frame_ms {
        return Err(AudioLinkError::bad_request(
            "all sessions on one engine must share the same frame length",
        ));
    }
    let hub = acquire_capture_hub(inner)?;
    let handle = spawn_session_encoder(
        hub,
        *codec,
        Arc::clone(&session.telemetry),
        new_stop_flag(),
        Arc::new(AtomicI32::new(codec.bitrate_bps)),
    )?;

    Ok((handle, random_u64()))
}

/// 取得（必要时创建）引擎级采集枢纽，并把本会话登记成一个使用中的订阅者。
fn acquire_capture_hub(inner: &Arc<Inner>) -> Result<Arc<CaptureHub>, AudioLinkError> {
    let mut slot = inner
        .capture_hub
        .lock()
        .map_err(|_| AudioLinkError::bad_request("capture hub poisoned"))?;
    if let Some(hub) = slot.as_ref() {
        if !hub.is_stopped() {
            hub.subscribers.fetch_add(1, Ordering::AcqRel);
            return Ok(Arc::clone(hub));
        }
        // 上一轮的枢纽已经随最后一个会话停掉了：摘掉它，下面重建一个。
        *slot = None;
    }
    let factory = inner.config.capture.clone().ok_or_else(|| {
        AudioLinkError::cap_unsupported("this node has no capture source configured")
    })?;
    let hub = Arc::new(start_capture_hub(
        factory,
        inner.config.measurement.clone(),
        inner.config.codec.frame_ms,
    )?);
    hub.subscribers.fetch_add(1, Ordering::AcqRel);
    *slot = Some(Arc::clone(&hub));
    Ok(hub)
}

/// 每会话一个编码线程：订阅共享采集帧 → 用**本会话**的 codec 编码 → 交给会话循环发送。
///
/// 编码留在会话侧有两个好处：各会话可以有自己的码率（§8 的自适应按对端丢包各自决策），
/// 而采样时间轴仍然是全组共享的。
fn spawn_session_encoder(
    hub: Arc<CaptureHub>,
    codec: CodecConfig,
    telemetry: Arc<Mutex<TelemetryAggregator>>,
    stop: Arc<AtomicBool>,
    target_bitrate_bps: Arc<AtomicI32>,
) -> Result<CaptureHandle, AudioLinkError> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), AudioLinkError>>();
    let (frame_tx, frame_rx) = mpsc::channel::<EncodedFrame>(ENCODE_QUEUE_FRAMES);
    let thread_stop = Arc::clone(&stop);
    let hub_frames = hub.frames.subscribe();
    let owned_hub = Arc::clone(&hub);
    let thread_bitrate = Arc::clone(&target_bitrate_bps);

    let join = std::thread::Builder::new()
        .name("audiolink-encode".to_string())
        .spawn(move || {
            session_encoder_main(
                hub_frames,
                codec,
                telemetry,
                thread_stop,
                thread_bitrate,
                frame_tx,
                ready_tx,
            );
        })
        .map_err(|_| AudioLinkError::bad_request("failed to spawn session encoder thread"))?;

    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(CaptureHandle {
            frames: frame_rx,
            stop,
            join: Some(join),
            target_bitrate_bps,
            hub: Some(owned_hub),
        }),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(AudioLinkError::bad_request(
            "session encoder did not report readiness in time",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn session_encoder_main(
    mut hub_frames: broadcast::Receiver<HubFrame>,
    codec: CodecConfig,
    telemetry: Arc<Mutex<TelemetryAggregator>>,
    stop: Arc<AtomicBool>,
    target_bitrate_bps: Arc<AtomicI32>,
    frame_tx: mpsc::Sender<EncodedFrame>,
    ready_tx: std::sync::mpsc::Sender<Result<(), AudioLinkError>>,
) {
    let mut encoder = match OpusEncoder::new(codec) {
        Ok(encoder) => encoder,
        Err(error) => {
            let _ = ready_tx.send(Err(audio_error(&error)));
            return;
        }
    };
    let _ = ready_tx.send(Ok(()));

    let mut opus_buf = vec![0u8; audiolink_types::DATAGRAM_MAX_PAYLOAD];
    let mut applied_bitrate_bps = codec.bitrate_bps;

    while !stop.load(Ordering::Relaxed) {
        let frame = match hub_frames.try_recv() {
            Ok(frame) => frame,
            Err(broadcast::error::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            // 慢会话被采集甩开：丢旧帧（与「不阻塞采集」同一条纪律），继续处理最新的。
            Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(broadcast::error::TryRecvError::Closed) => break,
        };

        let requested_bps = target_bitrate_bps.load(Ordering::Relaxed);
        if requested_bps != applied_bitrate_bps {
            match encoder.set_bitrate(requested_bps) {
                Ok(()) => applied_bitrate_bps = requested_bps,
                // 非法取值只可能来自实现缺陷：拒绝这次变更并保持原码率，绝不中断编码。
                Err(_) => applied_bitrate_bps = encoder.config().bitrate_bps,
            }
        }

        let Ok(written) = encoder.encode_into(&frame.pcm, &mut opus_buf) else {
            break;
        };
        if written == 0 {
            continue; // DTX 静默帧：不占序号
        }
        let packet = EncodedFrame {
            seq: frame.seq,
            sample_index: frame.sample_index,
            payload: opus_buf[..written].to_vec(),
            sealed_at: frame.sealed_at,
        };
        // 队列满 = 编码比网络快：丢最旧（与「不阻塞采集」同一条纪律）。
        if frame_tx.try_send(packet).is_err()
            && let Ok(mut telemetry) = telemetry.lock()
        {
            telemetry.record_late_drop();
        }
    }
}

/// 采集枢纽（M3 多会话）：**一路采集** → N 个会话共享同一串 PCM 帧。
///
/// 为什么必须共享（不是优化，而是正确性）：每个会话各自开一路采集，读取起点不同、
/// 丢样各不同，于是各会话的 seq / sample_index 时间轴彼此错开几十毫秒。接收端按**同一个
/// epoch** 播放时，这点错位会原封不动加到组内偏差上 —— 而 M3 的验收是「组内 ±10 ms」。
/// 共享采集把「同一帧编号」变成全组公共的起点。
struct CaptureHub {
    /// 广播给所有会话的编码线程；容量按「几帧」算，慢会话丢旧帧而不是拖住采集。
    frames: broadcast::Sender<HubFrame>,
    stop: Arc<AtomicBool>,
    join: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// 还在用这路采集的会话数：降到 0 就停采集线程、并把枢纽从引擎上摘掉。
    subscribers: AtomicUsize,
    /// M4 共同基准：接收端广播来的「编号 0 点」（**本端**单调 µs）；i64::MIN = 未收到。
    /// 采集线程每帧前读一次原子量 —— 实时路径不加锁、不分配。
    epoch_us: Arc<AtomicI64>,
}

impl CaptureHub {
    /// 一个会话放手；最后一个会话放手才真正停采集。
    ///
    /// 采集线程自己会在这之后退出，这里只放下它的句柄（detach）—— 不在会话停止的路径上
    /// 阻塞等采集线程收尾：它可能正卡在一次 200 ms 的读取里，而会话该走就走。
    fn release(&self) {
        if self.subscribers.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut guard) = self.join.lock() {
            let _ = guard.take();
        }
    }

    /// M4：当前共同基准（本端单调 µs）；`i64::MIN` = 还没收到接收端广播。
    fn epoch_us_value(&self) -> i64 {
        self.epoch_us.load(Ordering::Relaxed)
    }

    /// M4：接收端广播的共同基准到达 —— 采集线程下一帧前就会读到并据此对齐编号。
    fn apply_epoch(&self, epoch_us: u64) {
        let value = i64::try_from(epoch_us).unwrap_or(i64::MAX);
        self.epoch_us.store(value, Ordering::Relaxed);
    }

    /// 采集线程是否已经停了（最后一个会话放手之后为 true，新的会话要重建枢纽）。
    fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

/// 共享采集帧：seq / sample_index 由枢纽**统一分配**，这就是「同一根时间轴」。
#[derive(Clone)]
struct HubFrame {
    seq: u32,
    sample_index: u32,
    pcm: Arc<Vec<f32>>,
    sealed_at: Instant,
}

/// 帧编号分配器（枢纽里唯一被所有会话共享的状态；单独成类型是为了能单测）。
#[derive(Debug, Clone, Copy, Default)]
struct HubClock {
    seq: u32,
    sample_index: u32,
    /// M4 共同基准：本端单调时钟上的「编号 0 点」（µs）。None = 未对齐，
    /// 沿用「本机启流瞬间」作原点。见 HubClock::align_epoch。
    start_at_us: Option<u64>,
}

impl HubClock {
    /// 接收端广播的共同基准落到本端时钟之后，把编号的 0 点钉到它。
    ///
    /// 为什么需要：两台发送端各自启流，sample_index 的原点就是各自启流的瞬间，
    /// 于是**同一个采样**在两条流上的编号差着几十毫秒；接收端按编号混音时，两路取到的
    /// 就不是同一时刻的声音。对齐之后两路的 sample_index = 0 落在同一个绝对时刻。
    fn align_epoch(&mut self, start_at_us: u64) {
        self.start_at_us = Some(start_at_us);
        self.sample_index = 0;
    }

    /// 取下一帧的编号并推进；序号回绕按 u32 语义（接收侧本来就按回绕判丢包）。
    ///
    /// 返回 None = 共同起点还没到：**这一帧不该发**。发出去它的编号在接收端是
    /// 「epoch 之前的时间」，会和另一路同样错位的帧混在一起 —— 那正是要修掉的东西。
    fn next(&mut self, frame_samples: u32, now_us: u64) -> Option<(u32, u32)> {
        if let Some(start) = self.start_at_us {
            if now_us < start {
                return None;
            }
            // 到点：编号从 0 起算（seq 继续递增 —— 它只管丢包检测，不承担时间轴语义）。
            self.start_at_us = None;
        }
        let current = (self.seq, self.sample_index);
        self.seq = self.seq.wrapping_add(1);
        self.sample_index = self.sample_index.wrapping_add(frame_samples);
        Some(current)
    }
}

/// 共享采集帧在广播里的排队上限（帧数）。
const HUB_QUEUE_FRAMES: usize = 8;

/// 采集线程句柄。
struct CaptureHandle {
    frames: mpsc::Receiver<EncodedFrame>,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
    /// §8 自适应码率的目标值：会话循环（1 Hz）写，采集线程在**下一次编码前**读到就生效。
    ///
    /// 用 `AtomicI32` 而不是 channel：采集线程是实时线程，读一个原子量不会阻塞也不会分配；
    /// 丢一拍读到的旧值最多让新码率晚 20 ms 生效，没有别的代价。
    target_bitrate_bps: Arc<AtomicI32>,
    /// M3 多会话：本会话挂在哪个共享采集枢纽上（停止时用来放手）。
    hub: Option<Arc<CaptureHub>>,
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

/// 接收侧排播状态（§7 预约播放）：组基准 + 换算用的时钟偏移 + 首帧样本基准。
#[derive(Debug, Clone, Copy, Default)]
struct PlayoutSync {
    /// 发送端给的组基准；`None` = 不启用预约播放（走 M1 / M2 的本地游标）。
    schedule: Option<EpochSchedule>,
    /// 设置排播时的时钟偏移快照（对端 − 本机，µs）。
    offset_us: i64,
    /// 每帧样本数（48 kHz × frame_ms）。
    frame_samples: u32,
    /// 本流首个数据报的 (seq, sample_index)：序号 → 样本序号的换算基准。
    base: Option<(u32, u32)>,
}

impl PlayoutSync {
    /// 该序号对应的负载首样本序号（发送端按 48 kHz 单调递增，可由首帧推算）。
    fn sample_index_of(&self, seq: u32) -> Option<u32> {
        let (base_seq, base_sample) = self.base?;
        let frames = u64::from(seq.wrapping_sub(base_seq));
        let advance = frames.saturating_mul(u64::from(self.frame_samples));
        Some(base_sample.wrapping_add(advance as u32))
    }
}

/// 排播状态的共享句柄（会话任务写、播放线程读）。
type PlayoutSyncHandle = Arc<Mutex<PlayoutSync>>;

/// 播放管线句柄：发送端（会话任务）+ 停止标志 + 线程句柄 + §7 排播状态。
struct PlayoutHandle {
    frames: Sender<PlaybackFrame>,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
    sync: PlayoutSyncHandle,
    /// §4.1 音量：**每会话一份**（多会话各调各的），播放线程每帧取一次并走一个步长。
    gain: Arc<Mutex<GainState>>,
    /// M4 汇聚：本会话挂在哪台混音器上、用的是哪个源号（停止时要把这一路摘掉）。
    mixer: Option<PlayoutMix>,
    mix_source: Option<u32>,
    /// M4/FR-27：本会话的混音器槽位凭据 —— `Drop` 里据此让位（见下方 `Drop` 实现）。
    ///
    /// **每一路都持有它**（不再只有 owner 才拿）：接管是**懒**的，guest 随时可能成为 owner，
    /// 而每次成为 owner 都要有凭据让位。
    mix_slot: Arc<PlayoutMixSlot>,
}

impl PlayoutHandle {
    /// §7：记下本流首个数据报的序号与样本序号（只有第一次生效）。
    ///
    /// 之后所有包的样本序号都由它推算 —— 发送端按 48 kHz 单调递增，序号与样本序号只差一个常数。
    fn note_stream_base(&self, seq: u32, sample_index: u32) {
        if let Ok(mut sync) = self.sync.lock()
            && sync.base.is_none()
        {
            sync.base = Some((seq, sample_index));
        }
    }

    /// §4.1：把 SET_GAIN 落到本会话的播放上。
    ///
    /// 非法值（NaN / 负数 / 超上限）在这里就被拒绝 —— 上层据此记一条警告，
    /// 而不是让用户以为「音量调了但没反应」。
    fn set_gain(
        &self,
        gain: f32,
        ramp_ms: u32,
        frame_ms: u32,
    ) -> Result<u32, crate::gain::GainError> {
        let target = gain_x1000_from_f32(gain)?;
        let mut state = self.gain.lock().unwrap_or_else(|e| e.into_inner());
        state.set_target(target, ramp_ms, frame_ms)?;
        Ok(state.target_x1000())
    }

    /// §7：设置（或清除）预约播放基准，并返回是否真的启用了排播。
    fn set_schedule(
        &self,
        schedule: Option<EpochSchedule>,
        offset_us: i64,
        frame_samples: u32,
    ) -> bool {
        let Ok(mut sync) = self.sync.lock() else {
            return false;
        };
        sync.schedule = schedule;
        sync.offset_us = offset_us;
        sync.frame_samples = frame_samples.max(1);
        schedule.is_some()
    }

    /// 待播队列深度（帧数）。
    ///
    /// # 口径说明（别把它读成「播放器里积压了多少」）
    ///
    /// 这是**已解码、还没被播放线程取走**的帧数 —— 也就是引擎抖动缓冲本身。
    /// 起步按自适应目标攒 1--3 帧，此后每拍取走一帧；水位受调度抖动与收发速率差影响，
    /// 上限为 `PLAYBACK_QUEUE_FRAMES`。它不包含 sink、Android PCM 环或 AudioTrack 的水位。
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

/// 句柄被 drop 即收工：停线程、还混音源号、owner 让位。
///
/// # 为什么把这三件事放在 `Drop` 里（FR-27 审计 §4 的清单）
///
/// 播放句柄的退出路径不止「正常停止」一条：会话收尾（`stop_playout`）、`CloseStream`、
/// 建立播放管线失败、句柄被替换、引擎关闭……只在 `stop_playout` 里做清理，别的路径就会
/// 留下「线程还活着 + 源号没还 + owner 标志还立着」—— 后者的后果很具体：下一个会话抢不到
/// owner，于是又落回「有混音器但没人写设备」的哑状态。
impl Drop for PlayoutHandle {
    fn drop(&mut self) {
        // ① 让播放线程在下一拍退出（幂等：正常停止时 `stop_playout` 已经置过）。
        self.stop.store(true, Ordering::Relaxed);
        // ② M4：把这一路从混音器里摘掉 —— 停了就不该再占路数（否则重连几次就撞上 8 路上限）。
        if let (Some(mixer), Some(source)) = (self.mixer.as_ref(), self.mix_source)
            && let Ok(mut guard) = mixer.lock()
        {
            guard.remove_source(source);
        }
        // ③ M4/FR-27：owner **立刻**让位，不等播放线程 join 完。
        //    让位必须发生在别的会话尝试接管之前，否则它们一直抢不到 owner。
        //    世代 CAS：只有「仍是当前 owner」的这一路清得掉，旧 owner 迟到的退出不会误清新 owner。
        if let Some(source) = self.mix_source {
            release_playout_owner(&self.mix_slot, source);
        }
    }
}

/// 播放线程侧的 owner 让位守卫（FR-27）。
///
/// owner 线程自己也会死：设备被拔、写失败、建 sink 失败、线程 panic。三种情况都不经过
/// `PlayoutHandle`，所以线程必须自带一份让位凭据 —— `Drop` 保证**任何**退出路径都让位。
struct PlayoutOwnerGuard {
    slot: Arc<PlayoutMixSlot>,
    /// 本路的源号（世代凭据）。
    source: u32,
}

impl Drop for PlayoutOwnerGuard {
    fn drop(&mut self) {
        release_playout_owner(&self.slot, self.source);
    }
}

async fn stop_capture(handle: &mut Option<CaptureHandle>) {
    if let Some(handle) = handle.take() {
        handle.stop.store(true, Ordering::Relaxed);
        join_audio_thread(handle.join).await;
        // 放手共享采集：最后一个会话放手时采集线程才会停（M3 多会话）。
        if let Some(hub) = handle.hub.as_ref() {
            hub.release();
        }
    }
}

/// 停止并等待播放线程退出；先断开帧通道唤醒接收，再在阻塞池 join。
/// 会话退出即代表其平台回调已停止，不再把旧回调带入下一次引擎启动。
///
/// 「置 stop / 摘路数 / owner 让位」三件事都集中在 `PlayoutHandle::drop`（见其文档）：
/// 这里只负责把线程句柄取出来，**先 drop 句柄再 join** —— 让位必须立刻生效，
/// 等 join 会让重连后的新会话抢不到 owner，落回「有混音器但没人写设备」。
async fn stop_playout(handle: &mut Option<PlayoutHandle>) {
    if let Some(mut handle) = handle.take() {
        let join = handle.join.take();
        drop(handle);
        join_audio_thread(join).await;
    }
}

async fn join_audio_thread(join: Option<std::thread::JoinHandle<()>>) {
    if let Some(join) = join {
        let _ = tokio::task::spawn_blocking(move || join.join()).await;
    }
}

/// 取下一帧编码结果；没有采集管线时永久挂起（`select!` 需要一个永不就绪的分支）。
async fn next_encoded(handle: &mut Option<CaptureHandle>) -> Option<EncodedFrame> {
    match handle {
        Some(handle) => handle.frames.recv().await,
        None => std::future::pending().await,
    }
}

/// 采集枢纽线程：**在本线程内**创建 `CaptureSource`（`!Send`，谁用谁建）。
///
/// 它只干一件事：把采集到的 PCM 切成固定帧长、编号、广播出去。编码留给各会话自己 ——
/// 于是「同一串帧、同一套编号」被全组共享，而每会话仍可各用自己的码率。
fn start_capture_hub(
    factory: CaptureFactory,
    tap: Option<Arc<MeasurementTap>>,
    frame_ms: u32,
) -> Result<CaptureHub, AudioLinkError> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), AudioLinkError>>();
    let (frames, _rx) = broadcast::channel::<HubFrame>(HUB_QUEUE_FRAMES);
    let frames_tx = frames.clone();
    let stop = new_stop_flag();
    let thread_stop = Arc::clone(&stop);
    // M4：共同基准由会话线程写、采集线程读（i64::MIN = 还没收到接收端广播）。
    let epoch_us = Arc::new(AtomicI64::new(i64::MIN));
    let thread_epoch = Arc::clone(&epoch_us);

    let join = std::thread::Builder::new()
        .name("audiolink-capture-hub".to_string())
        .spawn(move || {
            hub_main(
                factory.as_ref(),
                tap,
                frame_ms,
                thread_stop,
                frames_tx,
                ready_tx,
                thread_epoch,
            );
        })
        .map_err(|_| AudioLinkError::bad_request("failed to spawn capture hub thread"))?;

    // 等采集源建好并过了「零重采样」断言再返回 —— 否则调用方会以为推流已经开始，
    // 而失败只留在一条日志里（旧版「异常即永久静音」的复发路径）。
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(CaptureHub {
            frames,
            stop,
            join: Mutex::new(Some(join)),
            subscribers: AtomicUsize::new(0),
            epoch_us,
        }),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(AudioLinkError::bad_request(
            "capture source did not report readiness in time",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn hub_main(
    factory: &(dyn Fn() -> Result<Box<dyn CaptureSource>, AudioError> + Send + Sync),
    tap: Option<Arc<MeasurementTap>>,
    frame_ms: u32,
    stop: Arc<AtomicBool>,
    frames: broadcast::Sender<HubFrame>,
    ready_tx: std::sync::mpsc::Sender<Result<(), AudioLinkError>>,
    epoch_us: Arc<AtomicI64>,
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

    let mut chunker = match FrameChunker::new(frame_ms, HUB_QUEUE_FRAMES * 2) {
        Ok(chunker) => chunker,
        Err(error) => {
            let _ = ready_tx.send(Err(audio_error(&error)));
            return;
        }
    };

    let _ = ready_tx.send(Ok(()));

    // 编号由枢纽统一分配：这是「全组同一根时间轴」的唯一来源。
    let mut clock = HubClock::default();
    let mut applied_epoch = i64::MIN;
    let frame_samples = frame_ms.saturating_mul(48_000) / 1000;
    let mut samples: Vec<f32> =
        Vec::with_capacity(usize::try_from(frame_samples.saturating_mul(8)).unwrap_or(7_680));

    while !stop.load(Ordering::Relaxed) {
        match capture.read(&mut samples, Duration::from_millis(200)) {
            Ok(Some(_packet)) => {
                chunker.push(&samples);
            }
            Ok(None) => {
                // 空闲端点零数据不是错误（坑清单 #12）：什么都不做，绝不报错断流。
                // 采集侧不再直接写会话遥测 —— 枢纽是引擎级的，没有「属于哪个会话」这回事。
            }
            Err(_) => {
                // 采集源失效：结束枢纽线程。会话状态机会各自迁移到 Failed（架构 §4 铁律 2）。
                break;
            }
        }

        while let Some(frame) = chunker.next_frame() {
            // M4：接收端广播的共同基准一到，编号就对到同一条时间轴上（只在这儿改一次）。
            let want_epoch = epoch_us.load(Ordering::Relaxed);
            if want_epoch != applied_epoch {
                applied_epoch = want_epoch;
                if let Ok(start_at_us) = u64::try_from(want_epoch) {
                    clock.align_epoch(start_at_us);
                    tracing::info!(start_at_us, "M4 发送端已对齐接收端广播的共同基准");
                }
            }
            let sealed_at = Instant::now();
            let Some((seq, sample_index)) = clock.next(frame_samples, now_monotonic_us()) else {
                // 共同起点还没到：这一帧不发（它的编号在接收端没有可比性）。
                continue;
            };
            // 探针记在枢纽这一层：一帧只封口一次，多会话不会各记一遍。
            if let Some(tap) = tap.as_ref() {
                tap.record_sealed(seq, sealed_at);
            }
            let hub_frame = HubFrame {
                seq,
                sample_index,
                pcm: Arc::new(frame.to_vec()),
                sealed_at,
            };
            // 没有订阅者时发送会失败（旧的帧被丢掉）—— 那是「会话都停了、采集还没停」的正常瞬态。
            let _ = frames.send(hub_frame);
        }
    }

    capture.stop();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioCopy {
    Primary,
    Redundant,
}

/// 把一帧编码音频封成 `AUDIO` 数据报并发出（**不含遥测**）。
///
/// 抽出来是为了 NACK 重传能复用同一条编码路径：重传不是「新的一帧」，绝不能进 `expected` 分母
/// （否则丢包率会被自己的重传擦干净 —— 那是最糟的一类自欺）。
async fn send_audio_frame(
    connection: &Connection,
    stream_id: u32,
    epoch_id: u64,
    seq: u32,
    sample_index: u32,
    payload: &[u8],
    flags: Flags,
) -> Result<(), AudioLinkError> {
    let budget = connection.max_audio_payload();

    // 单帧超过预算说明码率档与 MTU 不匹配（架构 §12 技术债：应用层分片属 M2+）。
    // 明确拒绝并把这一帧丢掉，而不是发出去让对端解不出来 —— 后者会表现成「对端一直没声音」，
    // 而这里至少会留下一条带具体字节数的错误。
    if payload.len() > budget {
        return Err(AudioLinkError::owned(
            ErrorCode::CodecUnsupported,
            format!(
                "opus frame of {} B exceeds the {} B datagram budget; \
                 lower the bitrate or shorten the frame",
                payload.len(),
                budget
            ),
        ));
    }

    let header = AudioDatagramHeader {
        version: audiolink_types::PROTO_MAJOR,
        ptype: Ptype::Audio,
        flags,
        stream_id,
        seq,
        sample_index,
        epoch_id,
    };
    let bytes = AudioDatagram { header, payload }.encode_to_vec()?;
    connection
        .send_datagram(&bytes)
        .await
        .map_err(|e| net_error(&e))?;
    Ok(())
}

/// 发送一个主音频数据报或延迟一帧的冗余副本（含本机**发送侧**遥测）。
async fn transmit_audio(
    connection: &Connection,
    stream_id: u32,
    epoch_id: u64,
    frame: &EncodedFrame,
    session: &Arc<PeerSession>,
    copy: AudioCopy,
) -> Result<(), AudioLinkError> {
    let flags = match copy {
        AudioCopy::Primary => Flags::NONE,
        AudioCopy::Redundant => Flags::FEC_REDUNDANT,
    };
    send_audio_frame(
        connection,
        stream_id,
        epoch_id,
        frame.seq,
        frame.sample_index,
        &frame.payload,
        flags,
    )
    .await?;

    if let Ok(mut telemetry) = session.telemetry.lock() {
        if copy == AudioCopy::Primary {
            telemetry.record_expected();
        }
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
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ReceiveReport {
    expected: u32,
    lost: u32,
    plc: u32,
    late_drops: u32,
}

/// 单路接收解码状态：序号时间轴、Opus 状态与 PCM 掩盖历史必须一起推进。
struct AudioReceiver {
    decoder: OpusDecoder,
    concealer: PcmConcealer,
    pcm: Vec<f32>,
    expected_seq: Option<u32>,
    max_conceal_frames: u32,
}

impl AudioReceiver {
    fn new(codec: CodecConfig) -> Result<Self, AudioError> {
        let frame_ms = usize::try_from(codec.frame_ms).unwrap_or(20).max(1);
        Ok(Self {
            decoder: OpusDecoder::new(codec)?,
            concealer: PcmConcealer::new(codec.frame_ms)?,
            pcm: vec![0.0; codec.interleaved_frame()],
            expected_seq: None,
            max_conceal_frames: u32::try_from(CONCEAL_FADE_MS.div_ceil(frame_ms))
                .unwrap_or(u32::MAX),
        })
    }

    fn receive(
        &mut self,
        seq: u32,
        payload: &[u8],
        mut emit: impl FnMut(PlaybackFrame) -> bool,
    ) -> ReceiveReport {
        let mut report = ReceiveReport::default();
        let gap = match self.expected_seq {
            None => 0,
            Some(expected) => {
                let forward = seq.wrapping_sub(expected);
                if forward >= (1 << 31) {
                    report.late_drops = 1;
                    return report;
                }
                if forward >= 1_000 {
                    // 同一条 M1 流不应跳过 20 s 以上。把它当时间轴重建，避免一次异常包触发
                    // 上千次解码和分配；新包成为新的基准，平台播放游标仍会按序号补静音。
                    if let Ok(decoder) = OpusDecoder::new(self.decoder.config()) {
                        self.decoder = decoder;
                    }
                    self.concealer.reset();
                    0
                } else {
                    forward
                }
            }
        };
        self.expected_seq = Some(seq.wrapping_add(1));
        report.expected = gap.saturating_add(1);
        report.lost = gap;

        let conceal_frames = gap.min(self.max_conceal_frames);
        for missing in 0..conceal_frames {
            if self.decoder.concealment_is_real() {
                let _ = self.decoder.conceal_into(&mut self.pcm);
            }
            if self.concealer.conceal_into(&mut self.pcm).is_err() {
                self.pcm.fill(0.0);
            }
            let missing_seq = seq.wrapping_sub(gap - missing);
            if !emit(PlaybackFrame {
                seq: missing_seq,
                samples: self.pcm.clone(),
            }) {
                report.late_drops = report.late_drops.saturating_add(1);
            }
            report.plc = report.plc.saturating_add(1);
        }
        if gap > conceal_frames {
            // 自建掩盖已经淡出到静音；剩余空洞由播放游标补静音。重建 Opus 状态，
            // 但保留 concealer 的静音尾部，让真实包回来时仍能从 0 平滑淡入。
            if let Ok(decoder) = OpusDecoder::new(self.decoder.config()) {
                self.decoder = decoder;
            }
        }

        let decoded = match self.decoder.decode_into(payload, &mut self.pcm) {
            Ok(decoded) if decoded == self.pcm.len() => {
                let _ = self.concealer.process_good(&mut self.pcm);
                decoded
            }
            Ok(_) | Err(_) => {
                if self.decoder.concealment_is_real() {
                    let _ = self.decoder.conceal_into(&mut self.pcm);
                }
                if self.concealer.conceal_into(&mut self.pcm).is_err() {
                    self.pcm.fill(0.0);
                }
                report.plc = report.plc.saturating_add(1);
                self.pcm.len()
            }
        };
        if !emit(PlaybackFrame {
            seq,
            samples: self.pcm[..decoded].to_vec(),
        }) {
            report.late_drops = report.late_drops.saturating_add(1);
        }
        report
    }
}

fn receive_audio(
    session: &Arc<PeerSession>,
    receiver: &mut AudioReceiver,
    playback: &Option<PlayoutHandle>,
    seq: u32,
    payload: &[u8],
) {
    let report = receiver.receive(seq, payload, |frame| {
        playback
            .as_ref()
            .is_none_or(|handle| handle.try_push(frame))
    });

    if let Ok(mut telemetry) = session.telemetry.lock() {
        telemetry.record_expected_frames(report.expected);
        telemetry.record_lost(report.lost);
        telemetry.record_plc_frames(report.plc);
        telemetry.record_late_drops(report.late_drops);
    }
}

fn deliver_reordered_audio(
    session: &Arc<PeerSession>,
    receiver: &mut AudioReceiver,
    playback: &Option<PlayoutHandle>,
    batch: ReorderBatch,
) {
    if batch.late_drops > 0
        && let Ok(mut telemetry) = session.telemetry.lock()
    {
        telemetry.record_late_drops(batch.late_drops);
    }
    for EncodedAudioPacket { seq, payload, .. } in batch.ready {
        receive_audio(session, receiver, playback, seq, &payload);
    }
}

async fn wait_for_optional_deadline(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

// ---------------------------------------------------------------------------
// §6 时钟同步（数据报路径）
// ---------------------------------------------------------------------------

/// 读到一个数据报之后、**会话建立之前**该做的事：只认 §6 的 `CLOCK_PROBE`。
///
/// 握手前不处理音频 / FEC / NACK：还没有会话就没有流，收到的一切都不该被当成数据
/// （阶段零与阶段一共用本函数，保证两处的判据完全一致）。
enum ProbeStep {
    /// 回了一个 `CLOCK_REPLY`。
    Replied,
    /// 不是探测、或载荷非法：按 §1.1 忽略并计数，不断流。
    Ignored,
    /// 回包失败（对端已走 / 连接已关）。
    SendFailed(AudioLinkError),
    /// 读数据报失败：连接已经不可用。
    LinkLost(AudioLinkError),
}

/// 见 `ProbeStep`：读一个数据报 → 需要的话回一个 `CLOCK_REPLY`。
async fn answer_probe_datagram(
    connection: &Connection,
    session: &Arc<PeerSession>,
    read: Result<usize, NetError>,
    buf: &[u8],
) -> ProbeStep {
    let len = match read {
        Ok(len) => len,
        Err(error) => return ProbeStep::LinkLost(net_error(&error)),
    };
    let Some(slice) = buf.get(..len) else {
        return ProbeStep::Ignored;
    };
    let Ok(datagram) = AudioDatagram::decode(slice) else {
        return ProbeStep::Ignored; // §1.1：非法数据报忽略并计数，不断流
    };
    if datagram.header.ptype != Ptype::ClockProbe {
        return ProbeStep::Ignored;
    }

    match respond_clock_probe(connection, session, &datagram).await {
        Ok(true) => ProbeStep::Replied,
        Ok(false) => ProbeStep::Ignored,
        Err(error) => ProbeStep::SendFailed(error),
    }
}

/// 进入「等人工输入 PIN」的窗口：把握手死线顺延到 `pin_wait`（**从此刻起算**）。
///
/// 为什么是「顺延」而不是「把总死线放宽」：§5 的握手本身（HELLO / AUTH 往返）是**机器时间**，
/// 毫秒级就该走完；把总死线放宽会让「对端发一半就装死」这种半开连接一起被容忍。
/// 顺延只发生在**确实有真人在看屏幕输数字**的那一刻（收到 / 发出 `PAIR_REQUIRED` 之后）。
fn arm_pin_wait(deadline: &mut tokio::time::Instant, pin_wait: Duration) {
    *deadline = tokio::time::Instant::now() + pin_wait;
}

/// 握手死线到点的统一处置（阶段零与阶段一共用，两处判据必须一致）。
///
/// 返回 `false` = 调用方应立刻放弃这条连接（既没握手、也没探测：10 s 无事发生的连接不值得留着）；
/// 返回 `true` = 已放开死线，继续服务到对端收工（对端一停，QUIC 的空闲超时会让读数据报返回 Err）。
fn handshake_deadline_expired(
    inner: &Arc<Inner>,
    session: &Arc<PeerSession>,
    ready: &mut Option<tokio::sync::oneshot::Sender<Result<(), AudioLinkError>>>,
    probes_answered: u64,
) -> bool {
    if probes_answered == 0 {
        let error = AudioLinkError::bad_request("handshake timed out");
        finish_ready(ready, Err(error));
        drop_session(inner, session, "handshake timed out");
        return false;
    }

    // 已经答过 §6 的探测：这是一条测量连接（或一个先探测后握手的对端），不该被握手死线收回。
    tracing::info!(
        "handshake deadline released: {} clock probes served before the session was established",
        probes_answered
    );
    true
}

/// 应答一个 `CLOCK_PROBE`：**立即**回 `CLOCK_REPLY{t2 = t3 = 本机单调 µs}`（§6）。
///
/// 返回 `Ok(true)` = 已回包；`Ok(false)` = 载荷非法（§1.1：忽略并计数，不断流）；
/// `Err` = 回包失败（对端已走 / 连接已关）。
///
/// 只做「取时刻 → 编码 → 送出去」三步，中间不加任何处理：§6 的 `t3 = t2` 把应答侧处理延迟
/// 主动归零，剩下的误差项才是要量的网络单程。**不依赖 §5 会话状态**，握手前也照回（理由见调用点）。
async fn respond_clock_probe(
    connection: &Connection,
    session: &Arc<PeerSession>,
    datagram: &AudioDatagram<'_>,
) -> Result<bool, AudioLinkError> {
    let now_us = now_monotonic_us();
    let bytes = {
        let mut clock = session
            .clock
            .lock()
            .map_err(|_| AudioLinkError::bad_request("clock state poisoned"))?;
        match clock.on_probe(datagram, now_us) {
            Ok(bytes) => bytes,
            // 载荷非法：按 §1.1 忽略并计数（互斥量中毒与否都不该升级成断流）。
            Err(_) => return Ok(false),
        }
    };

    connection
        .send_datagram(&bytes)
        .await
        .map_err(|error| net_error(&error))?;
    Ok(true)
}

/// 当前时钟估计（互斥量中毒时当作「还没有估计」—— 不 panic、不假装收敛）。
fn clock_estimate_of(session: &Arc<PeerSession>) -> Option<ClockEstimate> {
    session.clock.lock().ok()?.estimate()
}

/// §7：把一份组基准写到播放句柄上（顺带取**当时**的时钟偏移快照）。
///
/// 返回 `false` = 还没有播放句柄（流没开）。调用方据此决定「暂存」还是「应用」——
/// **不能丢**：组基准常常先于开流到达（第 58 轮定位的真缺陷）。
fn apply_schedule(
    session: &Arc<PeerSession>,
    playback: &Option<PlayoutHandle>,
    schedule: EpochSchedule,
    frame_ms: u32,
) -> bool {
    let Some(handle) = playback.as_ref() else {
        return false;
    };
    let offset_us = clock_estimate_of(session)
        .map(|estimate| estimate.offset_us)
        .unwrap_or(0);
    let frame_samples = frame_ms.max(1) * 48;
    handle.set_schedule(Some(schedule), offset_us, frame_samples)
}

/// §7：暂存「还没法应用」的组基准，等 `OpenStream` 建好播放句柄再补上。
fn stash_schedule(session: &Arc<PeerSession>, schedule: EpochSchedule) {
    if let Ok(mut slot) = session.pending_schedule.lock() {
        *slot = Some(schedule);
    }
}

/// §7：取出并清空暂存的组基准。
fn take_stashed_schedule(session: &Arc<PeerSession>) -> Option<EpochSchedule> {
    session
        .pending_schedule
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
}

/// §7：清空暂存的组基准（显式清除排播时用）。
fn clear_stashed_schedule(session: &Arc<PeerSession>) {
    if let Ok(mut slot) = session.pending_schedule.lock() {
        *slot = None;
    }
}

/// 把当前估计写进遥测：样本 < 8 时保持 0（见 [`publish_clock`]）。
fn publish_clock_to_telemetry(session: &Arc<PeerSession>) {
    let estimate = clock_estimate_of(session);
    if let Ok(mut telemetry) = session.telemetry.lock() {
        publish_clock(&mut telemetry, estimate);
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
    jitter_depth: &Arc<AtomicUsize>,
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

            match spawn_playout_thread(inner, session, codec, Arc::clone(jitter_depth)) {
                Ok(handle) => *playback = Some(handle),
                Err(error) => {
                    report_error(inner, session, &error);
                    return;
                }
            }

            // §7：流开之前收到的组基准在这里补上 —— 否则那次 `GROUP_EPOCH` 就被静默丢掉了。
            if let Some(schedule) = take_stashed_schedule(session) {
                let applied = apply_schedule(session, playback, schedule, codec.frame_ms);
                tracing::info!(
                    epoch_id = schedule.epoch_id,
                    lead_ms = schedule.lead_ms,
                    applied,
                    "§7 开流后补应用先前暂存的组基准"
                );
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

        ControlRequest::GroupCreate(payload) => {
            // §7：建组帧同时携带组基准 —— 接收侧据此排播，并让上层知道「我在哪个组」。
            //
            // 播放句柄还没建好（流没开）时**暂存**而不是跳过：`create_group` 只发这一帧
            // （`GROUP_EPOCH` 只用于动态加入），而「先建组、再推流」正是正常用法 ——
            // 跳过就等于排播从未生效、两端各自退回本地游标（第 58 轮定位的真缺陷，实测 20% 起播差一帧）。
            let schedule =
                EpochSchedule::new(payload.epoch_id, payload.epoch_local_us, payload.lead_ms);
            if !apply_schedule(session, playback, schedule, codec.frame_ms) {
                stash_schedule(session, schedule);
                tracing::info!(
                    epoch_id = payload.epoch_id,
                    lead_ms = payload.lead_ms,
                    group_id = payload.group_id,
                    "§7 GROUP_CREATE：播放句柄尚未就绪，组基准已暂存，开流后补应用"
                );
            }
            let _ = inner.events.send(EngineEvent::GroupUpdated {
                group_id: payload.group_id,
                epoch_id: payload.epoch_id,
                members: u32::try_from(payload.members.len()).unwrap_or(u32::MAX),
            });
        }
        ControlRequest::GroupJoin(payload) => {
            tracing::info!(
                group_id = payload.group_id,
                member = %payload.member.short(),
                "§7 有成员加入同步组"
            );
        }
        ControlRequest::GroupLeave(payload) => {
            tracing::info!(
                group_id = payload.group_id,
                member = %payload.member.short(),
                "§7 有成员退出同步组"
            );
        }
        ControlRequest::GroupEpoch(payload) => {
            // §7：发送端指定组基准 → 接收侧据此排播。
            //
            // 播放句柄还没建好（流没开）时**暂存**而不是丢掉：`GROUP_EPOCH` 常常先于开流到达，
            // 丢掉就等于排播从未生效、接收端退回本地游标（第 58 轮定位的真缺陷）。
            let schedule =
                EpochSchedule::new(payload.epoch_id, payload.epoch_local_us, payload.lead_ms);
            if apply_schedule(session, playback, schedule, codec.frame_ms) {
                tracing::info!(
                    epoch_id = payload.epoch_id,
                    lead_ms = payload.lead_ms,
                    "§7 GROUP_EPOCH：接收侧排播已生效"
                );
            } else {
                stash_schedule(session, schedule);
                tracing::info!(
                    epoch_id = payload.epoch_id,
                    lead_ms = payload.lead_ms,
                    "§7 GROUP_EPOCH：播放句柄尚未就绪，排播已暂存，开流后补应用"
                );
            }
        }
        ControlRequest::ReceiverEpoch(payload) => {
            // M4：本机是**发送端**，接收端广播了共同基准 —— 把自己的编号 0 点钉到它。
            let offset_us = clock_estimate_of(session)
                .map(|estimate| estimate.offset_us)
                .unwrap_or(0);
            // offset_us = 对端（接收端）时钟 − 本端时钟 → 本端时刻 = 接收端时刻 − offset_us。
            let local_epoch_us = i64::try_from(payload.epoch_local_us)
                .unwrap_or(i64::MAX)
                .saturating_sub(offset_us);
            match capture.as_ref().and_then(|handle| handle.hub.as_ref()) {
                Some(hub) => {
                    hub.apply_epoch(u64::try_from(local_epoch_us).unwrap_or(0));
                    tracing::info!(
                        epoch_id = payload.epoch_id,
                        remote_epoch_us = payload.epoch_local_us,
                        local_epoch_us,
                        offset_us,
                        "M4 RECEIVER_EPOCH：发送端已对齐共同基准"
                    );
                }
                None => tracing::warn!(
                    epoch_id = payload.epoch_id,
                    "M4 RECEIVER_EPOCH：本会话没有采集枢纽，无法对齐"
                ),
            }
        }
        ControlRequest::CloseStream(_) => {
            stop_capture(capture).await;
            stop_playout(playback).await;
            *stream_id = None;
        }

        ControlRequest::Bye(_) => {
            session.apply(SessionEvent::LinkLost);
            session.set_state(SessionState::Reconnecting);
        }

        ControlRequest::StreamStats(stats) => {
            // 对端视角的遥测：① 存成该 peer 的最近快照（Engine::peer_stats，按 peer 隔离）；
            // ② 透传给 UI。M1 不做自适应决策（M2）。
            //
            // 存储必须按 peer 分开：EngineEvent::Telemetry **不带对端 id**（契约 §5 的已知局限，
            // M3 修），事件本身区分不了来源，多对端时共用一份快照就会串流。
            if let Ok(mut slot) = session.peer_stats.lock() {
                *slot = Some(stats);
            }
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

        ControlRequest::SetGain(payload) => {
            // §4.1：音量属于**会话的播放侧** —— 每会话一份状态，多会话各调各的。
            let applies = payload.stream_id == crate::payload::STREAM_ID_ALL
                || Some(payload.stream_id) == *stream_id;
            if !applies {
                tracing::debug!(stream_id = payload.stream_id, "SET_GAIN 指向别的流，忽略");
            } else if let Some(handle) = playback.as_ref() {
                match handle.set_gain(payload.gain, u32::from(payload.ramp_ms), codec.frame_ms) {
                    Ok(target) => tracing::info!(
                        target_x1000 = target,
                        ramp_ms = payload.ramp_ms,
                        "§4.1 音量已更新"
                    ),
                    Err(error) => tracing::warn!(%error, "SET_GAIN 参数非法，已拒绝"),
                }
            }
        }

        ControlRequest::SetMute(payload) => {
            let applies = payload.stream_id == crate::payload::STREAM_ID_ALL
                || Some(payload.stream_id) == *stream_id;
            if applies && let Some(handle) = playback.as_ref() {
                let gain = if payload.mute { 0.0 } else { 1.0 };
                // 50 ms 渐变：静音/恢复都不该有咔哒声。
                let _ = handle.set_gain(gain, 50, codec.frame_ms);
            }
        }

        // ClockResult / Ping / Pong 及其它 M2/M3 命令：仍然**明确不做**而不是假装接受 ——
        // 音量这两条现在已经真的生效（见上面两个分支）。
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

/// 重连单飞门（FR-27）：请求在飞期间挡住重复投递，本函数的**任何**退出路径都会放门。
///
/// 放在 `Drop` 里而不是每个 `return` 前面：`reconnect_once` 有成功、预算耗尽、引擎关闭
/// 三条出口，漏掉任何一条都会让那条会话**永远**发不出重连请求（门再也没人放）。
struct ReconnectFlightGuard(Arc<PeerSession>);

impl Drop for ReconnectFlightGuard {
    fn drop(&mut self) {
        self.0.reconnect_in_flight.store(false, Ordering::SeqCst);
    }
}

/// 取对端在**当前会话表里**的那条会话。
///
/// 为什么不能用手边的 `Arc<PeerSession>`：重连会把表项**换成人** —— 旧对象已摘表并收到
/// `Shutdown`，而 UI 读的永远是表里这一条。回执落在旧对象上等于没有回执（审计 §2.6-④）。
fn current_session(inner: &Arc<Inner>, peer: NodeId) -> Option<Arc<PeerSession>> {
    inner
        .peers
        .lock()
        .ok()
        .and_then(|peers| peers.get(&peer).cloned())
}

/// 收掉一次失败尝试留下的会话：摘表 + 明确 `Shutdown`（不留孤儿）。
async fn discard_stale_session(engine: &Arc<Engine>, peer: NodeId) {
    let stale = engine
        .inner
        .peers
        .lock()
        .ok()
        .and_then(|mut peers| peers.remove(&peer));
    if let Some(stale) = stale {
        let _ = stale.commands.send(SessionCommand::Shutdown).await;
    }
}

/// 一次断链重连：摘掉旧会话 → 退避重拨 → 成功后把流接回去。
///
/// 四条纪律：
///
/// 1. **旧会话必须先摘掉**：`connect_inner` 会按地址查重，对同地址的第二次连接直接回 1009 BUSY，
///    不摘就是「重连必然失败」。
/// 2. **重拨必须走完整的 `connect_inner`**：它内部的 `is_trusted(peer_id)` 决定是否要求 PIN ——
///    重连**不绕过信任库**，已配对过的对端凭指纹免 PIN，陌生人一样要 PIN。
/// 3. **预算用尽要明确落 `ReconnectFailed`**：绝不留一条「看起来在重连」的僵尸状态。
/// 4. **接流失败不能吞**：上一版用 `let _ =` 吞掉，队友指出那会让「重连成功但没声音」变成静默故障。
/// 5. **回执必须落在表里当前那条会话上**：重连会把表项**换成人**（旧会话摘表 + Shutdown、
///    新会话插表），而 UI 读的永远是 `peers()` 表里的那一条 —— 把 `ReconnectOk` 打在旧对象上
///    等于没有回执（审计 §2.6-④）。
async fn reconnect_once(engine: Arc<Engine>, request: ReconnectRequest) {
    // 单飞门由**投递请求的那条会话**持有：本函数无论从哪条路径退出（成功、预算耗尽、
    // 引擎关闭、早退），门都要放掉 —— 否则这条会话再也发不出重连请求。
    let _flight = ReconnectFlightGuard(Arc::clone(&request.session));
    let deadline = Instant::now() + DEFAULT_RECONNECT_BUDGET;
    let mut backoff = RECONNECT_BACKOFF_MIN;
    let mut last_error: Option<AudioLinkError> = None;

    while Instant::now() < deadline {
        if engine.inner.shutdown.load(Ordering::Relaxed) {
            return;
        }

        // 每轮都重摘一次：上一轮尝试可能已经把新会话插进了表里。
        if let Some(stale) = engine
            .inner
            .peers
            .lock()
            .ok()
            .and_then(|mut peers| peers.remove(&request.peer))
        {
            // 摘表**之前/同时**必须让它收工：只摘表不发 Shutdown 会留下活着的僵尸会话 ——
            // 它攥着旧连接与采集枢纽一直跑到 QUIC idle_timeout（30 s），期间还会二次投递
            // 重连请求（审计 §2.6-①）。显式收工让「采集枢纽交接」变成确定行为。
            //
            // 除此之外**不要**再用这条旧会话做任何事：它已经不在表里，任何回执都到不了 UI。
            let _ = stale.commands.send(SessionCommand::Shutdown).await;
        }

        match tokio::time::timeout(
            RECONNECT_ATTEMPT_TIMEOUT,
            engine.connect_inner(request.addr),
        )
        .await
        {
            Ok(Ok(peer)) if peer == request.peer => {
                // 回执落在**表里当前那条会话**上（审计 §2.6-④）。
                //
                // 为什么不是手边的旧对象：`connect_inner` 建的是全新对象并插表，旧对象已在上面
                // 摘表 + 收到 Shutdown —— 把 `ReconnectOk` 打在它身上，`reconnects()` 记在了谁
                // 也看不到的地方，UI 通过 `peers()` 永远读到 0。
                if let Some(current) = current_session(&engine.inner, request.peer) {
                    // 两步迁移是刻意的：新会话在握手完成时已是 Streaming（`mark_streaming`），
                    // 而迁移表只承认 `(Reconnecting, ReconnectOk)` —— 先把它带回 Reconnecting
                    // （它确实刚经历一次链路丢失），`ReconnectOk` 才是合法迁移、计数才真的累加。
                    // 只在它确实处于 Streaming 时做这次往返：状态机若已被别的事件驱动，
                    // 就交给下面的 `set_state` 兜底，绝不硬把计数塞进去。
                    if current.machine_state() == SessionState::Streaming {
                        current.apply(SessionEvent::LinkLost);
                        current.apply(SessionEvent::ReconnectOk);
                    }
                    current.set_state(SessionState::Streaming);
                    // 拉取式读数之外，也让订阅方（UI 事件流）立刻看到这一次变化。
                    let _ = engine
                        .inner
                        .events
                        .send(EngineEvent::PeerUpdated(Box::new(current.snapshot())));
                }
                if let Err(error) = engine.start_send(request.peer).await {
                    // 「重连成功但流接不回来」是这一环最容易静默失败的地方：事件不订阅、
                    // 状态已是 Streaming，只有日志能把根因留在现场。
                    tracing::warn!(
                        peer = %request.peer.short(),
                        "重连成功，但恢复推流失败：{}",
                        error.context()
                    );
                    let _ = engine.inner.events.send(EngineEvent::Error {
                        code: error.code().as_u16(),
                        context: format!(
                            "reconnect succeeded but resuming the stream failed: {}",
                            error.context()
                        ),
                    });
                }
                return;
            }
            Ok(Ok(_)) => {
                last_error = Some(AudioLinkError::bad_request(
                    "reconnected peer identity does not match the session being restored",
                ));
            }
            Ok(Err(error)) => last_error = Some(error),
            Err(_) => {
                // 800 ms 只是**外层 future** 的截止：`connect_inner` 在此之前已经
                // `create_session` 插表并 spawn 了会话任务，丢弃外层 future 收不掉它
                // （审计 §2.6-③）。它若继续跑，会照旧握手、把自己标成 Streaming，而这一轮
                // 已被判失败 —— 下一轮摘表又不给它收尾，僵尸 +1（还可能占掉一路混音源号）。
                discard_stale_session(&engine, request.peer).await;
                last_error = Some(AudioLinkError::bad_request("reconnect attempt timed out"));
            }
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(RECONNECT_BACKOFF_MAX);
    }

    // 终局同样要落在**看得见的那条会话**上：表里此刻通常没有它（每轮都摘表），于是退回投递
    // 请求的那条会话 —— 无论如何不留一条「看起来还在重连」的僵尸状态。
    let session = current_session(&engine.inner, request.peer)
        .unwrap_or_else(|| Arc::clone(&request.session));
    session.apply(SessionEvent::ReconnectFailed);
    session.set_state(SessionState::Failed);
    let detail = last_error
        .map(|error| error.context().to_string())
        .unwrap_or_else(|| String::from("reconnect budget exhausted without a single attempt"));
    let _ = engine.inner.events.send(EngineEvent::PeerDisconnected {
        id: request.peer,
        reason: format!(
            "reconnect gave up after {} s: {detail}",
            DEFAULT_RECONNECT_BUDGET.as_secs()
        ),
    });
}

fn report_error(inner: &Arc<Inner>, session: &Arc<PeerSession>, error: &AudioLinkError) {
    session.apply(SessionEvent::LinkDegraded);
    let _ = inner.events.send(EngineEvent::Error {
        code: error.code().as_u16(),
        context: error.context().to_string(),
    });
}

/// 报告一条会话的对端已经不在（链路错误；对端主动 BYE 走的是状态机的另一条边）。
///
/// FR-27：`allow_reconnect` 为真时把重拨交给监督任务、会话先进 `Reconnecting`，而不是直接终局。
/// **判据必须是「本端是发起方**且正在推流」**（由调用点给出）—— 队友在 docs/50 §4.2 里指出：
/// 接收侧也会置 `stream_id`，若拿它单独当判据就会**双向拨号**，而接收侧去连一个没在监听的发送端
/// 必然失败，失败路径还会把接收侧自己的会话从表里摘掉（实测症状：声音在响、peers() 却是空表）。
fn report_peer_gone(
    inner: &Arc<Inner>,
    session: &Arc<PeerSession>,
    error: &AudioLinkError,
    allow_reconnect: bool,
) {
    session.apply(SessionEvent::LinkLost);
    let _ = inner.events.send(EngineEvent::PeerDisconnected {
        id: session.id,
        reason: format!("{}: {}", error.code().as_u16(), error.context()),
    });

    if allow_reconnect && !inner.shutdown.load(Ordering::Relaxed) {
        // 迁移表（session.rs）：Streaming/Degraded --LinkLost--> Reconnecting。
        session.set_state(SessionState::Reconnecting);
        // **单飞**（审计 §2.6-②）：请求至少有三个来源（两个链路错误出口 + 静默看门狗），
        // 两套 `reconnect_once` 并发时会互相摘掉对方刚插进表里的会话，症状是「接上又断」。
        // `swap` 返回旧值：已经有一条在飞就什么都不做，状态仍留在 Reconnecting。
        if !session.reconnect_in_flight.swap(true, Ordering::SeqCst) {
            let request = ReconnectRequest {
                addr: session.addr,
                peer: session.id,
                session: Arc::clone(session),
            };
            if inner.reconnect_tx.send(request).is_err() {
                // 监督任务已经没了（引擎正在关）：放门并落终局，不留「看起来在重连」的假状态。
                session.reconnect_in_flight.store(false, Ordering::SeqCst);
                session.set_state(SessionState::Failed);
            }
        }
    } else {
        session.set_state(SessionState::Failed);
    }
}

// ---------------------------------------------------------------------------
// 播放线程
// ---------------------------------------------------------------------------

/// 播放线程：**在本线程内**创建 `PlayoutSink`（`!Send`）。
/// 引擎级混音器句柄（M4 汇聚）。
type PlayoutMix = Arc<Mutex<PcmMixer>>;

/// 混音源号的全局计数器（混音器按 id 认路，不能让不同会话撞号）。
static NEXT_MIX_SOURCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// 混音路数超过 FR-12 上限时的错误上下文。
const MIXER_FULL: &str = "mixer is full (FR-12 allows 8 sources)";

/// 引擎级混音槽（M4）：混音器 + 「谁在真正持有播放设备」的世代标记。
///
/// # 为什么必须有它（FR-27 审计 §2.2 定位的断点）
///
/// 这个槽**从第一次建起永不置空**（M4 多路汇聚靠的就是复用同一个混音器），而旧代码把
/// 「槽里已经有东西」当成了「我不是 owner」：接收侧第一条会话结束、重连后的第二条会话建立
/// 播放管线时，新会话必然拿到 `is_owner = false` ⇒ 不建 sink ⇒ 每帧只把 PCM 混进混音器、
/// **一个字节都不写设备**。表现就是两侧状态都显示 `Streaming`、恢复延迟却是 `None`。
///
/// 断点的本质是：owner 是一个**身份**，却被记成了「槽里有没有东西」这种**一次性事实**。
/// 把 owner 变成可让位、可接管的状态，重连后的新会话就会重新打开设备并把混音输出接回
/// 扬声器，而 `PcmMixer` 对象继续复用 —— 多路混音语义不变。
struct PlayoutMixSlot {
    /// 多路汇聚的混音器（owner 与 guest 共用这一个对象）。
    mixer: PlayoutMix,
    /// 当前 owner 的**源号**（0 = 无人持有播放设备）。
    ///
    /// 用源号当世代、而不是一个裸布尔：源号由 `NEXT_MIX_SOURCE` 全局单调分配、永不重复，
    /// 于是「让位」可以写成一次 `compare_exchange(我的源号 → 0)` —— **只有仍是当前 owner 的
    /// 那一路清得掉**。裸布尔做不到这一点：旧 owner 迟到的退出会把刚接管的新 owner 的标志
    /// 清成 false，第三个会话随后就能抢到 owner，同一台设备被两路同时写。
    owner: AtomicU32,
}

impl PlayoutMixSlot {
    /// FR-27：当前是否真有人持有播放设备（观测用；测试的判据 ⑧ 想知道的就是它）。
    #[allow(dead_code)]
    fn owner_alive(&self) -> bool {
        self.owner.load(Ordering::SeqCst) != 0
    }
}

/// 一路会话在引擎级混音器上的登记结果。
struct PlayoutAssignment {
    /// 引擎级混音器（owner 与 guest 共用同一个对象）。
    mixer: PlayoutMix,
    /// 本路的源号（停止时用它 `remove_source`）。
    source: u32,
    /// 本路的槽位凭据。**每一路都有** —— 谁抢到 owner 谁写设备，而谁能抢到是运行期决定的。
    slot: Arc<PlayoutMixSlot>,
}

/// 取得（必要时创建）引擎级混音器，并给本会话分配一个源号。
///
/// 只有 owner 会真的打开播放设备，其余会话把解码后的 PCM 混进来 —— 这就是 M4 的
/// 「同一接收端多路混音」。路数超过 FR-12 的上限时明确报 STREAM_LIMIT，而不是悄悄丢掉一路。
///
/// # 这里**不**决定谁是 owner（M4/FR-27 懒接管）
///
/// 早先的版本在这里做一次 `compare_exchange(0 → 本路源号)` 定 owner，只在建立播放管线
/// （收到 `OPEN_STREAM`）时被调用一次。它有两个漏：
///
/// 1. **已经在混音的 guest 没有任何通路重新走这里** —— owner 退出后它只能永远哑着混音；
/// 2. 首路的 owner 身份与「这一次调用」绑死，运行期再无可让位/接管的时机。
///
/// 所以 owner 的取得与让出都挪进播放循环（见 `playout_main` 的接管块）：谁在运行期抢到谁写设备。
fn acquire_playout_mixer(
    inner: &Arc<Inner>,
    codec: &CodecConfig,
) -> Result<PlayoutAssignment, AudioLinkError> {
    let mut slot = inner
        .playout_mixer
        .lock()
        .map_err(|_| AudioLinkError::bad_request("playout mixer poisoned"))?;
    let source = NEXT_MIX_SOURCE.fetch_add(1, Ordering::Relaxed);
    // 协议 §3 锁定 48 kHz / 2ch：多路混音的前提就是同一时间轴、同一格式（NFR-13）。
    let format = MixFormat {
        sample_rate: 48_000,
        channels: 2,
    };
    // 槽里已有混音器就复用（M4 语义），没有才新建。
    let slot = match slot.as_ref() {
        Some(existing) => Arc::clone(existing),
        None => {
            let fresh = Arc::new(PlayoutMixSlot {
                mixer: Arc::new(Mutex::new(PcmMixer::new(
                    format,
                    codec.frame_samples(),
                    PLAYBACK_QUEUE_FRAMES,
                ))),
                owner: AtomicU32::new(0),
            });
            *slot = Some(Arc::clone(&fresh));
            fresh
        }
    };
    {
        let mut guard = slot.mixer.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .add_source(source)
            .map_err(|_| AudioLinkError::stream_limit(MIXER_FULL))?;
    }
    Ok(PlayoutAssignment {
        mixer: Arc::clone(&slot.mixer),
        source,
        slot,
    })
}

/// 设备打开失败后的重试间隔（M4/FR-27 懒接管）。
///
/// 为什么必须有退避：`factory()` 在设备忙、权限不足、音频服务没起来时会失败，而播放拍是
/// 20 ms 一拍 —— 不退避就是每秒 50 次打开设备尝试，既把设备句柄与 CPU 打满，也把日志刷成噪声。
/// 250 ms 足以让瞬时故障过去，又不至于让接管明显迟到（验收给接管的窗口是秒级）。
const OWNER_TAKEOVER_RETRY: Duration = Duration::from_millis(250);

/// 抢 owner：成功 = 本路接下来负责把混音结果写进设备。
///
/// owner 里存的是**源号**（0 = 无人持有），所以「让位」能写成一次
/// `compare_exchange(我的源号 → 0)` —— 只有仍是当前 owner 的那一路清得掉，旧 owner 迟到的
/// 退出不会误清新 owner。CAS 也保证并发接管只有一个赢家：不会两路同时写同一台设备。
fn claim_playout_owner(slot: &PlayoutMixSlot, source: u32) -> bool {
    slot.owner
        .compare_exchange(0, source, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// 让位（幂等）：对「已经不是 owner」的调用者是空操作。
fn release_playout_owner(slot: &PlayoutMixSlot, source: u32) {
    let _ = slot
        .owner
        .compare_exchange(source, 0, Ordering::SeqCst, Ordering::SeqCst);
}

/// 打开播放设备，并把「零重采样」硬闸（NFR-13）过一遍。
///
/// 失败时**不动**已经打开的 sink：调用方自己决定是「保留旧的」（看门狗重建失败）还是
/// 「本来就没有」（接管失败，让位后重试）。
fn open_playout_sink(
    factory: &(dyn Fn() -> Result<Box<dyn PlayoutSink>, AudioError> + Send + Sync),
    sink: &mut Option<Box<dyn PlayoutSink>>,
) -> Result<(), AudioLinkError> {
    let open = factory().map_err(|error| audio_error(&error))?;
    require_unified_format("playout", open.backend_name(), open.device_format())?;
    *sink = Some(open);
    Ok(())
}

/// 一拍的提交结果。
///
/// 三态而不是 `bool`，是因为「设备写失败」与「结构性错误」要分开：
/// 前者是 FR-28 的看门狗要接手的现场（**不终止**播放线程，否则一次异常就永久静音），
/// 后者（混音器锁中毒 / 推入失败）说明别处已经不对了，继续跑没有意义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameWrite {
    /// 成功（含非 owner：只混进引擎级混音器）。
    Ok,
    /// 设备侧写失败：由看门狗计时，到点重建 sink。
    SinkFailed,
    /// 结构性失败：调用方终止播放循环。
    Fatal,
}

/// 写出一帧。
///
/// 没接混音器时就是直通 sink（M1 起的行为）；接上之后本路先把样本混进去，
/// 只有 owner（**运行期抢到设备**的那一路）才把混音结果写出去 —— 于是多路只开一次设备。
///
/// 这里不看「建会话时我是不是 owner」：本路此时有没有 `sink` 就是答案（懒接管的全部状态
/// 只有这一个 `Option`）。没有 sink 只说明别人持有设备，帧照样要混进去。
fn write_frame(
    sink: &mut Option<Box<dyn PlayoutSink>>,
    mixer: &Option<Arc<Mutex<PcmMixer>>>,
    mix_source: u32,
    samples: &[f32],
    mixed: &mut Vec<f32>,
) -> FrameWrite {
    let Some(mixer) = mixer.as_ref() else {
        // 没有引擎级混音器：M1 的直通语义（本路直接写设备；没有设备就无处输出）。
        return match sink.as_mut() {
            None => FrameWrite::Ok,
            Some(open) => write_into(open, samples),
        };
    };
    let Ok(mut guard) = mixer.lock() else {
        return FrameWrite::Fatal;
    };
    if guard.push(mix_source, samples).is_err() {
        return FrameWrite::Fatal;
    }
    let Some(open) = sink.as_mut() else {
        return FrameWrite::Ok; // 还不是 owner：混进去就完事
    };
    guard.mix_frame(mixed);
    write_into(open, mixed)
}

/// sink 账本里的累计提交帧数（本路不持有设备时为 0）。
///
/// FR-28 的「有没有真的输出」就看它的增量：`PlayoutSink::write` 成功但账本不长的 sink
/// 与写失败的 sink 在这里是同一种事实。
fn sink_written_frames(sink: &Option<Box<dyn PlayoutSink>>) -> u64 {
    sink.as_ref().map_or(0, |open| open.stats().frames_written)
}

/// FR-28：把坏掉的播放 sink 换掉 —— 停掉旧的（幂等），再向工厂要一个新的。
///
/// 失败时**保留**旧 sink 并返回错误：看门狗会退避后再来；放弃等于把「一次异常永久静音」
/// 换成「一次异常永久不看门」，那正是这条需求要根治的东西。
fn reopen_playout_sink(
    factory: &(dyn Fn() -> Result<Box<dyn PlayoutSink>, AudioError> + Send + Sync),
    sink: &mut Option<Box<dyn PlayoutSink>>,
) -> Result<(), AudioError> {
    if let Some(open) = sink.as_mut() {
        open.stop();
    }
    let next = factory()?;
    *sink = Some(next);
    Ok(())
}

/// 提交给设备，并把失败区分为「设备写失败」而不是结构性错误。
fn write_into(open: &mut Box<dyn PlayoutSink>, samples: &[f32]) -> FrameWrite {
    match open.write(samples) {
        Ok(()) => FrameWrite::Ok,
        Err(error) => {
            // 不在这里刷日志：2 s 的判定窗口里会连续失败 100 次，刷日志只会把真正的原因埋掉。
            // 需要留痕的是「看门狗决定重建」那一次（见 rebuild_playout_sink）。
            tracing::debug!(%error, "播放 sink 写入失败（FR-28 看门狗开始计时）");
            FrameWrite::SinkFailed
        }
    }
}

fn spawn_playout_thread(
    inner: &Arc<Inner>,
    session: &Arc<PeerSession>,
    codec: &CodecConfig,
    jitter_depth: Arc<AtomicUsize>,
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
    // §7 排播状态：会话任务写、播放线程读；默认关闭（没有组基准时一切照旧）。
    let sync: PlayoutSyncHandle = Arc::new(Mutex::new(PlayoutSync::default()));
    let thread_sync = Arc::clone(&sync);
    // §4.1：音量状态每会话一份；起始 1.0（不做渐变起步）。
    let gain: Arc<Mutex<GainState>> = Arc::new(Mutex::new(GainState::new(1_000)));
    let thread_gain = Arc::clone(&gain);
    let events = inner.events.clone();

    let telemetry = Arc::clone(&session.telemetry);
    let tap = inner.config.measurement.clone();
    let frame_ms = u64::from(codec.frame_ms.max(1));
    let pcm = codec.interleaved_frame();

    // M4 汇聚：混音器是**引擎级**的。owner 真正打开 sink，其余会话把自己的帧混进来 ——
    // 同一台设备只被打开一次，多路在软件侧求和 + 软限幅。
    // FR-27：owner 是**可让位、可接管**的，所以重连后的新会话也会在这里抢到 owner 并重新开设备。
    let PlayoutAssignment {
        mixer,
        source: mix_source,
        slot,
    } = acquire_playout_mixer(inner, codec)?;
    let thread_mixer = Some(mixer.clone());
    // M4 懒接管：**每一路**都拿着工厂与槽位 —— 谁在运行期抢到 owner 谁写设备。
    // 所以不能再按「建会话时是不是 owner」裁剪工厂：那正是「已经在混音的 guest」哑掉的第二个原因
    // （它连打开设备的工具都没有）。
    let thread_factory = Arc::clone(factory);
    let thread_slot = Arc::clone(&slot);
    // 让位守卫跟着播放线程走：线程自己死掉（设备被拔、写失败、建 sink 失败、panic）也必须让位，
    // 否则引擎会永远以为「还有人持有设备」，别的路数一直抢不到。
    let owner_guard = PlayoutOwnerGuard {
        slot: Arc::clone(&slot),
        source: mix_source,
    };

    let join = std::thread::Builder::new()
        .name("audiolink-playout".to_string())
        .spawn(move || {
            // 守卫活到闭包结束：**所有**退出路径（含早期 return 与 panic）都会在 drop 时让位。
            let _owner_guard = owner_guard;
            playout_main(
                thread_factory.as_ref(),
                frame_ms,
                pcm,
                telemetry,
                tap,
                thread_stop,
                jitter_depth,
                thread_sync,
                events,
                thread_gain,
                thread_mixer.clone(),
                mix_source,
                thread_slot,
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
            sync,
            gain,
            mixer: Some(mixer),
            mix_source: Some(mix_source),
            mix_slot: slot,
        }),
        // 线程自己报的错：它已经在退出路径上让位了（线程侧的守卫）。
        Ok(Err(error)) => Err(error),
        Err(_) => {
            // 超时：线程可能还活着，甚至已经抢到 owner。置 stop 让它下一拍退出，
            // 由它自己的守卫让位 —— 不留下「线程还在、owner 标志还立着」的孤儿。
            stop.store(true, Ordering::Relaxed);
            Err(AudioLinkError::bad_request(
                "playout sink did not report readiness in time",
            ))
        }
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
    jitter_depth: Arc<AtomicUsize>,
    sync: PlayoutSyncHandle,
    events: broadcast::Sender<EngineEvent>,
    gain_state: Arc<Mutex<GainState>>,
    mixer: Option<PlayoutMix>,
    mix_source: u32,
    mix_slot: Arc<PlayoutMixSlot>,
    frames: Receiver<PlaybackFrame>,
    ready_tx: std::sync::mpsc::Sender<Result<(), AudioLinkError>>,
) {
    // M4 懒接管：本路有没有设备**不是**建会话时定下的，而是运行期抢来的 —— 所以这里
    // 一开始可以是 None，循环里再抢（见下方的接管块）。
    let mut sink: Option<Box<dyn PlayoutSink>> = None;
    // FR-28 看门狗只在「本路真的持有设备」时存在：非 owner 的「没有输出」是它的正常状态。
    let mut watchdog: Option<SinkWatchdog> = None;
    // 接管失败后的退避截止时刻（见 `OWNER_TAKEOVER_RETRY`）；初始为「现在」= 立即可试。
    let mut owner_retry_after = Instant::now();

    // 首次接管尝试：**每一条路都走这里**。首路必然抢到（owner 初值 0），后续路数抢不到就只混音。
    //
    // 为什么这一次要同步做完并把结果交给 `ready_tx`：建立播放管线那一步必须能回答「对端开流
    // 成功了吗」—— 抢到 owner 却打不开设备是**失败**，要上报，而不是安静地降级成 guest。
    if claim_playout_owner(&mix_slot, mix_source) {
        match open_playout_sink(factory, &mut sink) {
            Ok(()) => watchdog = Some(SinkWatchdog::new(Instant::now())),
            Err(error) => {
                // 打不开就不能占着 owner：让回，让别的路数去试。
                release_playout_owner(&mix_slot, mix_source);
                let _ = ready_tx.send(Err(error));
                return;
            }
        }
    }

    let _ = ready_tx.send(Ok(()));

    let period = Duration::from_millis(frame_ms);
    let silence = vec![0f32; pcm_len];
    let mut primed = false;
    let mut depth_state = PlayoutDepthState::new(jitter_depth.load(Ordering::Relaxed));
    let mut next_write = Instant::now();
    let mut expected_seq: Option<u32> = None;
    let mut pending: Option<PlaybackFrame> = None;
    let mut refill_after_underrun = false;
    let mut scheduled_reported = false;
    let mut mixed: Vec<f32> = Vec::new();

    'playout: while !stop.load(Ordering::Relaxed) {
        // 睡到下一个提交时刻。节奏必须由**本地时钟**决定，数据到没到只影响
        // 「这一拍写什么」，不影响「什么时候写」—— 否则每一个网络到达抖动都会直接
        // 变成出声时间抖动，§7 的「由 epoch 驱动而非到达时间驱动」就落空了。
        let now = Instant::now();
        if next_write > now {
            std::thread::sleep(next_write - now);
        }

        // 后端阻塞或系统抢占超过一帧时，那些播放拍已经过去，不能通过随后慢慢提交旧帧来
        // “补回来”：那会把一次短暂停顿变成余下整段音频的固定延迟。推进序号游标，随后由
        // take_due_frame 丢掉这些过期帧；保留当前所在的时钟相位，避免追赶式突发写入。
        let current = Instant::now();
        if primed && current > next_write {
            let missed = current.duration_since(next_write).as_nanos() / period.as_nanos();
            if missed > 0 {
                let missed = u32::try_from(missed).unwrap_or(u32::MAX);
                if let Some(seq) = expected_seq.as_mut() {
                    *seq = seq.wrapping_add(missed);
                }
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.record_underruns(missed);
                }
                next_write += period.saturating_mul(missed);
            }
        }
        next_write += period;

        // M4/FR-27 懒接管：owner 退出后，**已经在混音的 guest** 没有任何别的通路能重新拿到设备
        // ——它不会再收到一次 `OPEN_STREAM`，所以接管动作必须长在每一拍上。
        //
        // 抢到就开设备（成为 owner），抢不到就说明别人正持有 —— CAS 单赢家，不会两路同写一台设备。
        // 只在 `sink.is_none()`（本路还没有设备）时才试，且失败要退避，绝不每拍试开。
        if sink.is_none() && now >= owner_retry_after && claim_playout_owner(&mix_slot, mix_source)
        {
            match open_playout_sink(factory, &mut sink) {
                Ok(()) => {
                    watchdog = Some(SinkWatchdog::new(Instant::now()));
                    tracing::info!(
                        source = mix_source,
                        "M4 混音器 owner 接管：本路重新打开播放设备"
                    );
                }
                Err(error) => {
                    // 打开失败**必须让回**并退避（见 `OWNER_TAKEOVER_RETRY`）：
                    // 占着 owner 不放会让整台引擎永远没有设备写；每拍都试开则会把设备句柄与
                    // CPU 一起打满，还会把日志刷成噪声。
                    release_playout_owner(&mix_slot, mix_source);
                    owner_retry_after = Instant::now() + OWNER_TAKEOVER_RETRY;
                    tracing::warn!(
                        source = mix_source,
                        context = error.context(),
                        "M4 混音器 owner 接管失败，已让回并退避重试"
                    );
                    let _ = events.send(EngineEvent::Error {
                        code: ErrorCode::SinkRebuild.as_u16(),
                        context: format!(
                            "混音器 owner 接管失败（已让回，{} ms 后重试）：{}",
                            OWNER_TAKEOVER_RETRY.as_millis(),
                            error.context()
                        ),
                    });
                }
            }
        }

        // 起步攒帧：不足当前目标深度就继续等。这一拍**不补静音也不算欠载** ——
        // 还没开始播，谈不上「欠」；把攒帧期算成欠载会让欠载率失去意义。
        if !primed {
            let target = jitter_depth
                .load(Ordering::Relaxed)
                .clamp(MIN_TARGET_FRAMES, MAX_TARGET_FRAMES);
            if frames.len() >= target {
                depth_state = PlayoutDepthState::new(target);
                primed = true;
                // §7 起播对齐：有排播时**首拍直接落在该帧的 target 上**，没有排播时才退回全局帧网格。
                //
                // 为什么必须锚到 target 本身（2026-09-17 第三层修复，两次尝试的结论）：
                // ① 拍点原本锚在**全局**帧网格（相位 0），而 target 也被向上取整到同一个全局网格 ——
                //    两端的时钟估计只差 ε，各自取整就可能落到**相邻两格**，整段音频差一帧（实测 20~40%）；
                // ② 把拍点改成「该帧 target 的相位」也**不够**：相位点可能落在 target **之前**，这一端
                //    要多等一拍（+20 ms）才播，另一端恰好 ≥ target 立刻播 —— 依然差一帧（实测 10 轮 2 轮）。
                // 锚到 target 之后，两端的首拍只差各自的 ε（回环约 2 ms），此后每拍 +frame 与 target 序列同步。
                // 详见 docs/49-m3-group-start-phase.md。
                let now_us = i64::try_from(now_monotonic_us()).unwrap_or(i64::MAX);
                let (anchor_us, sched_frame_us) = playout_start_anchor(&sync);
                if sched_frame_us > 0 {
                    let wait_us = anchor_us.saturating_sub(now_us).max(0);
                    next_write = Instant::now() + Duration::from_micros(wait_us as u64);
                } else {
                    let frame_us = i64::try_from(period.as_micros()).unwrap_or(20_000).max(1);
                    let remainder = now_us.rem_euclid(frame_us);
                    next_write =
                        Instant::now() + Duration::from_micros((frame_us - remainder) as u64);
                }
                // 这一拍只用来对齐起拍，不写数据；下一拍起就在 target 序列上，与对端一致。
                continue;
            } else {
                continue;
            }
        }

        // FR-28：链路还在给音频、设备却连续 2 s 一帧都没接走 → 换一个 sink。
        //
        // 为什么在这里判（而不是在写失败的那一拍直接重建）：一次写失败很常见（设备忙、缓冲瞬时满），
        // 直接重建等于抖动一下就把设备重开一遍；需求要的是「持续 2 s 没有输出」，
        // 所以计时器只认「真实音频这一拍有没有真的进到 sink 账本里」。
        if let Some(watchdog) = watchdog.as_mut()
            && let Some(stall) = watchdog.poll(now)
        {
            let attempt = watchdog.attempts().saturating_add(1);
            let reopened = reopen_playout_sink(factory, &mut sink);
            let rebuilt = reopened.is_ok();
            watchdog.note_rebuild_attempt(Instant::now(), rebuilt);

            // 2002 SINK_REBUILD：这是「自愈动作真的发生过」的唯一可见证据
            // （UI 的错误横幅、日志、验收测试都看它）。
            let _ = events.send(EngineEvent::Error {
                code: ErrorCode::SinkRebuild.as_u16(),
                context: match reopened {
                    Ok(()) => format!(
                        "播放器 {} ms 没有接走任何一帧音频但链路正常，已重建（第 {attempt} 次尝试，累计成功 {} 次）",
                        stall.silent_for.as_millis(),
                        watchdog.rebuilds()
                    ),
                    Err(error) => format!(
                        "播放器 {} ms 没有输出且重建失败（第 {attempt} 次尝试，累计成功 {} 次）：{}",
                        stall.silent_for.as_millis(),
                        watchdog.rebuilds(),
                        error.context()
                    ),
                },
            });

            if rebuilt {
                // 【时序状态处置】停摆这几秒的音频**已经过期**：清积压、把时间轴从「下一个到达的帧」重启。
                //
                // 为什么不按旧游标继续播：那些帧的播放时刻已经过去，追播只会把永久延迟钉进链路
                // （与 §7 第 1 条「宁可丢一帧，也不延迟出声」、以及本函数开头「被抢占后不追赶」同一口径）。
                // 也不把这些积压记成 `late_drops` —— 它们是自愈动作主动作废的，不是网络迟到，
                // 严格档的零容忍只该给真迟到。
                //
                // §7 排播不受影响：`expected_seq` 只决定「跳过哪些旧帧」，目标时刻由 epoch 换算
                // （`PlayoutSync::sample_index_of` 走首帧基准），重建不会破坏组内对齐。
                while frames.try_recv().is_ok() {}
                pending = None;
                expected_seq = None;
                // 新 sink 的队列是空的：按当前抖动目标重新攒出余量，否则刚重建就连着欠载。
                refill_after_underrun = true;
                // 重建本身耗时（打开设备通常几十毫秒）：把节奏拉回「现在 + 一拍」，
                // 免得刚自愈就被算成一串「错过的播放拍」而记一堆欠载。
                next_write = Instant::now() + period;
            }
        }

        // 升档必须真的建立出额外余量。若队列还没攒到新目标，这一拍写静音但不推进序号；
        // 最多从 20/40 ms 升到 60 ms，因此重缓冲有严格上界，不会恢复成永久增长。
        let requested_target = jitter_depth
            .load(Ordering::Relaxed)
            .clamp(MIN_TARGET_FRAMES, MAX_TARGET_FRAMES);
        let buffered_frames = frames.len() + usize::from(pending.is_some());
        // 最高档位无法再靠“升档”触发补水。欠载后等到至少一个新帧到达，再按当前目标
        // 重新建立余量；完全断流时不进入 Hold，播放游标仍按真实时间推进。
        if refill_after_underrun && buffered_frames > 0 {
            depth_state.refill_after_underrun(buffered_frames);
            refill_after_underrun = false;
        }
        // §7 预约播放：有组基准时，起播时刻由 epoch 决定，而不是「队列攒够就播」。
        // 等待与丢弃都补静音（保持时间轴推进），但等待**不**推进游标 —— 那帧还要在目标时刻播。
        let mut scheduled_mode = false;
        if let Some((action, sample_index)) =
            schedule_action(&sync, peek_frame(&frames, &mut pending))
        {
            scheduled_mode = true;
            match action {
                PlayoutAction::Wait { wait_us } => {
                    report_playout_scheduled(
                        &events,
                        &sync,
                        sample_index,
                        wait_us,
                        &mut scheduled_reported,
                    );
                    if write_frame(&mut sink, &mixer, mix_source, &silence, &mut mixed)
                        == FrameWrite::Fatal
                    {
                        break 'playout;
                    }
                    continue;
                }
                PlayoutAction::Drop { late_us } => {
                    if let Some(frame) = pending.take()
                        && let Some(seq) = expected_seq.as_mut()
                    {
                        *seq = frame.seq.wrapping_add(1);
                    }
                    if let Ok(mut telemetry) = telemetry.lock() {
                        telemetry.record_late_drop();
                    }
                    tracing::debug!(late_us, "§7 预约播放：过期帧已丢弃");
                    if write_frame(&mut sink, &mixer, mix_source, &silence, &mut mixed)
                        == FrameWrite::Fatal
                    {
                        break 'playout;
                    }
                    continue;
                }
                PlayoutAction::Play => report_playout_scheduled(
                    &events,
                    &sync,
                    sample_index,
                    0,
                    &mut scheduled_reported,
                ),
            }
        }

        // §7：**有排播时不参与**抖动缓冲的「升档补余量」。
        //
        // 为什么（2026-09-17 第四次定位）：`Hold` 会「写一拍静音、不推进序号」—— 在本地游标模式下这
        // 是建立余量的正当手段，但在排播模式下它等于把这一端**整条时间轴后移一帧**。两端各自自适应升档，
        // 只要一端触发、另一端没触发，就出现「组内偏差不是 0 就是整整一帧」的失败样本
        // （实测 P50 20.03 ms、P95 20.38 ms，扣帧后 0.00 / 0.03 ms）。排播模式下时间轴由 epoch 说了算，
        // 余量该由协议 §7 的 lead_ms 提供，不该靠插入静音。
        let depth_action = depth_state.action(requested_target, buffered_frames);
        match depth_action {
            PlayoutDepthAction::Hold if scheduled_mode => {} // 排播模式：不 Hold，落到下面的正常取帧
            PlayoutDepthAction::Hold => {
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.record_underrun();
                }
                if write_frame(&mut sink, &mixer, mix_source, &silence, &mut mixed)
                    == FrameWrite::Fatal
                {
                    break 'playout;
                }
                continue;
            }
            PlayoutDepthAction::DropOldest(count) => {
                if !drop_oldest_due_frames(
                    &frames,
                    &mut pending,
                    &mut expected_seq,
                    &telemetry,
                    count,
                ) {
                    break 'playout;
                }
            }
            PlayoutDepthAction::Play => {}
        }

        match take_due_frame(&frames, &mut pending, &mut expected_seq, &telemetry) {
            DueFrame::Ready(mut frame) => {
                // §4.1：音量在**播放前**应用，并按帧走一个步长（硬切增益就是爆音）。
                let gain = gain_state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .next_frame_gain();
                if (gain - 1.0).abs() > f32::EPSILON {
                    for sample in frame.samples.iter_mut() {
                        *sample *= gain;
                    }
                }
                if let Some(watchdog) = watchdog.as_mut() {
                    watchdog.note_ready_frame(Instant::now());
                }
                let frames_before = sink_written_frames(&sink);
                let outcome =
                    write_frame(&mut sink, &mixer, mix_source, &frame.samples, &mut mixed);
                if let Some(watchdog) = watchdog.as_mut() {
                    // 「成功输出」= sink 账本这一拍真的长了一帧。写失败与「收下却不记账」
                    // 在这个口径下是同一种事实（见 sink_watchdog 模块文档）。
                    watchdog.note_audible_submit(
                        Instant::now(),
                        sink_written_frames(&sink).saturating_sub(frames_before),
                    );
                }
                if outcome == FrameWrite::Fatal {
                    break 'playout;
                }
                if let Some(tap) = tap.as_ref() {
                    tap.record_played(frame.seq, Instant::now());
                }
            }
            DueFrame::Missing => {
                // 欠载（§7 第 4 条）：补齐静音并计数。**不**静默忽略 ——
                // 「能听出卡顿」与「遥测显示欠载」必须是同一件事。
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.record_underrun();
                }
                // §7：排播模式下不升档 —— 升档会引入「Hold 一拍」，而那会把整条时间轴后移一帧。
                if !scheduled_mode {
                    let _ =
                        jitter_depth.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |depth| {
                            Some((depth + 1).min(MAX_TARGET_FRAMES))
                        });
                    refill_after_underrun = true;
                }
                if write_frame(&mut sink, &mixer, mix_source, &silence, &mut mixed)
                    == FrameWrite::Fatal
                {
                    break 'playout;
                }
            }
            DueFrame::Disconnected => break,
        }
    }

    if let Some(open) = sink.as_mut() {
        open.stop();
    }
}

/// 抖动深度**降档**：丢掉控制器要求的最旧 `count` 拍。
///
/// 记账口径（第 85 轮拆分）：这一路径记 [`TelemetryAggregator::record_depth_drop`]，**不**记
/// `late_drops` —— 降档是控制器为了降低排队延迟做出的**主动策略选择**，与链路质量无关，
/// 干净回环上每次降档都会发生一次（t≈30 s 首次降档）。`late_drops` 只留给「帧到得太晚」，
/// 即 `take_due_frame` 对落后于播放游标的帧的记账（保持原样，一字未改）。
///
/// 计的是**播放拍**而不是「帧」：控制器请求丢一拍时，那一拍取到的可能是真实帧（真的丢了音频），
/// 也可能是空拍（游标空推进）。两种都是这次降档的代价，一起计入。
///
/// 返回 `false` = 队列已断开，调用方应当结束播放循环。
fn drop_oldest_due_frames(
    frames: &Receiver<PlaybackFrame>,
    pending: &mut Option<PlaybackFrame>,
    expected_seq: &mut Option<u32>,
    telemetry: &Arc<Mutex<TelemetryAggregator>>,
    count: usize,
) -> bool {
    for _ in 0..count {
        match take_due_frame(frames, pending, expected_seq, telemetry) {
            DueFrame::Ready(_) | DueFrame::Missing => {
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.record_depth_drop();
                }
            }
            DueFrame::Disconnected => return false,
        }
    }
    true
}

enum DueFrame {
    Ready(PlaybackFrame),
    Missing,
    Disconnected,
}

/// 取当前播放拍对应的帧；迟到帧直接丢弃，未来帧留到它自己的播放拍。
///
/// `expected_seq` 是播放时钟游标，而不是“最后收到的序号”。欠载写入静音后它照样推进，
/// 因此刚错过死线才到达的包不会在下一拍被播放并永久增加延迟。半区间比较保留 u32 回绕语义。
fn take_due_frame(
    frames: &Receiver<PlaybackFrame>,
    pending: &mut Option<PlaybackFrame>,
    expected_seq: &mut Option<u32>,
    telemetry: &Arc<Mutex<TelemetryAggregator>>,
) -> DueFrame {
    loop {
        let frame = if let Some(frame) = pending.take() {
            frame
        } else {
            match frames.try_recv() {
                Ok(frame) => frame,
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    if let Some(seq) = expected_seq.as_mut() {
                        *seq = seq.wrapping_add(1);
                    }
                    return DueFrame::Missing;
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    return DueFrame::Disconnected;
                }
            }
        };

        let Some(expected) = *expected_seq else {
            *expected_seq = Some(frame.seq.wrapping_add(1));
            return DueFrame::Ready(frame);
        };
        let distance = frame.seq.wrapping_sub(expected);
        if distance == 0 {
            *expected_seq = Some(expected.wrapping_add(1));
            return DueFrame::Ready(frame);
        }
        if distance < (1 << 31) {
            *pending = Some(frame);
            *expected_seq = Some(expected.wrapping_add(1));
            return DueFrame::Missing;
        }

        if let Ok(mut telemetry) = telemetry.lock() {
            telemetry.record_late_drop();
        }
    }
}

/// 看一眼下一帧（不消费：必要时把它从队列挪进 `pending`）。
fn peek_frame<'a>(
    frames: &Receiver<PlaybackFrame>,
    pending: &'a mut Option<PlaybackFrame>,
) -> Option<&'a PlaybackFrame> {
    if pending.is_none() {
        match frames.try_recv() {
            Ok(frame) => *pending = Some(frame),
            Err(_) => return None,
        }
    }
    pending.as_ref()
}

/// 起拍锚点：返回（**本流首帧的 target 时刻** µs，排播网格长度 µs）。
/// 没有排播 / 还没收到首包时返回 (0, 0)，调用方据此退回全局帧网格（相位 0）。
///
/// # 为什么锚「首帧」而不是「队列里当前最旧的那一帧」（2026-09-17 第 58 轮）
///
/// 队列里最旧的一帧**不一定是本流的第一帧** —— 数据报到达顺序不保证，首帧可能还在网络上。
/// 一旦两端各自用「自己队列里的最旧帧」锚定，而这两个首帧序号相差 1，两端的拍点序列就整体差一帧：
/// 失败样本正是「**首次写出**就偏离 19.95 ms、两端写入次数只差 1」（见 docs/49 §5.4）—— 偏移发生在起拍那一拍，
/// 不是中途漂移。首帧样本序号由 `PlayoutSync.base` 给出（本流第一个数据报的 `(seq, sample_index)`），
/// 它与到达顺序无关，两端的取值必然一致。
fn playout_start_anchor(sync: &PlayoutSyncHandle) -> (i64, i64) {
    let Ok(state) = sync.lock().map(|guard| *guard) else {
        return (0, 0);
    };
    let Some(schedule) = state.schedule else {
        return (0, 0);
    };
    let Some((_base_seq, base_sample)) = state.base else {
        return (0, 0);
    };
    let frame_us = crate::epoch::frame_us(state.frame_samples);
    if frame_us <= 0 {
        return (0, 0);
    }
    (schedule.target_us(base_sample, state.offset_us), frame_us)
}

/// §7：有排播、且这一帧能换算成样本序号时给出判定；否则 `None`（走正常排播）。
fn schedule_action(
    sync: &PlayoutSyncHandle,
    frame: Option<&PlaybackFrame>,
) -> Option<(PlayoutAction, u32)> {
    let state = *sync.lock().ok()?;
    let schedule = state.schedule?;
    let sample_index = state.sample_index_of(frame?.seq)?;
    let now_us = i64::try_from(now_monotonic_us()).unwrap_or(i64::MAX);
    Some((
        schedule.action(now_us, sample_index, state.offset_us),
        sample_index,
    ))
}

/// 首次进入排播（等待或正好赶上）时报一次时间线，供验收与 UI 使用。
fn report_playout_scheduled(
    events: &broadcast::Sender<EngineEvent>,
    sync: &PlayoutSyncHandle,
    sample_index: u32,
    wait_us: u64,
    reported: &mut bool,
) {
    if *reported {
        return;
    }
    let Ok(state) = sync.lock().map(|guard| *guard) else {
        return;
    };
    let Some(schedule) = state.schedule else {
        return;
    };
    *reported = true;
    let _ = events.send(EngineEvent::PlayoutScheduled {
        epoch_id: schedule.epoch_id,
        target_local_us: schedule.local_target_us(sample_index, state.offset_us),
        wait_us,
    });
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

mod jitter;
mod nack;
#[cfg(test)]
mod pairing_tests;
#[cfg(test)]
mod playout_tests;
mod sink_watchdog;
