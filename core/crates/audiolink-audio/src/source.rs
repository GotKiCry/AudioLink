//! 采集抽象（`docs/02-architecture.md` §4、ADR-010）
//!
//! 内核只认一件事：**48 kHz / f32 / 2ch 交错样本 + 采集时刻**。平台实现（Windows 走 WASAPI loopback，
//! Android 走 Kotlin `AudioRecord` / `AudioPlaybackCapture`）负责把设备数据变成这件事。
//!
//! 「采集线程不许分配、不许加锁、不许日志」——因此 [`CaptureSource::read`] 的接口刻意设计成
//! **调用方提供可复用缓冲**（`&mut Vec<f32>`）与**一次性打包的返回值**（[`CapturedPacket`] 是 `Copy`）。

use std::time::{Duration, Instant};

use crate::error::AudioError;
use crate::format::DeviceFormat;

/// 一次采集读出的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapturedPacket {
    /// 本次读出的帧数（每声道样本数）。
    pub frames: usize,
    /// 设备时间戳（单位 100 ns，QPC 口径）；`None` = 设备未提供。
    pub device_timestamp_100ns: Option<i64>,
    /// 读到本包的单调时刻 —— 延迟分解的起点。
    pub read_at: Instant,
    /// 设备把本包标记为静音（loopback 上通常表示「当前没有音频在播放」）。
    pub silent: bool,
    /// 设备报告数据不连续（驱动丢样本 / 缓冲异常）。
    pub discontinuity: bool,
}

/// 采集统计（遥测口径）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureStats {
    /// 成功读出的包数。
    pub packets: u64,
    /// 累计读出帧数。
    pub frames: u64,
    /// 静音包数。
    pub silent_packets: u64,
    /// 设备报告的不连续次数。
    pub discontinuities: u64,
    /// 读取超时次数（超时 = 设备没有按预期交付，必须计数）。
    pub read_timeouts: u64,
    /// 「被唤醒但没有数据」的次数（事件驱动下偶发；持续发生说明端点不在播放）。
    pub empty_wakeups: u64,
    /// 环形缓冲溢出丢掉的样本数（编码/消费跟不上）。
    pub overruns: u64,
    /// 两次读出之间的间隔超过「包时长 + 容差」的次数（采集卡顿，M1 关键风险）。
    pub gaps: u64,
}

/// 采集源。
///
/// # 为什么没有 `Send` 约束（实测约束，别改回去）
///
/// WASAPI 的实现（[`crate::wasapi::LoopbackCapture`]）内部持有 `IAudioClient` / `IAudioCaptureClient` /
/// 事件句柄，而 `wasapi` 0.24 暴露的 COM 包装**全部是 `!Send + !Sync`**（`windows` 0.62 的接口类型
/// 不实现这两个 trait）。因此约定是：**谁用谁建** —— 使用 WASAPI 的线程自己 `init_thread_mta()`
/// 并构造对象，线程之间只传数据（例如 SPSC 环），不传句柄。这与 ADR-010「内核不持有平台音频线程」
/// 是同一条纪律。
pub trait CaptureSource {
    /// 设备实际格式（「零重采样」断言用：必须是 48k / 2ch / f32）。
    fn device_format(&self) -> DeviceFormat;

    /// 请求的采集缓冲时长（ms）——M1 的关键未知量，实际生效值由实现给出（见 [`Self::effective_buffer_ms`]）。
    fn requested_buffer_ms(&self) -> u32;

    /// 实际生效的缓冲时长（ms）：驱动可能不接受请求值，这里是设备真正给的。
    fn effective_buffer_ms(&self) -> u32;

    /// 阻塞读取下一包（最多等 `timeout`；超时返回 `Ok(None)` 并计数）。
    ///
    /// `dst` 会被**清空并填充**为交错 f32 样本；实现应复用 `dst` 的容量，稳态零分配。
    fn read(
        &mut self,
        dst: &mut Vec<f32>,
        timeout: Duration,
    ) -> Result<Option<CapturedPacket>, AudioError>;

    /// 统计快照。
    fn stats(&self) -> CaptureStats;

    /// 停止采集（幂等）。
    fn stop(&mut self);

    /// 实现名（报告与日志用，如 `"wasapi-loopback"`）。
    fn backend_name(&self) -> &'static str;
}
