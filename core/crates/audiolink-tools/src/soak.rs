//! soak-runner 的判定与报告逻辑。
//!
//! 「8 h 无故障」这句验收话必须能被机器判定：本模块把 1 Hz 的遥测采样变成
//! **异常快照**（每次越界都留现场）与**汇总报告**（JSON，可作 CI 产物）。
//!
//! 判据口径是「回环稳态不该出现的东西」：欠载、PCM 掩盖、丢包、迟到丢弃、NACK 重传
//! 在一条 127.0.0.1 的链路上都应当是 0 —— 任何非零值都是缺陷信号，不是噪声。
//!
//! 本模块**不碰套接字也不睡觉**：只吃采样、吐判定，因此可以完整单测（编排在 `bin/soak_runner.rs`）。

use std::fmt::Write as _;

use audiolink_types::StreamStats;
use serde_json::{Value, json};

/// 会话处于 streaming 时使用的状态名（与 `SessionState::name()` 一致）。
pub const STREAMING: &str = "streaming";

/// 粗采样粒度（秒）：报告里每分钟留一条，8 h 约 480 条 —— 既看得到趋势，又不至于把报告撑爆。
pub const COARSE_BUCKET_SECS: u64 = 60;

/// 判定阈值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoakThresholds {
    /// 允许的累计欠载上限（回环稳态默认 0）。
    pub max_underruns: u32,
    /// 允许的 PCM 掩盖帧上限（默认 0）。
    pub max_plc: u32,
    /// 允许的迟到丢弃上限（默认 0）。
    pub max_late_drops: u32,
    /// 允许的 NACK 重传请求上限（默认 0：本地回环不该丢包）。
    pub max_nack: u32,
    /// 允许的丢包率上限（百分比 ×100，默认 0）。
    pub max_loss_pct_x100: u16,
    /// 码率相对目标值的容忍比例（百分比 ×100，默认 30%）；0 = 不判。
    pub bitrate_tolerance_pct_x100: u16,
    /// 连续多少次采样码率为 0 判为挂住。
    pub stall_samples: u32,
    /// 留存的快照上限（超出只计数，不再存现场 —— 8 h 跑不该产出无限大的报告）。
    pub max_violations: usize,
}

impl Default for SoakThresholds {
    fn default() -> Self {
        Self {
            max_underruns: 0,
            max_plc: 0,
            max_late_drops: 0,
            max_nack: 0,
            max_loss_pct_x100: 0,
            bitrate_tolerance_pct_x100: 3_000,
            stall_samples: 5,
            max_violations: 200,
        }
    }
}

/// 一次采样的观测值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoakSample {
    /// 采样时刻（距开流多少秒）。
    pub at_secs: u64,
    /// 会话状态名（非 `STREAMING` 都算异常）。
    pub state: &'static str,
    /// **接收侧**遥测快照（欠载 / 掩盖 / 队列水位都只在那一边可见）。
    pub stats: StreamStats,
}

/// 异常种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationKind {
    /// 会话不在 streaming。
    NotStreaming,
    /// 播放欠载增加。
    Underrun,
    /// PCM 掩盖帧增加（真丢了包）。
    PlayoutConcealment,
    /// 丢包率超阈值。
    PacketLoss,
    /// 迟到丢弃增加。
    LateDrop,
    /// NACK 重传请求增加。
    NackRetransmit,
    /// 码率偏离目标过远。
    BitrateOutOfRange,
    /// 连续若干次采样码率为 0（链路挂住）。
    Stalled,
}

impl ViolationKind {
    /// 稳定名字（报告与 CI 断言用）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::NotStreaming => "not_streaming",
            Self::Underrun => "underrun",
            Self::PlayoutConcealment => "playout_concealment",
            Self::PacketLoss => "packet_loss",
            Self::LateDrop => "late_drop",
            Self::NackRetransmit => "nack_retransmit",
            Self::BitrateOutOfRange => "bitrate_out_of_range",
            Self::Stalled => "stalled",
        }
    }
}

/// 一条异常快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// 采样时刻（秒）。
    pub at_secs: u64,
    /// 种类。
    pub kind: ViolationKind,
    /// 人类可读细节。
    pub detail: String,
    /// 当时的接收侧遥测。
    pub stats: StreamStats,
}

/// 每分钟一条的粗采样。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoarseSample {
    /// 第几个 60 s 桶。
    pub bucket: u64,
    /// 该桶第一次采样时刻。
    pub at_secs: u64,
    /// 当时的遥测。
    pub stats: StreamStats,
}

/// 汇总（报告里 summary 段的数据源）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoakSummary {
    /// 采样次数。
    pub samples: u64,
    /// 留存下来的异常条数。
    pub violations: usize,
    /// 因为超过快照上限而未留存的异常条数。
    pub dropped_violations: u64,
    /// 最后一次采样的遥测。
    pub final_stats: Option<StreamStats>,
    /// 判定。
    pub verdict: &'static str,
}

/// 报告元信息（由调用方填：本次跑的参数）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoakMeta {
    /// 计划时长（秒）。
    pub planned_seconds: u64,
    /// 帧长（ms）。
    pub frame_ms: u32,
    /// 目标码率（bps）；0 = 不判码率。
    pub expected_bitrate_bps: u32,
    /// 开始时刻（Unix 秒）。
    pub started_at_unix: u64,
}
/// 采样监视器。
#[derive(Debug)]
pub struct SoakMonitor {
    thresholds: SoakThresholds,
    expected_bitrate_bps: u32,
    previous: Option<StreamStats>,
    previous_state: Option<&'static str>,
    stall_run: u32,
    samples: u64,
    violations: Vec<Violation>,
    dropped_violations: u64,
    coarse: Vec<CoarseSample>,
}

impl SoakMonitor {
    /// 新建监视器；`expected_bitrate_bps = 0` 表示不判码率。
    pub fn new(thresholds: SoakThresholds, expected_bitrate_bps: u32) -> Self {
        Self {
            thresholds,
            expected_bitrate_bps,
            previous: None,
            previous_state: None,
            stall_run: 0,
            samples: 0,
            violations: Vec::new(),
            dropped_violations: 0,
            coarse: Vec::new(),
        }
    }

    /// 吃掉一次采样，产出（并入账）全部异常。
    pub fn observe(&mut self, sample: SoakSample) {
        self.samples += 1;
        let stats = sample.stats;

        // 会话状态：只在「刚起步」或「状态发生变化」时判定一次，避免每个采样都刷同一条。
        let state_changed = self.previous_state != Some(sample.state);
        if state_changed && sample.state != STREAMING {
            self.push_violation(
                ViolationKind::NotStreaming,
                sample.at_secs,
                format!("会话状态 {}（应为 {STREAMING}）", sample.state),
                stats,
            );
        }
        self.previous_state = Some(sample.state);

        if let Some(previous) = self.previous {
            self.check_delta(
                ViolationKind::Underrun,
                "欠载",
                previous.underruns,
                stats.underruns,
                sample.at_secs,
                stats,
            );
            self.check_delta(
                ViolationKind::PlayoutConcealment,
                "PCM 掩盖帧",
                previous.plc_count,
                stats.plc_count,
                sample.at_secs,
                stats,
            );
            self.check_delta(
                ViolationKind::LateDrop,
                "迟到丢弃",
                previous.late_drops,
                stats.late_drops,
                sample.at_secs,
                stats,
            );
            self.check_delta(
                ViolationKind::NackRetransmit,
                "NACK 重传请求",
                previous.nack_count,
                stats.nack_count,
                sample.at_secs,
                stats,
            );
        }

        if stats.loss_pct_x100 > self.thresholds.max_loss_pct_x100 {
            self.push_violation(
                ViolationKind::PacketLoss,
                sample.at_secs,
                format!(
                    "丢包率 {}（上限 {}）",
                    stats.loss_pct_x100, self.thresholds.max_loss_pct_x100
                ),
                stats,
            );
        }

        if self.expected_bitrate_bps > 0 && stats.bitrate_bps > 0 {
            let expected = u64::from(self.expected_bitrate_bps);
            let actual = u64::from(stats.bitrate_bps);
            let deviation = actual.abs_diff(expected).saturating_mul(10_000) / expected;
            if deviation > u64::from(self.thresholds.bitrate_tolerance_pct_x100) {
                self.push_violation(
                    ViolationKind::BitrateOutOfRange,
                    sample.at_secs,
                    format!(
                        "码率 {actual} bps 偏离目标 {expected} bps 达 {}.{:02}%",
                        deviation / 100,
                        deviation % 100
                    ),
                    stats,
                );
            }
        }

        // 挂住：码率连续为 0 说明链路上已经没有音频在跑（断流 / 静默停止）。
        if stats.bitrate_bps == 0 {
            self.stall_run = self.stall_run.saturating_add(1);
            if self.stall_run == self.thresholds.stall_samples {
                self.push_violation(
                    ViolationKind::Stalled,
                    sample.at_secs,
                    format!("连续 {} 次采样码率为 0", self.stall_run),
                    stats,
                );
            }
        } else {
            self.stall_run = 0;
        }

        // 粗采样：每个 60 s 桶留第一条。
        let bucket = sample.at_secs / COARSE_BUCKET_SECS;
        if self.coarse.last().is_none_or(|last| last.bucket != bucket) {
            self.coarse.push(CoarseSample {
                bucket,
                at_secs: sample.at_secs,
                stats,
            });
        }

        self.previous = Some(stats);
    }

    /// 采样次数。
    pub const fn samples(&self) -> u64 {
        self.samples
    }

    /// 已留存的异常快照。
    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }

    /// 未留存（超上限）的异常条数。
    pub const fn dropped_violations(&self) -> u64 {
        self.dropped_violations
    }

    /// 粗采样。
    pub fn coarse(&self) -> &[CoarseSample] {
        &self.coarse
    }

    /// 汇总。
    pub fn summary(&self) -> SoakSummary {
        SoakSummary {
            samples: self.samples,
            violations: self.violations.len(),
            dropped_violations: self.dropped_violations,
            final_stats: self.previous,
            verdict: if self.violations.is_empty() && self.dropped_violations == 0 {
                "ok"
            } else {
                "failed"
            },
        }
    }

    /// 渲染 JSON 报告（`meta` 由调用方提供本次跑的参数）。
    pub fn report_json(&self, meta: &SoakMeta) -> String {
        let summary = self.summary();
        let violations: Vec<Value> = self
            .violations
            .iter()
            .map(|violation| {
                json!({
                    "at_secs": violation.at_secs,
                    "kind": violation.kind.name(),
                    "detail": violation.detail,
                    "stats": stats_json(violation.stats),
                })
            })
            .collect();
        let coarse: Vec<Value> = self
            .coarse
            .iter()
            .map(|sample| {
                json!({
                    "bucket": sample.bucket,
                    "at_secs": sample.at_secs,
                    "stats": stats_json(sample.stats),
                })
            })
            .collect();

        let report = json!({
            "tool": "soak-runner",
            "started_at_unix": meta.started_at_unix,
            "planned_seconds": meta.planned_seconds,
            "frame_ms": meta.frame_ms,
            "expected_bitrate_bps": meta.expected_bitrate_bps,
            "summary": {
                "samples": summary.samples,
                "violations": summary.violations,
                "dropped_violations": summary.dropped_violations,
                "verdict": summary.verdict,
                "final_stats": summary.final_stats.map(stats_json),
            },
            "violations": violations,
            "coarse": coarse,
        });
        report.to_string()
    }

    /// 人类可读摘要（控制台用）。
    pub fn summary_text(&self, meta: &SoakMeta) -> String {
        let summary = self.summary();
        let mut out = String::new();
        let _ = writeln!(
            out,
            "soak 汇总：采样 {} 次 · 计划 {} s · 帧长 {} ms",
            summary.samples, meta.planned_seconds, meta.frame_ms
        );
        if let Some(stats) = summary.final_stats {
            let _ = writeln!(
                out,
                "末次遥测：码率 {} bps · 丢包 {} · 欠载 {} · 掩盖 {} · 迟到 {} · NACK {} · 队列水位 {} µs · RTT {} µs",
                stats.bitrate_bps,
                stats.loss_pct_x100,
                stats.underruns,
                stats.plc_count,
                stats.late_drops,
                stats.nack_count,
                stats.buffer_level_us,
                stats.rtt_us,
            );
        }
        let _ = writeln!(
            out,
            "异常：{} 条（未留存 {} 条）→ 判定 {}",
            summary.violations, summary.dropped_violations, summary.verdict
        );
        for violation in self.violations.iter().take(10) {
            let _ = writeln!(
                out,
                "  t={:>5}s  [{}] {}",
                violation.at_secs,
                violation.kind.name(),
                violation.detail
            );
        }
        if self.violations.len() > 10 {
            let _ = writeln!(
                out,
                "  … 其余 {} 条见 JSON 报告",
                self.violations.len() - 10
            );
        }
        out
    }

    fn check_delta(
        &mut self,
        kind: ViolationKind,
        label: &str,
        before: u32,
        after: u32,
        at_secs: u64,
        stats: StreamStats,
    ) {
        if after > before {
            self.push_violation(
                kind,
                at_secs,
                format!("{label} +{}（累计 {after}）", after - before),
                stats,
            );
        }
    }

    fn push_violation(
        &mut self,
        kind: ViolationKind,
        at_secs: u64,
        detail: String,
        stats: StreamStats,
    ) {
        if self.violations.len() >= self.thresholds.max_violations {
            self.dropped_violations = self.dropped_violations.saturating_add(1);
            return;
        }
        self.violations.push(Violation {
            at_secs,
            kind,
            detail,
            stats,
        });
    }
}

/// 遥测快照 → JSON（手写字段，避免为一个报告把 `serde` feature 拉进依赖图）。
fn stats_json(stats: StreamStats) -> Value {
    json!({
        "stream_id": stats.stream_id,
        "rtt_us": stats.rtt_us,
        "jitter_us": stats.jitter_us,
        "jitter_p95_us": stats.jitter_p95_us,
        "loss_pct_x100": stats.loss_pct_x100,
        "bitrate_bps": stats.bitrate_bps,
        "clock_offset_us": stats.clock_offset_us,
        "drift_ppm": stats.drift_ppm,
        "buffer_level_us": stats.buffer_level_us,
        "underruns": stats.underruns,
        "plc_count": stats.plc_count,
        "nack_count": stats.nack_count,
        "e2e_latency_us": stats.e2e_latency_us,
        "late_drops": stats.late_drops,
    })
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn stats(bitrate_bps: u32) -> StreamStats {
        StreamStats {
            stream_id: 1,
            bitrate_bps,
            ..StreamStats::default()
        }
    }

    fn sample(at_secs: u64, stats: StreamStats) -> SoakSample {
        SoakSample {
            at_secs,
            state: STREAMING,
            stats,
        }
    }

    fn meta() -> SoakMeta {
        SoakMeta {
            planned_seconds: 60,
            frame_ms: 20,
            expected_bitrate_bps: 160_000,
            started_at_unix: 1_700_000_000,
        }
    }

    #[test]
    fn steady_state_produces_no_violations() {
        let mut monitor = SoakMonitor::new(SoakThresholds::default(), 160_000);
        for secs in 0..10 {
            monitor.observe(sample(secs, stats(160_500)));
        }
        let summary = monitor.summary();
        assert_eq!(summary.samples, 10);
        assert_eq!(summary.violations, 0);
        assert_eq!(summary.verdict, "ok");
        assert!(monitor.violations().is_empty());
    }

    #[test]
    fn counter_increases_become_snapshots_with_delta() {
        let mut monitor = SoakMonitor::new(SoakThresholds::default(), 160_000);
        monitor.observe(sample(0, stats(160_000)));

        let mut second = stats(160_000);
        second.underruns = 3;
        second.plc_count = 2;
        second.late_drops = 1;
        second.nack_count = 4;
        monitor.observe(sample(1, second));

        let kinds: Vec<&str> = monitor
            .violations()
            .iter()
            .map(|violation| violation.kind.name())
            .collect();
        assert_eq!(
            kinds,
            vec![
                "underrun",
                "playout_concealment",
                "late_drop",
                "nack_retransmit"
            ]
        );
        assert_eq!(monitor.violations()[0].detail, "欠载 +3（累计 3）");
        assert_eq!(monitor.violations()[3].at_secs, 1);
    }

    #[test]
    fn loss_bitrate_and_stall_are_detected() {
        let mut monitor = SoakMonitor::new(SoakThresholds::default(), 160_000);
        let mut lossy = stats(160_000);
        lossy.loss_pct_x100 = 250;
        monitor.observe(sample(0, lossy));
        assert_eq!(monitor.violations()[0].kind, ViolationKind::PacketLoss);

        monitor.observe(sample(1, stats(80_000)));
        assert!(
            monitor
                .violations()
                .iter()
                .any(|violation| violation.kind == ViolationKind::BitrateOutOfRange)
        );

        for secs in 2..7 {
            monitor.observe(sample(secs, stats(0)));
        }
        assert!(
            monitor
                .violations()
                .iter()
                .any(|violation| violation.kind == ViolationKind::Stalled),
            "连续零码率必须判为挂住"
        );
    }

    #[test]
    fn leaving_streaming_is_reported_once_per_transition() {
        let mut monitor = SoakMonitor::new(SoakThresholds::default(), 160_000);
        monitor.observe(sample(0, stats(160_000)));
        let mut down = sample(1, stats(160_000));
        down.state = "reconnecting";
        monitor.observe(down);
        monitor.observe(down);
        let count = monitor
            .violations()
            .iter()
            .filter(|violation| violation.kind == ViolationKind::NotStreaming)
            .count();
        assert_eq!(count, 1, "同一次状态变化只记一条，避免每采样刷屏");
    }

    #[test]
    fn violation_snapshots_are_bounded_but_counted() {
        let thresholds = SoakThresholds {
            max_violations: 2,
            ..SoakThresholds::default()
        };
        let mut monitor = SoakMonitor::new(thresholds, 160_000);
        monitor.observe(sample(0, stats(160_000)));
        for secs in 1..6 {
            let mut bumped = stats(160_000);
            bumped.underruns = secs as u32;
            monitor.observe(sample(secs, bumped));
        }
        assert_eq!(monitor.violations().len(), 2);
        assert_eq!(monitor.dropped_violations(), 3);
        assert_eq!(monitor.summary().verdict, "failed");
    }

    #[test]
    fn coarse_samples_are_one_per_minute_bucket() {
        let mut monitor = SoakMonitor::new(SoakThresholds::default(), 160_000);
        for secs in 0..180 {
            monitor.observe(sample(secs, stats(160_000)));
        }
        assert_eq!(monitor.coarse().len(), 3, "0/60/120 三个桶各一条");
        assert_eq!(monitor.coarse()[1].at_secs, 60);
    }

    #[test]
    fn report_json_carries_meta_summary_and_snapshots() {
        let mut monitor = SoakMonitor::new(SoakThresholds::default(), 160_000);
        monitor.observe(sample(0, stats(160_000)));
        let mut broken = stats(160_000);
        broken.underruns = 1;
        monitor.observe(sample(1, broken));

        let text = monitor.report_json(&meta());
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["tool"], "soak-runner");
        assert_eq!(parsed["planned_seconds"], 60);
        assert_eq!(parsed["summary"]["samples"], 2);
        assert_eq!(parsed["summary"]["verdict"], "failed");
        assert_eq!(parsed["violations"][0]["kind"], "underrun");
        assert_eq!(parsed["violations"][0]["stats"]["underruns"], 1);
        assert!(
            parsed["coarse"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
        );
    }

    #[test]
    fn summary_text_lists_the_first_snapshots() {
        let mut monitor = SoakMonitor::new(SoakThresholds::default(), 160_000);
        monitor.observe(sample(0, stats(160_000)));
        let mut broken = stats(160_000);
        broken.plc_count = 7;
        monitor.observe(sample(1, broken));
        let text = monitor.summary_text(&meta());
        assert!(
            text.contains("playout_concealment"),
            "摘要必须点名异常种类：{text}"
        );
        assert!(text.contains("判定 failed"));
    }
}
