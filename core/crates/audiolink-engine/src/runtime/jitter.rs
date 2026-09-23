//! 接收抖动控制：正常网络 20--60 ms，持续弱网最多 120 ms，与有界包重排。
//!
//! 所有保护垫都是**毫秒制**常量，运行时按帧长用 [`frames_for_ms`] 换算成帧数
//! （向上取整、至少一帧）：20 ms 帧下 20/40/120/60/60 ms → 1/2/6/3/3 帧，
//! 10 ms 帧下 → 2/4/12/6/6 帧。墙钟余量不随档位变化。
//!
//! # 两个「目标深度」不再共用一个常量（第 114 轮拆分）
//!
//! 这里有两个语义不同的量，曾经共用一个 `DEFAULT_TARGET`，于是「想调起步延迟」就得
//! 连带改「中等抖动该给多少余量」：
//!
//! * [`INITIAL_TARGET_MS`] = **起步档**：会话刚起播时攒够这么多毫秒才出声（= [`MIN_TARGET_MS`]，
//!   20 ms；20 ms 帧 = 1 帧，10 ms 帧 = 2 帧）。它是**出声延迟**的直接来源。
//! * [`DEFAULT_TARGET_MS`] = **中等抖动档**：`observe()` 在「抖动 ≤ 一帧」时选用的深度
//!   （40 ms）。它是**抗抖动保护**的一部分，由 M2 的口径决定，不该被起步延迟的需求牵动。
//!
//! 实测（本机，20 ms 帧）：起步档 40 ms → 20 ms 后，回环 P50 53 872 → 25 965 / 27 259 µs（两次）、
//! 接收侧水位 40 000 → 20 000 µs。
//!
//! ⚠️ **弱网代价在本机这个口径上判不出来，别把噪声当结论**：`soak-runner --tolerant` +
//! netem 2% / 15±15 ms 下 —— 300 s 档改前 20 拍 / 改后 23 拍；25 s 档两次重复分别是
//! 2 & 7 拍（改前）与 7 & 7 拍（改后）⇒ **区间重叠**，25 s 档自身的噪声就有 2~7 拍。
//! 要判「起步少攒一帧是否更易欠载」，得用更长档 + 多次重复，或者在真机弱网上量
//! （本机回环零丢包零抖动，本来就不覆盖这条路径）。

//! # 稳态水位结算
//!
//! 发送时钟、本机播放节拍与设备音频时钟是三个独立时钟域，全链路没有速率补偿：
//! ppm 级的供给/消费残差只能积在播放队列里。稳态（档位不变）下队列原本**只涨不跌**，
//! 直到撞上队列上限，然后以「投递失败 → 欠载 → 120 ms 淡出掩盖」的失控形式释放
//! （真机 30 min 水位 43 → 276 ms，见 docs/12 §11）。
//!
//! 结算机制：稳态下水位持续超出目标一整帧摆幅达 500 ms 时，播放层丢掉最旧一帧 ——
//! 游标正常推进、记 `depth_drops`、边界走既有 2.5 ms 平滑，把失控释放换成有界的内容跳过。
//! 盈余存在多久结算就发生多久，结算频率自然等于速率差。与被移除的两版护栏
//! （docs/12 §11.12/§11.14，代价 7×/103×）的区别：持续窗口过滤突发、每次只结算一帧、
//! 且目标之上永远保留一整帧抖动摆幅，绝不连续抽干余量制造新的欠载。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub(super) const MIN_TARGET_MS: u64 = 20;
/// **起步档**：会话起播时攒够这么多毫秒才出声（20 ms 帧 = 1 帧，10 ms 帧 = 2 帧）。
///
/// 与 [`DEFAULT_TARGET_MS`] 分开：起步深度是**延迟**，中等抖动档是**抗抖动保护**。
/// 详见模块文档。
pub(super) const INITIAL_TARGET_MS: u64 = 20;
/// **中等抖动档**：`observe()` 在「抖动 ≤ 一帧」时选用的深度（40 ms）。
///
/// 由 M2 的抗抖动口径决定 —— 不要拿起步延迟的需求去改它（那正是第 114 轮拆分的理由）。
pub(super) const DEFAULT_TARGET_MS: u64 = 40;
pub(super) const MAX_TARGET_MS: u64 = 120;
const FULL_REORDER_MS: u64 = 60;
const STABLE_WINDOWS_TO_SHRINK: u32 = 5;
const REORDER_CAPACITY_MS: u64 = 60;
// 发送端先发当前主包、再发上一帧副本。低缓冲档也须留出同批数据报的到达间隙，
// 否则主包刚丢就被确认成洞，紧随其后的冗余副本永远赶不上解码游标。
const REDUNDANT_REORDER_GRACE: Duration = Duration::from_millis(5);
const DISCONTINUITY_FRAMES: u32 = 1_000;
/// 稳态水位结算的盈余门槛：水位高出目标这么多帧才认定为持续盈余
/// （目标之上保留一整帧摆幅，正常抖动永远不触发结算）。
const SETTLE_EXCESS_FRAMES: usize = 1;
/// 盈余必须持续满一个窗口才结算一帧，结算后重新计时 —— 结算频率自然跟随速率差，
/// 与被移除的「每拍丢一帧」护栏（docs/12 §11.12，代价 7×）相区别。
const SETTLE_EXCESS_WINDOW: Duration = Duration::from_millis(500);

/// 毫秒保护垫 → 帧数：向上取整、至少一帧（20 ms → 20 ms 帧 1 帧 / 10 ms 帧 2 帧）。
pub(super) fn frames_for_ms(ms: u64, frame_ms: u32) -> usize {
    usize::try_from(ms.div_ceil(u64::from(frame_ms.max(1))))
        .unwrap_or(usize::MAX)
        .max(1)
}

/// 换档时按墙钟水位重换算帧数深度：旧帧长下的毫秒余量在新帧长下向上取整，并夹到新边界。
pub(super) fn rescale_depth_frames(frames: usize, from_frame_ms: u32, to_frame_ms: u32) -> usize {
    let depth_ms = u64::try_from(frames)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::from(from_frame_ms.max(1)));
    frames_for_ms(depth_ms, to_frame_ms).clamp(
        frames_for_ms(MIN_TARGET_MS, to_frame_ms),
        frames_for_ms(MAX_TARGET_MS, to_frame_ms),
    )
}

/// 1 Hz 更新的目标深度控制器。
///
/// 升档立即生效；降档要连续稳定 5 个窗口且每次只降一档，避免把短时波动固化为长延迟。
#[derive(Debug)]
pub(super) struct AdaptiveJitterDepth {
    target_frames: usize,
    stable_windows: u32,
    frame_ms: u32,
}

impl AdaptiveJitterDepth {
    pub(super) fn new(frame_ms: u32) -> Self {
        Self {
            // 起步 = 最小档：出声延迟从这里开始（第 114 轮：与中等抖动档拆开）。
            target_frames: frames_for_ms(INITIAL_TARGET_MS, frame_ms),
            stable_windows: 0,
            frame_ms: frame_ms.max(1),
        }
    }

    pub(super) fn observe(
        &mut self,
        jitter_p95_us: Option<u32>,
        underruns: u32,
        frame_us: u32,
    ) -> usize {
        let frame_us = u64::from(frame_us.max(1));
        let min_frames = frames_for_ms(MIN_TARGET_MS, self.frame_ms);
        let max_frames = frames_for_ms(MAX_TARGET_MS, self.frame_ms);
        let measured = jitter_p95_us.map(|jitter| {
            if u64::from(jitter) <= frame_us / 4 {
                min_frames
            } else if u64::from(jitter) <= frame_us {
                frames_for_ms(DEFAULT_TARGET_MS, self.frame_ms)
            } else {
                usize::try_from(u64::from(jitter).div_ceil(frame_us) + 1)
                    .unwrap_or(usize::MAX)
                    .min(max_frames)
            }
        });

        let mut wanted = measured.unwrap_or(self.target_frames);
        if underruns > 0 {
            wanted = wanted.max(self.target_frames);
        }

        if wanted > self.target_frames {
            self.target_frames = wanted.min(max_frames);
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
        let target_frames = target_frames.clamp(
            frames_for_ms(MIN_TARGET_MS, self.frame_ms),
            frames_for_ms(MAX_TARGET_MS, self.frame_ms),
        );
        if target_frames > self.target_frames {
            self.target_frames = target_frames;
            self.stable_windows = 0;
        }
    }

    /// 帧长协商换档：更新换算基准，当前深度按墙钟水位重换算到新帧长。
    pub(super) fn set_frame_ms(&mut self, frame_ms: u32) {
        let frame_ms = frame_ms.max(1);
        if frame_ms == self.frame_ms {
            return;
        }
        self.target_frames = rescale_depth_frames(self.target_frames, self.frame_ms, frame_ms);
        self.frame_ms = frame_ms;
        self.stable_windows = 0;
    }
}

/// 仅当共享目标仍是控制器观察到的旧值时发布决策。
///
/// 若播放线程在计算期间因欠载升档，比较交换会失败并保留更高的新值。
pub(super) fn publish_target(
    shared: &AtomicUsize,
    observed: usize,
    desired: usize,
    frame_ms: u32,
) -> usize {
    match shared.compare_exchange(observed, desired, Ordering::Relaxed, Ordering::Relaxed) {
        Ok(_) => desired,
        Err(current) => current.clamp(
            frames_for_ms(MIN_TARGET_MS, frame_ms),
            frames_for_ms(MAX_TARGET_MS, frame_ms),
        ),
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
    frame_ms: u32,
    /// 稳态水位结算：水位首次越过「目标 + 摆幅」的时刻；回落或结算后清零。
    settle_since: Option<Instant>,
}

impl PlayoutDepthState {
    pub(super) fn new(initial_frames: usize, frame_ms: u32) -> Self {
        let initial_frames = initial_frames.clamp(
            frames_for_ms(MIN_TARGET_MS, frame_ms),
            frames_for_ms(MAX_TARGET_MS, frame_ms),
        );
        Self {
            active_frames: initial_frames,
            planned_frames: initial_frames,
            holds_remaining: 0,
            frame_ms: frame_ms.max(1),
            settle_since: None,
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
        self.settle_since = None;
    }

    /// 升档用有界静音建立余量；降档只在确有积压时丢最旧帧，让目标深度真实下降。
    /// 稳态下做水位结算：持续盈余丢最旧一帧（见模块文档「稳态水位结算」）。
    pub(super) fn action(
        &mut self,
        requested_frames: usize,
        buffered_frames: usize,
        now: Instant,
    ) -> PlayoutDepthAction {
        let requested_frames = requested_frames.clamp(
            frames_for_ms(MIN_TARGET_MS, self.frame_ms),
            frames_for_ms(MAX_TARGET_MS, self.frame_ms),
        );
        if requested_frames < self.active_frames {
            let drop_frames = self
                .active_frames
                .saturating_sub(requested_frames)
                .min(buffered_frames.saturating_sub(requested_frames));
            self.active_frames = requested_frames;
            self.planned_frames = requested_frames;
            self.holds_remaining = 0;
            self.settle_since = None;
            return if drop_frames > 0 {
                PlayoutDepthAction::DropOldest(drop_frames)
            } else {
                PlayoutDepthAction::Play
            };
        }
        if requested_frames == self.active_frames {
            // 稳态水位结算：盈余持续满窗口才丢一帧；期间任意一拍回落即重新计时。
            if buffered_frames > requested_frames + SETTLE_EXCESS_FRAMES {
                let since = *self.settle_since.get_or_insert(now);
                if now.saturating_duration_since(since) >= SETTLE_EXCESS_WINDOW {
                    self.settle_since = None;
                    self.holds_remaining = 0;
                    return PlayoutDepthAction::DropOldest(1);
                }
            } else {
                self.settle_since = None;
            }
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

        self.settle_since = None;
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
/// 40 ms 及以下档位遇到洞时最多等 5 ms，让紧随其后的冗余副本补洞；
/// 60 ms 档位拥有一帧真实余量，允许等待最多一个帧周期。连续包始终立即交付。
/// 无论目标深度如何，未来包占用封顶 60 ms 等值帧数，防止异常序号或恶意流量无界占用内存。
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
    frame_ms: u32,
    target_frames: usize,
    /// 给洞一整个帧周期重排余量的档位下限（60 ms 等值帧数）。
    full_reorder_frames: usize,
    /// 待排未来包的容量上限（60 ms 等值帧数）。
    reorder_capacity: usize,
    /// 发现洞时给重传留的有界窗口（`Duration::ZERO` = 不等）。
    retransmit_grace: Duration,
    /// 当前这个洞是什么时候发现的；没有洞时为 `None`。
    gap_since: Option<Instant>,
}

impl PacketReorderBuffer {
    pub(super) fn new(frame_period: Duration) -> Self {
        let frame_ms = u32::try_from(frame_period.as_millis()).unwrap_or(20).max(1);
        let reorder_capacity = frames_for_ms(REORDER_CAPACITY_MS, frame_ms);
        Self {
            expected_seq: None,
            pending: Vec::with_capacity(reorder_capacity + 1),
            recent_delivered: Vec::with_capacity(reorder_capacity * 2),
            frame_period,
            frame_ms,
            // 起步档与共享播放目标一致；低档只为同批冗余副本留短暂重排窗口。
            target_frames: frames_for_ms(INITIAL_TARGET_MS, frame_ms),
            full_reorder_frames: frames_for_ms(FULL_REORDER_MS, frame_ms),
            reorder_capacity,
            retransmit_grace: Duration::ZERO,
            gap_since: None,
        }
    }

    /// 设置「等重传」的有界窗口；调用方在没有待补洞时应传 `Duration::ZERO`。
    pub(super) fn set_retransmit_grace(&mut self, grace: Duration) {
        self.retransmit_grace = grace;
    }

    pub(super) fn set_target_frames(&mut self, target_frames: usize) {
        self.target_frames = target_frames.clamp(
            frames_for_ms(MIN_TARGET_MS, self.frame_ms),
            frames_for_ms(MAX_TARGET_MS, self.frame_ms),
        );
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

        if self.reorder_wait().is_zero() || self.pending.len() > self.reorder_capacity {
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
        if !self.has_gap() {
            return Duration::ZERO;
        }
        if self.target_frames >= self.full_reorder_frames {
            return self.frame_period.max(self.gap_wait());
        }
        REDUNDANT_REORDER_GRACE
            .min(self.frame_period)
            .max(self.gap_wait())
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
        let recent_capacity = self.reorder_capacity * 2;
        if self.recent_delivered.len() >= recent_capacity {
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
        let mut depth = AdaptiveJitterDepth::new(20);
        depth.raise_to(frames_for_ms(DEFAULT_TARGET_MS, 20));
        depth.raise_to(3);
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
    fn one_playout_underrun_raises_only_once_at_ten_ms() {
        let mut depth = AdaptiveJitterDepth::new(10);
        depth.raise_to(3);
        assert_eq!(depth.observe(Some(2_000), 1, 10_000), 3);
        for _ in 0..STABLE_WINDOWS_TO_SHRINK - 1 {
            assert_eq!(depth.observe(Some(2_000), 0, 10_000), 3);
        }
        assert_eq!(depth.observe(Some(2_000), 0, 10_000), 2);
    }

    #[test]
    fn a_fresh_controller_starts_at_the_minimum_depth() {
        // 起步档 = 最小档 = 20 ms：它是**出声延迟**的直接来源。这条断言防止有人把起步值
        // 悄悄抬回中等抖动档 —— 那会白送一档延迟（第 114 轮实测：回环 P50 +27.9 ms）。
        let mut depth = AdaptiveJitterDepth::new(20);
        assert_eq!(depth.target_frames, frames_for_ms(INITIAL_TARGET_MS, 20));
        assert_eq!(INITIAL_TARGET_MS, MIN_TARGET_MS);
        assert_eq!(
            frames_for_ms(MIN_TARGET_MS, 20),
            1,
            "起步档就是 20 ms（20 ms 帧 = 1 帧，10 ms 帧 = 2 帧）"
        );
        // 还没有抖动测量时保持起步档，不得自己往上爬。
        assert_eq!(
            depth.observe(None, 0, 20_000),
            frames_for_ms(MIN_TARGET_MS, 20)
        );
    }

    #[test]
    fn jitter_thresholds_map_to_twenty_forty_and_sixty_ms() {
        let mut low = AdaptiveJitterDepth::new(20);
        for _ in 0..STABLE_WINDOWS_TO_SHRINK {
            low.observe(Some(5_000), 0, 20_000);
        }
        assert_eq!(low.target_frames, 1);

        let mut medium = AdaptiveJitterDepth::new(20);
        assert_eq!(medium.observe(Some(20_000), 0, 20_000), 2);

        let mut high = AdaptiveJitterDepth::new(20);
        assert_eq!(high.observe(Some(20_001), 0, 20_000), 3);
    }

    #[test]
    fn renewed_jitter_or_missing_measurements_cancel_fast_recovery() {
        let mut depth = AdaptiveJitterDepth::new(20);
        depth.raise_to(6);
        for _ in 0..4 {
            assert_eq!(depth.observe(Some(2_000), 0, 20_000), 6);
        }
        assert_eq!(depth.observe(None, 0, 20_000), 6);
        for _ in 0..4 {
            assert_eq!(depth.observe(Some(2_000), 0, 20_000), 6);
        }
        assert_eq!(depth.observe(Some(2_000), 1, 20_000), 6);
        for _ in 0..4 {
            assert_eq!(depth.observe(Some(2_000), 0, 20_000), 6);
        }
        assert_eq!(depth.observe(Some(2_000), 0, 20_000), 5);
    }

    #[test]
    fn burst_jitter_gets_bounded_headroom_and_eventually_returns_to_low_latency() {
        let mut depth = AdaptiveJitterDepth::new(20);
        assert_eq!(depth.observe(Some(70_000), 0, 20_000), 5);
        assert_eq!(depth.observe(Some(u32::MAX), 1, 20_000), 6);
        for _ in 0..25 {
            depth.observe(Some(2_000), 0, 20_000);
        }
        assert_eq!(depth.target_frames, 1);
    }

    #[test]
    fn ten_ms_frames_double_every_protection_pad() {
        // 毫秒保护垫在两种帧长下的换算表：20 ms 档必须与拆分前的帧数常量一致（1/2/6/3/3），
        // 10 ms 档全部翻倍（2/4/12/6/6），墙钟余量不随档位变化。
        assert_eq!(frames_for_ms(MIN_TARGET_MS, 20), 1);
        assert_eq!(frames_for_ms(INITIAL_TARGET_MS, 20), 1);
        assert_eq!(frames_for_ms(DEFAULT_TARGET_MS, 20), 2);
        assert_eq!(frames_for_ms(MAX_TARGET_MS, 20), 6);
        assert_eq!(frames_for_ms(FULL_REORDER_MS, 20), 3);
        assert_eq!(frames_for_ms(REORDER_CAPACITY_MS, 20), 3);

        assert_eq!(frames_for_ms(MIN_TARGET_MS, 10), 2);
        assert_eq!(frames_for_ms(INITIAL_TARGET_MS, 10), 2);
        assert_eq!(frames_for_ms(DEFAULT_TARGET_MS, 10), 4);
        assert_eq!(frames_for_ms(MAX_TARGET_MS, 10), 12);
        assert_eq!(frames_for_ms(FULL_REORDER_MS, 10), 6);
        assert_eq!(frames_for_ms(REORDER_CAPACITY_MS, 10), 6);
    }

    #[test]
    fn ten_ms_controller_uses_doubled_depths() {
        let mut depth = AdaptiveJitterDepth::new(10);
        assert_eq!(depth.target_frames, 2, "起步档 20 ms 在 10 ms 帧下 = 2 帧");
        assert_eq!(
            depth.observe(Some(10_000), 0, 10_000),
            4,
            "中等抖动档 40 ms 在 10 ms 帧下 = 4 帧"
        );

        let mut high = AdaptiveJitterDepth::new(10);
        high.raise_to(11);
        assert_eq!(
            high.observe(Some(u32::MAX), 1, 10_000),
            12,
            "上限 120 ms 在 10 ms 帧下 = 12 帧"
        );
    }

    #[test]
    fn ten_ms_reorder_buffer_repairs_reordering_at_sixty_ms_target() {
        let now = Instant::now();
        let mut buffer = PacketReorderBuffer::new(Duration::from_millis(10));
        buffer.set_target_frames(6);
        assert_eq!(seqs(&push(&mut buffer, 0, now)), [0]);
        assert!(
            push(&mut buffer, 2, now + Duration::from_millis(10))
                .ready
                .is_empty()
        );
        let repaired = push(&mut buffer, 1, now + Duration::from_millis(15));
        assert_eq!(seqs(&repaired), [1, 2]);
        assert_eq!(repaired.late_drops, 0);
    }

    #[test]
    fn ten_ms_reorder_capacity_holds_six_future_packets() {
        let now = Instant::now();
        let mut buffer = PacketReorderBuffer::new(Duration::from_millis(10));
        buffer.set_target_frames(6);
        push(&mut buffer, 0, now);
        // 60 ms 容量 = 6 帧：6 个未来包都留住，第 7 个越界才强迫放行最旧包。
        for seq in 2..=7 {
            assert!(
                push(&mut buffer, seq, now).ready.is_empty(),
                "序号 {seq} 应在 60 ms 容量内等待"
            );
        }
        let overflow = push(&mut buffer, 9, now);
        assert_eq!(seqs(&overflow), [2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn frame_length_switch_rescales_depth_by_wall_clock() {
        // 混档换档（OPEN_STREAM 协商）：当前深度的**毫秒**水位不变，帧数跟着帧长重换算。
        assert_eq!(rescale_depth_frames(1, 20, 10), 2, "20 ms → 2×10 ms");
        assert_eq!(rescale_depth_frames(2, 10, 20), 1, "2×10 ms → 20 ms");
        assert_eq!(rescale_depth_frames(6, 20, 10), 12, "120 ms → 12×10 ms");
        assert_eq!(rescale_depth_frames(12, 10, 20), 6, "12×10 ms → 120 ms");
        // 重换算结果始终夹在新帧长的边界内。
        assert_eq!(rescale_depth_frames(usize::MAX, 20, 10), 12);
        assert_eq!(rescale_depth_frames(0, 20, 10), 2);
    }

    #[test]
    fn controller_switch_to_ten_ms_rescales_target_and_bounds() {
        // 20 → 10：起步 1 帧（20 ms）重换算为 2 帧，随后 min/中等/上限边界全部按 10 ms 帧计。
        let mut depth = AdaptiveJitterDepth::new(20);
        depth.set_frame_ms(10);
        assert_eq!(depth.target_frames, 2, "20 ms 水位在 10 ms 帧下 = 2 帧");
        assert_eq!(depth.observe(Some(10_000), 0, 10_000), 4);
        let mut high = AdaptiveJitterDepth::new(20);
        high.raise_to(6);
        high.set_frame_ms(10);
        assert_eq!(high.target_frames, 12, "120 ms 水位在 10 ms 帧下 = 12 帧");
        assert_eq!(
            high.observe(Some(u32::MAX), 1, 10_000),
            12,
            "上限夹到 12 帧"
        );
    }

    #[test]
    fn controller_switch_to_twenty_ms_rescales_target_and_bounds() {
        // 10 → 20：4 帧（40 ms）重换算为 2 帧，升档上限回到 6 帧。
        let mut depth = AdaptiveJitterDepth::new(10);
        depth.raise_to(4);
        depth.set_frame_ms(20);
        assert_eq!(depth.target_frames, 2, "40 ms 水位在 20 ms 帧下 = 2 帧");
        assert_eq!(depth.observe(Some(u32::MAX), 1, 20_000), 6, "上限夹到 6 帧");
        // 同帧长重设是幂等空操作。
        depth.set_frame_ms(20);
        assert_eq!(depth.target_frames, 6);
    }

    #[test]
    fn playout_rebuffer_holds_only_the_bounded_depth_deficit() {
        let now = Instant::now();
        let mut from_forty = PlayoutDepthState::new(2, 20);
        assert_eq!(from_forty.action(3, 0, now), PlayoutDepthAction::Hold);
        assert_eq!(from_forty.action(3, 0, now), PlayoutDepthAction::Hold);
        assert_eq!(from_forty.action(3, 0, now), PlayoutDepthAction::Hold);
        assert_eq!(from_forty.action(3, 0, now), PlayoutDepthAction::Play);
        assert_eq!(from_forty.active_frames(), 3);

        let mut from_twenty = PlayoutDepthState::new(1, 20);
        assert_eq!(from_twenty.action(3, 1, now), PlayoutDepthAction::Hold);
        assert_eq!(from_twenty.action(3, 2, now), PlayoutDepthAction::Hold);
        assert_eq!(from_twenty.action(3, 3, now), PlayoutDepthAction::Play);
        assert_eq!(from_twenty.active_frames(), 3);
    }

    #[test]
    fn playout_rebuffer_finishes_early_when_target_depth_arrives() {
        let now = Instant::now();
        let mut state = PlayoutDepthState::new(2, 20);
        assert_eq!(state.action(3, 1, now), PlayoutDepthAction::Hold);
        assert_eq!(state.action(3, 3, now), PlayoutDepthAction::Play);
        assert_eq!(state.active_frames(), 3);
    }

    #[test]
    fn max_depth_underrun_can_refill_without_another_depth_raise() {
        let now = Instant::now();
        let mut state = PlayoutDepthState::new(3, 20);
        state.refill_after_underrun(1);

        assert_eq!(state.action(3, 1, now), PlayoutDepthAction::Hold);
        assert_eq!(state.action(3, 2, now), PlayoutDepthAction::Hold);
        assert_eq!(state.action(3, 3, now), PlayoutDepthAction::Play);
        assert_eq!(state.active_frames(), 3);
    }

    #[test]
    fn playout_downshift_drops_only_real_excess_depth() {
        let now = Instant::now();
        let mut state = PlayoutDepthState::new(3, 20);
        assert_eq!(state.action(2, 3, now), PlayoutDepthAction::DropOldest(1));
        assert_eq!(state.active_frames(), 2);

        let mut already_shallow = PlayoutDepthState::new(3, 20);
        assert_eq!(already_shallow.action(2, 2, now), PlayoutDepthAction::Play);
        assert_eq!(already_shallow.active_frames(), 2);
    }

    #[test]
    fn steady_state_settles_sustained_excess_one_frame_per_window() {
        let start = Instant::now();
        let mut state = PlayoutDepthState::new(2, 20);
        // 持续盈余（目标 2 帧、水位 5 帧）：窗口内不结算，到点结算一帧，然后重新计时。
        assert_eq!(state.action(2, 5, start), PlayoutDepthAction::Play);
        assert_eq!(
            state.action(
                2,
                5,
                start + SETTLE_EXCESS_WINDOW - Duration::from_millis(1)
            ),
            PlayoutDepthAction::Play
        );
        assert_eq!(
            state.action(2, 5, start + SETTLE_EXCESS_WINDOW),
            PlayoutDepthAction::DropOldest(1)
        );
        // 结算后窗口重置：水位仍超标也不连丢。
        let after = start + SETTLE_EXCESS_WINDOW + Duration::from_millis(20);
        assert_eq!(state.action(2, 4, after), PlayoutDepthAction::Play);
        assert_eq!(
            state.action(2, 4, after + SETTLE_EXCESS_WINDOW),
            PlayoutDepthAction::DropOldest(1)
        );
    }

    #[test]
    fn transient_excess_restarts_the_settlement_window() {
        let start = Instant::now();
        let mut state = PlayoutDepthState::new(2, 20);
        assert_eq!(state.action(2, 5, start), PlayoutDepthAction::Play);
        // 突发回落 → 计时清零；再次超标要重新计满一个窗口才结算。
        assert_eq!(
            state.action(2, 2, start + Duration::from_millis(300)),
            PlayoutDepthAction::Play
        );
        let again = start + Duration::from_millis(400);
        assert_eq!(state.action(2, 5, again), PlayoutDepthAction::Play);
        assert_eq!(
            state.action(2, 5, again + SETTLE_EXCESS_WINDOW),
            PlayoutDepthAction::DropOldest(1)
        );
    }

    #[test]
    fn one_frame_swing_above_target_never_settles() {
        // 目标之上的一整帧摆幅是正常抖动：永不结算（与被移除的「每拍丢帧」护栏相区别）。
        let start = Instant::now();
        let mut state = PlayoutDepthState::new(2, 20);
        for beat in 0..100u64 {
            assert_eq!(
                state.action(2, 3, start + Duration::from_millis(beat * 20)),
                PlayoutDepthAction::Play
            );
        }
    }

    #[test]
    fn gear_change_resets_the_settlement_window() {
        let start = Instant::now();
        let mut state = PlayoutDepthState::new(3, 20);
        assert_eq!(state.action(3, 6, start), PlayoutDepthAction::Play);
        // 降档按自身口径丢一帧，同时结算计时清零。
        assert_eq!(
            state.action(2, 6, start + Duration::from_millis(300)),
            PlayoutDepthAction::DropOldest(1)
        );
        // 新档位下的超标重新计窗口：不到点不结算。
        let raised = start + Duration::from_millis(600);
        assert_eq!(state.action(2, 5, raised), PlayoutDepthAction::Play);
        assert_eq!(
            state.action(2, 5, raised + SETTLE_EXCESS_WINDOW),
            PlayoutDepthAction::DropOldest(1)
        );
    }

    #[test]
    fn controller_publish_does_not_overwrite_concurrent_underrun_raise() {
        let shared = AtomicUsize::new(3);
        assert_eq!(publish_target(&shared, 2, 1, 20), 3);
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
    fn closing_retransmit_grace_keeps_only_the_short_redundancy_window() {
        let mut buffer = PacketReorderBuffer::new(Duration::from_millis(20));
        buffer.set_retransmit_grace(Duration::from_millis(30));
        let start = Instant::now();
        push(&mut buffer, 0, start);
        buffer.set_retransmit_grace(Duration::ZERO);
        let batch = push(&mut buffer, 2, start + Duration::from_millis(20));
        assert!(batch.ready.is_empty());
        assert_eq!(
            buffer.next_deadline(),
            Some(start + Duration::from_millis(25))
        );
        assert!(
            buffer
                .flush_expired(start + Duration::from_millis(24))
                .ready
                .is_empty()
        );
        assert_eq!(
            seqs(&buffer.flush_expired(start + Duration::from_millis(25))),
            [2]
        );
        assert_eq!(
            push(&mut buffer, 1, start + Duration::from_millis(26)).late_drops,
            1
        );
    }
}
