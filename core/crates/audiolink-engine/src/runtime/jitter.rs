//! M2 接收抖动控制：20--60 ms 目标深度与有界包重排。
//!
//! # 两个「帧数」不再共用一个常量（第 114 轮拆分）
//!
//! 这里有两个语义不同的量，曾经共用一个 `DEFAULT_TARGET_FRAMES`，于是「想调起步延迟」就得
//! 连带改「中等抖动该用几帧」：
//!
//! * [`INITIAL_TARGET_FRAMES`] = **起步档**：会话刚起播时攒几帧才出声（= `MIN_TARGET_FRAMES`，
//!   20 ms @20 ms 帧）。它是**出声延迟**的直接来源 —— 多攒一帧就多一帧的延迟。
//! * [`DEFAULT_TARGET_FRAMES`] = **中等抖动档**：`observe()` 在「抖动 ≤ 一帧」时选用的深度
//!   （40 ms）。它是**抗抖动保护**的一部分，由 M2 的口径决定，不该被起步延迟的需求牵动。
//!
//! 实测（本机）：起步档 2 帧 → 1 帧后，回环 P50 53 872 → 25 965 / 27 259 µs（两次）、
//! 接收侧水位 40 000 → 20 000 µs。
//!
//! ⚠️ **弱网代价在本机这个口径上判不出来，别把噪声当结论**：`soak-runner --tolerant` +
//! netem 2% / 15±15 ms 下 —— 300 s 档改前 20 拍 / 改后 23 拍；25 s 档两次重复分别是
//! 2 & 7 拍（改前）与 7 & 7 拍（改后）⇒ **区间重叠**，25 s 档自身的噪声就有 2~7 拍。
//! 要判「起步少攒一帧是否更易欠载」，得用更长档 + 多次重复，或者在真机弱网上量
//! （本机回环零丢包零抖动，本来就不覆盖这条路径）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub(super) const MIN_TARGET_FRAMES: usize = 1;
/// **起步档**：会话起播时攒够这么多帧才出声（20 ms @20 ms 帧）。
///
/// 与 [`DEFAULT_TARGET_FRAMES`] 分开：起步帧数是**延迟**，中等抖动档是**抗抖动保护**。
/// 详见模块文档。
pub(super) const INITIAL_TARGET_FRAMES: usize = MIN_TARGET_FRAMES;
/// **中等抖动档**：`observe()` 在「抖动 ≤ 一帧」时选用的深度（40 ms @20 ms 帧）。
///
/// 由 M2 的抗抖动口径决定 —— 不要拿起步延迟的需求去改它（那正是第 114 轮拆分的理由）。
pub(super) const DEFAULT_TARGET_FRAMES: usize = 2;
pub(super) const MAX_TARGET_FRAMES: usize = 3;
const STABLE_WINDOWS_TO_SHRINK: u32 = 30;
const REORDER_CAPACITY: usize = 3;
const DISCONTINUITY_FRAMES: u32 = 1_000;

/// 1 Hz 更新的目标深度控制器。
///
/// 升档立即生效；降档要连续稳定 30 个窗口且每次只降一档，避免在阈值附近反复重缓冲。
#[derive(Debug)]
pub(super) struct AdaptiveJitterDepth {
    target_frames: usize,
    stable_windows: u32,
}

impl AdaptiveJitterDepth {
    pub(super) const fn new() -> Self {
        Self {
            // 起步 = 最小档：出声延迟从这里开始（第 114 轮：与中等抖动档拆开）。
            target_frames: INITIAL_TARGET_FRAMES,
            stable_windows: 0,
        }
    }

    pub(super) fn observe(
        &mut self,
        jitter_p95_us: Option<u32>,
        underruns: u32,
        frame_us: u32,
    ) -> usize {
        let frame_us = frame_us.max(1);
        let measured = jitter_p95_us.map(|jitter| {
            if jitter <= frame_us / 4 {
                MIN_TARGET_FRAMES
            } else if jitter <= frame_us {
                DEFAULT_TARGET_FRAMES
            } else {
                MAX_TARGET_FRAMES
            }
        });

        let mut wanted = measured.unwrap_or(self.target_frames);
        if underruns > 0 {
            wanted = wanted.max((self.target_frames + 1).min(MAX_TARGET_FRAMES));
        }

        if wanted > self.target_frames {
            self.target_frames = wanted.min(MAX_TARGET_FRAMES);
            self.stable_windows = 0;
        } else if wanted < self.target_frames && underruns == 0 && measured.is_some() {
            self.stable_windows = self.stable_windows.saturating_add(1);
            if self.stable_windows >= STABLE_WINDOWS_TO_SHRINK {
                self.target_frames = self.target_frames.saturating_sub(1).max(wanted);
                self.stable_windows = 0;
            }
        } else {
            self.stable_windows = 0;
        }

        self.target_frames
    }

    /// 接收播放线程的即时升档，防止 1 Hz 控制器随后用旧值把它覆盖。
    pub(super) fn raise_to(&mut self, target_frames: usize) {
        let target_frames = target_frames.clamp(MIN_TARGET_FRAMES, MAX_TARGET_FRAMES);
        if target_frames > self.target_frames {
            self.target_frames = target_frames;
            self.stable_windows = 0;
        }
    }
}

/// 仅当共享目标仍是控制器观察到的旧值时发布决策。
///
/// 若播放线程在计算期间因欠载升档，比较交换会失败并保留更高的新值。
pub(super) fn publish_target(shared: &AtomicUsize, observed: usize, desired: usize) -> usize {
    match shared.compare_exchange(observed, desired, Ordering::Relaxed, Ordering::Relaxed) {
        Ok(_) => desired,
        Err(current) => current.clamp(MIN_TARGET_FRAMES, MAX_TARGET_FRAMES),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlayoutDepthAction {
    Play,
    Hold,
    DropOldest(usize),
}

/// 播放线程的升档计划。按实际水位缺口补齐，最多冻结目标帧数，网络断开时也不会无限等待。
#[derive(Debug)]
pub(super) struct PlayoutDepthState {
    active_frames: usize,
    planned_frames: usize,
    holds_remaining: usize,
}

impl PlayoutDepthState {
    pub(super) fn new(initial_frames: usize) -> Self {
        let initial_frames = initial_frames.clamp(MIN_TARGET_FRAMES, MAX_TARGET_FRAMES);
        Self {
            active_frames: initial_frames,
            planned_frames: initial_frames,
            holds_remaining: 0,
        }
    }

    #[cfg(test)]
    const fn active_frames(&self) -> usize {
        self.active_frames
    }

    /// 已在当前档位发生欠载时，按现有目标重新建立真实水位。
    ///
    /// 调用方只在队列重新出现数据后触发；完全断流时继续推进播放游标，不能反复补静音冻结时间轴。
    pub(super) fn refill_after_underrun(&mut self, buffered_frames: usize) {
        self.planned_frames = self.active_frames;
        self.holds_remaining = self.active_frames.saturating_sub(buffered_frames);
    }

    /// 升档用有界静音建立余量；降档只在确有积压时丢最旧帧，让目标深度真实下降。
    pub(super) fn action(
        &mut self,
        requested_frames: usize,
        buffered_frames: usize,
    ) -> PlayoutDepthAction {
        let requested_frames = requested_frames.clamp(MIN_TARGET_FRAMES, MAX_TARGET_FRAMES);
        if requested_frames < self.active_frames {
            let drop_frames = self
                .active_frames
                .saturating_sub(requested_frames)
                .min(buffered_frames.saturating_sub(requested_frames));
            self.active_frames = requested_frames;
            self.planned_frames = requested_frames;
            self.holds_remaining = 0;
            return if drop_frames > 0 {
                PlayoutDepthAction::DropOldest(drop_frames)
            } else {
                PlayoutDepthAction::Play
            };
        }
        if requested_frames == self.active_frames {
            if buffered_frames >= requested_frames {
                self.holds_remaining = 0;
                return PlayoutDepthAction::Play;
            }
            if self.holds_remaining > 0 {
                self.holds_remaining -= 1;
                return PlayoutDepthAction::Hold;
            }
            return PlayoutDepthAction::Play;
        }

        if requested_frames != self.planned_frames {
            self.planned_frames = requested_frames;
            self.holds_remaining = requested_frames.saturating_sub(buffered_frames);
        }
        if buffered_frames >= requested_frames {
            self.active_frames = requested_frames;
            self.holds_remaining = 0;
            return PlayoutDepthAction::Play;
        }
        if self.holds_remaining > 0 {
            self.holds_remaining -= 1;
            return PlayoutDepthAction::Hold;
        }

        // 网络未在预算内补齐：接受未达目标的事实并恢复时钟推进，不能把短期升档变成永久延迟。
        self.active_frames = requested_frames;
        PlayoutDepthAction::Play
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EncodedAudioPacket {
    pub(super) seq: u32,
    pub(super) payload: Vec<u8>,
    arrived_at: Instant,
}

#[derive(Debug, Default)]
pub(super) struct ReorderBatch {
    pub(super) ready: Vec<EncodedAudioPacket>,
    pub(super) late_drops: u32,
    /// 这是该序号第一次进入窗口；重复副本和已经过期的包为 `false`。
    pub(super) accepted_new_seq: bool,
}

/// 小型序号重排窗。
///
/// 40 ms 及以下档位不额外等待；60 ms 档位拥有一帧真实余量，才允许等待最多一个帧周期。
/// 无论目标深度如何，最多只保留三个未来包，防止异常序号或恶意流量无界占用内存。
///
/// **还有第二个等待理由（§8.1 的 NACK）**：当窗口里出现洞、且会话已经把重传请求发出去时，
/// 由调用方通过 [`PacketReorderBuffer::set_retransmit_grace`] 给出一个有界等待窗口。
/// 没有这个窗口，重传回来的包会直接撞上「游标已推进」被判迟到 —— 请求发了却救不回音频。
#[derive(Debug)]
pub(super) struct PacketReorderBuffer {
    expected_seq: Option<u32>,
    pending: Vec<EncodedAudioPacket>,
    recent_delivered: Vec<u32>,
    frame_period: Duration,
    target_frames: usize,
    /// 发现洞时给重传留的有界窗口（`Duration::ZERO` = 不等）。
    retransmit_grace: Duration,
    /// 当前这个洞是什么时候发现的；没有洞时为 `None`。
    gap_since: Option<Instant>,
}

impl PacketReorderBuffer {
    pub(super) fn new(frame_period: Duration) -> Self {
        Self {
            expected_seq: None,
            pending: Vec::with_capacity(REORDER_CAPACITY + 1),
            recent_delivered: Vec::with_capacity(REORDER_CAPACITY * 2),
            frame_period,
            // 同样取起步档：`reorder_wait()` 只在 `>= MAX_TARGET_FRAMES` 时才等一帧，
            // 所以 1 与 2 的行为完全一致 —— 改它只是为了不再有第二个「初值」来源（它会
            // 每秒被运行时写入的共享目标覆盖）。
            target_frames: INITIAL_TARGET_FRAMES,
            retransmit_grace: Duration::ZERO,
            gap_since: None,
        }
    }

    /// 设置「等重传」的有界窗口；调用方在没有待补洞时应传 `Duration::ZERO`。
    pub(super) fn set_retransmit_grace(&mut self, grace: Duration) {
        self.retransmit_grace = grace;
    }

    pub(super) fn set_target_frames(&mut self, target_frames: usize) {
        self.target_frames = target_frames.clamp(MIN_TARGET_FRAMES, MAX_TARGET_FRAMES);
    }

    pub(super) fn push(&mut self, seq: u32, payload: Vec<u8>, arrived_at: Instant) -> ReorderBatch {
        let mut batch = ReorderBatch::default();
        let expected = self.expected_seq.get_or_insert(seq);
        let distance = seq.wrapping_sub(*expected);

        // 默认冗余双发会让同一序号出现两次。近期已交付或仍在窗口中的副本是正常去重，
        // 不能污染 late_drops；从未交付却已越过播放游标的包仍按真正迟到记账。
        if self.recent_delivered.contains(&seq)
            || self.pending.iter().any(|packet| packet.seq == seq)
        {
            return batch;
        }
        if distance >= (1 << 31) {
            batch.late_drops = 1;
            return batch;
        }

        if distance >= DISCONTINUITY_FRAMES {
            batch.late_drops = u32::try_from(self.pending.len()).unwrap_or(u32::MAX);
            self.pending.clear();
            self.recent_delivered.clear();
            self.expected_seq = Some(seq);
        }

        batch.accepted_new_seq = true;
        self.pending.push(EncodedAudioPacket {
            seq,
            payload,
            arrived_at,
        });
        self.sort_pending();
        self.drain_contiguous(&mut batch.ready);

        if self.reorder_wait().is_zero() || self.pending.len() > REORDER_CAPACITY {
            self.force_earliest(&mut batch.ready);
        }
        self.refresh_gap(arrived_at);
        batch
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        let wait = self.reorder_wait();
        if wait.is_zero() {
            return None;
        }
        // 有洞：从**发现洞**那一刻起算有界等待（等的是重传，不是后续包）。
        // 无洞：沿用原先的「最旧待排包到达 + 一帧」口径。
        match self.gap_since {
            Some(since) => Some(since + wait),
            None => self
                .pending
                .iter()
                .map(|packet| packet.arrived_at + wait)
                .min(),
        }
    }

    pub(super) fn flush_expired(&mut self, now: Instant) -> ReorderBatch {
        let mut batch = ReorderBatch::default();
        if (!self.pending.is_empty() && self.reorder_wait().is_zero())
            || self.next_deadline().is_some_and(|deadline| deadline <= now)
        {
            self.force_earliest(&mut batch.ready);
            self.refresh_gap(now);
        }
        batch
    }

    fn reorder_wait(&self) -> Duration {
        if self.target_frames >= MAX_TARGET_FRAMES {
            return self.frame_period.max(self.gap_wait());
        }
        self.gap_wait()
    }

    /// 当前该给洞的等待时长：没有洞、或调用方没开窗口时为 0。
    fn gap_wait(&self) -> Duration {
        if self.has_gap() {
            self.retransmit_grace
        } else {
            Duration::ZERO
        }
    }

    /// 待排窗里第一个包不是「下一个该交付的序号」→ 存在洞。
    fn has_gap(&self) -> bool {
        let Some(expected) = self.expected_seq else {
            return false;
        };
        self.pending
            .first()
            .is_some_and(|packet| packet.seq != expected)
    }

    /// 洞出现时记下发现时刻；洞补上就清空（下一段等待重新计时）。
    fn refresh_gap(&mut self, now: Instant) {
        if self.has_gap() {
            if self.gap_since.is_none() {
                self.gap_since = Some(now);
            }
        } else {
            self.gap_since = None;
        }
    }

    fn sort_pending(&mut self) {
        let Some(expected) = self.expected_seq else {
            return;
        };
        self.pending
            .sort_by_key(|packet| packet.seq.wrapping_sub(expected));
    }

    fn drain_contiguous(&mut self, ready: &mut Vec<EncodedAudioPacket>) {
        loop {
            self.sort_pending();
            let Some(expected) = self.expected_seq else {
                return;
            };
            if self
                .pending
                .first()
                .is_none_or(|packet| packet.seq != expected)
            {
                return;
            }
            let packet = self.pending.remove(0);
            self.expected_seq = Some(expected.wrapping_add(1));
            self.remember_delivered(packet.seq);
            ready.push(packet);
        }
    }

    fn remember_delivered(&mut self, seq: u32) {
        const RECENT_CAPACITY: usize = REORDER_CAPACITY * 2;
        if self.recent_delivered.len() >= RECENT_CAPACITY {
            self.recent_delivered.remove(0);
        }
        self.recent_delivered.push(seq);
    }

    fn force_earliest(&mut self, ready: &mut Vec<EncodedAudioPacket>) {
        self.sort_pending();
        let Some(packet) = self.pending.first() else {
            return;
        };
        self.expected_seq = Some(packet.seq);
        self.drain_contiguous(ready);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn packet(seq: u32, now: Instant) -> (u32, Vec<u8>, Instant) {
        (seq, vec![seq as u8], now)
    }

    fn push(buffer: &mut PacketReorderBuffer, seq: u32, now: Instant) -> ReorderBatch {
        let (seq, payload, arrived_at) = packet(seq, now);
        buffer.push(seq, payload, arrived_at)
    }

    fn seqs(batch: &ReorderBatch) -> Vec<u32> {
        batch.ready.iter().map(|packet| packet.seq).collect()
    }

    #[test]
    fn in_order_packets_pass_without_waiting() {
        let now = Instant::now();
        let mut buffer = PacketReorderBuffer::new(Duration::from_millis(20));
        assert_eq!(seqs(&push(&mut buffer, 7, now)), [7]);
        assert_eq!(seqs(&push(&mut buffer, 8, now)), [8]);
        assert!(buffer.next_deadline().is_none());
    }

    #[test]
    fn sixty_ms_target_repairs_one_packet_reordering() {
        let now = Instant::now();
        let mut buffer = PacketReorderBuffer::new(Duration::from_millis(20));
        buffer.set_target_frames(3);
        assert_eq!(seqs(&push(&mut buffer, 0, now)), [0]);
        assert!(
            push(&mut buffer, 2, now + Duration::from_millis(20))
                .ready
                .is_empty()
        );
        let repaired = push(&mut buffer, 1, now + Duration::from_millis(25));
        assert_eq!(seqs(&repaired), [1, 2]);
        assert_eq!(repaired.late_drops, 0);
    }

    #[test]
    fn deadline_releases_future_packet_and_late_packet_is_dropped() {
        let now = Instant::now();
        let mut buffer = PacketReorderBuffer::new(Duration::from_millis(20));
        buffer.set_target_frames(3);
        assert_eq!(seqs(&push(&mut buffer, 0, now)), [0]);
        assert!(
            push(&mut buffer, 2, now + Duration::from_millis(20))
                .ready
                .is_empty()
        );
        let expired = buffer.flush_expired(now + Duration::from_millis(40));
        assert_eq!(seqs(&expired), [2]);
        let late = push(&mut buffer, 1, now + Duration::from_millis(41));
        assert!(late.ready.is_empty());
        assert_eq!(late.late_drops, 1);
    }

    #[test]
    fn sequence_wrap_is_reordered_in_half_range_order() {
        let now = Instant::now();
        let mut buffer = PacketReorderBuffer::new(Duration::from_millis(20));
        buffer.set_target_frames(3);
        assert_eq!(seqs(&push(&mut buffer, u32::MAX, now)), [u32::MAX]);
        assert!(push(&mut buffer, 1, now).ready.is_empty());
        assert_eq!(seqs(&push(&mut buffer, 0, now)), [0, 1]);
    }

    #[test]
    fn delayed_redundant_copies_are_silently_deduplicated() {
        let now = Instant::now();
        let mut buffer = PacketReorderBuffer::new(Duration::from_millis(20));
        buffer.set_target_frames(3);

        assert!(push(&mut buffer, 0, now).accepted_new_seq);
        assert!(push(&mut buffer, 1, now + Duration::from_millis(20)).accepted_new_seq);
        let duplicate = push(&mut buffer, 0, now + Duration::from_millis(20));
        assert!(!duplicate.accepted_new_seq);
        assert_eq!(duplicate.late_drops, 0);
        assert!(duplicate.ready.is_empty());
    }

    #[test]
    fn duplicate_pending_copy_does_not_consume_reorder_capacity() {
        let now = Instant::now();
        let mut buffer = PacketReorderBuffer::new(Duration::from_millis(20));
        buffer.set_target_frames(3);

        assert_eq!(seqs(&push(&mut buffer, 0, now)), [0]);
        assert!(push(&mut buffer, 2, now).accepted_new_seq);
        let duplicate = push(&mut buffer, 2, now + Duration::from_millis(1));
        assert!(!duplicate.accepted_new_seq);
        assert_eq!(duplicate.late_drops, 0);
        assert_eq!(seqs(&push(&mut buffer, 1, now)), [1, 2]);
    }

    #[test]
    fn depth_rises_immediately_and_shrinks_only_after_stable_hysteresis() {
        // 起步档与中等抖动档已拆成两个常量（第 114 轮）：这条测的是**升降档迟滞**，
        // 所以显式把起点抬到中等抖动档，不依赖 new() 的起步值 —— 否则一调起步延迟就会连带改这里。
        let mut depth = AdaptiveJitterDepth::new();
        depth.raise_to(DEFAULT_TARGET_FRAMES);
        assert_eq!(depth.observe(Some(2_000), 1, 20_000), 3);
        for _ in 0..STABLE_WINDOWS_TO_SHRINK - 1 {
            assert_eq!(depth.observe(Some(2_000), 0, 20_000), 3);
        }
        assert_eq!(depth.observe(Some(2_000), 0, 20_000), 2);
        for _ in 0..STABLE_WINDOWS_TO_SHRINK {
            depth.observe(Some(2_000), 0, 20_000);
        }
        assert_eq!(depth.target_frames, 1);
    }

    #[test]
    fn a_fresh_controller_starts_at_the_minimum_depth() {
        // 起步档 = 最小档 = 20 ms：它是**出声延迟**的直接来源。这条断言防止有人把起步值
        // 悄悄抬回中等抖动档 —— 那会白送一帧延迟（第 114 轮实测：回环 P50 +27.9 ms）。
        let mut depth = AdaptiveJitterDepth::new();
        assert_eq!(depth.target_frames, INITIAL_TARGET_FRAMES);
        assert_eq!(INITIAL_TARGET_FRAMES, MIN_TARGET_FRAMES);
        assert_eq!(
            MIN_TARGET_FRAMES, 1,
            "起步档就是一帧（20 ms 帧长下 = 20 ms）"
        );
        // 还没有抖动测量时保持起步档，不得自己往上爬。
        assert_eq!(depth.observe(None, 0, 20_000), MIN_TARGET_FRAMES);
    }

    #[test]
    fn jitter_thresholds_map_to_twenty_forty_and_sixty_ms() {
        let mut low = AdaptiveJitterDepth::new();
        for _ in 0..STABLE_WINDOWS_TO_SHRINK {
            low.observe(Some(5_000), 0, 20_000);
        }
        assert_eq!(low.target_frames, 1);

        let mut medium = AdaptiveJitterDepth::new();
        assert_eq!(medium.observe(Some(20_000), 0, 20_000), 2);

        let mut high = AdaptiveJitterDepth::new();
        assert_eq!(high.observe(Some(20_001), 0, 20_000), 3);
    }

    #[test]
    fn playout_rebuffer_holds_only_the_bounded_depth_deficit() {
        let mut from_forty = PlayoutDepthState::new(2);
        assert_eq!(from_forty.action(3, 0), PlayoutDepthAction::Hold);
        assert_eq!(from_forty.action(3, 0), PlayoutDepthAction::Hold);
        assert_eq!(from_forty.action(3, 0), PlayoutDepthAction::Hold);
        assert_eq!(from_forty.action(3, 0), PlayoutDepthAction::Play);
        assert_eq!(from_forty.active_frames(), 3);

        let mut from_twenty = PlayoutDepthState::new(1);
        assert_eq!(from_twenty.action(3, 1), PlayoutDepthAction::Hold);
        assert_eq!(from_twenty.action(3, 2), PlayoutDepthAction::Hold);
        assert_eq!(from_twenty.action(3, 3), PlayoutDepthAction::Play);
        assert_eq!(from_twenty.active_frames(), 3);
    }

    #[test]
    fn playout_rebuffer_finishes_early_when_target_depth_arrives() {
        let mut state = PlayoutDepthState::new(2);
        assert_eq!(state.action(3, 1), PlayoutDepthAction::Hold);
        assert_eq!(state.action(3, 3), PlayoutDepthAction::Play);
        assert_eq!(state.active_frames(), 3);
    }

    #[test]
    fn max_depth_underrun_can_refill_without_another_depth_raise() {
        let mut state = PlayoutDepthState::new(3);
        state.refill_after_underrun(1);

        assert_eq!(state.action(3, 1), PlayoutDepthAction::Hold);
        assert_eq!(state.action(3, 2), PlayoutDepthAction::Hold);
        assert_eq!(state.action(3, 3), PlayoutDepthAction::Play);
        assert_eq!(state.active_frames(), 3);
    }

    #[test]
    fn playout_downshift_drops_only_real_excess_depth() {
        let mut state = PlayoutDepthState::new(3);
        assert_eq!(state.action(2, 3), PlayoutDepthAction::DropOldest(1));
        assert_eq!(state.active_frames(), 2);

        let mut already_shallow = PlayoutDepthState::new(3);
        assert_eq!(already_shallow.action(2, 2), PlayoutDepthAction::Play);
        assert_eq!(already_shallow.active_frames(), 2);
    }

    #[test]
    fn controller_publish_does_not_overwrite_concurrent_underrun_raise() {
        let shared = AtomicUsize::new(3);
        assert_eq!(publish_target(&shared, 2, 1), 3);
        assert_eq!(shared.load(Ordering::Relaxed), 3);
    }
    #[test]
    fn retransmit_grace_holds_a_gap_until_the_deadline() {
        let mut buffer = PacketReorderBuffer::new(Duration::from_millis(20));
        buffer.set_retransmit_grace(Duration::from_millis(30));
        let start = Instant::now();
        let batch = push(&mut buffer, 0, start);
        assert_eq!(seqs(&batch), vec![0]);

        // 序号 1 丢了、2 到了：开了等重传窗口就不能把 2 提前交付（那会让 1 永远补不上）
        let batch = push(&mut buffer, 2, start + Duration::from_millis(20));
        assert!(batch.ready.is_empty(), "有洞时必须等重传窗口，不许提前交付");
        assert!(buffer.next_deadline().is_some(), "有洞就必须有等待死线");

        // 重传在窗口内回来 → 连续交付 1、2
        let batch = push(&mut buffer, 1, start + Duration::from_millis(40));
        assert_eq!(seqs(&batch), vec![1, 2]);
        assert!(buffer.next_deadline().is_none(), "洞补上后不再等待");
    }

    #[test]
    fn retransmit_grace_expires_and_original_behaviour_resumes() {
        let mut buffer = PacketReorderBuffer::new(Duration::from_millis(20));
        buffer.set_retransmit_grace(Duration::from_millis(30));
        let start = Instant::now();
        push(&mut buffer, 0, start);
        push(&mut buffer, 2, start + Duration::from_millis(20));

        let deadline = buffer.next_deadline().expect("有洞就有死线");
        let batch = buffer.flush_expired(deadline + Duration::from_millis(1));
        assert_eq!(
            seqs(&batch),
            vec![2],
            "窗口到点后按原策略跳过洞，绝不无限等"
        );
    }

    #[test]
    fn retransmit_grace_does_not_delay_a_contiguous_stream() {
        let mut buffer = PacketReorderBuffer::new(Duration::from_millis(20));
        buffer.set_retransmit_grace(Duration::from_millis(30));
        let start = Instant::now();
        let batch = push(&mut buffer, 0, start);
        assert_eq!(seqs(&batch), vec![0]);
        let batch = push(&mut buffer, 1, start + Duration::from_millis(20));
        assert_eq!(seqs(&batch), vec![1]);
        assert!(
            buffer.next_deadline().is_none(),
            "无丢包的正常链路一个字节的额外延迟都不该有"
        );
    }

    #[test]
    fn closing_the_grace_window_falls_back_to_immediate_delivery() {
        // 会话循环在播放队列没余量时会把窗口设回 0：此时必须立刻回到「不等」的行为
        let mut buffer = PacketReorderBuffer::new(Duration::from_millis(20));
        buffer.set_retransmit_grace(Duration::from_millis(30));
        let start = Instant::now();
        push(&mut buffer, 0, start);
        buffer.set_retransmit_grace(Duration::ZERO);
        let batch = push(&mut buffer, 2, start + Duration::from_millis(20));
        assert_eq!(seqs(&batch), vec![2], "窗口关闭后立刻交付，由掩盖兜底");
    }
}
