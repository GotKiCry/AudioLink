//! 端到端测量探针（`e2e_latency_us` 的**实测**来源）
//!
//! # 为什么需要它，以及它到底在量什么
//!
//! `docs/05-roadmap.md` 的 M1 验收主指标是 `e2e_latency_us` P50 ≤ 110 ms / P95 ≤ 150 ms。
//! 但「端到端延迟」只有在**同一时间基准**上相减才有意义，而两个端点是两台机器（或两个进程）——
//! 各自的单调时钟起点不同。跨机建立公共基准正是 §6 时钟同步的事（M3）。
//!
//! M1 本机 PC↔PC 验收时有个便利条件：**两端共享同一个进程级单调时钟**。
//! 于是可以绕过时钟同步直接量真实延迟 —— 探针在发送侧记下「这一帧序号封口于何时」，
//! 在接收侧记下「这一帧序号何时被写进 sink」，再按序号配对相减。
//!
//! # 口径必须说清（否则数字会骗人）
//!
//! 探针量的是 **「帧封口 → sink 写出」**，也就是：
//!
//! ```text
//!   采集(PCM 读入)  ──►  组帧封口  ──Opus 编码──►  QUIC 数据报  ──►  抖动/解码  ──►  sink.write 返回
//!                        ▲                                                       ▲
//!                        └────────────── 探针量的是这一段 ─────────────────────┘
//! ```
//!
//! 它**不含**「采集半缓冲」（样本要攒满一帧才封口，平均等半帧 = `frame_ms / 2`）。
//! 报告里必须把这一段单独加上并标明是模型值，不能把它混进实测数字里 ——
//! 这正是既有 `self-loop` 工具的分段账本口径（采集半周期 + 组帧 + 编码 + 解码 + 播放）。
//!
//! # 跨机时怎么办
//!
//! 探针**只在同一进程内有效**（它直接比较 `Instant`）。跨机时 `Instant` 不可相减，
//! 必须走 §6：用 `audiolink_net::ClockEstimator` 估出 `offset`，把对端的时间戳换算到本机基准后再相减。
//! 本类型不试图「猜」那个 offset —— 猜出来的数字看起来一样漂亮，但没有任何意义。

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

use audiolink_audio::{SampleStats, Summary};

/// 序号 → 时刻的配对缓冲容量。
///
/// 取 4096：20 ms 帧下约等于 82 秒。足够覆盖「接收端缓冲很深、发送端已经跑了很多帧」的错位，
/// 又不会在长时间 soak 里无限增长。超容量时丢最旧的 —— 丢的只会是永远配不上对的孤儿。
const DEFAULT_CAPACITY: usize = 4_096;

/// 端到端测量探针（本机 PC↔PC 验收用）。
///
/// 线程安全：发送线程与播放线程会并发写入，内部用一把 `Mutex` 保护两个队列。
/// **它不在音频实时路径上**（调用点在帧封口与 `sink.write` 之后），所以这里的锁与分配是可接受的；
/// 但它也不是免费的，因此生产路径默认不装（`EngineConfig::measurement` 为 `None`）。
#[derive(Debug)]
pub struct MeasurementTap {
    sealed: Mutex<VecDeque<(u32, Instant)>>,
    played: Mutex<VecDeque<(u32, Instant)>>,
    latencies: Mutex<SampleStats>,
    capacity: usize,
}

impl Default for MeasurementTap {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl MeasurementTap {
    /// 指定配对缓冲容量。
    pub fn new(capacity: usize) -> Self {
        Self {
            sealed: Mutex::new(VecDeque::with_capacity(capacity.min(64))),
            played: Mutex::new(VecDeque::with_capacity(capacity.min(64))),
            // 延迟样本窗口：20 ms 帧、60 s 观测 → 3000 样本；取 4096 留余量。
            latencies: Mutex::new(SampleStats::new(4_096)),
            capacity: capacity.max(1),
        }
    }

    /// **发送侧**：记录「序号 `seq` 的帧刚刚封口（可以送去编码了）」。
    pub fn record_sealed(&self, seq: u32, at: Instant) {
        self.pair_up(seq, at, Side::Sealed);
    }

    /// **接收侧**：记录「序号 `seq` 的帧刚刚写进 sink」。
    pub fn record_played(&self, seq: u32, at: Instant) {
        self.pair_up(seq, at, Side::Played);
    }

    /// 记一侧，并在另一侧找同一个序号配对；配不上就留在本侧等对方。
    ///
    /// **必须双向查**：采集线程与播放线程是独立线程，谁先写完全不确定
    /// （接收端缓冲浅时，播放记录可能先到）。只单向查的话，先到的那个会被永远孤立，
    /// 结果是「延迟样本比实际帧数少一截」——数字看着正常，其实是漏统计。
    fn pair_up(&self, seq: u32, at: Instant, side: Side) {
        let (mine, theirs) = match side {
            Side::Sealed => (&self.sealed, &self.played),
            Side::Played => (&self.played, &self.sealed),
        };

        let counterpart = theirs
            .lock()
            .ok()
            .and_then(|mut queue| take_seq(&mut queue, seq));

        let Some(other_at) = counterpart else {
            push_bounded(mine, seq, at, self.capacity);
            return;
        };

        let (sealed_at, played_at) = match side {
            Side::Sealed => (at, other_at),
            Side::Played => (other_at, at),
        };

        // `saturating_duration_since`：极端情况下 played_at < sealed_at（两个队列的写入顺序
        // 与时钟读取顺序不一致），那说明配对错了 —— 算 0，而不是 panic 或回绕成一个天文数字。
        let latency_us = u32::try_from(played_at.saturating_duration_since(sealed_at).as_micros())
            .unwrap_or(u32::MAX);

        if let Ok(mut latencies) = self.latencies.lock() {
            latencies.push(latency_us);
        }
    }

    /// 已成功配对的样本数。
    pub fn matched(&self) -> usize {
        self.latencies
            .lock()
            .map(|stats| stats.len())
            .unwrap_or_default()
    }

    /// 延迟分位摘要（`None` = 还没配上任何一对）。
    pub fn summary(&self) -> Option<Summary> {
        self.latencies.lock().ok().and_then(|stats| stats.summary())
    }

    /// 尚未配上对的记录数（发送侧 / 接收侧）。
    ///
    /// 这个数长期不归零说明序号配对逻辑有问题（例如两端 `stream_id` 混在一起），
    /// 是「数字看起来正常但其实是错的」这类事故的预警。
    pub fn orphan_counts(&self) -> (usize, usize) {
        let sealed = self
            .sealed
            .lock()
            .map(|queue| queue.len())
            .unwrap_or_default();
        let played = self
            .played
            .lock()
            .map(|queue| queue.len())
            .unwrap_or_default();
        (sealed, played)
    }

    /// 清空全部记录（会话重建时调用）。
    pub fn reset(&self) {
        if let Ok(mut sealed) = self.sealed.lock() {
            sealed.clear();
        }
        if let Ok(mut played) = self.played.lock() {
            played.clear();
        }
        if let Ok(mut latencies) = self.latencies.lock() {
            latencies.clear();
        }
    }
}

/// 配对的两侧。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    /// 发送侧（帧封口）。
    Sealed,
    /// 接收侧（写进 sink）。
    Played,
}

/// 有界入队：满了就丢最旧（孤儿记录没有长期保留价值）。
fn push_bounded(queue: &Mutex<VecDeque<(u32, Instant)>>, seq: u32, at: Instant, capacity: usize) {
    let Ok(mut queue) = queue.lock() else {
        return;
    };
    while queue.len() >= capacity {
        queue.pop_front();
    }
    queue.push_back((seq, at));
}

/// 从队列里取出指定序号（线性扫描：配对窗口通常只有几个元素）。
///
/// `VecDeque::remove` 是 O(n)，但 n 就是配对窗口里的未配对记录数（正常只有个位数），
/// 而它保证剩余记录**保持插入顺序** —— 顺序在这里有用：队列满时丢的是最旧的孤儿，
/// 用 `swap_remove` 会把最新的记录换到最前面，淘汰顺序就乱了。
fn take_seq(queue: &mut VecDeque<(u32, Instant)>, seq: u32) -> Option<Instant> {
    let index = queue.iter().position(|(candidate, _)| *candidate == seq)?;
    queue.remove(index).map(|(_, at)| at)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn matched_pair_produces_the_real_delta() {
        let tap = MeasurementTap::default();
        let base = Instant::now();

        tap.record_sealed(1, base);
        tap.record_played(1, base + Duration::from_millis(42));

        let summary = tap.summary().unwrap();
        assert_eq!(summary.count, 1);
        assert_eq!(summary.p50, 42_000, "42 ms 就是 42000 µs");
        assert_eq!(tap.matched(), 1);
    }

    #[test]
    fn out_of_order_playback_still_pairs_up() {
        // 网络乱序是常态：接收侧先播 seq=7 再播 seq=6 不该让配对崩掉。
        let tap = MeasurementTap::default();
        let base = Instant::now();

        tap.record_sealed(6, base);
        tap.record_sealed(7, base + Duration::from_millis(20));

        tap.record_played(
            7,
            base + Duration::from_millis(20) + Duration::from_millis(30),
        );
        tap.record_played(6, base + Duration::from_millis(60));

        assert_eq!(tap.matched(), 2);
        let (orphan_sealed, orphan_played) = tap.orphan_counts();
        assert_eq!((orphan_sealed, orphan_played), (0, 0), "两条都该配上对");
    }

    #[test]
    fn unpaired_playback_does_not_fabricate_a_number() {
        // 收侧来了个发侧没记过的序号（重传 / 串流）→ 必须**不产样本**，
        // 而不是拿一个随便的基准算出一个漂亮的假数字。
        let tap = MeasurementTap::default();

        tap.record_played(99, Instant::now());

        assert_eq!(tap.matched(), 0, "配不上对就不该有样本");
        assert!(tap.summary().is_none());
        assert_eq!(tap.orphan_counts().1, 1, "但要留下痕迹供排查");
    }

    #[test]
    fn p50_and_p95_are_available_for_the_acceptance_report() {
        // M1 验收要的是 P50 与 P95 两个数，缺一不可。
        let tap = MeasurementTap::default();
        let base = Instant::now();

        for seq in 0..100u32 {
            // 前 95 帧 40 ms，最后 5 帧 150 ms
            let latency = if seq < 95 { 40 } else { 150 };
            tap.record_sealed(seq, base);
            tap.record_played(seq, base + Duration::from_millis(latency));
        }

        let summary = tap.summary().unwrap();
        assert_eq!(summary.count, 100);
        assert_eq!(summary.p50, 40_000);
        assert_eq!(summary.p95, 40_000, "95 分位恰好在分界点上");
        assert_eq!(summary.max, 150_000, "尾部尖刺必须可见");
    }

    #[test]
    fn orphan_buffer_is_bounded_under_a_flood() {
        // 长时间 soak 里孤儿记录会不断堆积；容量必须真的生效（否则就是内存泄漏）。
        let tap = MeasurementTap::new(8);

        for seq in 0..1_000u32 {
            tap.record_sealed(seq, Instant::now());
        }

        let (orphan_sealed, _) = tap.orphan_counts();
        assert_eq!(orphan_sealed, 8, "容量必须被真正遵守");
    }

    #[test]
    fn reset_clears_everything_for_session_rebuild() {
        let tap = MeasurementTap::default();
        let base = Instant::now();
        tap.record_sealed(1, base);
        tap.record_played(1, base + Duration::from_millis(10));

        tap.reset();

        assert_eq!(tap.matched(), 0);
        assert!(tap.summary().is_none());
        assert_eq!(tap.orphan_counts(), (0, 0));
    }

    #[test]
    fn concurrent_writers_do_not_lose_pairs() {
        // 发送线程与播放线程会并发写入，配对不能因为竞态丢样本。
        use std::sync::Arc;

        let tap = Arc::new(MeasurementTap::default());
        let base = Instant::now();

        let writer = {
            let tap = Arc::clone(&tap);
            std::thread::spawn(move || {
                for seq in 0..2_000u32 {
                    tap.record_sealed(seq, base);
                }
            })
        };

        for seq in 0..2_000u32 {
            tap.record_played(seq, base + Duration::from_millis(25));
        }
        writer.join().unwrap();

        // 双向配对：无论哪一侧先写，最终都该配上对（这正是 `pair_up` 双向查的意义）。
        assert_eq!(tap.matched(), 2_000, "并发写入不得丢配对");
        assert_eq!(tap.orphan_counts(), (0, 0));
    }

    #[test]
    fn zero_length_ranges_are_not_reported_as_latency() {
        // 同一时刻封口与播放 = 0 µs，是一个合法（虽然不真实）的值；它不该被当成「没有样本」。
        let tap = MeasurementTap::default();
        let now = Instant::now();
        tap.record_sealed(1, now);
        tap.record_played(1, now);

        let summary = tap.summary().unwrap();
        assert_eq!(summary.count, 1);
        assert_eq!(summary.p50, 0);
    }
}
