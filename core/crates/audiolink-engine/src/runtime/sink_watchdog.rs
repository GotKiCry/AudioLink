//! FR-28 播放看门狗：**链路还在给音频，设备却 ≥ 2 s 一帧都没接走** → 重建播放器。
//!
//! # 它修的是什么
//!
//! 旧版的「一次异常永久静音」（`docs/07-migration-notes.md`）：播放器某次异常之后就再也不出声，
//! 而引擎侧「提交成功」照旧 —— 状态在握、没有报错、音频永远回不来。所以需要一个**独立的观察面**：
//! 链路正常（还在给真实音频帧）而设备连续 2 s 没有接走任何一帧真实音频，就换一个 sink
//! （`docs/01-requirements.md` FR-28、`docs/02-architecture.md` §看门狗）。
//!
//! # 「成功输出」怎么判
//!
//! 引擎能看到的只有 sink 自己的账本（`PlayoutStats::frames_written`）：
//!
//! - `write()` 返回 `Err`（设备掉线 / 缓冲抽干）→ 账本不增长；
//! - `write()` 返回 `Ok` 却没把数据接走（「假成功」的坏 sink）→ 账本也不增长。
//!
//! 两种故障在「账本有没有增长」这一个口径下是**同一种事实**，因此判据只有一条：
//! **真实音频那一拍的账本增量必须 > 0**。补静音、升档 Hold、排播等待这些拍不算 ——
//! 「设备一直在放静音」正是要修的故障，不能拿它当健康证据。
//!
//! # 阈值与「不误伤」
//!
//! 窗口是需求写死的 **2 s**，且两个条件必须同时成立：
//!
//! 1. 距上一次**成功输出真实音频** ≥ 2 s；
//! 2. 这 2 s 内**链路给过真实帧** —— 否则那是断流，该由 FR-27 重连负责，不是 sink 的锅。
//!
//! 正常链路上每 20 ms 就成功输出一次（2 s = 100 次反例），**任何一次成功都会把计时清空**，
//! 所以偶发欠载、单次写失败、250 ms 的调度停顿都不会触发（对照见
//! `tests/engine/playout_recovery.rs`）。
//!
//! 重建失败（工厂也坏了）时**指数退避**（2 s → 4 s → … → 30 s）而不是放弃：
//! 放弃就等于把「一次异常永久静音」换成「一次异常永久不看门」，那正是这条需求要根治的。

use std::time::{Duration, Instant};

/// 「无输出」判定窗口：FR-28 与架构 §看门狗都写死 2 s。
pub(super) const OUTPUT_STALL_WINDOW: Duration = Duration::from_secs(2);

/// 重建失败后的退避起点（= 一个完整窗口：先让新 sink 有机会证明自己）。
const REBUILD_BACKOFF_START: Duration = Duration::from_secs(2);

/// 重建失败后的退避上限（设备被拔掉时不该每 2 s 就刷一次建 sink）。
const REBUILD_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// 触发重建的事实（写进事件 context，也供单测断言）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OutputStall {
    /// 距上一次成功输出真实音频多久。
    pub(super) silent_for: Duration,
    /// 这段时间里链路仍在给真实帧（`poll` 只在为真时才返回 `Some`）。
    pub(super) link_fed_frames: bool,
}

/// FR-28 的播放看门狗状态。
#[derive(Debug)]
pub(super) struct SinkWatchdog {
    /// 上一次**成功输出真实音频**的时刻。
    last_output: Instant,
    /// 上一次链路送来真实帧的时刻（判断「链路正常」）。
    last_ready: Instant,
    /// 下一次允许重建的时刻（退避用）。
    next_attempt: Instant,
    /// 当前退避时长。
    backoff: Duration,
    /// 成功重建次数（测试与事件用）。
    rebuilds: u32,
    /// 重建尝试次数（含失败）。
    attempts: u32,
}

impl SinkWatchdog {
    /// 新建；`now` 通常是开始播放的时刻（起播前不该触发）。
    pub(super) fn new(now: Instant) -> Self {
        Self {
            last_output: now,
            last_ready: now,
            next_attempt: now,
            backoff: REBUILD_BACKOFF_START,
            rebuilds: 0,
            attempts: 0,
        }
    }

    /// 链路送来一帧真实音频（`DueFrame::Ready`）。
    pub(super) fn note_ready_frame(&mut self, now: Instant) {
        self.last_ready = now;
    }

    /// 真实音频这一拍提交完了：`frames_delta` 是 sink 账本在这一拍的增长。
    ///
    /// `0` = 这一帧没被真的接走（写失败，或 sink 收下了却不记账）。
    pub(super) fn note_audible_submit(&mut self, now: Instant, frames_delta: u64) {
        if frames_delta > 0 {
            self.last_output = now;
        }
    }

    /// 到点该重建了吗？返回 `Some` 即调用方应当重建 sink。
    pub(super) fn poll(&mut self, now: Instant) -> Option<OutputStall> {
        if now < self.next_attempt {
            return None;
        }
        let silent_for = now.saturating_duration_since(self.last_output);
        if silent_for < OUTPUT_STALL_WINDOW {
            return None;
        }
        // 链路也得是好的：这几秒里必须真的到过音频帧。
        // 否则「没输出」的解释是断流（§FR-27 的活），重建 sink 治不了它，还会白扔一个设备。
        if now.saturating_duration_since(self.last_ready) >= OUTPUT_STALL_WINDOW {
            return None;
        }
        Some(OutputStall {
            silent_for,
            link_fed_frames: true,
        })
    }

    /// 记录一次重建尝试的结果：成功就重新开始计时，失败则指数退避。
    pub(super) fn note_rebuild_attempt(&mut self, now: Instant, rebuilt: bool) {
        self.attempts = self.attempts.saturating_add(1);
        if rebuilt {
            self.rebuilds = self.rebuilds.saturating_add(1);
            self.backoff = REBUILD_BACKOFF_START;
            // 新 sink 从零开始：给它一个完整窗口证明自己，否则刚重建就又被判死。
            self.last_output = now;
            self.next_attempt = now + OUTPUT_STALL_WINDOW;
        } else {
            // 第一次失败只等一个窗口（设备可能只是被拔了一下，2 s 后就好了），
            // 再失败才翻倍 —— 顺序反了的话第一次就要等 4 s，白等。
            self.next_attempt = now + self.backoff;
            self.backoff = (self.backoff * 2).min(REBUILD_BACKOFF_MAX);
        }
    }

    /// 成功重建次数。
    pub(super) const fn rebuilds(&self) -> u32 {
        self.rebuilds
    }

    /// 重建尝试次数（含失败）。
    pub(super) const fn attempts(&self) -> u32 {
        self.attempts
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// 模拟「每 20 ms 一拍」的播放线程：`frames_delta` 由闭包给定。
    fn run(
        dog: &mut SinkWatchdog,
        start: Instant,
        for_secs: u64,
        mut delta: impl FnMut(usize) -> u64,
    ) -> Option<OutputStall> {
        let ticks = for_secs * 50;
        for tick in 0..ticks {
            let now = start + Duration::from_millis(20 * tick);
            dog.note_ready_frame(now);
            dog.note_audible_submit(now, delta(tick as usize));
            if let Some(stall) = dog.poll(now) {
                return Some(stall);
            }
        }
        None
    }

    #[test]
    fn healthy_playout_never_triggers_a_rebuild() {
        let start = Instant::now();
        let mut dog = SinkWatchdog::new(start);
        assert_eq!(run(&mut dog, start, 10, |_| 960), None, "每拍都在真的输出");
        assert_eq!(dog.rebuilds(), 0);
    }

    #[test]
    fn a_full_two_second_silence_with_a_live_link_rebuilds_once() {
        let start = Instant::now();
        let mut dog = SinkWatchdog::new(start);
        // 前 0.5 s 正常，然后设备再也不接数据（链路照旧给帧）。
        let stall = run(&mut dog, start, 10, |tick| if tick < 25 { 960 } else { 0 });
        let stall = stall.expect("2 s 无输出必须触发重建");
        assert!(stall.silent_for >= OUTPUT_STALL_WINDOW);
        assert!(stall.link_fed_frames);
    }

    #[test]
    fn a_short_write_failure_is_forgiven() {
        let start = Instant::now();
        let mut dog = SinkWatchdog::new(start);
        // 前 1.5 s 每一拍都失败，然后恢复一次成功：计时从那一刻重新开始，
        // 于是之后 1.5 s（< 2 s 窗口）都不该触发 —— 这就是「不误伤」的机制。
        let stall = run(&mut dog, start, 3, |tick| if tick == 75 { 960 } else { 0 });
        assert_eq!(stall, None, "任何一次成功输出都要把 2 s 计时清空");
    }

    #[test]
    fn a_dead_link_is_not_blamed_on_the_sink() {
        let start = Instant::now();
        let mut dog = SinkWatchdog::new(start);
        // 链路也停了（没有 Ready 帧）、输出也没有：这是断流，不是 sink 故障。
        let mut stall = None;
        for tick in 0..250 {
            let now = start + Duration::from_millis(20 * tick);
            dog.note_audible_submit(now, 0);
            stall = dog.poll(now);
            assert_eq!(stall, None, "链路没给帧时不许重建 sink");
        }
        assert_eq!(stall, None);
    }

    #[test]
    fn failed_rebuilds_back_off_instead_of_hammering_the_device() {
        let start = Instant::now();
        let mut dog = SinkWatchdog::new(start);
        let stall = run(&mut dog, start, 3, |_| 0).expect("先触发一次");
        assert!(stall.silent_for >= OUTPUT_STALL_WINDOW);

        let now = start + Duration::from_secs(3);
        // 播放线程每拍都会 note_ready_frame；手动推进时间时也要一起模拟，
        // 否则会被「链路也不新鲜」那个条件否决（那是另一条判据，不是这里要测的）。
        dog.note_ready_frame(now);
        dog.note_rebuild_attempt(now, false);
        assert_eq!(dog.attempts(), 1);
        assert_eq!(dog.rebuilds(), 0);
        assert_eq!(
            dog.poll(now + Duration::from_secs(1)),
            None,
            "退避期内不重试"
        );

        // 退避翻倍：第一次失败后 2 s 可重试，再失败就退到 4 s。
        let retry = now + Duration::from_secs(2);
        dog.note_ready_frame(retry);
        assert!(
            dog.poll(retry).is_some(),
            "退避到期后允许再试（设备可能只是被拔了一下）"
        );
        dog.note_rebuild_attempt(retry, false);
        let later = retry + Duration::from_secs(3);
        dog.note_ready_frame(later);
        assert_eq!(dog.poll(later), None, "第二次失败后要等更久");
    }

    #[test]
    fn a_successful_rebuild_restarts_the_clock() {
        let start = Instant::now();
        let mut dog = SinkWatchdog::new(start);
        run(&mut dog, start, 3, |_| 0).expect("先触发一次");

        let now = start + Duration::from_secs(3);
        dog.note_ready_frame(now);
        dog.note_rebuild_attempt(now, true);
        assert_eq!(dog.rebuilds(), 1);
        assert_eq!(
            dog.poll(now + Duration::from_secs(1)),
            None,
            "新 sink 有一个完整窗口"
        );
        // 新 sink 也不干活 → 再判一次（链路照旧在给帧）。
        let later = now + Duration::from_secs(3);
        dog.note_ready_frame(later);
        assert!(dog.poll(later).is_some());
    }
}
