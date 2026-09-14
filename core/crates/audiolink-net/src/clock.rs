//! §6 时钟同步的四时间戳估计器（契约 `docs/11-m1-contract.md` §3「时钟估计」）。
//!
//! 规格：`docs/03-protocol.md` §6 —— NTP-like 四时间戳采样、最近 200 个样本的滑动窗口、
//! 取 RTT 最小的 8 个样本的 `offset` 中位数、对窗口内 `(t_mid, offset)` 线性回归得漂移。
//!
//! # 口径一致性（刻意为之）
//!
//! 本实现与 `audiolink-tools` 的 `ClockSamples`（`latency-probe` 用的那套，M0 已在真实网络上量过）
//! **保持同一口径**：同样的「RTT 最小 8 个 + 中位数」，同样的「代表 RTT = 第 8 小的 RTT」。
//! 否则同一批样本在工具里和在会话里会给出两个不同的质量分级，遥测面板与实测报告就对不上了。
//!
//! # 本模块的边界
//!
//! **纯逻辑、不做 I/O**：探测报文的编解码（`ptype = 0x03/0x04`）属 `audiolink-proto`，
//! 收发节奏（首连 100 ms × 50 次、稳态 1 s）与会话编排属 `audiolink-engine`（§6.0/§6.1）。

use std::collections::VecDeque;

use audiolink_types::ClockQuality;

/// §6.3「RTT 最小的 8 个样本」中的 8。
pub const BEST_RTT_SAMPLES: usize = 8;

/// §6.2 的滑动窗口长度（最近 200 个样本）。
pub const WINDOW_SAMPLES: usize = 200;

/// 一次四时间戳采样的结果（§6）。
///
/// `t1`/`t4` 是本机单调 µs，`t2`/`t3` 是对端单调 µs —— 两套时钟原点不同，无法直接相减，
/// 所以只有 [`ClockSample::offset_us`]（对端 − 本机）与 [`ClockSample::rtt_us`] 是有意义的量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSample {
    /// 探测序号（对端在 `CLOCK_REPLY` 里回填）。
    pub probe_seq: u32,
    /// 本机发出探测的时刻（µs，本机单调时钟）。
    pub t1: u64,
    /// 对端收到探测的时刻（µs，对端单调时钟）。
    pub t2: u64,
    /// 对端发出应答的时刻（µs，对端单调时钟）。
    pub t3: u64,
    /// 本机收到应答的时刻（µs，本机单调时钟）。
    pub t4: u64,
    /// `((t2 - t1) + (t3 - t4)) / 2`：时钟偏移估计（对端 − 本机，µs）。
    pub offset_us: i64,
    /// `(t4 - t1) - (t3 - t2)`：往返时延（µs）。负值样例会置 0 并标记 [`ClockSample::anomalous`]。
    pub rtt_us: u64,
    /// `(t1 + t4) / 2`：漂移回归的横坐标（本机时钟中点，µs）。
    pub t_mid_us: i64,
    /// 是否异常样本（算出的 RTT 为负 ⇒ 时钟回退 / 重复应答）。
    ///
    /// 异常样本**不入窗**：负 RTT 会被「RTT 最小 8 个」排到最前面，把污染最严重的样本当成最可信的，
    /// 这与过滤的目的正好相反。处置与 `latency-probe` 一致（丢弃 + 计数）。
    pub anomalous: bool,
}

/// 当前时钟估计（§6.3 / §6.4 / §6.5 的合成结果）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockEstimate {
    /// 偏移估计（对端 − 本机，µs）：`estimate()` 返回 `None` 之外唯一可用来换算 epoch 的量。
    pub offset_us: i64,
    /// 漂移估计（ppm）：窗口内 `(t_mid, offset)` 线性回归斜率 × 1e6。
    pub drift_ppm: i32,
    /// 代表 RTT（µs）= 被选中样本里**最大**的 RTT（即第 8 小的 RTT）。
    pub rtt_us: u64,
    /// §6.5 质量分级（由代表 RTT 与样本数决定）。
    pub quality: ClockQuality,
    /// 当前窗口内的有效样本数。
    pub samples: usize,
}

/// §6 的时钟偏移 / 漂移估计器（纯逻辑，见模块文档）。
#[derive(Debug, Clone, Default)]
pub struct ClockEstimator {
    /// 滑动窗口（§6.2：最近 [`WINDOW_SAMPLES`] 个**有效**样本）。
    window: VecDeque<ClockSample>,
}

impl ClockEstimator {
    /// 空窗口。
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一次四时间戳样本，返回按 §6 公式算出的结果。
    ///
    /// 负 RTT 的样例会标 [`ClockSample::anomalous`] 并**不入窗**（但仍原样回报，便于调用方计数/日志）。
    pub fn record(&mut self, probe_seq: u32, t1: u64, t2: u64, t3: u64, t4: u64) -> ClockSample {
        let (l1, l2, l3, l4) = (as_i64(t1), as_i64(t2), as_i64(t3), as_i64(t4));

        // §6 的公式。全程 `saturating`：时间戳来自对端数据报，非法值最多得到「无效估计」，
        // 绝不能变成 panic（实时路径铁律，docs/02-architecture.md §4）。
        let offset_us = l2.saturating_sub(l1).saturating_add(l3.saturating_sub(l4)) / 2;
        let rtt_signed = l4.saturating_sub(l1).saturating_sub(l3.saturating_sub(l2));
        let anomalous = rtt_signed < 0;

        let sample = ClockSample {
            probe_seq,
            t1,
            t2,
            t3,
            t4,
            offset_us,
            rtt_us: rtt_signed.max(0) as u64,
            t_mid_us: l1.saturating_add(l4) / 2,
            anomalous,
        };

        if !anomalous {
            self.window.push_back(sample);
            if self.window.len() > WINDOW_SAMPLES {
                // 每次 record 至多多 1 个样本，所以最多超 1 个
                let _ = self.window.pop_front();
            }
        }
        sample
    }

    /// 当前估计；**有效样本 < 8 时返回 `None`**。
    ///
    /// 契约明文：样本不足时**不得**用退化值假装收敛 —— 一个基于 2 个样本的「偏移」
    /// 会被 engine 当成可用 epoch 基准，直接把组内同步搞坏。
    pub fn estimate(&self) -> Option<ClockEstimate> {
        if self.window.len() < BEST_RTT_SAMPLES {
            return None;
        }

        // §6.3：RTT 最小的 8 个样本。窗口只有 200 个，整体排序的代价可以忽略。
        let mut best: Vec<ClockSample> = self.window.iter().copied().collect();
        best.sort_by_key(|sample| sample.rtt_us);
        best.truncate(BEST_RTT_SAMPLES);

        // 代表 RTT = 被选中样本里**最大**的 RTT（= 第 8 小的 RTT），与 `ClockSamples::best_rtt_us` 同口径：
        // 单次高 RTT 抖动不该把长期质量判差（§6.5 只写「RTT ≤ 5 ms」，未指明统计量）。
        let rtt_us = best.iter().map(|sample| sample.rtt_us).max().unwrap_or(0);

        // 中位数（8 个是偶数 → 取中间两个的平均）。排序后 `b >= a`，
        // 用 `saturating_add` 既与工具实现同口径，又不会在极端输入上溢出 panic。
        let mut offsets: Vec<i64> = best.iter().map(|sample| sample.offset_us).collect();
        offsets.sort_unstable();
        let mid = offsets.len() / 2;
        let offset_us = if offsets.len().is_multiple_of(2) {
            let a = offsets.get(mid.saturating_sub(1)).copied().unwrap_or(0);
            let b = offsets.get(mid).copied().unwrap_or(0);
            a.saturating_add(b) / 2
        } else {
            offsets.get(mid).copied().unwrap_or(0)
        };

        Some(ClockEstimate {
            offset_us,
            drift_ppm: self.drift_ppm(),
            rtt_us,
            quality: ClockQuality::from_measurement(rtt_us, self.window.len()),
            samples: self.window.len(),
        })
    }

    /// 当前窗口内的有效样本数。
    pub fn samples(&self) -> usize {
        self.window.len()
    }

    /// 清空窗口（换对端 / 会话重建后必须调用：旧样本属于另一台机器的时钟）。
    pub fn clear(&mut self) {
        self.window.clear();
    }

    /// §6.4：窗口内 `(t_mid, offset)` 的线性回归斜率（µs/µs）× 1e6 = `drift_ppm`。
    ///
    /// 回归用**整个窗口**（§6.4 明文「窗口内」）而不是被 min-RTT 过滤后的 8 个：漂移需要长基线，
    /// 而 8 个低 RTT 样本可能挤在几秒内，斜率会被时序噪声吞掉。代价是一个极端离群点（如 +500 ms）
    /// 会把最小二乘直线带歪 —— 那是 §6.4 口径本身的性质，本实现按规格执行。
    fn drift_ppm(&self) -> i32 {
        if self.window.len() < 2 {
            return 0;
        }
        let count = self.window.len() as f64;
        let x_mean = self
            .window
            .iter()
            .map(|sample| sample.t_mid_us as f64)
            .sum::<f64>()
            / count;
        let y_mean = self
            .window
            .iter()
            .map(|sample| sample.offset_us as f64)
            .sum::<f64>()
            / count;

        // 先中心化再求和：`t_mid` 是「开机至今微秒」量级（1e9+），直接算 Σxy 会把有效位丢光
        // —— 斜率的信息量全在「两个大数相减的小差值」里。
        let mut sxx = 0.0_f64;
        let mut sxy = 0.0_f64;
        for sample in &self.window {
            let dx = sample.t_mid_us as f64 - x_mean;
            let dy = sample.offset_us as f64 - y_mean;
            sxx += dx * dx;
            sxy += dx * dy;
        }
        if sxx <= 0.0 {
            // 所有样本落在同一时刻：没有斜率可言（退化输入不是错误，也不该 panic）
            return 0;
        }

        let ppm = (sxy / sxx) * 1e6;
        if !ppm.is_finite() {
            return 0;
        }
        // 典型晶振 ±20 ppm（§6.4）；能超出 i32 的斜率只可能是时间戳被污染 → 夹紧而不是 panic。
        ppm.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    }
}

/// `u64` 时间戳 → `i64`（超出 `i64` 范围时饱和到 `i64::MAX`）。
///
/// §6 的差值公式必须在**有符号**空间算：`(t3 - t4)` 天然可能为负（对端时钟落后），
/// 用 `u64` 会把它下溢成天文数字，进而算出一个「看起来很像真的」的错误偏移。
fn as_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    // 测试代码不受实时路径的 unwrap / expect / panic 禁令约束（那三条针对运行时音频路径）
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// 合成样本的真值偏移（对端时钟 − 本机时钟）。
    const OFFSET_US: i64 = 12_345;

    /// 造一次「去程 = 回程 = rtt/2」的采样：真值偏移 `offset_us`、真实 RTT `rtt_us`。
    ///
    /// 校验公式：`offset = ((t2-t1) + (t3-t4))/2`、`rtt = (t4-t1) - (t3-t2)`。
    fn synth(seq: u32, t1: u64, rtt_us: u64, offset_us: i64) -> (u32, u64, u64, u64, u64) {
        let t1 = i64::try_from(t1).expect("测试时间戳");
        let rtt = i64::try_from(rtt_us).expect("测试 RTT");
        let half = rtt / 2;
        let t2 = t1 + offset_us + half;
        let t3 = t2; // §6：t3 = t2（紧随其后，不引入额外处理延迟）
        let t4 = t1 + rtt;
        (seq, t1 as u64, t2 as u64, t3 as u64, t4 as u64)
    }

    /// 把 `synth` 的结果记进估计器。
    fn record(estimator: &mut ClockEstimator, seq: u32, t1: u64, rtt_us: u64, offset_us: i64) {
        let (seq, t1, t2, t3, t4) = synth(seq, t1, rtt_us, offset_us);
        estimator.record(seq, t1, t2, t3, t4);
    }

    #[test]
    fn 公式与契约一致() {
        let mut estimator = ClockEstimator::new();
        let sample = estimator.record(7, 1_000_000, 1_012_500, 1_012_500, 1_001_000);
        // offset = ((1012500-1000000) + (1012500-1001000)) / 2 = (12500 + 11500)/2 = 12000
        assert_eq!(sample.offset_us, 12_000);
        // rtt = (1001000-1000000) - 0 = 1000
        assert_eq!(sample.rtt_us, 1_000);
        assert_eq!(sample.t_mid_us, 1_000_500);
        assert!(!sample.anomalous);
        assert_eq!(sample.probe_seq, 7);
        assert_eq!(estimator.samples(), 1);
    }

    #[test]
    fn 少于8个样本返回none() {
        let mut estimator = ClockEstimator::new();
        assert!(estimator.estimate().is_none(), "空窗口必须返回 None");

        for i in 0..BEST_RTT_SAMPLES as u32 {
            record(
                &mut estimator,
                i,
                1_000_000 + u64::from(i) * 1_000_000,
                1_000,
                OFFSET_US,
            );
            assert_eq!(estimator.samples(), i as usize + 1);
            if (i as usize) < BEST_RTT_SAMPLES - 1 {
                assert!(
                    estimator.estimate().is_none(),
                    "{} 个样本时不得返回退化估计",
                    i + 1
                );
            }
        }
        assert!(
            estimator.estimate().is_some(),
            "第 {BEST_RTT_SAMPLES} 个样本起必须给出估计"
        );
    }

    #[test]
    fn 固定偏移无漂移时误差不超过1ms() {
        let mut estimator = ClockEstimator::new();
        for i in 0..20_u32 {
            record(
                &mut estimator,
                i,
                1_000_000 + u64::from(i) * 1_000_000,
                1_000 + u64::from(i) * 10,
                OFFSET_US,
            );
        }

        let estimate = estimator.estimate().expect("20 个样本必须有估计");
        let error = (estimate.offset_us - OFFSET_US).abs();
        assert!(error <= 1_000, "offset 误差 {error} µs 超过 1 ms");
        assert_eq!(estimate.offset_us, OFFSET_US, "无抖动样本应精确复原真值");
        assert_eq!(estimate.drift_ppm, 0, "无漂移样本的回归斜率必须为 0");
        assert_eq!(estimate.samples, 20);
        assert_eq!(estimate.rtt_us, 1_070, "代表 RTT = 第 8 小的 RTT");
        assert_eq!(estimate.quality, ClockQuality::Good);
    }

    #[test]
    fn 注入500ms异常样本后中位数不被带偏() {
        let mut estimator = ClockEstimator::new();
        // 19 个正常样本：RTT 2010…2190 µs，offset 恒为真值
        for i in 1..20_u32 {
            record(
                &mut estimator,
                i,
                1_000_000 + u64::from(i) * 1_000_000,
                2_000 + u64::from(i) * 10,
                OFFSET_US,
            );
        }
        // 异常样本：offset 偏 +500 ms，且 RTT 最小（500 µs）→ **一定会**被判据选进 best-8。
        // 这是刻意构造的最坏情况：光靠最小 RTT 过滤救不了，只有中位数能救。
        let (seq, t1, t2, t3, t4) = synth(0, 1_000_000, 500, OFFSET_US + 500_000);
        let anomaly = estimator.record(seq, t1, t2, t3, t4);
        assert!(!anomaly.anomalous, "500 ms 偏移不是「负 RTT」类异常");
        assert_eq!(anomaly.offset_us, OFFSET_US + 500_000);
        assert_eq!(anomaly.rtt_us, 500, "该样本必须是最小 RTT 才构成最坏情况");

        let estimate = estimator.estimate().expect("20 个样本必须有估计");
        assert_eq!(estimate.samples, 20);
        let error = (estimate.offset_us - OFFSET_US).abs();
        assert!(
            error <= 1_000,
            "1/8 的异常样本不得动摇中位数，实得 offset={} µs（真值 {OFFSET_US}）",
            estimate.offset_us
        );

        // 漂移回归用的是整个窗口（§6.4 明文），一个 500 ms 的极端离群点会把最小二乘直线带歪
        // —— 这是 §6.4 口径本身的性质（真实场景收敛后 offset 抖动 ≤ 2 ms，不会出现这种输入）。
        // 这里只钉住「没有变成 NaN / 溢出垃圾」，把敏感性记录在案。
        assert!(
            estimate.drift_ppm.abs() < 1_000_000,
            "漂移估计必须是有限值，实得 {} ppm",
            estimate.drift_ppm
        );
    }

    #[test]
    fn 合成漂移被回归正确识别() {
        // 100 ppm ≈ 每 1 s 偏 100 µs
        let mut estimator = ClockEstimator::new();
        for i in 0..40_i64 {
            let t1 = u64::try_from(1_000_000 + i * 1_000_000).expect("测试时间戳");
            record(
                &mut estimator,
                u32::try_from(i).expect("序号"),
                t1,
                2_000,
                OFFSET_US + i * 100,
            );
        }
        let estimate = estimator.estimate().expect("40 个样本必须有估计");
        assert!(
            (estimate.drift_ppm - 100).abs() <= 5,
            "漂移估计 {} ppm，期望 ≈ 100 ppm",
            estimate.drift_ppm
        );
    }

    #[test]
    fn 窗口只保留最近200个样本且clear可清空() {
        let mut estimator = ClockEstimator::new();
        let total = WINDOW_SAMPLES + 25;
        for i in 0..total {
            record(
                &mut estimator,
                u32::try_from(i).expect("序号"),
                1_000_000 + u64::try_from(i).expect("时间戳") * 1_000,
                1_000,
                OFFSET_US,
            );
        }
        assert_eq!(estimator.samples(), WINDOW_SAMPLES, "窗口必须封顶在 200");

        estimator.clear();
        assert_eq!(estimator.samples(), 0);
        assert!(estimator.estimate().is_none(), "清空后不得残留估计");
    }

    #[test]
    fn 负rtt样本不入窗但仍回报() {
        let mut estimator = ClockEstimator::new();
        // t4 < t1（本机时钟回退）：rtt 算出负数
        let sample = estimator.record(1, 5_000_000, 1_000, 1_000, 4_000_000);
        assert!(sample.anomalous);
        assert_eq!(sample.rtt_us, 0);
        assert_eq!(estimator.samples(), 0, "异常样本不得进入窗口");
    }

    #[test]
    fn 极端时间戳不panic() {
        let mut estimator = ClockEstimator::new();
        // 全 u64 极值：必须得到饱和后的结果，而不是溢出 panic
        let sample = estimator.record(0, u64::MAX, 0, u64::MAX, 0);
        // t_mid = saturating_add(i64::MAX, 0) / 2 —— 饱和只发生在加法与差值上，除法照常
        assert_eq!(sample.t_mid_us, i64::MAX / 2);
        assert!(sample.anomalous, "u64 极值必然算出负 RTT");
        assert_eq!(estimator.samples(), 0);
        let _ = estimator.estimate();
    }
}
