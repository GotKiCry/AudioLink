//! 遥测聚合（`docs/03-protocol.md` §10、`docs/02-architecture.md` §10）
//!
//! 实时路径上每秒会产生几千个「此刻发生了什么」的原始量（帧延迟、到达间隔、丢包…）。
//! 这些量**不能**直接往 UI 或控制流上灌 —— 架构 §4 要求遥测聚合「每 500 ms 一次，写入无锁环形缓冲」。
//! 本模块是那一层的核心：把原始量记进**固定容量**的窗口，再按 **1 Hz** 汇总成一份 [`StreamStats`]。
//!
//! # 两个刻意的口径决定
//!
//! 1. **`e2e_latency_us` 取窗口 P50，不取均值。** 均值会被偶发的调度尖刺（GC、驱动重排、进程被抢占）
//!    整体拉高，看起来像「链路变慢了」，其实是本机抖动；中位数回答的才是「典型体验多少毫秒」。
//! 2. **丢包与码率按 1 Hz 窗口结算，不用累计值。** 累计值只会单调上升，看不出「现在」好不好；
//!    而自适应码率（M2）与 UI 曲线要的都是瞬时量。

use audiolink_audio::{SampleStats, Summary};
use audiolink_types::{CodecStats, StreamStats};

use crate::silence::{SilenceSegment, SilenceSnapshot, SilenceTracker};

/// 端到端延迟样本窗口的默认容量。
///
/// 1 Hz 汇总、每帧一个样本（20 ms 帧 = 50 样本/秒）→ 1200 样本 ≈ 24 秒。
/// 取 24 秒是为了让 P95 有足够样本量（1200 × 5% = 60 个样本落在尾部），
/// 同时窗口又足够短，能在十几秒内反映链路变化。
pub const DEFAULT_E2E_WINDOW: usize = 1_200;

/// 到达间隔抖动样本窗口的默认容量（同样按 24 秒估）。
pub const DEFAULT_JITTER_WINDOW: usize = 1_200;

/// 1 Hz 遥测聚合器。
///
/// 用法：实时路径每秒调若干次 `record_*` / `set_*`，然后**每秒调一次 [`Self::roll`]**，
/// 把返回的快照通过控制帧 `STREAM_STATS` 发出去或推给 UI。
#[derive(Debug)]
pub struct TelemetryAggregator {
    stream_id: u32,
    codec: CodecStats,

    e2e: SampleStats,
    jitter: SampleStats,

    // ---- 窗口计数（每次 roll 清零）----
    window_expected: u32,
    window_lost: u32,
    window_bytes: u64,

    // ---- 累计计数（只增不减）----
    packets_received: u64,
    underruns: u32,
    late_drops: u32,
    /// 抖动深度**降档**时控制器主动丢弃的最旧拍数。
    ///
    /// 与 `late_drops` 的分工（第 83/85 轮定位，Lead 的定性依据）：`late_drops` 是
    /// 「帧**到得太晚、来不及播**」—— 网络 / 调度质量问题；这里是**控制器的主动策略选择**
    /// （`PlayoutDepthAction::DropOldest`：丢最旧一帧换取更低的排队延迟），干净链路上每次降档
    /// 必然发生。两者成因不同，共用计数器会让严格档把一个「设计上每次都会发生的策略代价」
    /// 判成质量违规。
    ///
    /// **刻意不进 [`StreamStats`]**：那是 `STREAM_STATS`(0x13) 的 postcard 载荷，字段顺序即
    /// wire 顺序，而解码端**拒绝尾随字节**（`audiolink-proto` 的 `payload_decode`）——
    /// 往结构体末尾追加字段会让旧版本节点的解码直接报错（破坏 §13 兼容）。
    /// 因此它是**进程内可见**的观测计数：同进程消费方用 [`Self::depth_drops`] 读；
    /// 要跨机 / 上 UI，必须走协议版本升级那条路，不能靠给 `StreamStats` 加字段。
    depth_drops: u32,
    plc_count: u32,
    nack_count: u32,

    /// **连续静音**（「长断音」）统计 —— M2 待决口径第 ④ 条。
    ///
    /// **刻意不进 [`StreamStats`]**：与 `depth_drops` 同理，那是冻结的 wire schema（追加字段
    /// 会让旧版本节点解码报错）。进程内消费方用 [`Self::silence`] 读；跨机 / 上 UI 走
    /// [`crate::EngineEvent::SilenceDetected`] 事件。
    ///
    /// 为什么不能从既有计数推出来：`underruns` / `late_drops` / `plc_count` 都是**累计计数**，
    /// 而「静音总占比 1%」与「一次 600 ms 的连续静音」是两件事 —— 前者用户听不出、后者听得出，
    /// 累计计数对二者给同一个数字（`docs/22-m2-soak-runner.md` §11.5）。
    silence: SilenceTracker,

    // ---- 外部注入的瞬时值 ----
    rtt_us: u32,
    clock_offset_us: i32,
    drift_ppm: i32,
    buffer_level_us: u32,

    // ---- 最近一次 roll 的结算结果 ----
    loss_pct_x100: u16,
    bitrate_bps: u32,
}

impl TelemetryAggregator {
    /// 新建聚合器（窗口容量见 [`DEFAULT_E2E_WINDOW`] / [`DEFAULT_JITTER_WINDOW`]）。
    pub fn new(stream_id: u32, codec: CodecStats) -> Self {
        Self::with_windows(stream_id, codec, DEFAULT_E2E_WINDOW, DEFAULT_JITTER_WINDOW)
    }

    /// 指定窗口容量（测试用较小容量以便构造分位场景）。
    pub fn with_windows(
        stream_id: u32,
        codec: CodecStats,
        e2e_capacity: usize,
        jitter_capacity: usize,
    ) -> Self {
        Self {
            stream_id,
            codec,
            e2e: SampleStats::new(e2e_capacity),
            jitter: SampleStats::new(jitter_capacity),
            window_expected: 0,
            window_lost: 0,
            window_bytes: 0,
            packets_received: 0,
            underruns: 0,
            late_drops: 0,
            depth_drops: 0,
            plc_count: 0,
            nack_count: 0,
            silence: SilenceTracker::default(),
            rtt_us: 0,
            clock_offset_us: 0,
            drift_ppm: 0,
            buffer_level_us: 0,
            loss_pct_x100: 0,
            bitrate_bps: 0,
        }
    }

    /// 流 ID。
    pub const fn stream_id(&self) -> u32 {
        self.stream_id
    }

    // -----------------------------------------------------------------
    // 实时路径写入点（调用方保证不分配、不加锁）
    // -----------------------------------------------------------------

    /// 记录一帧的端到端延迟（µs）—— 采集时刻 → 预计出声时刻，**验收主指标**。
    pub fn record_e2e(&mut self, latency_us: u32) {
        self.e2e.push(latency_us);
    }

    /// 记录一次到达间隔抖动（µs）＝ |实际到达间隔 − 标称帧时长|。
    pub fn record_jitter(&mut self, jitter_us: u32) {
        self.jitter.push(jitter_us);
    }

    /// 记录「本周期本该收到的一个包」—— 无论后来是收到了还是丢了。
    ///
    /// 这个口径让丢包率有稳定的分母：分母随发送端的包率走，不受「丢包后收不到包」影响，
    /// 否则丢包会让分母一起塌陷，算出来的丢包率会系统性偏低。
    pub fn record_expected(&mut self) {
        self.record_expected_frames(1);
    }

    /// 一次记录多个本应收到的包（序号跳跃时避免逐包加锁/循环）。
    pub fn record_expected_frames(&mut self, count: u32) {
        self.window_expected = self.window_expected.saturating_add(count);
    }

    /// 记录成功收到一个数据报（`payload_len` 为载荷字节数，用于算实际码率）。
    pub fn record_received(&mut self, payload_len: usize) {
        self.packets_received = self.packets_received.saturating_add(1);
        self.window_bytes = self.window_bytes.saturating_add(payload_len as u64);
    }

    /// 记录一次**主动丢帧**（`PlayoutDepthAction::DropOldest`）。
    ///
    /// ⚠️ 自 2026-09-18 起它有**两个来源**，速率**不再等于「降档频率」**：
    /// ① 抖动深度降档（控制器决策）；② **水位护栏** —— 引擎播放环积压超过
    /// 「目标 + `PLAYOUT_DEPTH_GUARD_FRAMES`」时，播放线程每拍丢最旧一帧（真机 30 min
    /// 水位 43 → 276 ms 的直接治理；排播模式下护栏不生效，不会动组内时间轴）。
    ///
    /// 与 [`Self::record_late_drop`] 是两个口径：那条是「帧到得太晚」，这条是「主动用
    /// 丢弃换低延迟」。详见本结构体 `depth_drops` 字段的说明。
    pub fn record_depth_drop(&mut self) {
        self.record_depth_drops(1);
    }

    /// 一次记录多拍降档主动丢弃（降一档最多两帧）。
    pub fn record_depth_drops(&mut self, count: u32) {
        self.depth_drops = self.depth_drops.saturating_add(count);
    }

    /// 记录一个数据报确定为丢失（乱序等待超时 / 序号跳跃确认）。
    pub fn record_lost(&mut self, count: u32) {
        self.window_lost = self.window_lost.saturating_add(count);
    }

    /// 记录一次播放欠载（`2001`）。
    pub fn record_underrun(&mut self) {
        self.record_underruns(1);
    }

    /// 一次记录多个已经错过的播放拍。
    ///
    /// 系统挂起或音频后端短暂停顿时，播放线程可能跨过多帧；逐帧循环计数会让恢复路径
    /// 与停顿时长成正比。聚合写入既保留真实账本，也让恢复成本保持常数级。
    pub fn record_underruns(&mut self, count: u32) {
        self.underruns = self.underruns.saturating_add(count);
    }

    /// 记录一次迟到丢弃（§7 第 3 条：超过目标时刻 20 ms 的块直接跳过）。
    pub fn record_late_drop(&mut self) {
        self.record_late_drops(1);
    }

    /// 一次记录多个过期帧丢弃。
    pub fn record_late_drops(&mut self, count: u32) {
        self.late_drops = self.late_drops.saturating_add(count);
    }

    /// 记录一次丢包隐藏（`plc_count`）。
    pub fn record_plc(&mut self) {
        self.record_plc_frames(1);
    }

    /// 一次记录多个丢包隐藏帧。
    pub fn record_plc_frames(&mut self, count: u32) {
        self.plc_count = self.plc_count.saturating_add(count);
    }

    /// 记录一次重传请求（M2 起启用；M1 恒为 0）。
    pub fn record_nack(&mut self) {
        self.nack_count = self.nack_count.saturating_add(1);
    }

    /// 记录一拍播放输出：`peak` 是这一拍**写出去**的 PCM 峰值，`synthetic` = 这一拍是引擎补的
    /// 静音（而非真实帧内容）。
    ///
    /// 返回**刚结算**的连续静音段（没有 → `None`）：播放线程据此发
    /// [`crate::EngineEvent::SilenceDetected`]。统计器本身不碰事件总线，因此可以单测。
    ///
    /// 实时路径：内部只做比较与饱和算术，不分配；锁由调用方（播放线程）持有。
    pub fn record_playout_beat(
        &mut self,
        peak: f32,
        frame_ms: u32,
        synthetic: bool,
    ) -> Option<SilenceSegment> {
        self.silence.observe(peak, frame_ms, synthetic)
    }

    /// 结算进行中的静音段（关流 / 会话结束时调用）：不结算就会漏掉「关流时正在静音」那一段。
    pub fn finish_silence(&mut self) -> Option<SilenceSegment> {
        self.silence.finish()
    }

    /// 连续静音快照（本机视角；**不在 [`StreamStats`] 里**，理由见 `silence` 字段说明）。
    pub fn silence(&self) -> SilenceSnapshot {
        self.silence.snapshot()
    }

    // -----------------------------------------------------------------
    // 外部注入的瞬时值
    // -----------------------------------------------------------------

    /// 更新 QUIC 平滑 RTT（µs）。
    pub fn set_rtt_us(&mut self, rtt_us: u32) {
        self.rtt_us = rtt_us;
    }

    /// 更新时钟偏移与漂移（§6 的估计结果）。
    pub fn set_clock(&mut self, offset_us: i32, drift_ppm: i32) {
        self.clock_offset_us = offset_us;
        self.drift_ppm = drift_ppm;
    }

    /// 更新播放环水位（µs）。
    pub fn set_buffer_level_us(&mut self, level_us: u32) {
        self.buffer_level_us = level_us;
    }

    /// 覆盖编码参数（协商完成后由 `OPEN_STREAM_ACK` 的结果决定）。
    pub fn set_codec(&mut self, codec: CodecStats) {
        self.codec = codec;
    }

    // -----------------------------------------------------------------
    // 1 Hz 结算
    // -----------------------------------------------------------------

    /// 结算并滚动 1 Hz 窗口：算丢包率与瞬时码率，清零窗口计数。
    ///
    /// 返回**本次窗口**的 `(loss_pct_x100, bitrate_bps)`，便于调用方单独记账。
    pub fn roll(&mut self) -> (u16, u32) {
        self.loss_pct_x100 = if self.window_expected == 0 {
            0
        } else {
            let scaled = u64::from(self.window_lost).saturating_mul(10_000)
                / u64::from(self.window_expected);
            u16::try_from(scaled.min(10_000)).unwrap_or(10_000)
        };

        // 一个 1 Hz 窗口内的字节数 × 8 = 瞬时码率（bps）。
        self.bitrate_bps = u32::try_from(self.window_bytes.saturating_mul(8)).unwrap_or(u32::MAX);

        self.window_expected = 0;
        self.window_lost = 0;
        self.window_bytes = 0;

        (self.loss_pct_x100, self.bitrate_bps)
    }

    /// 当前快照（§10 的 `StreamStats`），可直接作为 `STREAM_STATS` 载荷发送。
    ///
    /// `e2e_latency_us` 取窗口 P50：它是**验收主指标**，必须反映「典型体验」而不是被尖刺污染。
    ///
    /// **`depth_drops` 不在这个快照里**（见同名字段说明）：`StreamStats` 是冻结的 wire schema，
    /// 追加字段会打断旧版本节点的解码。要读它请用 [`Self::depth_drops`]。
    pub fn snapshot(&self) -> StreamStats {
        StreamStats {
            stream_id: self.stream_id,
            rtt_us: self.rtt_us,
            jitter_us: self.jitter.summary().map_or(0, |s| s.p50),
            jitter_p95_us: self.jitter.summary().map_or(0, |s| s.p95),
            loss_pct_x100: self.loss_pct_x100,
            bitrate_bps: self.bitrate_bps,
            codec: self.codec,
            clock_offset_us: self.clock_offset_us,
            drift_ppm: self.drift_ppm,
            buffer_level_us: self.buffer_level_us,
            underruns: self.underruns,
            plc_count: self.plc_count,
            nack_count: self.nack_count,
            e2e_latency_us: self.e2e.summary().map_or(0, |s| s.p50),
            late_drops: self.late_drops,
        }
    }

    /// 端到端延迟的完整分位摘要（`min / p50 / p95 / p99 / max`）。
    ///
    /// 验收报告要的是 P50 **与** P95（`docs/05-roadmap.md` M1），所以这里给出全部而不是只给中位数。
    pub fn e2e_summary(&self) -> Option<Summary> {
        self.e2e.summary()
    }

    /// 抖动摘要。
    pub fn jitter_summary(&self) -> Option<Summary> {
        self.jitter.summary()
    }

    /// 累计收到的包数。
    pub const fn packets_received(&self) -> u64 {
        self.packets_received
    }

    /// 降档主动丢帧的累计拍数（**不在 [`StreamStats`] 里**，见字段说明；真迟到仍看
    /// `StreamStats::late_drops`）。
    pub const fn depth_drops(&self) -> u32 {
        self.depth_drops
    }

    /// 端到端窗口内的样本数（判断「有没有跑够样本」用）。
    pub fn e2e_samples(&self) -> usize {
        self.e2e.len()
    }

    /// 清空所有统计（会话重建时调用）。
    pub fn reset(&mut self) {
        self.e2e.clear();
        self.jitter.clear();
        self.window_expected = 0;
        self.window_lost = 0;
        self.window_bytes = 0;
        self.packets_received = 0;
        self.underruns = 0;
        self.late_drops = 0;
        self.depth_drops = 0;
        self.plc_count = 0;
        self.nack_count = 0;
        self.silence.reset();
        self.loss_pct_x100 = 0;
        self.bitrate_bps = 0;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn codec() -> CodecStats {
        CodecStats {
            frame_ms: 20,
            channels: 2,
            complexity: 10,
        }
    }

    #[test]
    fn fresh_aggregator_reports_zeros_not_garbage() {
        let aggregator = TelemetryAggregator::new(7, codec());
        let stats = aggregator.snapshot();

        assert_eq!(stats.stream_id, 7);
        assert_eq!(
            stats.e2e_latency_us, 0,
            "没有样本时必须是 0，不能是未初始化值"
        );
        assert_eq!(stats.jitter_us, 0);
        assert_eq!(stats.loss_pct_x100, 0);
        assert_eq!(stats.bitrate_bps, 0);
        assert_eq!(stats.underruns, 0);
        assert!(aggregator.e2e_summary().is_none());
    }

    #[test]
    fn e2e_uses_p50_so_spikes_do_not_move_the_headline_number() {
        // 典型链路 + 偶发调度尖刺：中位数必须稳住，均值不是我们要的口径。
        let mut aggregator = TelemetryAggregator::with_windows(1, codec(), 64, 64);

        for _ in 0..19 {
            aggregator.record_e2e(40_000); // 19 个 40 ms 的正常帧
        }
        aggregator.record_e2e(400_000); // 1 个 400 ms 的尖刺（被抢占 / 驱动重排）

        let stats = aggregator.snapshot();
        assert_eq!(stats.e2e_latency_us, 40_000, "P50 必须不受单个尖刺影响");

        let summary = aggregator.e2e_summary().unwrap();
        assert_eq!(summary.count, 20);
        assert_eq!(summary.min, 40_000);
        assert_eq!(summary.max, 400_000, "尖刺仍应可见（用来发现本机抖动）");
        assert!(
            summary.mean > 50_000.0 && summary.mean < 60_000.0,
            "均值被尖刺抬高"
        );
    }

    #[test]
    fn loss_rate_uses_expected_as_denominator() {
        // 关键口径：分母是「本该收到多少包」，不是「实际收到多少包」。
        // 否则丢包会让分母一起塌陷，算出来的丢包率系统性偏低。
        let mut aggregator = TelemetryAggregator::with_windows(1, codec(), 64, 64);

        for _ in 0..98 {
            aggregator.record_expected();
            aggregator.record_received(400);
        }
        for _ in 0..2 {
            aggregator.record_expected();
            aggregator.record_lost(1);
        }

        let (loss, _) = aggregator.roll();
        assert_eq!(loss, 200, "2/100 = 2.00%");
        assert_eq!(aggregator.snapshot().loss_pct_x100, 200);
    }

    #[test]
    fn bitrate_is_settled_per_one_second_window_then_resets() {
        let mut aggregator = TelemetryAggregator::with_windows(1, codec(), 64, 64);

        // 一个 1 Hz 窗口内收到 50 个 400 B 的包：50 × 400 × 8 = 160_000 bps = 160 kbps
        for _ in 0..50 {
            aggregator.record_expected();
            aggregator.record_received(400);
        }
        let (_, bitrate) = aggregator.roll();
        assert_eq!(bitrate, 160_000, "20 ms 帧 / 160 kbps 的量级应当对上");

        // 下一窗口还没数据：码率必须归零，而不是保留上一秒的值（否则 UI 会显示假的活动）。
        let (loss, bitrate) = aggregator.roll();
        assert_eq!(bitrate, 0);
        assert_eq!(loss, 0, "空窗口不得除零");
    }

    #[test]
    fn counters_accumulate_across_rolls_while_window_metrics_reset() {
        let mut aggregator = TelemetryAggregator::with_windows(1, codec(), 64, 64);

        aggregator.record_underrun();
        aggregator.record_late_drop();
        aggregator.record_plc();
        aggregator.record_nack();

        aggregator.record_expected();
        aggregator.record_lost(1);
        aggregator.roll();

        // 窗口再滚一次：丢包率归零，但累计计数必须保留。
        aggregator.roll();
        let stats = aggregator.snapshot();

        assert_eq!(stats.loss_pct_x100, 0, "窗口结算后丢包率归零");
        assert_eq!(stats.underruns, 1, "累计计数不得被窗口滚动清零");
        assert_eq!(stats.late_drops, 1);
        assert_eq!(stats.plc_count, 1);
        assert_eq!(stats.nack_count, 1);
    }

    #[test]
    fn depth_drops_are_booked_apart_from_late_drops() {
        // 第 85 轮：降档主动丢帧（DropOldest）与真迟到必须分开记账 ——
        // 共用一个计数器会让严格档把一个每次降档必现的策略代价判成质量违规。
        let mut aggregator = TelemetryAggregator::with_windows(1, codec(), 64, 64);

        aggregator.record_depth_drop();
        aggregator.record_depth_drops(2);
        aggregator.record_late_drop();

        assert_eq!(aggregator.depth_drops(), 3, "主动丢帧按拍累计");
        assert_eq!(
            aggregator.snapshot().late_drops,
            1,
            "主动丢帧不得污染 late_drops"
        );

        // 真迟到仍只走 late_drops，反向也不串。
        aggregator.record_late_drop();
        assert_eq!(aggregator.snapshot().late_drops, 2);
        assert_eq!(aggregator.depth_drops(), 3, "真迟到不得污染 depth_drops");

        aggregator.reset();
        assert_eq!(aggregator.depth_drops(), 0, "会话重建必须清零");
        assert_eq!(aggregator.snapshot().late_drops, 0);
    }

    #[test]
    fn jitter_reports_p50_and_p95_separately() {
        // 抖动要分 P50/P95 两档：P50 决定缓冲深度，P95 决定最坏情况下的欠载风险。
        let mut aggregator = TelemetryAggregator::with_windows(1, codec(), 64, 200);

        for _ in 0..95 {
            aggregator.record_jitter(1_000);
        }
        for _ in 0..5 {
            aggregator.record_jitter(30_000);
        }

        let stats = aggregator.snapshot();
        assert_eq!(stats.jitter_us, 1_000, "P50 落在多数派上");
        assert_eq!(stats.jitter_p95_us, 1_000, "95 分位恰好是分界点");
        assert_eq!(aggregator.jitter_summary().unwrap().max, 30_000);
    }

    #[test]
    fn window_capacity_is_respected_under_overload() {
        // 回归护栏：stats 曾经出现过「声明容量但是无界增长」的缺陷（见 git log）。
        // 这里从遥测侧再确认一次：窗口满后仍能正常出分位数，且不会 panic。
        let mut aggregator = TelemetryAggregator::with_windows(1, codec(), 8, 8);

        for value in 1..=1_000u32 {
            aggregator.record_e2e(value * 100);
            aggregator.record_jitter(value * 10);
        }

        assert_eq!(aggregator.e2e_samples(), 8, "窗口容量必须被真正遵守");
        let summary = aggregator.e2e_summary().unwrap();
        assert!(summary.p50 >= 99_200, "窗口内应当是最近的样本");
        assert_eq!(summary.count, 8);
    }

    #[test]
    fn set_values_are_echoed_verbatim() {
        let mut aggregator = TelemetryAggregator::new(3, codec());
        aggregator.set_rtt_us(4_321);
        aggregator.set_clock(-1_500, 23);
        aggregator.set_buffer_level_us(60_000);
        aggregator.set_codec(CodecStats {
            frame_ms: 10,
            channels: 1,
            complexity: 5,
        });

        let stats = aggregator.snapshot();
        assert_eq!(stats.rtt_us, 4_321);
        assert_eq!(stats.clock_offset_us, -1_500);
        assert_eq!(stats.drift_ppm, 23);
        assert_eq!(stats.buffer_level_us, 60_000);
        assert_eq!(stats.codec.frame_ms, 10);
    }

    #[test]
    fn reset_clears_everything_for_session_rebuild() {
        let mut aggregator = TelemetryAggregator::with_windows(1, codec(), 64, 64);
        aggregator.record_e2e(40_000);
        aggregator.record_underrun();
        aggregator.record_expected();
        aggregator.record_received(400);
        aggregator.roll();

        aggregator.reset();

        let stats = aggregator.snapshot();
        assert_eq!(stats.e2e_latency_us, 0);
        assert_eq!(stats.underruns, 0);
        assert_eq!(stats.bitrate_bps, 0);
        assert_eq!(aggregator.packets_received(), 0);
        assert!(aggregator.e2e_summary().is_none());
    }

    #[test]
    fn playout_beats_feed_the_silence_tracker() {
        // 「有流但内容是静音」这一半是现有遥测的盲区：链路一切正常、计数器全 0，
        // 而用户听到的是长时间的静音。这条路径就是为它准备的。
        let mut aggregator = TelemetryAggregator::with_windows(1, codec(), 8, 8);

        for _ in 0..50 {
            assert!(
                aggregator.record_playout_beat(0.0, 20, false).is_none(),
                "进行中的段不发事件"
            );
        }
        assert_eq!(aggregator.silence().current_ms, 1_000);
        assert_eq!(aggregator.silence().longest_ms, 1_000);

        let segment = aggregator
            .record_playout_beat(0.5, 20, false)
            .expect("回到有声时应当结算这一段");
        assert_eq!(segment.duration_ms, 1_000);
        assert_eq!(segment.content_ms, 1_000);
        assert_eq!(segment.synthetic_ms, 0);
        assert_eq!(aggregator.silence().segments, 1);

        // 分母是「观察过的总拍时长」——静音占比靠它算，与欠载占比同一口径。
        assert_eq!(aggregator.silence().planned_ms, 51 * 20);
        assert_eq!(aggregator.silence().silent_ms, 1_000);
    }

    #[test]
    fn reset_clears_the_silence_ledger_too() {
        let mut aggregator = TelemetryAggregator::with_windows(1, codec(), 8, 8);
        for _ in 0..50 {
            aggregator.record_playout_beat(0.0, 20, true);
        }
        aggregator.reset();
        let snapshot = aggregator.silence();
        assert_eq!(snapshot.longest_ms, 0, "会话重建必须清零静音账");
        assert_eq!(snapshot.current_ms, 0);
        assert_eq!(snapshot.segments, 0);
        assert_eq!(snapshot.planned_ms, 0);
    }

    #[test]
    fn saturation_does_not_overflow_on_absurd_inputs() {
        // 极端输入（时钟跳变 / 伪造包）不得 panic，也不得产生回绕的负数式结果。
        let mut aggregator = TelemetryAggregator::with_windows(1, codec(), 4, 4);
        aggregator.record_lost(u32::MAX);
        aggregator.record_lost(u32::MAX);
        aggregator.record_expected();

        let (loss, _) = aggregator.roll();
        assert_eq!(loss, 10_000, "丢包率上限 100%（saturating，不回绕）");

        aggregator.record_received(usize::MAX);
        aggregator.record_received(usize::MAX);
        let (_, bitrate) = aggregator.roll();
        assert_eq!(bitrate, u32::MAX, "码率上限饱和而不是回绕");
    }
}
