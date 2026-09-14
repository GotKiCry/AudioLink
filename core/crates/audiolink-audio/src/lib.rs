//! audiolink-audio —— 采集/播放抽象、组帧、Opus 编解码、无锁环、延迟分解
//!
//! 规格：`docs/02-architecture.md`（§3 统一格式、§4 线程模型与实时铁律）、
//! `docs/04-tech-stack.md`（ADR-003 编解码、ADR-005 平台音频、ADR-010 平台抽象）。
//!
//! # 模块
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`format`] | 统一内部格式（48 kHz / f32 / 2ch 交错）与设备格式转换 |
//! | [`chunker`] | 固定帧长组帧（环形槽，零拷贝出帧，队列满丢最旧） |
//! | [`codec`] | Opus 编解码封装（参数基线、PLC、遥测计数） |
//! | [`ring`] | 采集线程 → 编码线程的无锁 SPSC 采样环 |
//! | [`source`] / [`sink`] | 采集与播放的平台抽象（ADR-010） |
//! | [`synth`] | 无设备的合成实现（CI / 无声卡环境下的链路验证） |
//! | [`latency`] | 四段延迟分解账本（采集 / 组帧 / 编码 / 解码 / 播放） |
//! | [`stats`] | 分位数统计（固定容量，遥测与工具共用） |
//! | [`error`] | 统一错误类型（映射到协议错误码） |
//!
//! Windows 实现（WASAPI）在 `wasapi` 模块内，**只在 Windows 上参与编译** —— Android 交叉编译
//! 与 Linux 构建都不会拉进 `wasapi` / `windows` 依赖。
//!
//! # 实时纪律（`docs/02-architecture.md` §4）
//!
//! 1. 音频线程内不做分配/加锁/日志；本 crate 的实时友好 API 都接受**调用方提供的可复用缓冲**；
//! 2. `unwrap` / `expect` / `panic` 一律禁用（workspace lint 已 `deny`）；
//! 3. 错误必须显式返回并计数，**不允许静默停止**（欠载是统计项，不是断流理由）。

#![deny(unsafe_code)] // 必须使用 unsafe 的 crate（如 FFI 绑定）在文件顶部显式 #[allow] 并注明理由
#![deny(clippy::unwrap_used, clippy::expect_used)] // 实时路径禁止 panic；确需处用 #[allow] 并注明理由

pub mod chunker;
pub mod codec;
pub mod error;
pub mod format;
pub mod latency;
pub mod ring;
pub mod sink;
pub mod source;
pub mod stats;
pub mod synth;

#[cfg(windows)]
pub mod wasapi;

pub use chunker::{ChunkerStats, FrameChunker};
pub use codec::{CodecConfig, OpusDecoder, OpusEncoder};
pub use error::AudioError;
pub use format::{
    CHANNELS, DEFAULT_FRAME_INTERLEAVED, DEFAULT_FRAME_MS, DEFAULT_FRAME_SAMPLES, DeviceFormat,
    SAMPLE_RATE_HZ, SampleFormat, convert_from_internal, convert_to_internal,
};
pub use latency::{LatencyLedger, LatencyReport, SegmentSample};
pub use ring::{SampleRingReader, SampleRingWriter, sample_ring};
pub use sink::{PlayoutSink, PlayoutStats};
pub use source::{CaptureSource, CaptureStats, CapturedPacket};
pub use stats::{SampleStats, Summary};
pub use synth::{NullPlayout, SyntheticCapture};
