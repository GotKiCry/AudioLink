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
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, AtomicUsize, Ordering};
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
        }
    }

    /// 声明本端能力（见 [`EngineConfig::capabilities`]）。
    #[must_use]
    pub const fn with_capabilities(mut self, capabilities: u32) -> Self {
        self.capabilities = capabilities;
        self
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
    /// §13 能力协商结果；`None` = 还没走完能力交换。
    pub capabilities: Option<PeerCapabilities>,
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
    /// §13 能力协商结果（`None` = 还没协商完）。
    capabilities: Mutex<Option<PeerCapabilities>>,
}

#[derive(Default)]
struct PairingState {
    displayed: Option<(String, Instant)>,
    needs_pin: bool,
}

impl PeerSession {
    /// M4 观测：记下「最近一帧的编号 ↔ 到达时刻」。
    ///
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
    /// 注册任务与关闭入口共用此锁，停止后不能再生成漏出清理范围的任务。
    tasks: Mutex<TaskTracker>,
    listen_addr: std::net::SocketAddr,
    /// §7 临时同步组的账本（发送侧写、查询用）：组 ID → 基准与成员。
    groups: Mutex<HashMap<u32, GroupState>>,
    /// M3 多会话：引擎级共享采集枢纽（第一个会话启流时创建，最后一个停止时摘掉）。
    capture_hub: Mutex<Option<Arc<CaptureHub>>>,
    /// M4 汇聚：引擎级混音器（第一个接收会话创建 sink，后续会话把自己的帧混进来）。
    playout_mixer: Mutex<Option<Arc<Mutex<PcmMixer>>>>,
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
                groups: Mutex::new(HashMap::new()),
                capture_hub: Mutex::new(None),
                playout_mixer: Mutex::new(None),
                events,
                shutdown: AtomicBool::new(false),
                tasks: Mutex::new(TaskTracker::new()),
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
        let (epoch_id, lead_ms, count) = {
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
            (state.epoch.epoch_id, state.lead_ms, count)
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
                epoch_local_us: now_monotonic_us(),
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
    pub fn mixer_stats(&self) -> Option<audiolink_audio::mixer::MixSnapshot> {
        let slot = self.inner.playout_mixer.lock().ok()?;
        let mixer = slot.as_ref()?;
        let guard = mixer.lock().ok()?;
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
                        let result = match playback.as_ref() {
                            Some(handle) => {
                                let offset_us = clock_estimate_of(&session)
                                    .map(|estimate| estimate.offset_us)
                                    .unwrap_or(0);
                                let frame_samples = u32::try_from(frame_ms).unwrap_or(20) * 48;
                                let _ = handle.set_schedule(schedule, offset_us, frame_samples);
                                Ok(())
                            }
                            None => Err(AudioLinkError::cap_unsupported(
                                "this node has no playout sink to schedule",
                            )),
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
                    &jitter_depth,
                ).await;
            }

            datagram = connection.read_datagram_into(&mut rx_buf) => {
                match datagram {
                    Ok(len) => {
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
                        report_peer_gone(&inner, &session, &net_error(&error));
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
        capabilities: Mutex::new(None),
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
async fn stop_playout(handle: &mut Option<PlayoutHandle>) {
    if let Some(handle) = handle.take() {
        handle.stop.store(true, Ordering::Relaxed);
        // M4：把这一路从混音器里摘掉 —— 停了就不该再占路数（否则重连几次就会撞上 8 路上限）。
        if let (Some(mixer), Some(source)) = (handle.mixer.as_ref(), handle.mix_source)
            && let Ok(mut guard) = mixer.lock()
        {
            guard.remove_source(source);
        }
        drop(handle.frames);
        join_audio_thread(handle.join).await;
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
            if let Some(handle) = playback.as_ref() {
                let offset_us = clock_estimate_of(session)
                    .map(|estimate| estimate.offset_us)
                    .unwrap_or(0);
                let frame_samples = codec.frame_ms.max(1) * 48;
                let schedule =
                    EpochSchedule::new(payload.epoch_id, payload.epoch_local_us, payload.lead_ms);
                let _ = handle.set_schedule(Some(schedule), offset_us, frame_samples);
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
            // §7：发送端指定组基准 → 接收侧据此排播（只有配了播放输出才真正生效）。
            if let Some(handle) = playback.as_ref() {
                let offset_us = clock_estimate_of(session)
                    .map(|estimate| estimate.offset_us)
                    .unwrap_or(0);
                let frame_samples = codec.frame_ms.max(1) * 48;
                let schedule =
                    EpochSchedule::new(payload.epoch_id, payload.epoch_local_us, payload.lead_ms);
                let enabled = handle.set_schedule(Some(schedule), offset_us, frame_samples);
                tracing::info!(
                    epoch_id = payload.epoch_id,
                    lead_ms = payload.lead_ms,
                    offset_us,
                    enabled,
                    "§7 GROUP_EPOCH：接收侧排播已更新"
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
/// 引擎级混音器句柄（M4 汇聚）。
type PlayoutMix = Arc<Mutex<PcmMixer>>;

/// 混音源号的全局计数器（混音器按 id 认路，不能让不同会话撞号）。
static NEXT_MIX_SOURCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// 混音路数超过 FR-12 上限时的错误上下文。
const MIXER_FULL: &str = "mixer is full (FR-12 allows 8 sources)";

/// 取得（必要时创建）引擎级混音器，并给本会话分配一个源号。
///
/// 返回 (混音器, 源号, 是否是 owner)：只有 owner 会真的打开播放设备，
/// 其余会话把解码后的 PCM 混进来 —— 这就是 M4 的「同一接收端多路混音」。
/// 路数超过 FR-12 的上限时明确报 STREAM_LIMIT，而不是悄悄丢掉一路。
fn acquire_playout_mixer(
    inner: &Arc<Inner>,
    codec: &CodecConfig,
) -> Result<(Option<PlayoutMix>, Option<u32>, bool), AudioLinkError> {
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
    match slot.as_ref() {
        Some(mixer) => {
            let mut guard = mixer.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .add_source(source)
                .map_err(|_| AudioLinkError::stream_limit(MIXER_FULL))?;
            Ok((Some(Arc::clone(mixer)), Some(source), false))
        }
        None => {
            let mixer = Arc::new(Mutex::new(PcmMixer::new(
                format,
                codec.frame_samples(),
                PLAYBACK_QUEUE_FRAMES,
            )));
            {
                let mut guard = mixer.lock().unwrap_or_else(|e| e.into_inner());
                guard
                    .add_source(source)
                    .map_err(|_| AudioLinkError::stream_limit(MIXER_FULL))?;
            }
            *slot = Some(Arc::clone(&mixer));
            Ok((Some(mixer), Some(source), true))
        }
    }
}

/// 写出一帧。
///
/// 没接混音器时就是直通 sink（M1 起的行为）；接上之后本路先把样本混进去，
/// 只有 owner（真正持有设备的那一路）才把混音结果写出去 —— 于是多路只开一次设备。
fn write_frame(
    sink: &mut Option<Box<dyn PlayoutSink>>,
    mixer: &Option<Arc<Mutex<PcmMixer>>>,
    mix_source: Option<u32>,
    samples: &[f32],
    mixed: &mut Vec<f32>,
) -> bool {
    let (Some(mixer), Some(source)) = (mixer.as_ref(), mix_source) else {
        return sink.as_mut().is_none_or(|open| open.write(samples).is_ok());
    };
    let Ok(mut guard) = mixer.lock() else {
        return false;
    };
    if guard.push(source, samples).is_err() {
        return false;
    }
    let Some(open) = sink.as_mut() else {
        return true; // 非 owner：混进去就完事
    };
    guard.mix_frame(mixed);
    open.write(mixed).is_ok()
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

    // M4 汇聚：混音器是**引擎级**的。第一个接收会话是 owner（真正打开 sink），
    // 后续会话把自己的帧混进来 —— 同一台设备只被打开一次，多路在软件侧求和 + 软限幅。
    let (mixer, mix_source, is_owner) = acquire_playout_mixer(inner, codec)?;
    let thread_mixer = mixer.clone();
    // 只有 owner 会真的建 sink；其余会话只把帧混进去。
    let factory = if is_owner {
        Some(Arc::clone(factory))
    } else {
        None
    };

    let join = std::thread::Builder::new()
        .name("audiolink-playout".to_string())
        .spawn(move || {
            playout_main(
                factory.as_ref().map(|factory| factory.as_ref()),
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
            mixer,
            mix_source,
        }),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(AudioLinkError::bad_request(
            "playout sink did not report readiness in time",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn playout_main(
    factory: Option<&(dyn Fn() -> Result<Box<dyn PlayoutSink>, AudioError> + Send + Sync)>,
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
    mix_source: Option<u32>,
    frames: Receiver<PlaybackFrame>,
    ready_tx: std::sync::mpsc::Sender<Result<(), AudioLinkError>>,
) {
    let mut sink: Option<Box<dyn PlayoutSink>> = match factory {
        Some(factory) => match factory() {
            Ok(sink) => Some(sink),
            Err(error) => {
                let _ = ready_tx.send(Err(audio_error(&error)));
                return;
            }
        },
        // 非 owner：本路不建 sink，样本只混进 owner 的输出。
        None => None,
    };

    // 「零重采样」硬闸（NFR-13）：播放侧同样不得静默 SRC（只有真的打开设备的那一路才需要查）。
    if let Some(open) = sink.as_ref()
        && let Err(error) =
            require_unified_format("playout", open.backend_name(), open.device_format())
    {
        let _ = ready_tx.send(Err(error));
        return;
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

        // 起步攒帧：不足当前目标深度就继续等。这一拍**不补静音也不算欠载** ——
        // 还没开始播，谈不上「欠」；把攒帧期算成欠载会让欠载率失去意义。
        if !primed {
            let target = jitter_depth
                .load(Ordering::Relaxed)
                .clamp(MIN_TARGET_FRAMES, MAX_TARGET_FRAMES);
            if frames.len() >= target {
                depth_state = PlayoutDepthState::new(target);
                primed = true;
                // §7 起播对齐：把**拍点锚到全局帧网格**（`now_monotonic_us()` 的 frame_us 整数倍）。
                //
                // 为什么必须锚（2026-09-17 定位到的真缺陷）：拍点就是排播判定的时刻，而目标时刻
                // 也对齐到同一网格。两端线程各自从「启动那一刻」起拍，相位互不相同 —— 一个拍点落在
                // 目标之前、另一个落在之后，于是一头等一拍、一头立刻播，**起播整整差一帧**
                // （回环实测反复出现 0 或 ~20 ms 两种结果，扣掉这一帧后抖动 < 1 ms）。
                // 锚到同一网格后，两端在**同一拍**上做同一个决定。
                let frame_us = frame_ms.max(1) * 1_000;
                let remainder = now_monotonic_us() % frame_us;
                next_write = Instant::now() + Duration::from_micros(frame_us - remainder);
                // 这一拍只用来对齐相位，不写数据；下一拍起就在网格上，与对端一致。
                continue;
            } else {
                continue;
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
        if let Some((action, sample_index)) =
            schedule_action(&sync, peek_frame(&frames, &mut pending))
        {
            match action {
                PlayoutAction::Wait { wait_us } => {
                    report_playout_scheduled(
                        &events,
                        &sync,
                        sample_index,
                        wait_us,
                        &mut scheduled_reported,
                    );
                    if !write_frame(&mut sink, &mixer, mix_source, &silence, &mut mixed) {
                        break;
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
                    if !write_frame(&mut sink, &mixer, mix_source, &silence, &mut mixed) {
                        break;
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

        match depth_state.action(requested_target, buffered_frames) {
            PlayoutDepthAction::Hold => {
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.record_underrun();
                }
                if !write_frame(&mut sink, &mixer, mix_source, &silence, &mut mixed) {
                    break;
                }
                continue;
            }
            PlayoutDepthAction::DropOldest(count) => {
                for _ in 0..count {
                    match take_due_frame(&frames, &mut pending, &mut expected_seq, &telemetry) {
                        DueFrame::Ready(_) | DueFrame::Missing => {
                            if let Ok(mut telemetry) = telemetry.lock() {
                                telemetry.record_late_drop();
                            }
                        }
                        DueFrame::Disconnected => break 'playout,
                    }
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
                if !write_frame(&mut sink, &mixer, mix_source, &frame.samples, &mut mixed) {
                    break;
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
                let _ = jitter_depth.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |depth| {
                    Some((depth + 1).min(MAX_TARGET_FRAMES))
                });
                refill_after_underrun = true;
                if !write_frame(&mut sink, &mixer, mix_source, &silence, &mut mixed) {
                    break;
                }
            }
            DueFrame::Disconnected => break,
        }
    }

    if let Some(open) = sink.as_mut() {
        open.stop();
    }
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
        schedule.action(now_us, sample_index, state.offset_us, state.frame_samples),
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
