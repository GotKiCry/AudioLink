//! 四段延迟分解（`docs/05-roadmap.md` M1：采集 / 编码 / 解码 / 播放）
//!
//! # 模型（每一项都可由遥测复核）
//!
//! ```text
//! 端到端 ≈ 采集缓冲 + 组帧 + 编码 + 解码 + 播放缓冲
//! ```
//!
//! | 段 | 口径 | 来源 |
//! |---|---|---|
//! | 采集缓冲 | 读出时刻 − 设备时间戳 | **实测**（设备提供 QPC 时间戳时）；不可用时退化为「缓冲时长 ÷ 2」**估算**，并单独计数 |
//! | 组帧 | 帧长本身（20 ms） | 结构性常量（`chunker` 被动引入，无法回避） |
//! | 编码 | `encode()` 耗时 + 编码器 lookahead | 前者实测；lookahead 由编解码器给出（样本 ÷ 48 kHz） |
//! | 解码 | `decode()` 耗时 | 实测 |
//! | 播放缓冲 | 提交后水位（帧数 ÷ 48 kHz） | **实测** |
//!
//! 「实测/估算」必须分开计数并分开出分位数 —— 把估算值混进 P95 会让 `M1` 验收失去意义
//! （验收见 `docs/05-roadmap.md` M1，其中 P50 ≤ 110 ms / P95 ≤ 150 ms 就靠这张表）。

use crate::stats::{SampleStats, Summary};

/// 一帧音频在链路上经历的**四段延迟**（微秒）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SegmentSample {
    /// 采集段：样本进入设备缓冲 → 被内核读出。
    pub capture_us: u32,
    /// 组帧段：攒够一帧（= 帧长）。
    pub assemble_us: u32,
    /// 编码段：Opus 编码耗时 + 编码器固有时延。
    pub encode_us: u32,
    /// 解码段：Opus 解码耗时（+ 解码管线）。
    pub decode_us: u32,
    /// 播放段：提交后到出声（水位）。
    pub playout_us: u32,
}

impl SegmentSample {
    /// 五段之和（= 该帧的端到端估算）。
    pub const fn total_us(&self) -> u32 {
        self.capture_us
            .saturating_add(self.assemble_us)
            .saturating_add(self.encode_us)
            .saturating_add(self.decode_us)
            .saturating_add(self.playout_us)
    }
}

/// 组帧段的理论值：帧长（ms）→ μs。
pub const fn frame_assembly_us(frame_ms: u32) -> u32 {
    frame_ms.saturating_mul(1_000)
}

/// 采集段退化为估算时的值：按「半缓冲」计（平均等待时长）。
pub const fn half_buffer_us(buffer_ms: u32) -> u32 {
    buffer_ms.saturating_mul(1_000) / 2
}

/// 延迟账本：逐帧记账，事后出分位数。
#[derive(Debug, Clone)]
pub struct LatencyLedger {
    capture: SampleStats,
    assemble: SampleStats,
    encode: SampleStats,
    decode: SampleStats,
    playout: SampleStats,
    e2e_estimated: SampleStats,
    e2e_measured: SampleStats,
    frames: u64,
    capture_measured_frames: u64,
    capture_estimated_frames: u64,
}

impl LatencyLedger {
    /// 建账本；每个指标最多保留 `capacity` 个样本。
    pub fn new(capacity: usize) -> Self {
        Self {
            capture: SampleStats::new(capacity),
            assemble: SampleStats::new(capacity),
            encode: SampleStats::new(capacity),
            decode: SampleStats::new(capacity),
            playout: SampleStats::new(capacity),
            e2e_estimated: SampleStats::new(capacity),
            e2e_measured: SampleStats::new(capacity),
            frames: 0,
            capture_measured_frames: 0,
            capture_estimated_frames: 0,
        }
    }

    /// 记一帧的五段分解。
    ///
    /// `capture_from_device_timestamp`：采集段是**实测**（设备给了时间戳）还是**估算**（半缓冲近似）。
    pub fn record(&mut self, sample: &SegmentSample, capture_from_device_timestamp: bool) {
        self.capture.push(sample.capture_us);
        self.assemble.push(sample.assemble_us);
        self.encode.push(sample.encode_us);
        self.decode.push(sample.decode_us);
        self.playout.push(sample.playout_us);
        self.e2e_estimated.push(sample.total_us());
        self.frames += 1;
        if capture_from_device_timestamp {
            self.capture_measured_frames += 1;
        } else {
            self.capture_estimated_frames += 1;
        }
    }

    /// 记一次**标记法实测**的端到端（设备往返，见 `tools/self-loop`）。
    pub fn record_measured_roundtrip(&mut self, total_us: u32) {
        self.e2e_measured.push(total_us);
    }

    /// 已记账帧数。
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// 汇总报告（复制 + 排序，允许分配；**非实时路径**）。
    pub fn report(&self) -> LatencyReport {
        LatencyReport {
            frames: self.frames,
            capture_measured_frames: self.capture_measured_frames,
            capture_estimated_frames: self.capture_estimated_frames,
            capture: self.capture.summary(),
            assemble: self.assemble.summary(),
            encode: self.encode.summary(),
            decode: self.decode.summary(),
            playout: self.playout.summary(),
            e2e_estimated: self.e2e_estimated.summary(),
            e2e_measured: self.e2e_measured.summary(),
        }
    }
}

/// 四段延迟报告（各项单位：μs）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LatencyReport {
    /// 记账帧数。
    pub frames: u64,
    /// 采集段为**实测**的帧数（设备提供时间戳）。
    pub capture_measured_frames: u64,
    /// 采集段为**估算**的帧数（半缓冲近似）。
    pub capture_estimated_frames: u64,
    /// 采集段分位数。
    pub capture: Option<Summary>,
    /// 组帧段分位数。
    pub assemble: Option<Summary>,
    /// 编码段分位数。
    pub encode: Option<Summary>,
    /// 解码段分位数。
    pub decode: Option<Summary>,
    /// 播放段分位数。
    pub playout: Option<Summary>,
    /// 端到端（五段之和）分位数。
    pub e2e_estimated: Option<Summary>,
    /// 端到端（标记法实测，设备往返）分位数。
    pub e2e_measured: Option<Summary>,
}

impl LatencyReport {
    /// 采集段是否**全部**为实测（M1 报告里用来决定能否宣称「实测分解」）。
    pub fn capture_fully_measured(&self) -> bool {
        self.capture_measured_frames > 0 && self.capture_estimated_frames == 0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 五段求和与常量() {
        let sample = SegmentSample {
            capture_us: 10_000,
            assemble_us: frame_assembly_us(20),
            encode_us: 700,
            decode_us: 300,
            playout_us: 10_000,
        };
        assert_eq!(sample.total_us(), 41_000);
        assert_eq!(half_buffer_us(20), 10_000);
    }

    #[test]
    fn 账本区分实测与估算的采集段() {
        let mut ledger = LatencyLedger::new(16);
        let sample = SegmentSample {
            capture_us: 5_000,
            assemble_us: 20_000,
            encode_us: 1_000,
            decode_us: 500,
            playout_us: 6_000,
        };
        ledger.record(&sample, true);
        ledger.record(&sample, false);
        ledger.record_measured_roundtrip(55_000);
        let report = ledger.report();
        assert_eq!(report.frames, 2);
        assert_eq!(report.capture_measured_frames, 1);
        assert_eq!(report.capture_estimated_frames, 1);
        assert!(!report.capture_fully_measured());
        assert_eq!(report.e2e_estimated.unwrap().p50, 32_500);
        assert_eq!(report.e2e_measured.unwrap().p95, 55_000);
    }

    #[test]
    fn 全实测时报告标记为实测() {
        let mut ledger = LatencyLedger::new(4);
        ledger.record(&SegmentSample::default(), true);
        assert!(ledger.report().capture_fully_measured());
    }
}
