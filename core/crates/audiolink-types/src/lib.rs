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

/// 线上基础类型宽度（§1：多字节字段一律小端）。
///
/// 这些常量存在的唯一目的，是让「定长载荷长度」能写成**字段宽度之和**，而不是散落的字面量 ——
/// 「改了字段宽度、忘了改 LEN」这类缺陷由此从源头消失。§3 载荷表的独立副本在
/// `audiolink-proto/tests/payload_len_table.rs` 里再把它钉一遍（护栏测试）。
pub const U8_LEN: usize = 1;

/// 2 B 小端字段宽度。
pub const U16_LEN: usize = 2;

/// 4 B 小端字段宽度（`u32` / `i32`）。
pub const U32_LEN: usize = 4;

/// 8 B 小端字段宽度（`u64`）。
pub const U64_LEN: usize = 8;

/// 8 B 小端字段宽度（`i64`；与 [`U64_LEN`] 同宽，分开命名只为让载荷表自解释）。
pub const I64_LEN: usize = 8;

/// 音频数据报帧头长度（§3）：`ver`(1) + `ptype`(1) + `flags`(2) + `stream_id`(4)
/// + `seq`(4) + `sample_index`(4) + `epoch_id`(8) = **24 B**。
pub const DATAGRAM_HEADER_LEN: usize =
    U8_LEN + U8_LEN + U16_LEN + U32_LEN + U32_LEN + U32_LEN + U64_LEN;

/// 单个音频数据报总长上限（§3 MTU 约束，避免 QUIC 分片）。
pub const DATAGRAM_MAX_LEN: usize = 1200;

/// 单个音频数据报的载荷上限（§3 载荷表：`DATAGRAM_MAX_LEN` − 帧头）。
pub const DATAGRAM_MAX_PAYLOAD: usize = DATAGRAM_MAX_LEN - DATAGRAM_HEADER_LEN;

/// 控制帧固定帧头长度（§4）：`ver`(1) + `type`(1) + `flags`(2) + `payload_len`(4) = **8 B**。
///
/// 其后的 `request_id`(4) 是 §4 的另一段固定前缀，见 `audiolink-proto::control::MIN_FRAME_LEN`。
pub const CONTROL_HEADER_LEN: usize = U8_LEN + U8_LEN + U16_LEN + U32_LEN;

/// 控制帧载荷上限（§4：单帧 64 KiB）。
pub const CONTROL_MAX_PAYLOAD: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// ptype 专用定长载荷的长度（docs/03-protocol.md §3 载荷表）
//
// 一律写成「字段宽度之和」：长度常量、`Ptype::payload_len_rule()` 与各类型的 `LEN`
// 三处引用同一份算式，剩下的唯一错法是「算式本身与 §3 表不符」—— 那由护栏测试
// `audiolink-proto/tests/payload_len_table.rs` 的独立副本负责抓。
// ---------------------------------------------------------------------------

/// `CLOCK_PROBE` 载荷内 `probe_seq` 偏移（§3）= 0。
pub const CLOCK_PROBE_PROBE_SEQ_OFFSET: usize = 0;

/// `CLOCK_PROBE` 载荷内 `t1` 偏移（§3）= `probe_seq` 之后。
pub const CLOCK_PROBE_T1_OFFSET: usize = CLOCK_PROBE_PROBE_SEQ_OFFSET + U32_LEN;

/// `CLOCK_PROBE` 载荷长度（§3）：`probe_seq`(u32) + `t1`(i64) = **12 B**。
///
/// 长度由**最后一个字段的偏移 + 该字段宽度**得出 —— 长度与偏移同源，不存在两份账本。
pub const CLOCK_PROBE_PAYLOAD_LEN: usize = CLOCK_PROBE_T1_OFFSET + I64_LEN;

/// `CLOCK_REPLY` 载荷内 `probe_seq` 偏移（§3）= 0。
pub const CLOCK_REPLY_PROBE_SEQ_OFFSET: usize = 0;

/// `CLOCK_REPLY` 载荷内 `t1` 偏移（§3）。
pub const CLOCK_REPLY_T1_OFFSET: usize = CLOCK_REPLY_PROBE_SEQ_OFFSET + U32_LEN;

/// `CLOCK_REPLY` 载荷内 `t2` 偏移（§3）。
pub const CLOCK_REPLY_T2_OFFSET: usize = CLOCK_REPLY_T1_OFFSET + I64_LEN;

/// `CLOCK_REPLY` 载荷内 `t3` 偏移（§3）。
pub const CLOCK_REPLY_T3_OFFSET: usize = CLOCK_REPLY_T2_OFFSET + I64_LEN;

/// `CLOCK_REPLY` 载荷长度（§3）：`probe_seq`(u32) + `t1`/`t2`/`t3`(i64) = **28 B**。
///
/// 同样由「最后一个字段的偏移 + 该字段宽度」得出。
pub const CLOCK_REPLY_PAYLOAD_LEN: usize = CLOCK_REPLY_T3_OFFSET + I64_LEN;

/// `KEEPALIVE` 载荷长度（§3：无载荷）= **0 B**。
pub const KEEPALIVE_PAYLOAD_LEN: usize = 0;

/// `NACK` 列表单项宽度（§3：`u32` 列表）。
pub const NACK_ITEM_LEN: usize = U32_LEN;

/// `NACK` 列表最少项数（§3：1 ≤ n）。
pub const NACK_MIN_ITEMS: usize = 1;

/// `NACK` 列表最多项数（§3：1 ≤ n ≤ 16）。
pub const NACK_MAX_ITEMS: usize = 16;

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
    /// 时钟探测应答（载荷恰 28 B：`probe_seq(u32)` + `t1/t2/t3(i64)`，见 §3 载荷表）。
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

    /// 人类可读名称（与 `docs/03-protocol.md` §3 表一致；`alp2-dump` 与日志用）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Audio => "AUDIO",
            Self::Fec => "FEC",
            Self::ClockProbe => "CLOCK_PROBE",
            Self::ClockReply => "CLOCK_REPLY",
            Self::Keepalive => "KEEPALIVE",
            Self::Nack => "NACK",
        }
    }

    /// 该类型的载荷长度规则（§3 载荷表）。
    pub const fn payload_len_rule(self) -> PayloadLenRule {
        match self {
            Self::Audio | Self::Fec => PayloadLenRule::Opaque {
                max: DATAGRAM_MAX_PAYLOAD,
            },
            Self::ClockProbe => PayloadLenRule::Exact {
                len: CLOCK_PROBE_PAYLOAD_LEN,
            },
            Self::ClockReply => PayloadLenRule::Exact {
                len: CLOCK_REPLY_PAYLOAD_LEN,
            },
            Self::Keepalive => PayloadLenRule::Exact {
                len: KEEPALIVE_PAYLOAD_LEN,
            },
            Self::Nack => PayloadLenRule::U32List {
                min_items: NACK_MIN_ITEMS,
                max_items: NACK_MAX_ITEMS,
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

/// 按 §11 错误码表批量生成 [`AudioLinkError`]：每个变体 4 个成员（`const` 构造器 / 动态构造器 /
/// `code()` 分支 / `context()` 分支），避免 13 份手写样板漂移。
///
/// 语法：`Variant => snake_name, ErrorCode::Variant, is_statistical;`
macro_rules! define_audio_link_errors {
    ($($(#[$meta:meta])* $variant:ident => $ctor:ident, $code:ident, $statistical:literal;)*) => {
        /// 统一错误类型（`docs/02-architecture.md` §11）：所有错误收敛于此，带「错误码 + 上下文」，
        /// 跨 FFI 可直接映射。
        ///
        /// **构造纪律**：优先用 `const` 构造器（静态上下文，零分配）；只有错误路径才用 `*_owned`。
        ///
        /// **处置纪律**：「这条错误该不该断流」只能由 [`AudioLinkError::is_statistical`] 回答，
        /// 调用方**不得**自行按 `code()` 猜测 —— 这是旧版「异常即永久静音」的根治点之一。
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum AudioLinkError {
            $(
                $(#[$meta])*
                $variant {
                    /// 失败细节（静态字面量，或错误路径上格式化的动态串）。
                    context: Cow<'static, str>,
                },
            )*
        }

        impl AudioLinkError {
            $(
                #[doc = concat!("静态上下文的 `", stringify!($variant), "`：`const` 友好，零分配。")]
                pub const fn $ctor(context: &'static str) -> Self {
                    Self::$variant { context: Cow::Borrowed(context) }
                }
            )*

            /// 错误码（跨 FFI 用 [`ErrorCode::as_u16`] 取值）。
            pub fn code(&self) -> ErrorCode {
                match self {
                    $(Self::$variant { .. } => ErrorCode::$code,)*
                }
            }

            /// 失败细节。
            pub fn context(&self) -> &str {
                match self {
                    $(Self::$variant { context } => context.as_ref(),)*
                }
            }

            /// 是否**统计类**（不致命）：计数后继续，绝不因此断流或停播。
            ///
            /// §11 的 `2001` / `2002` / `2003` 与 `3001` 属于此类 —— 它们是「链路还在跑，但需要
            /// 记账 / 自愈 / 限速」的信号，与 `1001`–`1009` 的契约级失败有本质区别。
            pub const fn is_statistical(&self) -> bool {
                match self {
                    $(Self::$variant { .. } => $statistical,)*
                }
            }

            /// 是否**契约级失败**（需要状态机迁移，通常是重连或拒绝会话）。
            pub const fn is_fatal(&self) -> bool {
                !self.is_statistical()
            }
        }
    };
}

define_audio_link_errors! {
    /// `1001`：协议主版本不兼容 → 提示升级并断开。
    VersionMismatch => version_mismatch, VersionMismatch, false;
    /// `1002`：未配对 → 触发配对流程。
    NotPaired => not_paired, NotPaired, false;
    /// `1003`：PIN 错误 / 超时（含剩余尝试次数）。
    PairRejected => pair_rejected, PairRejected, false;
    /// `1004`：签名校验失败（可能是中间人）→ 断开并告警。
    AuthFailed => auth_failed, AuthFailed, false;
    /// `1005`：对方不支持所需能力（如内录）→ UI 置灰。
    CapUnsupported => cap_unsupported, CapUnsupported, false;
    /// `1006`：混音路数超上限 → 拒绝并提示。
    StreamLimit => stream_limit, StreamLimit, false;
    /// `1007`：无共同编解码 → 建议切 PCM 档。
    CodecUnsupported => codec_unsupported, CodecUnsupported, false;
    /// `1008`：字节流非法（截断 / 超长 / 未知 ptype / 保留位非 0 / 长度越界 …）。
    ///
    /// 这是 L1 严格解码层（`audiolink-proto`）的**唯一拒绝出口**：调用方不得把它升级为断连（§1.1）。
    BadRequest => bad_request, BadRequest, false;
    /// `1009`：正在握手 / 配对中 → 稍后重试。
    Busy => busy, Busy, false;
    /// `2001`：播放欠载（统计用途，不致命）。
    PlayoutUnderrun => playout_underrun, PlayoutUnderrun, true;
    /// `2002`：播放器重建（自愈路径，FR-28）。
    SinkRebuild => sink_rebuild, SinkRebuild, true;
    /// `2003`：采集源失效（设备拔出 / 权限回收）。
    CaptureLost => capture_lost, CaptureLost, true;
    /// `3001`：请求过频（防滥用）。
    RateLimited => rate_limited, RateLimited, true;
}

impl AudioLinkError {
    /// 动态上下文的同码错误（仅错误路径使用，允许分配）。
    ///
    /// 与各 `const` 构造器一一对应，按 [`AudioLinkError::code`] 分派。
    pub fn owned(code: ErrorCode, context: String) -> Self {
        let context = Cow::Owned(context);
        match code {
            ErrorCode::VersionMismatch => Self::VersionMismatch { context },
            ErrorCode::NotPaired => Self::NotPaired { context },
            ErrorCode::PairRejected => Self::PairRejected { context },
            ErrorCode::AuthFailed => Self::AuthFailed { context },
            ErrorCode::CapUnsupported => Self::CapUnsupported { context },
            ErrorCode::StreamLimit => Self::StreamLimit { context },
            ErrorCode::CodecUnsupported => Self::CodecUnsupported { context },
            ErrorCode::BadRequest => Self::BadRequest { context },
            ErrorCode::Busy => Self::Busy { context },
            ErrorCode::PlayoutUnderrun => Self::PlayoutUnderrun { context },
            ErrorCode::SinkRebuild => Self::SinkRebuild { context },
            ErrorCode::CaptureLost => Self::CaptureLost { context },
            ErrorCode::RateLimited => Self::RateLimited { context },
        }
    }

    /// 动态上下文的 `BadRequest`（保留 M0 签名，等价于 [`AudioLinkError::owned`] 的 `BadRequest` 分支）。
    pub fn bad_request_owned(context: String) -> Self {
        Self::BadRequest {
            context: Cow::Owned(context),
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
/// v1 只有两种平台；[`Platform::Unknown`] 用于「未来版本引入的第三种平台」，
/// 落实「未知值可忽略、不丢弃节点」的演进规则（§13）。
/// 文本载体（mDNS TXT / 广播 JSON）只接受 `win` / `android`，见 [`Platform::from_str_exact`]。
///
/// 注：**本类型暂无二进制编码**（v1 的发现报文用文本；`HELLO.node_info` 的二进制取值冻结于 M1，
/// 届时按 §4.1 + §13 补规格，不在 M0-02 里先行发明）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Platform {
    /// Windows 桌面端（文本形式 `win`）。
    Windows,
    /// Android 移动端（文本形式 `android`）。
    Android,
    /// 未知平台（未来版本）。
    Unknown,
}

impl Platform {
    /// 文本载体的字符串形式（§9.2）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "win",
            Self::Android => "android",
            Self::Unknown => "unknown",
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

    /// 按位并集（能力组合）：`Caps::CAN_SEND.union(Caps::CAN_RECEIVE)`，也支持 `|` 运算符。
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// 按位交集。
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// 掩掉 v1 未知的位（§13 演进规则：未知能力位必须被忽略，否则新版本节点会被旧版本判成能力异常）。
    pub const fn known_only(self) -> Self {
        Self(self.0 & Self::KNOWN_MASK)
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

/// 运算符形式的能力组合：`Caps::CAN_SEND | Caps::CAN_RECEIVE`。
///
/// 提供运算符而不只是 [`Caps::union`]，是因为能力组合在构造 `NodeInfo` 时是高频操作，
/// 写成一串 `.union()` 会把「有哪些能力」这个关键信息淹掉。
impl std::ops::BitOr for Caps {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for Caps {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

impl std::ops::BitAnd for Caps {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        self.intersection(rhs)
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

// ---------------------------------------------------------------------------
// 时钟同步质量（docs/03-protocol.md §6.5）
// ---------------------------------------------------------------------------

/// 时钟同步质量分级（§6 第 5 条）：用于遥测面板展示、`CLOCK_RESULT` 载荷与「自动加深播放环」判据。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ClockQuality {
    /// `RTT ≤ 5 ms` 且样本数 ≥ 8：满足 FR-22 的同步前提。
    #[default]
    Good = 0,
    /// `RTT ≤ 20 ms`：可用，但同步精度按比例劣化。
    Fair = 1,
    /// 其它：UI 必须明示「该设备同步质量差」，并加深播放环。
    Poor = 2,
}

impl ClockQuality {
    /// 由 RTT 与有效样本数分级（§6 第 5 条的唯一实现，避免两端各写一套阈值）。
    pub const fn from_measurement(rtt_us: u64, samples: usize) -> Self {
        if rtt_us <= 5_000 && samples >= 8 {
            Self::Good
        } else if rtt_us <= 20_000 {
            Self::Fair
        } else {
            Self::Poor
        }
    }

    /// 线上 / FFI 的数值形式。
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// 由数值解析；未知值 → `None`（§13：未知取值按 `Poor` 处理，不得 panic）。
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Good),
            1 => Some(Self::Fair),
            2 => Some(Self::Poor),
            _ => None,
        }
    }

    /// 展示名（遥测面板 / 日志）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Fair => "fair",
            Self::Poor => "poor",
        }
    }
}

// ---------------------------------------------------------------------------
// 节点身份（docs/02-architecture.md §3、docs/03-protocol.md §5 / §9）
// ---------------------------------------------------------------------------

/// 节点指纹长度（SHA-256）。
pub const NODE_ID_LEN: usize = 32;

/// 节点短码长度（UI 展示与发现报文 `id` 字段用）：16 个 hex 字符 = 前 8 字节。
pub const NODE_ID_SHORT_HEX_LEN: usize = 16;

/// 小写 hex 编码（本 crate 默认零依赖，故自备；只在身份展示路径上调用）。
pub fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

/// 单个 hex 字符 → 半字节（大小写均可）。
fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// 节点标识 = 自签证书 DER 的 SHA-256 指纹（32 B）。
///
/// 这是 AudioLink 的**唯一身份**：TLS 握手已保证对端持有对应私钥，控制面
/// `AUTH_CHALLENGE` / `AUTH_RESPONSE` 再证明「私钥持有者 = 证书主体」（§5「认证强度」）。
/// 因此信任库只需存这个值，UI 上也只展示它的[短码](NodeId::short)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NodeId(pub [u8; NODE_ID_LEN]);

impl NodeId {
    /// 完整指纹长度（SHA-256 = 32 B）。
    pub const LEN: usize = NODE_ID_LEN;

    /// 由原始字节构造。
    pub const fn from_bytes(bytes: [u8; NODE_ID_LEN]) -> Self {
        Self(bytes)
    }

    /// 原始字节。
    pub const fn as_bytes(&self) -> &[u8; NODE_ID_LEN] {
        &self.0
    }

    /// 全量小写 hex（64 字符）。
    pub fn to_hex(self) -> String {
        hex_encode(&self.0)
    }

    /// 短码（16 个 hex 字符 = 指纹前 8 字节）：UI 展示与发现报文 `id` 字段用。
    ///
    /// 注意：短码**只用于展示与发现提示**，任何信任判定必须比对完整 [`NodeId`]。
    pub fn short(self) -> String {
        hex_encode(&self.0[..NODE_ID_SHORT_HEX_LEN / 2])
    }

    /// 解析 64 字符 hex 的完整指纹；长度或字符非法 → `None`。
    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != NODE_ID_LEN * 2 {
            return None;
        }
        let raw = text.as_bytes();
        let mut out = [0u8; NODE_ID_LEN];
        for (slot, pair) in out.iter_mut().zip(raw.chunks_exact(2)) {
            let [high, low] = pair else { return None };
            *slot = (hex_nibble(*high)? << 4) | hex_nibble(*low)?;
        }
        Some(Self(out))
    }

    /// 该短码是否与另一完整指纹的[短码](NodeId::short)一致（发现流程的初筛，**不构成信任**）。
    pub fn short_matches(self, text: &str) -> bool {
        text.eq_ignore_ascii_case(&self.short())
    }

    /// 是否未初始化哨兵（全 0）。
    pub fn is_zero(self) -> bool {
        self.0 == [0u8; NODE_ID_LEN]
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.short())
    }
}

/// 节点描述（`docs/02-architecture.md` §3）：控制面 `HELLO` / `HELLO_ACK` 的 `node_info` 载荷，
/// 并供发现协议（§9）与 UI 展示复用。
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NodeInfo {
    /// 节点身份（完整 32 B 指纹）—— 信任判定的唯一依据。
    pub id: NodeId,
    /// 用户可改的展示名。**仅展示**，不参与任何身份 / 信任判定。
    pub name: String,
    /// 平台。
    pub platform: Platform,
    /// 能力位图。
    pub caps: Caps,
    /// 协议版本（取值 [`PROTO_VERSION`]）。
    pub proto_version: u16,
}

impl NodeInfo {
    /// 构造一个使用当前协议版本、且已归一化能力位（掩掉未知位）的节点描述。
    pub fn new(id: NodeId, name: impl Into<String>, platform: Platform, caps: Caps) -> Self {
        Self {
            id,
            name: name.into(),
            platform,
            // §13 演进规则：未知能力位必须被忽略（掩掉），否则新版本节点会被旧版本判成「能力异常」
            caps: Caps::from_bits(caps.bits() & Caps::KNOWN_MASK),
            proto_version: PROTO_VERSION,
        }
    }
}
