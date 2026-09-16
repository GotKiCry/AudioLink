//! §8.1 的 NACK 重传：**双发都丢时**的辅助手段（主防线仍是冗余双发 + PCM 掩盖）。
//!
//! 一条链路的两个角色各占一半，这里都实现成**确定性状态机**（不碰套接字、不碰遥测，因此可单测）：
//!
//! - 接收侧：[`MissingTracker`] 从到达序号的**洞**里认出缺了哪几帧，按「重试 ≤ 5 次、间隔 10 ms、
//!   窗口 1 s」（§8.1）给出本轮该请求的序号。**是否启用由调用方门控**：`RTT < 30 ms` 才发，
//!   否则重传只会把延迟尖峰拉得更长；
//! - 发送侧：[`RetransmitBuffer`] 保留最近 1 s 的已编码帧，收到 `NACK` 后按序号取出重发。
//!
//! 只跟踪「本来就在重传窗口里」的洞：一次 1 000 帧以上的跳变按时间轴重建处理，与解码侧
//! （`AudioReceiver`）用同一个阈值 —— 两份账本对「这是丢包还是断流」必须给出同一个答案。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use audiolink_types::NACK_MAX_ITEMS;

/// 单次重传请求的最大重试次数（§8.1：重试 ≤ 5 次）。
pub const NACK_MAX_RETRIES: u8 = 5;

/// 同一序号的两次请求间隔（§8.1：间隔 10 ms）。
pub const NACK_RETRY_INTERVAL: Duration = Duration::from_millis(10);

/// 重传窗口（§8.1：窗口 1 s）：超过它就不再请求，也不再保留发送副本。
pub const NACK_WINDOW: Duration = Duration::from_secs(1);

/// 启用 NACK 的 RTT 上限（§8.1：仅在 RTT < 30 ms 时启用，避免延迟尖峰）。
pub const NACK_MAX_RTT_US: u64 = 30_000;

/// 一条 `NACK` 报文最多携带的序号数（§3 载荷表：1 ≤ n ≤ 16）。
pub const NACK_BATCH_MAX: usize = NACK_MAX_ITEMS;

/// 同时跟踪的缺失序号上限：一次异常跳变不该把内存与 CPU 打满。
pub const NACK_MAX_TRACKED: usize = 64;

/// 序号跳变达到它 → 时间轴重建（与 `AudioReceiver` 的 1 000 帧规则同源）。
pub const NACK_RESYNC_GAP: u32 = 1_000;

/// 重传缓冲容量（帧）。1 s 窗口在 10 ms 帧长下是 100 帧，留一倍余量。
pub const NACK_RETRANSMIT_CAPACITY: usize = 128;

/// 重排窗为「等重传」额外给的有界时间。
///
/// 取值与 §8.1 的 RTT 门槛一致：NACK 只在 `RTT < 30 ms` 时启用，所以「请求发出去 → 重传回来」
/// 的上界就是 30 ms。**这是延迟预算的硬上限** —— 每个洞只等这一次，超时就按原策略放弃。
pub const NACK_RETRANSMIT_GRACE: Duration = Duration::from_millis(30);

/// 一个仍然缺失的音频序号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Missing {
    /// 缺失的序号。
    seq: u32,
    /// 已经请求过几次。
    attempts: u8,
    /// 第一次发现缺失的时刻（重传窗口从这里算起）。
    first_seen: Instant,
    /// 下一次最早可以再请求的时刻。
    next_at: Instant,
}

/// 接收侧缺失序号跟踪器（§8.1 的「重试次数 / 间隔 / 窗口」全在这里）。
#[derive(Debug, Default)]
pub struct MissingTracker {
    entries: Vec<Missing>,
    /// 下一个期望到达的序号（= 已到达的最高序号 + 1）。
    expected: Option<u32>,
    /// 因为超过 [`NACK_MAX_TRACKED`] 而放弃跟踪的序号累计个数。
    overflow: u64,
}

impl MissingTracker {
    /// 空跟踪器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 看见一个到达的音频序号（主包与冗余副本都算到达）。
    ///
    /// 返回本次**新识别**出的缺失个数（仅供调用方记账，不参与决策）。
    pub fn observe(&mut self, seq: u32, now: Instant) -> usize {
        let Some(expected) = self.expected else {
            self.expected = Some(seq.wrapping_add(1));
            return 0;
        };

        // 迟到 / 重复（落在已到达区间里）：它可能正好是某个洞的补件。
        if seq.wrapping_sub(expected) >= (1 << 31) {
            self.resolve(seq);
            return 0;
        }

        self.resolve(seq);
        let forward = seq.wrapping_sub(expected);
        if forward == 0 {
            self.expected = Some(seq.wrapping_add(1));
            return 0;
        }
        if forward >= NACK_RESYNC_GAP {
            // 时间轴重建：旧洞全部作废（解码侧同样不追这 1 000 帧）。
            self.entries.clear();
            self.expected = Some(seq.wrapping_add(1));
            return 0;
        }

        let mut added = 0;
        for offset in 0..forward {
            if self.track(expected.wrapping_add(offset), now) {
                added += 1;
            }
        }
        self.expected = Some(seq.wrapping_add(1));
        added
    }

    /// 丢弃超出重传窗口的条目（调用方节奏自定，1 Hz 或每包都行）。
    pub fn expire(&mut self, now: Instant) {
        self.entries
            .retain(|entry| now.saturating_duration_since(entry.first_seen) < NACK_WINDOW);
    }

    /// 取本轮该请求的序号（≤ [`NACK_BATCH_MAX`] 个，越靠近 `expected` 越先要）。
    ///
    /// 取出的序号**当场推进重试计数**：调用方只要把它发出去，账簿就是对的。
    pub fn due_batch(&mut self, now: Instant) -> Vec<u32> {
        let Some(expected) = self.expected else {
            return Vec::new();
        };
        let mut due: Vec<u32> = self
            .entries
            .iter()
            .filter(|entry| entry.attempts < NACK_MAX_RETRIES && now >= entry.next_at)
            .filter(|entry| now.saturating_duration_since(entry.first_seen) < NACK_WINDOW)
            .map(|entry| entry.seq)
            .collect();
        // **最旧的洞先请求**：它的 1 s 窗口最先到期，来不及的话就是永久静音；
        // 刚丢的那几个下一拍还能再要（重试上限 5 次足够覆盖）。
        due.sort_by_key(|seq| expected.wrapping_sub(*seq));
        due.reverse();
        due.truncate(NACK_BATCH_MAX);

        for seq in &due {
            if let Some(entry) = self.entries.iter_mut().find(|entry| entry.seq == *seq) {
                entry.attempts = entry.attempts.saturating_add(1);
                entry.next_at = now + NACK_RETRY_INTERVAL;
            }
        }
        due
    }

    /// 仍在缺失中的序号个数。
    ///
    /// 会话循环用它决定重排窗要不要开「等重传」窗口：有洞才等（`jitter` 侧的 `set_retransmit_grace`）。
    pub fn pending(&self) -> usize {
        self.entries.len()
    }

    /// 放弃跟踪的序号累计个数（测试观察点）。
    #[cfg(test)]
    pub const fn overflowed(&self) -> u64 {
        self.overflow
    }

    /// 清空（换流 / 会话重建时调用）。
    pub fn clear(&mut self) {
        self.entries.clear();
        self.expected = None;
        self.overflow = 0;
    }

    /// 已到达的序号把它从缺失表里摘掉。
    fn resolve(&mut self, seq: u32) {
        if let Some(index) = self.entries.iter().position(|entry| entry.seq == seq) {
            self.entries.remove(index);
        }
    }

    /// 新增一个待跟踪的缺失序号；已有或超限 → `false`。
    fn track(&mut self, seq: u32, now: Instant) -> bool {
        if self.entries.iter().any(|entry| entry.seq == seq) {
            return false;
        }
        if self.entries.len() >= NACK_MAX_TRACKED {
            self.overflow = self.overflow.saturating_add(1);
            return false;
        }
        self.entries.push(Missing {
            seq,
            attempts: 0,
            first_seen: now,
            next_at: now,
        });
        true
    }
}

/// 一条可重传的帧。
#[derive(Debug, Clone, PartialEq, Eq)]
struct RetransmitEntry {
    /// 序号（重传时必须原样保留）。
    seq: u32,
    /// 负载首样本的 epoch 样本序号（同样原样保留，否则对端排播会错位）。
    sample_index: u32,
    /// 已编码负载。
    payload: Vec<u8>,
    /// 首次发出的时刻（窗口从这里算起）。
    at: Instant,
}

/// 发送侧重传缓冲：保留最近 [`NACK_WINDOW`] 内已发出的帧。
#[derive(Debug)]
pub struct RetransmitBuffer {
    window: Duration,
    frames: VecDeque<RetransmitEntry>,
}

impl Default for RetransmitBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl RetransmitBuffer {
    /// 默认窗口（[`NACK_WINDOW`]）的缓冲。
    pub fn new() -> Self {
        Self::with_window(NACK_WINDOW)
    }

    /// 指定窗口的缓冲。
    pub fn with_window(window: Duration) -> Self {
        Self {
            window,
            frames: VecDeque::new(),
        }
    }

    /// 记下一个刚发出的帧。
    ///
    /// 主包与冗余副本同序号：只留第一份（谁先到都是同一个负载），也不重复占容量。
    pub fn record(&mut self, seq: u32, sample_index: u32, payload: &[u8], at: Instant) {
        self.expire(at);
        if self.frames.iter().any(|entry| entry.seq == seq) {
            return;
        }
        while self.frames.len() >= NACK_RETRANSMIT_CAPACITY {
            self.frames.pop_front();
        }
        self.frames.push_back(RetransmitEntry {
            seq,
            sample_index,
            payload: payload.to_vec(),
            at,
        });
    }

    /// 按序号取出 `(sample_index, 负载)`；窗口外或从未发过 → `None`。
    pub fn find(&self, seq: u32) -> Option<(u32, &[u8])> {
        self.frames
            .iter()
            .find(|entry| entry.seq == seq)
            .map(|entry| (entry.sample_index, entry.payload.as_slice()))
    }

    /// 清空缓冲（换流 / 会话重建时调用）。
    pub fn clear(&mut self) {
        self.frames.clear();
    }

    /// 丢掉窗口外的帧（缓冲按时间升序，从头开始即可）。
    pub fn expire(&mut self, now: Instant) {
        while let Some(entry) = self.frames.front() {
            if now.saturating_duration_since(entry.at) >= self.window {
                self.frames.pop_front();
            } else {
                break;
            }
        }
    }

    /// 缓冲里的帧数（测试观察点）。
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// 缓冲是否为空（测试观察点）。
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// 测试用相对时刻（固定基准，不依赖真实时钟）。
    fn at(millis: u64) -> Instant {
        static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *BASE.get_or_init(Instant::now) + Duration::from_millis(millis)
    }

    #[test]
    fn in_order_arrivals_create_no_missing_entries() {
        let mut tracker = MissingTracker::new();
        for seq in 0..10u32 {
            assert_eq!(tracker.observe(seq, at(u64::from(seq) * 20)), 0);
        }
        assert_eq!(tracker.pending(), 0);
        assert!(tracker.due_batch(at(1_000)).is_empty());
    }

    #[test]
    fn gap_is_tracked_and_requested_oldest_first() {
        let mut tracker = MissingTracker::new();
        tracker.observe(0, at(0));
        // 1..=4 丢了，5 到了
        assert_eq!(tracker.observe(5, at(100)), 4);
        assert_eq!(tracker.pending(), 4);
        let batch = tracker.due_batch(at(100));
        assert_eq!(batch, vec![1, 2, 3, 4], "最旧的洞窗口最先到期，先请求它");
    }

    #[test]
    fn retry_is_capped_at_five_with_ten_millisecond_spacing() {
        let mut tracker = MissingTracker::new();
        tracker.observe(0, at(0));
        tracker.observe(2, at(0)); // 序号 1 缺失

        let mut requests = 0;
        let mut sent_at = Vec::new();
        let mut now = at(0);
        while now < at(500) {
            for _ in tracker.due_batch(now) {
                requests += 1;
                sent_at.push(now);
            }
            now += Duration::from_millis(1);
        }
        assert_eq!(requests, usize::from(NACK_MAX_RETRIES), "重试上限 5 次");
        let gaps: Vec<Duration> = sent_at.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert!(
            gaps.iter().all(|gap| *gap >= NACK_RETRY_INTERVAL),
            "两次请求之间不得少于 10 ms：{gaps:?}"
        );
    }

    #[test]
    fn window_closes_after_one_second_and_entries_expire() {
        let mut tracker = MissingTracker::new();
        tracker.observe(0, at(0));
        tracker.observe(2, at(0));
        assert!(tracker.due_batch(at(1_001)).is_empty(), "1 s 之后窗口关闭");
        tracker.expire(at(1_001));
        assert_eq!(tracker.pending(), 0);
    }

    #[test]
    fn late_arrival_fills_the_hole_and_stops_requests() {
        let mut tracker = MissingTracker::new();
        tracker.observe(0, at(0));
        tracker.observe(2, at(0));
        assert_eq!(tracker.pending(), 1);
        tracker.observe(1, at(15)); // 迟到的 1 回来了（主包或副本都行）
        assert_eq!(tracker.pending(), 0);
        assert!(tracker.due_batch(at(20)).is_empty());
    }

    #[test]
    fn duplicate_and_out_of_order_copies_do_not_disturb_tracking() {
        let mut tracker = MissingTracker::new();
        tracker.observe(5, at(0));
        tracker.observe(5, at(5)); // 副本
        tracker.observe(4, at(10)); // 迟到补洞
        assert_eq!(tracker.pending(), 0, "副本与迟到补洞都不该造出洞");
        assert_eq!(tracker.overflowed(), 0);
    }

    #[test]
    fn big_jump_rebuilds_the_timeline_instead_of_tracking_thousand_gaps() {
        let mut tracker = MissingTracker::new();
        tracker.observe(0, at(0));
        // forward = 1001 - 1 = 1000 → 达到阈值（与 `AudioReceiver` 的 `forward >= 1_000` 同源）
        tracker.observe(NACK_RESYNC_GAP + 1, at(1));
        assert_eq!(
            tracker.pending(),
            0,
            "跳变 ≥ 1000 帧按时间轴重建，不追这 1000 个洞"
        );
    }

    #[test]
    fn tracked_holes_are_bounded_and_overflow_is_counted() {
        let mut tracker = MissingTracker::new();
        tracker.observe(0, at(0));
        // 一次跨到 200 的跳变：洞是 1..=199（199 个），只跟踪最旧的 64 个，其余计入 overflow
        tracker.observe(200, at(1));
        assert_eq!(tracker.pending(), NACK_MAX_TRACKED);
        assert_eq!(tracker.overflowed(), 199 - NACK_MAX_TRACKED as u64);
        assert_eq!(
            tracker.due_batch(at(2)).len(),
            NACK_BATCH_MAX,
            "一条 NACK 最多 16 个序号"
        );
    }

    #[test]
    fn sequence_wrap_is_tracked_in_the_normal_order() {
        let mut tracker = MissingTracker::new();
        tracker.observe(u32::MAX - 2, at(0));
        tracker.observe(1, at(40)); // MAX-1、MAX、0 三个洞
        assert_eq!(tracker.pending(), 3);
        assert_eq!(tracker.due_batch(at(40)), vec![u32::MAX - 1, u32::MAX, 0]);
    }

    #[test]
    fn retransmit_buffer_keeps_only_the_window_and_finds_by_seq() {
        let mut buffer = RetransmitBuffer::with_window(NACK_WINDOW);
        buffer.record(10, 4_800, b"ten", at(0));
        buffer.record(11, 5_760, b"eleven", at(20));
        buffer.record(11, 5_760, b"copy", at(30)); // 副本：不覆盖第一份

        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer.find(11), Some((5_760, &b"eleven"[..])));

        buffer.expire(at(1_000));
        assert_eq!(buffer.len(), 1, "1 s 窗口外的旧帧被丢弃");
        assert_eq!(buffer.find(10), None);
        assert_eq!(buffer.find(11), Some((5_760, &b"eleven"[..])));

        buffer.expire(at(1_021));
        assert!(buffer.is_empty());
    }

    #[test]
    fn retransmit_buffer_capacity_is_bounded() {
        let mut buffer = RetransmitBuffer::new();
        for seq in 0..(NACK_RETRANSMIT_CAPACITY as u32 + 10) {
            buffer.record(seq, seq.wrapping_mul(960), b"x", at(u64::from(seq)));
        }
        assert_eq!(buffer.len(), NACK_RETRANSMIT_CAPACITY);
        assert_eq!(buffer.find(0), None, "最旧的先被挤出去");
        assert!(buffer.find(NACK_RETRANSMIT_CAPACITY as u32 + 9).is_some());
    }
}
