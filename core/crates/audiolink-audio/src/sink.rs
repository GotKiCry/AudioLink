//! 播放输出抽象（`docs/02-architecture.md` §4、ADR-010）
//!
//! 内核产出 PCM（48 kHz / f32 / 2ch 交错）与「目标播放时刻」，平台实现负责按时提交到硬件。
//! v1 只落地「提交 + 水位」两件事：预约播放（按 epoch 调度）属 M3，见 §6。

use crate::error::AudioError;
use crate::format::DeviceFormat;

/// 播放统计（遥测口径）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayoutStats {
    /// 提交次数。
    pub writes: u64,
    /// 累计提交帧数。
    pub frames_written: u64,
    /// 提交时水位不足（可能已欠载）的次数 —— 对应 `2001 PLAYOUT_UNDERRUN`。
    pub underruns: u64,
    /// 欠载时补的静音帧数。
    pub silent_padding_frames: u64,
    /// 写入失败次数（设备掉线等）。
    pub write_errors: u64,
}

/// 播放输出。
///
/// 与 [`crate::source::CaptureSource`] 同理，**不要求 `Send`**：WASAPI 的 COM 对象是线程绑定的，
/// 由使用它的线程自行创建（见该 trait 的文档）。
pub trait PlayoutSink {
    /// 设备实际格式（「零重采样」断言用）。
    fn device_format(&self) -> DeviceFormat;

    /// 请求的播放缓冲时长（ms）。
    fn requested_buffer_ms(&self) -> u32;

    /// 实际生效的缓冲时长（ms）。
    fn effective_buffer_ms(&self) -> u32;

    /// 当前水位：已提交但尚未播出的帧数（延迟分解「播放缓冲」一段的实测值）。
    fn buffered_frames(&mut self) -> u32;

    /// 提交一帧内部格式（48k / f32 / 2ch 交错）样本。
    fn write(&mut self, samples: &[f32]) -> Result<(), AudioError>;

    /// 统计快照。
    fn stats(&self) -> PlayoutStats;

    /// 停止输出（幂等）。
    fn stop(&mut self);

    /// 实现名（如 `"wasapi-render"`）。
    fn backend_name(&self) -> &'static str;
}
