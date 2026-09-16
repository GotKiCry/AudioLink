//! §4 控制帧载荷（ALP/2 控制面，postcard 编码）
//!
//! 规格：`docs/03-protocol.md` §4 / §4.1（命令表）、§5（握手时序）、§10（遥测字段）。
//!
//! # 为什么要在这里定义，而不是在 `audiolink-proto`
//!
//! `audiolink-proto` 是 **L1 严格解码层**：它只认「8 B 信封 + `request_id` + 不透明载荷」，不解释载荷语义。
//! 「哪个命令码配哪个载荷结构体」属于 **L2 分发层**，而 L2 是 `audiolink-engine` 的职责
//! （见 `audiolink-proto/src/lib.rs` 的模块文档）。因此这些结构体住在这里。
//!
//! # 冻结纪律
//!
//! 这些结构体的**字段顺序就是 postcard 的 wire 顺序**（`docs/03-protocol.md` §13 版本策略）。
//! 任何增删改字段都会让新旧版本互相解不出帧 —— 改之前先看 §13，并同步改 `docs/11-m1-contract.md`。
//!
//! 载荷只在**握手 / 控制路径**上编解码（1 Hz 遥测、一次性握手），不在音频实时路径上，
//! 因此这里允许分配（`Vec` / `String`）—— 实时纪律约束的是音频线程，不是控制面。

use audiolink_proto::payload_encode;
use audiolink_types::{AudioLinkError, ClockQuality, NodeInfo, OpCode, StreamStats};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// 枚举字段（§4.1 各命令的取值域）
// ---------------------------------------------------------------------------

/// 采集源种类（`OPEN_STREAM.source_desc`）。
///
/// §4.1 只说「采集源描述」，这里把它冻结成显式枚举：**未知值必须可忽略**（§13），
/// 所以用一个 `Other(u8)` 兜底而不是靠 `from_u8` 返回 `None` 把整帧判非法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    /// 系统回环（Windows：WASAPI loopback 抓扬声器混音）。
    SystemLoopback,
    /// 麦克风。
    Microphone,
    /// Android 内录（`AudioPlaybackCapture`，API 29+）。
    PlaybackCapture,
    /// 未来版本引入的其它采集源 —— **保留原值**，不因此拒绝整帧。
    Other(u8),
}

impl SourceKind {
    /// 线上数值形式。
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::SystemLoopback => 0,
            Self::Microphone => 1,
            Self::PlaybackCapture => 2,
            Self::Other(raw) => raw,
        }
    }
}

/// 编解码偏好 / 协商结果（`OPEN_STREAM.codec_prefs[]`、`OPEN_STREAM_ACK.codec_chosen`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodecPref {
    /// Opus（v1 默认档）。
    Opus {
        /// 帧长（ms）：10 / 20 / 40 / 60。
        frame_ms: u8,
        /// 目标码率（kbps）。
        bitrate_kbps: u16,
        /// 是否启用 in-band FEC（音乐场景 **关闭**，见 `docs/04-tech-stack.md` ADR-003）。
        fec: bool,
        /// 是否 VBR。
        vbr: bool,
    },
    /// PCM16 无损档（1.536 Mbps）。
    Pcm16,
}

// ---------------------------------------------------------------------------
// §5 握手与配对
// ---------------------------------------------------------------------------

/// `HELLO`（`0x01`，发起方 →）：`proto_version, node_info, nonce`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloPayload {
    /// 对端应等于 [`audiolink_types::PROTO_VERSION`]，否则回 `1001 VERSION_MISMATCH`。
    pub proto_version: u16,
    /// 发起方节点描述。
    pub node: NodeInfo,
    /// 本连接的随机数（16 B），参与 §5 的认证绑定，防重放。
    pub nonce: Vec<u8>,
}

/// `HELLO_ACK`（`0x02`，响应方 →）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloAckPayload {
    /// 响应方协议版本。
    pub proto_version: u16,
    /// 响应方节点描述。
    pub node: NodeInfo,
    /// 是否接受本次连接（版本不兼容 / 忙碌时为 `false`）。
    pub accepted: bool,
    /// 拒绝原因（`accepted == false` 时必须有内容，UI 直接展示）。
    pub reason: String,
}

/// `AUTH_CHALLENGE`（`0x03`，双方）：`nonce(32 B)`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthChallengePayload {
    /// 32 B 挑战随机数。
    pub nonce: Vec<u8>,
}

/// `AUTH_RESPONSE`（`0x04`，双方）：`ECDSA(privkey, nonce ‖ fp_pair)`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResponsePayload {
    /// DER 编码的 ECDSA-P256 签名。
    pub signature: Vec<u8>,
}

/// `PAIR_REQUIRED`（`0x05`，接收方 →）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairRequiredPayload {
    /// `true` 时接收端屏幕显示 6 位码（桌面端为窗口提示，Android 为通知/界面）。
    pub pin_display: bool,
}

/// `PAIR_SUBMIT`（`0x06`，发起方 →）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairSubmitPayload {
    /// 6 位数字 PIN（经加密通道提交，防窃听）。
    pub pin: String,
}

/// `PAIR_RESULT`（`0x07`，接收方 →）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairResultPayload {
    /// 是否通过。
    pub ok: bool,
    /// 失败原因（如「PIN 错误，剩余 3 次」）。
    pub reason: String,
    /// 是否已写入双方白名单。
    pub persist: bool,
}

// ---------------------------------------------------------------------------
// §4.1 会话与流控制
// ---------------------------------------------------------------------------

/// `OPEN_STREAM`（`0x10`，发送方 →）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenStreamPayload {
    /// 发送方分配的会话 ID（ACK 回填同一值）。
    pub session_id: u32,
    /// 采集源种类。
    pub source: SourceKind,
    /// 编解码偏好，**按优先级降序**；接收端选第一个自己能做的。
    pub codec_prefs: Vec<CodecPref>,
    /// 目标码率（kbps）。
    pub target_rate_kbps: u32,
    /// 声道数（1 / 2）。
    pub channels: u8,
    /// 所属临时同步组（M3）；M1 恒为 `None`。
    pub group: Option<u32>,
}

/// `OPEN_STREAM_ACK`（`0x11`，接收方 →）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenStreamAckPayload {
    /// 回填 `OPEN_STREAM.session_id`。
    pub session_id: u32,
    /// 接收方分配的流 ID（数据报头里的 `stream_id`）。
    pub stream_id: u32,
    /// 协商结果。
    pub codec_chosen: CodecPref,
    /// 会话基准标识（会话重建后换新值）。
    pub epoch_id: u64,
    /// epoch 在**接收端**单调时钟上的对应值（µs）—— 预约播放的基准（§7）。
    pub epoch_local_us: u64,
}

/// `CLOSE_STREAM`（`0x12`，双方）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseStreamPayload {
    /// 流 ID。
    pub stream_id: u32,
    /// 关闭原因。
    pub reason: String,
}

/// `GROUP_EPOCH`（`0x43`，发送方 →）：同步组的公共时间基准与预约提前量（§7）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupEpochPayload {
    /// 组基准标识（会话重建后换新值）。
    pub epoch_id: u64,
    /// epoch 在**发送端**单调时钟上的对应时刻（µs）—— 接收端用时钟偏移换算到本机轴。
    pub epoch_local_us: u64,
    /// 预约提前量（ms）：接收端据此给排播留余量。
    pub lead_ms: u32,
}

/// `SET_GAIN`（`0x20`，双方）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SetGainPayload {
    /// 流 ID；`u32::MAX` = 全部流（§4.1 的 `ALL`）。
    pub stream_id: u32,
    /// 增益（0.0–2.0）。
    pub gain: f32,
    /// 渐变时长（ms），避免硬切产生爆音。
    pub ramp_ms: u16,
}

/// `SET_MUTE`（`0x21`，双方）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetMutePayload {
    /// 流 ID；`u32::MAX` = 全部流。
    pub stream_id: u32,
    /// 是否静音。
    pub mute: bool,
}

/// `CLOCK_RESULT`（`0x30`，发送方 →）：用于对端诊断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockResultPayload {
    /// 相对本机的时钟偏移（µs）。
    pub offset_us: i64,
    /// 漂移（ppm）。
    pub drift_ppm: i32,
    /// 质量分级。
    pub quality: ClockQuality,
}

// ---------------------------------------------------------------------------
// §4.1 诊断与关闭
// ---------------------------------------------------------------------------

/// `PING` / `PONG`（`0x60` / `0x61`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PingPayload {
    /// 发起时刻（发送方单调时钟，µs）。
    pub t1: u64,
    /// 应答时刻（接收方单调时钟，µs）。
    pub t2: u64,
}

/// `ERROR`（`0x70`，双方）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorPayload {
    /// §11 错误码。
    pub code: u16,
    /// 人类可读说明。
    pub message: String,
    /// 可选上下文（`AudioLinkError::context`）。
    pub context: Option<String>,
}

/// `BYE`（`0x80`，双方）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByePayload {
    /// 断开原因。
    pub reason: String,
}

// ---------------------------------------------------------------------------
// 编解码辅助
// ---------------------------------------------------------------------------

/// `SET_GAIN` / `SET_MUTE` 的 `stream_id` 通配值（§4.1 的 `ALL`）。
pub const STREAM_ID_ALL: u32 = u32::MAX;

/// 把载荷序列化为 postcard 字节（§4）。
///
/// 直接转发 `audiolink-proto` 的实现：**编码器只有一份**，避免两处各写一个「差不多」的版本。
pub fn encode_payload<T: Serialize>(value: &T) -> Result<Vec<u8>, AudioLinkError> {
    payload_encode(value)
}

/// 从 postcard 字节解码载荷（**必须恰好消费全部字节**，§4 的 L1 规则）。
///
/// 复用 `audiolink_proto::payload_decode`：它已实现「尾随字节 = 非法」这条硬约束，
/// 保证两端 schema 漂移不会被静默接受。
pub fn decode_payload<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, AudioLinkError> {
    audiolink_proto::payload_decode(bytes)
}

/// 遥测载荷就是 `StreamStats` 本身（§10），这里给个别名让 §4.1 的调用点读起来更顺。
pub type StreamStatsPayload = StreamStats;

/// `STREAM_STATS` 对应的命令码（`0x13`），避免调用点写魔法数。
pub const OP_STREAM_STATS: OpCode = OpCode::StreamStats;
