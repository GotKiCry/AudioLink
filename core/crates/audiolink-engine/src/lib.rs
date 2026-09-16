//! audiolink-engine —— 编排层：引擎、会话管理、遥测、事件总线
//!
//! 规格：`docs/02-architecture.md`（§4 线程模型、§11 状态机）、`docs/03-protocol.md`（§4 控制帧、§5 握手、§10 遥测）、
//! `docs/11-m1-contract.md`（§5 对外 API 契约）。
//!
//! # 本 crate 的位置
//!
//! ```text
//! types ──► proto ──► { audio , identity } ──► net ──► **engine** ──► ffi ──► { desktop , android }
//! ```
//!
//! 上游 `audiolink-net` 只搬字节，`audiolink-audio` 只处理样本，`audiolink-identity` 只管身份材料；
//! **把三者拼成一条会说话的链路**是本 crate 的职责。因此这里也是 L2 分发层
//! （§1.1「未知类型忽略并计数、不断流」）与错误收敛点（所有错误 →
//! [`audiolink_types::AudioLinkError`]）。
//!
//! # 模块
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`adaptive`] | §8 自适应码率规则表（纯状态机：丢包 → 降级 / 恢复） |
//! | [`clock`] | §6 时钟同步接线：`CLOCK_PROBE` / `CLOCK_REPLY` 的节奏、配对与 RTT 分位数 |
//! | [`payload`] | §4 控制帧载荷结构体（postcard），字段顺序即 wire 顺序 |
//! | [`dispatch`] | L2 分发层：不透明载荷 → 有类型命令，并落实 §1.1 的忽略/计数纪律 |
//! | [`session`] | 会话状态机（§11 的迁移图落成数据表） |
//! | [`telemetry`] | 1 Hz 遥测聚合（§10 的 `StreamStats`） |

#![deny(unsafe_code)] // 必须使用 unsafe 的 crate（如 FFI 绑定）在文件顶部显式 #[allow] 并注明理由
#![deny(clippy::unwrap_used, clippy::expect_used)] // 实时路径禁止 panic；确需处用 #[allow] 并注明理由

pub mod adaptive;
pub mod clock;
pub mod dispatch;
pub mod format_guard;
pub mod handshake;
pub mod measure;
pub mod payload;
pub mod runtime;
pub mod session;
pub mod telemetry;

pub use adaptive::{AdaptiveBitrate, BitrateChange, BitrateReason, LinkSeverity};
pub use clock::{
    ClockProbeStats, FAST_INTERVAL_MS, FAST_PROBES, RTT_WINDOW, STEADY_INTERVAL_MS,
    now_monotonic_us,
};
pub use dispatch::{ControlRequest, DispatchOutcome, DispatchStats, dispatch, dispatch_into};
pub use format_guard::{require_unified_format, require_unified_link};
pub use handshake::{Handshake, HandshakeEvent, HandshakePhase, HandshakeStep, Outgoing, Role};
pub use measure::MeasurementTap;
pub use payload::{
    AuthChallengePayload, AuthResponsePayload, ByePayload, ClockResultPayload, CloseStreamPayload,
    CodecPref, ErrorPayload, HelloAckPayload, HelloPayload, OpenStreamAckPayload,
    OpenStreamPayload, PairRequiredPayload, PairResultPayload, PairSubmitPayload, PingPayload,
    STREAM_ID_ALL, SetGainPayload, SetMutePayload, SourceKind, StreamStatsPayload, decode_payload,
    encode_payload,
};
pub use runtime::{CaptureFactory, Engine, EngineConfig, EngineEvent, PeerStatus, PlayoutFactory};
pub use session::{SessionEvent, SessionMachine, SessionState, SessionTransition, next_state};
pub use telemetry::TelemetryAggregator;
