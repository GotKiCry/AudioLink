//! 连续静音（「长断音」）统计 —— M2 待决口径第 ④ 条的实现缺口
//!
//! # 它解决什么问题
//!
//! M2 的验收原文是「2% 随机丢包下可懂、**无长断音**」（`docs/01-requirements.md` NFR-03），
//! 而**「长断音」此前判不了**：现有遥测只有累计计数，且接收侧 1 Hz 才发布一次快照，从
//! 「一秒内 k 拍静音」最多只能推出**下界** ⌈k ÷ (B − k + 1)⌉ 拍（`docs/22-m2-soak-runner.md` §11.5）。
//! 「静音总占比」与「最长连续静音」是两件事：总占比 1% 可能是一次 600 ms 的静音，
//! 也可能是分散的 30 个散拍 —— 前者用户听得出「断了」，后者听不出。
//!
//! 本模块把连续性指标**直接量出来**：在播放线程里逐拍观察「写出去的这一拍有多响」，
//! 维护当前连续静音段的长度，并在段结束时产出一段账。
//!
//! # 三个刻意的口径决定
//!
//! 1. **观察的是「本路写出去的拍」**，不是 sink 的实际输出波形：播放线程每拍要么写真实帧
//!    （`DueFrame::Ready`），要么补静音（排播等待 / 过期丢弃 / 升档 Hold / 欠载）。四种
//!    补静音各有既有计数，本模块**不重复记它们**，只回答「这一路连续多久没有真声音」。
//!    `synthetic` 标志把两者分开：`content_ms` 是**有流但内容是静音**（采集端没人说话、
//!    上游静音），`synthetic_ms` 是**引擎自己补的**（欠载 / 等待 / 丢弃）。
//!    这正是现有遥测的盲区所在 —— 后者有计数，前者没有。
//! 2. **漏拍的持续时间不计入**：播放线程被抢占而跨过的拍（`missed`）没有任何「这一拍写了什么」
//!    的事实，它们已由 `record_underruns` 记账。把不确定的东西算进连续性指标会让数字失去意义。
//! 3. **只在段结束时上报事件，进行中的段走快照**：事件总线是广播式的、有容量上限，
//!    一个持续数分钟的静音若按拍 / 按秒发事件会把 UI 真正关心的事件挤掉；而「现在正在静音」
//!    这件事由 1 Hz 快照的 `current_ms` 回答更自然（外壳每 500 ms 轮询一次遥测）。
//!
//! # 阈值是暂定值
//!
//! `MIN_SILENCE_MS` 与 `SILENCE_PEAK_THRESHOLD` 是**工程默认值**，不是产品定案：
//! 「连续静音多久算断音」属待决口径（`docs/22` §11 第 ② 条），产品确认前不写进验收表。
//! 因此两者都是构造参数，测试与工具可以按需覆盖。

/// 一拍 PCM 的峰值低于它就视为静音。
///
/// 取 −80 dBFS（1e-4）：48 kHz / f32 全链路下，这是「听不见」与「很轻」的公认分界，
/// 也是本项目既有的样本统计口径（`audiolink_audio::SampleStats`）同一量级。
pub const SILENCE_PEAK_THRESHOLD: f32 = 1.0e-4;

/// 连续静音达到这一长度才算「一段」并上报（毫秒）。
///
/// 200 ms 的依据：抖动缓冲的目标深度上限是 60 ms（3 帧 × 20 ms），排播等待与升档 Hold
/// 产生的静音都在单拍量级；而**用户能察觉的「断音」**远大于此。低于它的静音是正常抖动，
/// 报出来只会淹没真信号。
pub const MIN_SILENCE_MS: u64 = 200;

/// 一段连续静音完成后产出的账。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SilenceSegment {
    /// 段起始时刻（会话起播以来的毫秒）。
    pub start_ms: u64,
    /// 段长度（毫秒）。
    pub duration_ms: u64,
    /// 其中「真实帧内容本身静音」的毫秒数。
    pub content_ms: u64,
    /// 其中「引擎补静音」的毫秒数。
    pub synthetic_ms: u64,
    /// 段内观察到的最大峰值（**低于门限**；给出它是为了核对判定的边界情形）。
    pub peak: f32,
}

/// 进程内可读的连续静音快照（不随 `StreamStats` 出网，理由同 `depth_drops`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SilenceSnapshot {
    /// 观察过的总拍时长（毫秒）—— 静音占比的分母。
    pub planned_ms: u64,
    /// 静音总时长（毫秒，含进行中的段）。
    pub silent_ms: u64,
    /// 历史最长连续静音（毫秒，含进行中的段）。
    pub longest_ms: u64,
    /// 当前进行中的连续静音（毫秒；0 = 此刻不是静音）。
    pub current_ms: u64,
    /// 已结算的达标段数。
    pub segments: u32,
}

#[derive(Debug, Clone, Copy)]
struct Run {
    start_ms: u64,
    duration_ms: u64,
    content_ms: u64,
    synthetic_ms: u64,
    peak: f32,
}

/// 逐拍喂入的连续静音统计器。
///
/// 实时路径约束：`observe` 不做分配、不加锁、不 panic（饱和算术）。
#[derive(Debug)]
pub struct SilenceTracker {
    peak_threshold: f32,
    min_ms: u64,
    elapsed_ms: u64,
    planned_ms: u64,
    silent_ms: u64,
    longest_ms: u64,
    segments: u32,
    run: Option<Run>,
}

impl Default for SilenceTracker {
    fn default() -> Self {
        Self::new(SILENCE_PEAK_THRESHOLD, MIN_SILENCE_MS)
    }
}

impl SilenceTracker {
    /// 按给定门限与最短段长新建。
    pub fn new(peak_threshold: f32, min_ms: u64) -> Self {
        Self {
            peak_threshold,
            min_ms,
            elapsed_ms: 0,
            planned_ms: 0,
            silent_ms: 0,
            longest_ms: 0,
            segments: 0,
            run: None,
        }
    }

    /// 观察一拍：`peak` 是这一拍写出去的 PCM 峰值，`frame_ms` 是拍长，`synthetic` 表示这一拍
    /// 是引擎补的静音（而非真实帧内容）。
    ///
    /// 返回 `Some(段)` **仅当**这一拍终止了一段已达标的连续静音。
    pub fn observe(&mut self, peak: f32, frame_ms: u32, synthetic: bool) -> Option<SilenceSegment> {
        let frame_ms = u64::from(frame_ms);
        let start_ms = self.elapsed_ms;
        self.elapsed_ms = self.elapsed_ms.saturating_add(frame_ms);
        self.planned_ms = self.planned_ms.saturating_add(frame_ms);

        // NaN / 无穷不算静音：那是数据异常，不该被「静音」这个类别掩盖。
        let is_silent = peak.is_finite() && peak < self.peak_threshold;

        if !is_silent {
            return self.close_run();
        }

        self.silent_ms = self.silent_ms.saturating_add(frame_ms);
        let run = self.run.get_or_insert(Run {
            start_ms,
            duration_ms: 0,
            content_ms: 0,
            synthetic_ms: 0,
            peak: 0.0,
        });
        run.duration_ms = run.duration_ms.saturating_add(frame_ms);
        if synthetic {
            run.synthetic_ms = run.synthetic_ms.saturating_add(frame_ms);
        } else {
            run.content_ms = run.content_ms.saturating_add(frame_ms);
        }
        // 段内峰值取上界：门限判定已保证每拍都低于它，这里是留给边界核对的观测值。
        if peak > run.peak {
            run.peak = peak;
        }
        if run.duration_ms > self.longest_ms {
            self.longest_ms = run.duration_ms;
        }
        None
    }

    /// 结算进行中的段（会话关闭 / 关流时调用），把未收尾的静音也纳入账本。
    pub fn finish(&mut self) -> Option<SilenceSegment> {
        self.close_run()
    }

    fn close_run(&mut self) -> Option<SilenceSegment> {
        let run = self.run.take()?;
        if run.duration_ms < self.min_ms {
            return None;
        }
        self.segments = self.segments.saturating_add(1);
        Some(SilenceSegment {
            start_ms: run.start_ms,
            duration_ms: run.duration_ms,
            content_ms: run.content_ms,
            synthetic_ms: run.synthetic_ms,
            peak: run.peak,
        })
    }

    /// 当前快照（进行中的段也计入 `longest_ms` / `current_ms`）。
    pub fn snapshot(&self) -> SilenceSnapshot {
        SilenceSnapshot {
            planned_ms: self.planned_ms,
            silent_ms: self.silent_ms,
            longest_ms: self.longest_ms,
            current_ms: self.run.map_or(0, |r| r.duration_ms),
            segments: self.segments,
        }
    }

    /// 清空所有统计（会话重建时调用）。
    pub fn reset(&mut self) {
        self.elapsed_ms = 0;
        self.planned_ms = 0;
        self.silent_ms = 0;
        self.longest_ms = 0;
        self.segments = 0;
        self.run = None;
    }
}

/// 计算一拍交错 PCM 的峰值（f32 绝对值最大者）。
///
/// 实时路径上每拍调用一次（20 ms 帧 = 1920 个样本），成本远低于一次解码；只做比较与取绝对值，
/// 不分配、不改变样本。空切片返回 0（「没有样本」在静音口径下就是无声）。
pub fn frame_peak(samples: &[f32]) -> f32 {
    let mut peak = 0.0f32;
    for &sample in samples {
        let magnitude = sample.abs();
        if magnitude > peak {
            peak = magnitude;
        }
    }
    peak
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const FRAME: u32 = 20;

    #[test]
    fn a_short_dip_is_not_a_segment() {
        // 抖动缓冲与排播等待产生的单拍静音是正常行为，不能报成「断音」。
        let mut tracker = SilenceTracker::default();
        for _ in 0..5 {
            // 5 × 20 ms = 100 ms < 200 ms
            assert!(tracker.observe(0.0, FRAME, true).is_none());
        }
        assert!(tracker.observe(0.5, FRAME, false).is_none());
        assert_eq!(tracker.snapshot().segments, 0);
        assert_eq!(tracker.snapshot().longest_ms, 100, "短段仍计入最长值");
    }

    #[test]
    fn a_long_silence_run_is_reported_when_it_ends() {
        let mut tracker = SilenceTracker::default();
        // 1 s 真实内容静音（有流，但内容是静音）—— 现有遥测的这一半盲区。
        for _ in 0..50 {
            assert!(
                tracker.observe(0.0, FRAME, false).is_none(),
                "进行中的段不发事件"
            );
        }
        let segment = tracker
            .observe(0.8, FRAME, false)
            .expect("段结束应当产出账");
        assert_eq!(segment.duration_ms, 1_000);
        assert_eq!(segment.content_ms, 1_000);
        assert_eq!(segment.synthetic_ms, 0);
        assert_eq!(segment.start_ms, 0);
        assert_eq!(tracker.snapshot().segments, 1);
        assert_eq!(
            tracker.snapshot().current_ms,
            0,
            "段结束后不再有进行中的静音"
        );
        assert_eq!(tracker.snapshot().longest_ms, 1_000);
    }

    #[test]
    fn content_and_synthetic_silence_are_booked_apart() {
        // 「引擎自己补的」与「有流但内容是静音」成因完全不同，必须分开给数。
        let mut tracker = SilenceTracker::default();
        for _ in 0..10 {
            assert!(tracker.observe(0.0, FRAME, true).is_none()); // 200 ms 补静音
        }
        for _ in 0..10 {
            assert!(tracker.observe(0.0, FRAME, false).is_none()); // 200 ms 内容静音
        }
        let segment = tracker.observe(0.9, FRAME, false).unwrap();
        assert_eq!(segment.duration_ms, 400);
        assert_eq!(segment.synthetic_ms, 200);
        assert_eq!(segment.content_ms, 200);
    }

    #[test]
    fn in_progress_silence_is_visible_in_the_snapshot() {
        // 只有段结束时才发事件，所以「现在正在静音」必须能从快照读到 —— 否则持续数分钟的
        // 静音在它结束前完全不可观测（那正是「用户听出断音、遥测一片正常」的场景）。
        let mut tracker = SilenceTracker::default();
        for _ in 0..150 {
            // 3 s
            tracker.observe(0.0, FRAME, false);
        }
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.current_ms, 3_000);
        assert_eq!(snapshot.longest_ms, 3_000, "进行中的段也计入最长值");
        assert_eq!(snapshot.silent_ms, 3_000);
        assert_eq!(snapshot.planned_ms, 3_000);
        assert_eq!(snapshot.segments, 0, "尚未结束，段数不增");
    }

    #[test]
    fn the_peak_threshold_decides_the_boundary() {
        let mut tracker = SilenceTracker::new(1.0e-4, MIN_SILENCE_MS);
        for _ in 0..12 {
            tracker.observe(0.0, FRAME, false);
        }
        // 恰好等于门限：不算静音（判定用严格小于），于是这一下就收尾了上面那段。
        let segment = tracker.observe(1.0e-4, FRAME, false).expect("应当收尾");
        assert_eq!(segment.duration_ms, 240);

        // 略低于门限：仍算静音，段继续。
        let mut tracker = SilenceTracker::new(1.0e-4, MIN_SILENCE_MS);
        for _ in 0..12 {
            tracker.observe(0.0, FRAME, false);
        }
        assert!(tracker.observe(9.9e-5, FRAME, false).is_none());
        assert_eq!(tracker.snapshot().current_ms, 260);
    }

    #[test]
    fn non_finite_peaks_are_not_treated_as_silence() {
        // NaN / inf 是数据异常，不能被「静音」这一类掩盖掉。
        let mut tracker = SilenceTracker::default();
        for _ in 0..20 {
            tracker.observe(0.0, FRAME, false);
        }
        assert!(
            tracker.observe(f32::NAN, FRAME, false).is_some(),
            "NaN 必须打断静音段（否则异常数据会伪装成静音）"
        );
        assert_eq!(tracker.snapshot().current_ms, 0);

        let mut tracker2 = SilenceTracker::default();
        for _ in 0..20 {
            tracker2.observe(0.0, FRAME, false);
        }
        assert!(tracker2.observe(f32::INFINITY, FRAME, false).is_some());
    }

    #[test]
    fn several_runs_accumulate_into_the_longest_one() {
        let mut tracker = SilenceTracker::default();
        // 段 1：300 ms
        for _ in 0..15 {
            tracker.observe(0.0, FRAME, false);
        }
        tracker.observe(0.5, FRAME, false);
        // 段 2：600 ms（最长）
        for _ in 0..30 {
            tracker.observe(0.0, FRAME, false);
        }
        tracker.observe(0.5, FRAME, false);
        // 段 3：260 ms
        for _ in 0..13 {
            tracker.observe(0.0, FRAME, true);
        }
        tracker.finish().unwrap();

        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.segments, 3);
        assert_eq!(snapshot.longest_ms, 600);
        assert_eq!(snapshot.silent_ms, 300 + 600 + 260);
        assert_eq!(snapshot.planned_ms, 15 * 20 + 20 + 30 * 20 + 20 + 13 * 20);
    }

    #[test]
    fn finish_closes_an_open_run_for_the_ledger() {
        // 关流时正在静音：不结算就会漏掉最后一段（长跑被判「无静音」的典型漏报）。
        let mut tracker = SilenceTracker::default();
        for _ in 0..25 {
            tracker.observe(0.0, FRAME, false);
        }
        let segment = tracker.finish().expect("关流必须结算进行中的段");
        assert_eq!(segment.duration_ms, 500);
        assert!(tracker.finish().is_none(), "重复结算不产生第二段");
        assert_eq!(tracker.snapshot().segments, 1);
    }

    #[test]
    fn reset_clears_every_field() {
        let mut tracker = SilenceTracker::default();
        for _ in 0..50 {
            tracker.observe(0.0, FRAME, false);
        }
        tracker.observe(1.0, FRAME, false);
        tracker.reset();
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.planned_ms, 0);
        assert_eq!(snapshot.silent_ms, 0);
        assert_eq!(snapshot.longest_ms, 0);
        assert_eq!(snapshot.current_ms, 0);
        assert_eq!(snapshot.segments, 0);
    }

    #[test]
    fn absurd_inputs_saturate_instead_of_panicking() {
        let mut tracker = SilenceTracker::default();
        for _ in 0..1_000 {
            tracker.observe(0.0, u32::MAX, false);
        }
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.planned_ms, u64::from(u32::MAX) * 1_000);
        assert!(snapshot.longest_ms >= snapshot.current_ms);
    }

    #[test]
    fn frame_peak_reports_the_loudest_sample() {
        assert_eq!(frame_peak(&[]), 0.0, "空帧就是无声");
        assert_eq!(frame_peak(&[0.0, -0.75, 0.5, 0.25]), 0.75);
        assert_eq!(frame_peak(&[-1.0, 1.0]), 1.0);
        assert_eq!(frame_peak(&[1.0e-9, -2.0e-9]), 2.0e-9);
    }
}
