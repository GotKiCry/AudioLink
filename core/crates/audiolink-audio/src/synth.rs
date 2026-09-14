//! 无设备的合成采集/播放实现。
//!
//! **存在理由**：链路正确性（组帧 → 编码 → 解码 → 提交、以及延迟记账）必须能在**没有声卡**的地方验证
//! —— CI runner、远程会话、以及本机音频设备被虚拟声卡占用时的兜底。它按真实时间节奏产出样本，
//! 因此能暴露节奏错误；但它**不产生设备时间戳**，所以采集段在报告里会被标为「估算」（见 [`crate::latency`]）。

use std::time::{Duration, Instant};

use crate::error::AudioError;
use crate::format::{CHANNELS, DeviceFormat, SAMPLE_RATE_HZ, SampleFormat};
use crate::sink::{PlayoutSink, PlayoutStats};
use crate::source::{CaptureSource, CaptureStats, CapturedPacket};

/// 合成采集源：按实时节奏产出正弦波。
#[derive(Debug)]
pub struct SyntheticCapture {
    format: DeviceFormat,
    buffer_ms: u32,
    frames_per_packet: usize,
    tone_hz: f32,
    phase: f64,
    next_due: Instant,
    stats: CaptureStats,
    running: bool,
}

impl SyntheticCapture {
    /// `buffer_ms` 同时充当「设备缓冲时长」与「每包帧数」的口径（1 ms = 48 帧）。
    pub fn new(buffer_ms: u32, tone_hz: f32) -> Result<Self, AudioError> {
        if buffer_ms == 0 {
            return Err(AudioError::invalid_config("合成采集缓冲时长必须 ≥ 1 ms"));
        }
        let frames_per_packet = (buffer_ms as usize) * 48;
        Ok(Self {
            format: DeviceFormat::new(SAMPLE_RATE_HZ, CHANNELS, SampleFormat::F32),
            buffer_ms,
            frames_per_packet,
            tone_hz,
            phase: 0.0,
            next_due: Instant::now(),
            stats: CaptureStats::default(),
            running: false,
        })
    }
}

impl CaptureSource for SyntheticCapture {
    fn device_format(&self) -> DeviceFormat {
        self.format
    }

    fn requested_buffer_ms(&self) -> u32 {
        self.buffer_ms
    }

    fn effective_buffer_ms(&self) -> u32 {
        self.buffer_ms
    }

    fn read(
        &mut self,
        dst: &mut Vec<f32>,
        timeout: Duration,
    ) -> Result<Option<CapturedPacket>, AudioError> {
        if !self.running {
            self.running = true;
            self.next_due = Instant::now() + Self::period(self.frames_per_packet);
        }
        let now = Instant::now();
        if now < self.next_due {
            let wait = self.next_due - now;
            if wait > timeout {
                std::thread::sleep(timeout);
                self.stats.read_timeouts += 1;
                return Ok(None);
            }
            std::thread::sleep(wait);
        }

        // 节奏对齐：落后超过一整包就认作卡顿并重新对表（合成源不掩盖卡顿）
        let now = Instant::now();
        let mut discontinuity = false;
        let mut late = now.duration_since(self.next_due);
        let period = Self::period(self.frames_per_packet);
        if late > period {
            self.stats.gaps += 1;
            discontinuity = true;
            late = Duration::ZERO;
        }
        self.next_due = now + period - late;

        dst.clear();
        dst.reserve(self.frames_per_packet * CHANNELS as usize);
        let step = f64::from(self.tone_hz) / f64::from(SAMPLE_RATE_HZ);
        for _ in 0..self.frames_per_packet {
            let value = (self.phase * std::f64::consts::TAU).sin() as f32 * 0.25;
            self.phase += step;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            dst.push(value);
            dst.push(value);
        }

        self.stats.packets += 1;
        self.stats.frames += self.frames_per_packet as u64;
        Ok(Some(CapturedPacket {
            frames: self.frames_per_packet,
            device_timestamp_100ns: None,
            read_at: Instant::now(),
            silent: false,
            discontinuity,
        }))
    }

    fn stats(&self) -> CaptureStats {
        self.stats
    }

    fn stop(&mut self) {
        self.running = false;
    }

    fn backend_name(&self) -> &'static str {
        "synth"
    }
}

impl SyntheticCapture {
    fn period(frames: usize) -> Duration {
        Duration::from_secs_f64(frames as f64 / f64::from(SAMPLE_RATE_HZ))
    }
}

/// 空输出：按实时速率「消耗」样本并统计欠载，不接触任何硬件。
#[derive(Debug)]
pub struct NullPlayout {
    format: DeviceFormat,
    buffer_ms: u32,
    /// 已提交但「尚未播出」的帧数（按真实时间衰减）——模拟水位。
    pending_frames: f64,
    last_update: Instant,
    stats: PlayoutStats,
}

impl NullPlayout {
    /// 构造。
    pub fn new(buffer_ms: u32) -> Self {
        Self {
            format: DeviceFormat::new(SAMPLE_RATE_HZ, CHANNELS, SampleFormat::F32),
            buffer_ms,
            pending_frames: 0.0,
            last_update: Instant::now(),
            stats: PlayoutStats::default(),
        }
    }

    fn decay(&mut self) -> f64 {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        self.last_update = now;
        self.pending_frames = (self.pending_frames - elapsed * f64::from(SAMPLE_RATE_HZ)).max(0.0);
        self.pending_frames
    }
}

impl PlayoutSink for NullPlayout {
    fn device_format(&self) -> DeviceFormat {
        self.format
    }

    fn requested_buffer_ms(&self) -> u32 {
        self.buffer_ms
    }

    fn effective_buffer_ms(&self) -> u32 {
        self.buffer_ms
    }

    fn buffered_frames(&mut self) -> u32 {
        self.pending_frames = self.decay();
        self.pending_frames as u32
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), AudioError> {
        let pending = self.decay();
        let frames = samples.len() / CHANNELS as usize;
        if pending <= 0.0 {
            self.stats.underruns += 1;
        }
        self.pending_frames += frames as f64;
        self.stats.writes += 1;
        self.stats.frames_written += frames as u64;
        Ok(())
    }

    fn stats(&self) -> PlayoutStats {
        self.stats
    }

    fn stop(&mut self) {}

    fn backend_name(&self) -> &'static str {
        "null"
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 合成源按实时节奏产出正确帧数() {
        let mut capture = SyntheticCapture::new(10, 1_000.0).unwrap();
        let mut buffer = Vec::new();
        let started = Instant::now();
        for _ in 0..5 {
            let packet = capture
                .read(&mut buffer, Duration::from_millis(200))
                .unwrap();
            let packet = packet.unwrap();
            assert_eq!(packet.frames, 480);
            assert_eq!(buffer.len(), 960);
            assert!(packet.device_timestamp_100ns.is_none());
        }
        // 5 包 × 10 ms ≈ 50 ms（宽松断言，避免 CI 抖动导致 flaky）
        assert!(
            started.elapsed() >= Duration::from_millis(35),
            "{:?}",
            started.elapsed()
        );
        assert!(buffer.iter().all(|value| value.is_finite()));
        let stats = capture.stats();
        assert_eq!(stats.packets, 5);
        assert_eq!(stats.frames, 2_400);
        assert_eq!(stats.read_timeouts, 0);
    }

    #[test]
    fn 合成源超时返回_none_并计数() {
        let mut capture = SyntheticCapture::new(50, 440.0).unwrap();
        let mut buffer = Vec::new();
        let packet = capture.read(&mut buffer, Duration::from_millis(1)).unwrap();
        assert!(packet.is_none());
        assert_eq!(capture.stats().read_timeouts, 1);
    }

    #[test]
    fn 空输出统计水位与欠载() {
        let mut sink = NullPlayout::new(20);
        let frame = vec![0.0f32; 1_920];
        assert_eq!(sink.buffered_frames(), 0);
        sink.write(&frame).unwrap();
        let stats = sink.stats();
        // 第一次写入时水位为 0 → 记一次欠载（这正是「刚起步必欠载」的真实行为）
        assert_eq!(stats.underruns, 1);
        assert_eq!(stats.frames_written, 960);
        let buffered = sink.buffered_frames();
        assert!(buffered <= 960, "{buffered}");
    }
}
