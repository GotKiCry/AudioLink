//! audiolink-types —— 协议常量、枚举、错误码、遥测结构
//!
//! 规格：`docs/03-protocol.md`（契约）、`docs/02-architecture.md`
//!
//! 本 crate **默认零依赖**；`serde` 是可选依赖（feature `serde`），仅由 `audiolink-proto` 开启，
//! 用于控制帧载荷（§4）与遥测结构（§10）的 postcard 编解码。
//! 纪律：本 crate 不 panic（workspace lint deny `unwrap_used` / `expect_used` / `panic`）。

#![deny(unsafe_code)] // 必须使用 unsafe 的 crate（如 FFI 绑定）在文件顶部显式 #[allow] 并注明理由
#![deny(clippy::unwrap_used, clippy::expect_used)] // 实时路径禁止 panic；确需处用 #[allow] 并注明理由
#![deny(clippy::panic)]

use std::borrow::Cow;
use std::fmt;

// ---------------------------------------------------------------------------
// 协议版本、端口与长度上限（docs/03-protocol.md §1、§3、§4、§13）
// ---------------------------------------------------------------------------

/// ALP 主版本（音频数据报 / 控制帧头部的 `version` 字节，§3）。
pub const PROTO_MAJOR: u8 = 0x02;

/// ALP 小版本（`HELLO` 协商与 mDNS `proto` 字段携带，§13）。
pub const PROTO_MINOR: u8 = 0x01;

/// 完整协议版本：高字节主版本 + 低字节小版本（§13：`0x0201`）。
pub const PROTO_VERSION: u16 = 0x0201;

/// QUIC 监听端口（控制流 #0 与音频数据报共用，§1）。
pub const DEFAULT_QUIC_PORT: u16 = 58290;

/// UDP 广播兜底发现端口（§1、§9.2）。
pub const DEFAULT_DISCOVERY_PORT: u16 = 58280;

/// mDNS 标准端口（§1）。
pub const MDNS_PORT: u16 = 5353;

/// mDNS / DNS-SD 服务类型（§9.1）。
pub const MDNS_SERVICE_TYPE: &str = "_audiolink._udp.local.";

/// 发现协议版本（广播头部 `ver` 与 TXT / JSON 的 `v` 字段，§9）。
pub const DISCOVERY_VERSION: u8 = 1;

/// 音频数据报帧头长度（§3）：`ver`(1) + `ptype`(1) + `flags`(2) + `stream_id`(4)
/// + `seq`(4) + `sample_index`(4) + `epoch_id`(8)。
pub const DATAGRAM_HEADER_LEN: usize = 24;

/// 单个音频数据报总长上限（§3 MTU 约束，避免 QUIC 分片）。
pub const DATAGRAM_MAX_LEN: usize = 1200;

/// 单个音频数据报的载荷上限（§3 载荷表：`DATAGRAM_MAX_LEN` − 帧头）。
pub const DATAGRAM_MAX_PAYLOAD: usize = DATAGRAM_MAX_LEN - DATAGRAM_HEADER_LEN;

/// 控制帧固定帧头长度（§4）：`ver`(1) + `type`(1) + `flags`(2) + `payload_len`(4)。
pub const CONTROL_HEADER_LEN: usize = 8;

/// 控制帧载荷上限（§4：单帧 64 KiB）。
pub const CONTROL_MAX_PAYLOAD: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// ptype（docs/03-protocol.md §3）
// ---------------------------------------------------------------------------

/// 音频数据报类型（§3 ptype 表）。
///
/// [`Ptype::from_u8`] 对未知值返回 `None` —— L1 严格解码层据此拒绝（§1.1）；
/// 「忽略未知类型并计数」是 L2 分发层（`audiolink-engine`）的职责。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Ptype {
    /// 音频负载（Opus 帧 / PCM16LE 片）。
    Audio = 0x01,
    /// 冗余包（XOR 组）；组格式冻结于 M2，解码层只当作不透明字节串。
    Fec = 0x02,
    /// 时钟探测请求（载荷恰 12 B）。
    ClockProbe = 0x03,
    /// 时钟探测应答（载荷恰 24 B）。
    ClockReply = 0x04,
    /// 心跳（无载荷；状态只走 `flags`）。
    Keepalive = 0x05,
    /// 请求重传 seq 列表（载荷 4×n B，1 ≤ n ≤ 16）。
    Nack = 0x06,
}

impl Ptype {
    /// 全部已知取值（供遍历测试与解码工具使用）。
    pub const ALL: [Ptype; 6] = [
        Ptype::Audio,
        Ptype::Fec,
        Ptype::ClockProbe,
        Ptype::ClockReply,
        Ptype::Keepalive,
        Ptype::Nack,
    ];

    /// 由线上字节解析；未知值 → `None`（调用方按 §1.1 返回 `1008 BAD_REQUEST`）。
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(Self::Audio),
            0x02 => Some(Self::Fec),
            0x03 => Some(Self::ClockProbe),
            0x04 => Some(Self::ClockReply),
            0x05 => Some(Self::Keepalive),
            0x06 => Some(Self::Nack),
            _ => None,
        }
    }

    /// 线上字节表示。
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// 该类型的载荷长度规则（§3 载荷表）。
    pub const fn payload_len_rule(self) -> PayloadLenRule {
        match self {
            Self::Audio | Self::Fec => PayloadLenRule::Opaque {
                max: DATAGRAM_MAX_PAYLOAD,
            },
            Self::ClockProbe => PayloadLenRule::Exact { len: 12 },
            Self::ClockReply => PayloadLenRule::Exact { len: 24 },
            Self::Keepalive => PayloadLenRule::Exact { len: 0 },
            Self::Nack => PayloadLenRule::U32List {
                min_items: 1,
                max_items: 16,
            },
        }
    }
}

/// ptype 专用载荷的长度规则（§3 载荷表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadLenRule {
    /// 不透明变长字节串：长度须落在 `0..=max`（`AUDIO` / `FEC`）。
    Opaque {
        /// 允许的最大长度（字节）。
        max: usize,
    },
    /// 定长载荷：长度必须**恰好**等于 `len`（`CLOCK_PROBE` / `CLOCK_REPLY` / `KEEPALIVE`）。
    Exact {
        /// 要求的长度（字节）。
        len: usize,
    },
    /// `u32` 列表：元素个数落在 `[min_items, max_items]`，且长度为 4 的倍数（`NACK`）。
    U32List {
        /// 最少元素个数。
        min_items: usize,
        /// 最多元素个数。
        max_items: usize,
    },
}

// ---------------------------------------------------------------------------
// flags（docs/03-protocol.md §3）
// ---------------------------------------------------------------------------

/// v1 已知位掩码（bit 0–4）；其余位是保留位，发送端必须置 0。
pub const FLAGS_KNOWN_MASK: u16 = 0x001F;

/// 保留位掩码（bit 5–15）；L1 层对非 0 一律拒绝（§1.1）。
pub const FLAGS_RESERVED_MASK: u16 = 0xFFE0;

/// 音频数据报 `flags`（§3 flags 位定义）。
///
/// 发送端必须把保留位（bit 5–15）置 0；L1 严格解码层对保留位非 0 返回 `1008 BAD_REQUEST`（§1.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Flags(u16);

impl Flags {
    /// 无任何标志。
    pub const NONE: Self = Self(0);
    /// bit0：本包是 FEC 冗余副本（精确语义冻结于 M2，见 §8.1）。
    pub const FEC_REDUNDANT: Self = Self(1 << 0);
    /// bit1：静音段（负载可能为空）。
    pub const DTX: Self = Self(1 << 1);
    /// bit2：该流使用 10 ms 帧长。
    pub const FRAME_10MS: Self = Self(1 << 2);
    /// bit3：单声道负载。
    pub const MONO: Self = Self(1 << 3);
    /// bit4：标志突发结束（拥塞 / 延迟统计用）。
    pub const LAST_IN_BURST: Self = Self(1 << 4);

    /// 由原始位构造。**不过滤**未知位 —— 保留位是否非 0 由校验方判断（§1.1）。
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// 原始位。
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// 是否包含 `other` 的全部位。
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// 保留位（bit 5–15）是否被置位 —— L1 层据此拒绝（§1.1）。
    pub const fn has_reserved_bits(self) -> bool {
        (self.0 & FLAGS_RESERVED_MASK) != 0
    }
}

// ---------------------------------------------------------------------------
// 控制帧命令码（docs/03-protocol.md §4.1）
// ---------------------------------------------------------------------------

/// 控制帧命令码（§4.1 命令表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum OpCode {
    /// `0x01` 发起方 →：`proto_version, node_info, nonce`。
    Hello = 0x01,
    /// `0x02` 响应方 →：`proto_version, node_info, accepted, reason`。
    HelloAck = 0x02,
    /// `0x03` 双方：`nonce(32B)`。
    AuthChallenge = 0x03,
    /// `0x04` 双方：`ECDSA(privkey, nonce ‖ fp_pair)`。
    AuthResponse = 0x04,
    /// `0x05` 接收方 →：`pin_display`。
    PairRequired = 0x05,
    /// `0x06` 发起方 →：`pin(6 位数字)`。
    PairSubmit = 0x06,
    /// `0x07` 接收方 →：`ok, reason, persist`。
    PairResult = 0x07,
    /// `0x10` 发送方 →：开流协商。
    OpenStream = 0x10,
    /// `0x11` 接收方 →：`stream_id, codec_chosen, epoch_id, epoch_local_us`。
    OpenStreamAck = 0x11,
    /// `0x12` 双方：`stream_id, reason`。
    CloseStream = 0x12,
    /// `0x13` 双方：1 Hz 遥测上报（`StreamStats`，§10）。
    StreamStats = 0x13,
    /// `0x20` 双方：`stream_id | ALL, gain, ramp_ms`。
    SetGain = 0x20,
    /// `0x21` 双方：`stream_id | ALL, mute`。
    SetMute = 0x21,
    /// `0x22` 接收方 →：`max_gain`。
    SetVolumeLock = 0x22,
    /// `0x30` 发送方 →：`offset_us, drift_ppm, quality`。
    ClockResult = 0x30,
    /// `0x40` 发送方 →：临时同步组创建（FR-22）。
    GroupCreate = 0x40,
    /// `0x41` 发送方 →：成员加入。
    GroupJoin = 0x41,
    /// `0x42` 发送方 →：成员退出。
    GroupLeave = 0x42,
    /// `0x43` 发送方 →：`epoch_id, epoch_local_us, lead_ms`。
    GroupEpoch = 0x43,
    /// `0x50` 双方：汇总指标快照。
    TelemetryPush = 0x50,
    /// `0x60` 双方：可靠流版本 ping（`t1, t2`）。
    Ping = 0x60,
    /// `0x61` 双方：`t1, t2`。
    Pong = 0x61,
    /// `0x70` 双方：`code, message, context`。
    Error = 0x70,
    /// `0x80` 双方：`reason`。
    Bye = 0x80,
}

impl OpCode {
    /// 全部已知取值（供遍历测试与解码工具使用）。
    pub const ALL: [OpCode; 24] = [
        OpCode::Hello,
        OpCode::HelloAck,
        OpCode::AuthChallenge,
        OpCode::AuthResponse,
        OpCode::PairRequired,
        OpCode::PairSubmit,
        OpCode::PairResult,
        OpCode::OpenStream,
        OpCode::OpenStreamAck,
        OpCode::CloseStream,
        OpCode::StreamStats,
        OpCode::SetGain,
        OpCode::SetMute,
        OpCode::SetVolumeLock,
        OpCode::ClockResult,
        OpCode::GroupCreate,
        OpCode::GroupJoin,
        OpCode::GroupLeave,
        OpCode::GroupEpoch,
        OpCode::TelemetryPush,
        OpCode::Ping,
        OpCode::Pong,
        OpCode::Error,
        OpCode::Bye,
    ];

    /// 由线上字节解析；未知值 → `None`（L1 层据此拒绝，§1.1）。
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(Self::Hello),
            0x02 => Some(Self::HelloAck),
            0x03 => Some(Self::AuthChallenge),
            0x04 => Some(Self::AuthResponse),
            0x05 => Some(Self::PairRequired),
            0x06 => Some(Self::PairSubmit),
            0x07 => Some(Self::PairResult),
            0x10 => Some(Self::OpenStream),
            0x11 => Some(Self::OpenStreamAck),
            0x12 => Some(Self::CloseStream),
            0x13 => Some(Self::StreamStats),
            0x20 => Some(Self::SetGain),
            0x21 => Some(Self::SetMute),
            0x22 => Some(Self::SetVolumeLock),
            0x30 => Some(Self::ClockResult),
            0x40 => Some(Self::GroupCreate),
            0x41 => Some(Self::GroupJoin),
            0x42 => Some(Self::GroupLeave),
            0x43 => Some(Self::GroupEpoch),
            0x50 => Some(Self::TelemetryPush),
            0x60 => Some(Self::Ping),
            0x61 => Some(Self::Pong),
            0x70 => Some(Self::Error),
            0x80 => Some(Self::Bye),
            _ => None,
        }
    }

    /// 线上字节表示。
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// 人类可读名称（`tools/alp2-dump` 用）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Hello => "HELLO",
            Self::HelloAck => "HELLO_ACK",
            Self::AuthChallenge => "AUTH_CHALLENGE",
            Self::AuthResponse => "AUTH_RESPONSE",
            Self::PairRequired => "PAIR_REQUIRED",
            Self::PairSubmit => "PAIR_SUBMIT",
            Self::PairResult => "PAIR_RESULT",
            Self::OpenStream => "OPEN_STREAM",
            Self::OpenStreamAck => "OPEN_STREAM_ACK",
            Self::CloseStream => "CLOSE_STREAM",
            Self::StreamStats => "STREAM_STATS",
            Self::SetGain => "SET_GAIN",
            Self::SetMute => "SET_MUTE",
            Self::SetVolumeLock => "SET_VOLUME_LOCK",
            Self::ClockResult => "CLOCK_RESULT",
            Self::GroupCreate => "GROUP_CREATE",
            Self::GroupJoin => "GROUP_JOIN",
            Self::GroupLeave => "GROUP_LEAVE",
            Self::GroupEpoch => "GROUP_EPOCH",
            Self::TelemetryPush => "TELEMETRY_PUSH",
            Self::Ping => "PING",
            Self::Pong => "PONG",
            Self::Error => "ERROR",
            Self::Bye => "BYE",
        }
    }
}

// ---------------------------------------------------------------------------
// 错误码与统一错误类型（docs/03-protocol.md §11、docs/02-architecture.md §11）
// ---------------------------------------------------------------------------

/// 协议错误码（§11 错误码表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ErrorCode {
    /// `1001`：协议主版本不兼容 → 提示升级并断开。
    VersionMismatch = 1001,
    /// `1002`：未配对 → 触发配对流程。
    NotPaired = 1002,
    /// `1003`：PIN 错误 / 超时。
    PairRejected = 1003,
    /// `1004`：签名校验失败（可能是中间人）。
    AuthFailed = 1004,
    /// `1005`：对方不支持所需能力。
    CapUnsupported = 1005,
    /// `1006`：混音路数超上限。
    StreamLimit = 1006,
    /// `1007`：无共同编解码。
    CodecUnsupported = 1007,
    /// `1008`：载荷非法（截断 / 超长 / 未知类型 / 保留位 / 长度越界）。
    BadRequest = 1008,
    /// `1009`：正在握手 / 配对中。
    Busy = 1009,
    /// `2001`：播放欠载（统计用途，不致命）。
    PlayoutUnderrun = 2001,
    /// `2002`：播放器重建（自愈路径）。
    SinkRebuild = 2002,
    /// `2003`：采集源失效。
    CaptureLost = 2003,
    /// `3001`：请求过频。
    RateLimited = 3001,
}

impl ErrorCode {
    /// 线上 / FFI 的数值形式。
    pub const fn as_u16(self) -> u16 {
        self as u16
    }

    /// 由数值解析；未知值 → `None`。
    pub const fn from_u16(value: u16) -> Option<Self> {
        match value {
            1001 => Some(Self::VersionMismatch),
            1002 => Some(Self::NotPaired),
            1003 => Some(Self::PairRejected),
            1004 => Some(Self::AuthFailed),
            1005 => Some(Self::CapUnsupported),
            1006 => Some(Self::StreamLimit),
            1007 => Some(Self::CodecUnsupported),
            1008 => Some(Self::BadRequest),
            1009 => Some(Self::Busy),
            2001 => Some(Self::PlayoutUnderrun),
            2002 => Some(Self::SinkRebuild),
            2003 => Some(Self::CaptureLost),
            3001 => Some(Self::RateLimited),
            _ => None,
        }
    }

    /// 人类可读名称（§11 表）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::VersionMismatch => "VERSION_MISMATCH",
            Self::NotPaired => "NOT_PAIRED",
            Self::PairRejected => "PAIR_REJECTED",
            Self::AuthFailed => "AUTH_FAILED",
            Self::CapUnsupported => "CAP_UNSUPPORTED",
            Self::StreamLimit => "STREAM_LIMIT",
            Self::CodecUnsupported => "CODEC_UNSUPPORTED",
            Self::BadRequest => "BAD_REQUEST",
            Self::Busy => "BUSY",
            Self::PlayoutUnderrun => "PLAYOUT_UNDERRUN",
            Self::SinkRebuild => "SINK_REBUILD",
            Self::CaptureLost => "CAPTURE_LOST",
            Self::RateLimited => "RATE_LIMITED",
        }
    }
}

/// 统一错误类型（`docs/02-architecture.md` §11）：所有错误收敛于此，带「错误码 + 上下文」，跨 FFI 可直接映射。
///
/// M0-02 只落地 L1 解码层需要的 [`AudioLinkError::BadRequest`]（§1.1）；
/// 握手 / 配对 / 采集等变体随 M1 各模块实现补入（新增变体是源码兼容变更）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioLinkError {
    /// `1008 BAD_REQUEST`：字节流非法（截断 / 超长 / 未知 ptype / 保留位非 0 / 长度越界 …）。
    ///
    /// 这是 L1 严格解码层（`audiolink-proto`）的**唯一拒绝出口**：调用方不得把它升级为断连（§1.1）。
    BadRequest {
        /// 失败细节（静态字面量，或错误路径上格式化的动态串）。
        context: Cow<'static, str>,
    },
}

impl AudioLinkError {
    /// 静态上下文的 `BadRequest`：`const` 友好，零分配。
    pub const fn bad_request(context: &'static str) -> Self {
        Self::BadRequest {
            context: Cow::Borrowed(context),
        }
    }

    /// 动态上下文的 `BadRequest`（仅错误路径使用，允许分配）。
    pub fn bad_request_owned(context: String) -> Self {
        Self::BadRequest {
            context: Cow::Owned(context),
        }
    }

    /// 错误码（跨 FFI 用 [`ErrorCode::as_u16`] 取值）。
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::BadRequest { .. } => ErrorCode::BadRequest,
        }
    }

    /// 失败细节。
    pub fn context(&self) -> &str {
        match self {
            Self::BadRequest { context } => context.as_ref(),
        }
    }
}

impl fmt::Display for AudioLinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}: {}",
            self.code().as_u16(),
            self.code().name(),
            self.context()
        )
    }
}

impl std::error::Error for AudioLinkError {}

// ---------------------------------------------------------------------------
// 节点平台与能力（docs/03-protocol.md §9）
// ---------------------------------------------------------------------------

/// 节点平台（§9.1 / §9.2 的 `platform` 字段）。
///
/// [`Platform::Unknown`] 承载 0 / 1 之外的未来取值，落实「未知值必须可忽略」的演进规则（§13）；
/// 文本载体（mDNS TXT / 广播 JSON）只接受 `win` / `android`，见 [`Platform::from_str_exact`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    /// Windows 桌面端（文本形式 `win`）。
    Windows,
    /// Android 移动端（文本形式 `android`）。
    Android,
    /// 未知平台（未来版本；构造时应避免 0 / 1 —— 那两个值由 [`Platform::from_u8`] 映射到已知平台）。
    Unknown(u8),
}

impl Platform {
    /// 二进制载体的编码值（文本载体见 [`Platform::as_str`]）。
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Windows => 0,
            Self::Android => 1,
            Self::Unknown(value) => value,
        }
    }

    /// 由二进制编码值解析；`0` / `1` 之外一律记作 [`Platform::Unknown`]（**绝不丢弃**节点）。
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Windows,
            1 => Self::Android,
            other => Self::Unknown(other),
        }
    }

    /// 文本载体的字符串形式（§9.2）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "win",
            Self::Android => "android",
            Self::Unknown(_) => "unknown",
        }
    }

    /// 解析文本载体的 `platform`；只接受 `win` / `android`（§9.2：其它值丢弃该报文）。
    pub fn from_str_exact(value: &str) -> Option<Self> {
        match value {
            "win" => Some(Self::Windows),
            "android" => Some(Self::Android),
            _ => None,
        }
    }
}

/// 能力位图（§9.1 / §9.2 的 `caps` 字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Caps(u16);

impl Caps {
    /// 无能力。
    pub const NONE: Self = Self(0);
    /// bit0：可发送。
    pub const CAN_SEND: Self = Self(1 << 0);
    /// bit1：可接收。
    pub const CAN_RECEIVE: Self = Self(1 << 1);
    /// bit2：支持内录（`AudioPlaybackCapture`）。
    pub const CAN_LOOPBACK: Self = Self(1 << 2);
    /// bit3：支持混音。
    pub const CAN_MIX: Self = Self(1 << 3);
    /// v1 已知位掩码（bit 0–3）。
    pub const KNOWN_MASK: u16 = 0x000F;

    /// 由原始位构造。
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// 原始位。
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// 是否包含 `other` 的全部位。
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// 文本载体的 hex 形式：**无前缀小写**（§9.2），如 `"3"`。
    pub fn to_hex(self) -> String {
        format!("{:x}", self.0)
    }

    /// 解析文本载体的 hex（大小写均可，长度 1–4）。
    pub fn from_hex(text: &str) -> Option<Self> {
        if text.is_empty() || text.len() > 4 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u16::from_str_radix(text, 16).ok().map(Self)
    }
}

// ---------------------------------------------------------------------------
// 遥测结构（docs/03-protocol.md §10）
// ---------------------------------------------------------------------------

/// 编码参数快照（§10）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CodecStats {
    /// 帧长（ms）：10 / 20 / 40 / 60。
    pub frame_ms: u8,
    /// 声道数：1 / 2。
    pub channels: u8,
    /// Opus 复杂度：0..=10。
    pub complexity: u8,
}

/// 每会话遥测快照（§10）：控制帧 `STREAM_STATS` 的 postcard 载荷，1 Hz 双方互发。
///
/// **字段顺序即 wire 顺序**，增删改字段都会破坏兼容（§13）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StreamStats {
    /// 流 ID。
    pub stream_id: u32,
    /// 平滑 RTT。
    pub rtt_us: u32,
    /// 到达间隔抖动 P50。
    pub jitter_us: u32,
    /// 到达间隔抖动 P95。
    pub jitter_p95_us: u32,
    /// 丢包率（百分比 ×100，避免浮点）。
    pub loss_pct_x100: u16,
    /// 实际编码码率。
    pub bitrate_bps: u32,
    /// 编码参数。
    pub codec: CodecStats,
    /// 相对发送端的时钟偏移。
    pub clock_offset_us: i32,
    /// 时钟漂移（ppm）。
    pub drift_ppm: i32,
    /// 播放环水位。
    pub buffer_level_us: u32,
    /// 累计欠载次数。
    pub underruns: u32,
    /// 丢包隐藏次数。
    pub plc_count: u32,
    /// 重传请求次数。
    pub nack_count: u32,
    /// 估算端到端延迟（验收主指标）。
    pub e2e_latency_us: u32,
    /// 迟到丢弃包数。
    pub late_drops: u32,
}
