//! §6 时钟同步接线：`CLOCK_PROBE` / `CLOCK_REPLY` 的收发节奏、四时间戳配对与遥测写入。
//!
//! 规格：`docs/03-protocol.md` §6（四时间戳、首连 100 ms × 50、稳态 1 s、200 样本窗口）、
//! `docs/11-m1-contract.md` §3（`ClockEstimator` 冻结 API）、
//! `docs/10-handoff.md` §4.2（跨机端到端测量的前置）。
//!
//! # 三层分工
//!
//! | 层 | 负责 |
//! |---|---|
//! | `audiolink-proto` | `CLOCK_PROBE`(12 B) / `CLOCK_REPLY`(28 B) 的载荷编解码与整包组装 |
//! | `audiolink-net::ClockEstimator` | **纯逻辑**估计（200 样本窗口 / RTT 最小 8 个取中位数 / 漂移回归），不做 I/O |
//! | 本模块 | 收发节奏、序号配对、错误计数、RTT 分位数、把估计写进遥测 |
//!
//! `ClockEstimator` 的模块文档写得很清楚：「收发节奏（首连 100 ms × 50 次、稳态 1 s）与会话编排属
//! `audiolink-engine`」—— 本模块就是那句话的落地。
//!
//! # 为什么两端都探测（而不是只让发起方探）
//!
//! §6 写的是「单向探测（**默认，双向都可用**）」：一个方向的一次探测只能让**发起方**算出
//! `offset = 对端 − 本机`。而 M1 要的跨机端到端延迟是**接收侧**算的：
//! `e2e = 本地出声时刻 − (对端采集时刻 + offset)`（`docs/10-handoff.md` §4.2）——
//! 若只有发起方探测，接收侧（真机验收里的 Android）永远拿不到 offset，跨机那一段就只能继续空着。
//! 所以两端各自独立跑 §6 的节奏、各自维护自己的估计器与 RTT 环、各自应答对方的探测；
//! 线上仍只有协议里那一对 ptype，**没有新增任何报文**。
//!
//! # 两个刻意的口径
//!
//! 1. **应答探测不依赖 §5 会话状态**：`Ptype::ClockProbe` 的处理点在会话任务的**阶段一（握手前）**
//!    与阶段二都开着，所以 `audiolink-tools` 的 `latency-probe` 那种「只发数据报、不跑握手」的
//!    测量连接照样能拿到 REPLY（它是 M0 交付物，也是 `docs/05-roadmap.md` M1 验收点名的跨机测量手段；
//!    若只在会话建立后才回包，真机验收量 PC→手机 RTT 这条路就是死的）。
//!    安全口径（Lead 已认可）：这里**不**额外要求 §5 认证 —— TLS 握手已经保证「对端可达且能完成握手」，
//!    且 12 B 进 → 28 B 出（放大 2.33×，还必须先完成 TLS 握手才到得了这一层），不构成源地址伪造的
//!    放大面。§5 认证保护的是「能不能收音频」，不是「能不能问时间」。
//! 2. **RTT 分位数与偏移估计同源不同窗**：偏移用 `ClockEstimator` 的 200 样本窗口（带抗差逻辑：
//!    RTT 最小 8 个取中位数），RTT 分位数另开一个 200 样本的 `SampleStats` 环。两者混用会把 P95
//!    变成「最好的 8 个样本里的 P95」——那回答不了「这条链路抖不抖」。
//!
//! # 实时纪律
//!
//! 本模块只在 **tokio 会话任务**里被调用：采集线程与播放线程既不探测、也不碰这里的互斥量，
//! 因此这里可以分配（`Vec` / `HashMap`，控制面而非音频线程）。但仍然**不许 panic**：
//! 对端送来的时间戳再离谱，也只能得到「样本不入窗 + 计数」，不能变成进程崩溃
//! （`docs/02-architecture.md` §4 的铁律）。

use std::collections::{HashMap, VecDeque};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use audiolink_audio::SampleStats;
use audiolink_net::{ClockEstimate, ClockEstimator, ClockSample};
use audiolink_proto::{AudioDatagram, ClockProbe, ClockReply};
use audiolink_types::AudioLinkError;

use crate::telemetry::TelemetryAggregator;

/// 首连快速同步的探测次数（§6 第 0 条：`100 ms` × 50 ≈ 5 s）。
pub const FAST_PROBES: u32 = 50;

/// 首连快速同步的间隔（ms）。
pub const FAST_INTERVAL_MS: u64 = 100;

/// 稳态探测间隔（ms，§6 第 1 条：每 1 s 一次，会话期间不断）。
pub const STEADY_INTERVAL_MS: u64 = 1_000;

/// RTT 环的容量（与 §6.2 的估计窗口同为 200 个样本）。
pub const RTT_WINDOW: usize = 200;

/// 给 RTT 分位数所需的最少样本数。
///
/// 与 §6.3「RTT 最小的 8 个样本」同阈值：不足 8 个样本的 P95 只是噪声，
/// 与其发布一个会被当成「链路很差」的数字，不如明确说「还没测到」（`None`）。
pub const RTT_MIN_SAMPLES: usize = 8;

/// 未决探测表的上限。
///
/// 快速阶段 10 次/秒，表里正常只有 0–2 条。设上限只是为了「对端整段不回」时不无界增长：
/// 超限时淘汰**最旧**的一条，将来若真收到那条的应答，会计入 `unmatched`（如实反映「配不上」），
/// 而不是被静默当成有效样本。
const MAX_OUTSTANDING: usize = 64;

/// 本机单调时钟的进程基准。
///
/// 不用系统时间：`SystemTime` 会被 NTP 校时 / 手动改时间跳变，而 §6 的四个时间戳
/// 必须在**同一套不倒退的时钟**上才谈得上差值。
static MONOTONIC_ORIGIN: LazyLock<Instant> = LazyLock::new(Instant::now);

/// 本机单调时钟（µs）。
///
/// `CLOCK_PROBE.t1` 与 `CLOCK_REPLY.t2 = t3` 都取这里。进程内所有会话共享同一基准，
/// 所以同进程里的两台引擎之间，`offset` 的真值就是 0 —— 集成测试正是据此断言「误差 ≤ 2 ms」。
pub fn now_monotonic_us() -> u64 {
    u64::try_from(MONOTONIC_ORIGIN.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// 时钟探测的收发计数与 RTT 分位数（`Engine::clock_probe_stats` 的载荷）。
///
/// 计数刻意分开：它们的诊断含义完全不同 —— 「本机没发出去」和「发出去了对端没回」
/// 是两条完全不同的排查路径，混成一个「失败数」等于把线索丢掉。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClockProbeStats {
    /// 本机发出的 `CLOCK_PROBE` 数。
    pub sent: u64,
    /// 本机收到的 `CLOCK_PROBE` 数（本机作为响应方）。**不**含解不出来的包。
    pub received: u64,
    /// 收到且**成功配对**上未决探测的 `CLOCK_REPLY` 数。每 1 个 = 1 个四时间戳样本。
    pub replies: u64,
    /// 收到但**配不上**的 `CLOCK_REPLY` 数：`probe_seq` 不在未决表里（从没发过 /
    /// 已被淘汰 / 重复应答），或回填的 `t1` 不是本机发出的那个值，或时间戳为负。
    pub unmatched: u64,
    /// 最近 `RTT_WINDOW` 个有效 RTT 样本的 **P50**（µs）。
    ///
    /// `None` = RTT 环里的样本 < `RTT_MIN_SAMPLES`（**不是** 0：0 会被读成「RTT 为零」）。
    pub rtt_p50_us: Option<u32>,
    /// 同上，95 分位。最坏情况口径（决定缓冲该留多少余量）。
    pub rtt_p95_us: Option<u32>,
    /// RTT 环当前的样本数（0..=`RTT_WINDOW`）。
    ///
    /// 口径：只统计**有效**样本（负 RTT 的异常样本不入环，见 `ClockSample::anomalous`），
    /// 因此它可能小于 `ClockProbeStats::replies`。
    pub rtt_samples: u32,
}

/// 一次会话的时钟探测状态：估计器 + RTT 环 + 未决探测表 + 计数 + 节奏。
///
/// 每个会话（每个对端）一份，随会话建立而新建、随会话结束而丢弃 —— §6 的估计属于
/// 「本机 ↔ 这一个对端」这一对时钟，换对端必须重新收敛（`ClockEstimator::clear` 的文档口径）。
#[derive(Debug)]
pub(crate) struct ClockProbeState {
    estimator: ClockEstimator,
    /// RTT 分位数的环形窗口（§6.2 同容量）。**只在 tokio 会话任务里 push / summary**。
    rtt: SampleStats,
    /// 未决探测：`probe_seq → t1`。应答靠它配对，也靠它挡住伪造/串话的应答。
    outstanding: HashMap<u32, u64>,
    /// 未决探测的插入顺序（淘汰最旧的一条用；至多 `MAX_OUTSTANDING` 项）。
    order: VecDeque<u32>,
    next_seq: u32,
    /// 已发出的探测总数 —— 首连快速阶段与稳态阶段的切换条件（§6 第 0/1 条）。
    probes: u32,
    stats: ClockProbeStats,
}

impl Default for ClockProbeState {
    fn default() -> Self {
        Self {
            estimator: ClockEstimator::new(),
            rtt: SampleStats::new(RTT_WINDOW),
            outstanding: HashMap::new(),
            order: VecDeque::new(),
            next_seq: 0,
            probes: 0,
            stats: ClockProbeStats::default(),
        }
    }
}

impl ClockProbeState {
    /// 空状态（未收敛）。
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 当前计数 + RTT 分位数快照。
    pub(crate) fn stats(&self) -> ClockProbeStats {
        let mut stats = self.stats;
        stats.rtt_samples = u32::try_from(self.rtt.len()).unwrap_or(u32::MAX);
        // 样本不足时给 None：**不许**用 0 假装有值（0 会被读成「延迟为零」）。
        if let Some(summary) = self.rtt.summary()
            && summary.count >= RTT_MIN_SAMPLES
        {
            stats.rtt_p50_us = Some(summary.p50);
            stats.rtt_p95_us = Some(summary.p95);
        }
        stats
    }

    /// 当前估计；**有效样本 < 8 时返回 `None`**（不得用退化值假装收敛）。
    pub(crate) fn estimate(&self) -> Option<ClockEstimate> {
        self.estimator.estimate()
    }

    /// 下一次探测的间隔：前 `FAST_PROBES` 次 100 ms，之后 1 s。
    pub(crate) fn probe_interval(&self) -> Duration {
        if self.probes < FAST_PROBES {
            Duration::from_millis(FAST_INTERVAL_MS)
        } else {
            Duration::from_millis(STEADY_INTERVAL_MS)
        }
    }

    /// 组一个探测包（`t1` = `now_us`）并登记为未决，返回待发送的**整包**字节。
    ///
    /// `t1` 由调用方在**紧邻发送前**取，这样「单程 ≈ RTT/2」这条假设才成立：
    /// 时间戳取早了，整段本地排队时间都会被算进网络延迟里。
    pub(crate) fn build_probe(&mut self, now_us: u64) -> Result<Vec<u8>, AudioLinkError> {
        let probe_seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.probes = self.probes.saturating_add(1);
        self.stats.sent = self.stats.sent.saturating_add(1);
        self.remember(probe_seq, now_us);

        ClockProbe {
            probe_seq,
            t1: as_i64(now_us),
        }
        .to_datagram_bytes()
    }

    /// 处理收到的 `CLOCK_PROBE`：返回**立即回填**的 `CLOCK_REPLY` 整包字节。
    ///
    /// §6 明文 `t3 = t2`（「紧随其后，不引入额外处理延迟」）：这不是偷懒，而是把
    /// 「应答侧处理延迟」这一误差项**主动归零** —— 本函数只做一次定长编码。
    pub(crate) fn on_probe(
        &mut self,
        datagram: &AudioDatagram<'_>,
        now_us: u64,
    ) -> Result<Vec<u8>, AudioLinkError> {
        let probe = ClockProbe::from_datagram(datagram)?;
        self.stats.received = self.stats.received.saturating_add(1);

        let t2 = as_i64(now_us);
        ClockReply {
            probe_seq: probe.probe_seq,
            t1: probe.t1,
            t2,
            t3: t2,
        }
        .to_datagram_bytes()
    }

    /// 处理收到的 `CLOCK_REPLY`：配对成功则记一个四时间戳样本。
    ///
    /// 返回 `Ok(None)` = 与未决探测配不上（已计入 `unmatched`）；
    /// `Err` = 这个数据报本身不合法（调用方按 §1.1「忽略并计数、不断流」处置）。
    pub(crate) fn on_reply(
        &mut self,
        datagram: &AudioDatagram<'_>,
        t4_us: u64,
    ) -> Result<Option<ClockSample>, AudioLinkError> {
        let reply = ClockReply::from_datagram(datagram)?;

        let Some(t1) = self.take_outstanding(reply.probe_seq) else {
            // 从没发过这个序号，或它已经被淘汰 / 已经答过一次：重复应答同样必须挡住，
            // 否则一个坏掉的对端可以靠重放把窗口塞满同一时刻的样本。
            self.stats.unmatched = self.stats.unmatched.saturating_add(1);
            return Ok(None);
        };

        // 对端必须**原样回填** t1：序号对但时间戳不对，说明这不是我们那次探测的应答。
        let (Ok(echoed_t1), Ok(t2), Ok(t3)) = (
            u64::try_from(reply.t1),
            u64::try_from(reply.t2),
            u64::try_from(reply.t3),
        ) else {
            self.stats.unmatched = self.stats.unmatched.saturating_add(1);
            return Ok(None);
        };
        if echoed_t1 != t1 {
            self.stats.unmatched = self.stats.unmatched.saturating_add(1);
            return Ok(None);
        }

        let sample = self.estimator.record(reply.probe_seq, t1, t2, t3, t4_us);
        self.stats.replies = self.stats.replies.saturating_add(1);
        // 异常样本（负 RTT）**不入 RTT 环**：它的 rtt 被夹成 0，混进去会把 P50 拉低，
        // 让「链路变好了」这种假象出现在面板上。与估计窗口的处置保持一致。
        if !sample.anomalous {
            self.rtt
                .push(u32::try_from(sample.rtt_us).unwrap_or(u32::MAX));
        }
        Ok(Some(sample))
    }

    /// 登记一条未决探测（超过上限就淘汰最旧的一条）。
    fn remember(&mut self, probe_seq: u32, t1: u64) {
        if self.outstanding.len() >= MAX_OUTSTANDING
            && let Some(oldest) = self.order.pop_front()
        {
            let _ = self.outstanding.remove(&oldest);
        }
        let _ = self.outstanding.insert(probe_seq, t1);
        self.order.push_back(probe_seq);
    }

    /// 取出并移除一条未决探测。
    fn take_outstanding(&mut self, probe_seq: u32) -> Option<u64> {
        let t1 = self.outstanding.remove(&probe_seq)?;
        if let Some(position) = self.order.iter().position(|seq| *seq == probe_seq) {
            let _ = self.order.remove(position);
        }
        Some(t1)
    }
}

/// 把当前的时钟估计写进遥测：**只写偏移与漂移**。
///
/// **样本 < 8（`estimate() == None`）时什么都不写**：`clock_offset_us` / `drift_ppm`
/// 保持 0。契约（§3）与 §6 都明确禁止用退化值假装收敛 —— 一个基于 2 个样本的「偏移」
/// 会被下游当成可用的 epoch 基准，直接把组内同步搞坏。
///
/// # 为什么这里**不**碰 `rtt_us`（task-9 的口径裁决）
///
/// `docs/03-protocol.md` §10 把 `StreamStats.rtt_us` 定义为「平滑 RTT（QUIC）」。
/// §6 的代表 RTT（RTT 最小的 8 个样本里最大的那个）是**另一种统计**：数值可能相近，定义不同。
/// 让同一个冻结字段在「收敛前 / 收敛后」承载两种口径，UI 与验收报告就会读出无法解释的数字
/// —— 那正是本项目最忌讳的那类事。§6 的数字已经有正式出口：
/// `clock_estimate()` 的 `rtt_us` / `quality` / `samples`，
/// 以及 `clock_probe_stats()` 的 `rtt_p50_us` / `rtt_p95_us` / `rtt_samples`。
/// 所以 `rtt_us` 只有一种来源：会话任务里的 QUIC 平滑 RTT（未收敛时也一样）。
pub(crate) fn publish_clock(telemetry: &mut TelemetryAggregator, estimate: Option<ClockEstimate>) {
    let Some(estimate) = estimate else {
        return;
    };
    telemetry.set_clock(clamp_i64(estimate.offset_us), estimate.drift_ppm);
}

/// `u64` µs → `i64`（越界饱和：时间戳来自对端，非法值最多得到无效估计，绝不 panic）。
fn as_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// `i64` → `i32`（越界夹紧到 `i32` 端点，不回绕）。
fn clamp_i64(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

#[cfg(test)]
// 测试代码不受实时路径的 unwrap / expect / panic 禁令约束（那三条针对运行时音频路径）
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use audiolink_types::{CodecStats, DATAGRAM_HEADER_LEN, Ptype};

    /// 合成样本的真值偏移（对端 − 本机）。取负值是为了让「符号写反」这类错误无处可藏。
    const OFFSET_US: i64 = -37_500;

    fn codec() -> CodecStats {
        CodecStats {
            frame_ms: 20,
            channels: 2,
            complexity: 10,
        }
    }

    /// 造一个 CLOCK_PROBE 整包（走生产编码路径）。
    fn wire_probe(probe_seq: u32, t1: i64) -> Vec<u8> {
        ClockProbe { probe_seq, t1 }
            .to_datagram_bytes()
            .expect("编码 CLOCK_PROBE")
    }

    /// 造一个 CLOCK_REPLY 整包（走生产编码路径）。
    fn wire_reply(probe_seq: u32, t1: i64, t2: i64, t3: i64) -> Vec<u8> {
        ClockReply {
            probe_seq,
            t1,
            t2,
            t3,
        }
        .to_datagram_bytes()
        .expect("编码 CLOCK_REPLY")
    }

    fn decode(bytes: &[u8]) -> AudioDatagram<'_> {
        AudioDatagram::decode(bytes).expect("整包解码")
    }

    /// 走一次完整的「发探测 → 对端回填 → 记 t4」，并把估计写进遥测。
    ///
    /// 合成口径与 audiolink-net::clock 的单测一致：去程 = 回程 = rtt/2、t3 = t2，
    /// 于是样本的 offset 必须**精确**等于 offset_us。
    fn round_trip(
        state: &mut ClockProbeState,
        telemetry: &mut TelemetryAggregator,
        now_us: u64,
        rtt_us: u64,
        offset_us: i64,
    ) -> ClockSample {
        let probe_wire = state.build_probe(now_us).expect("组探测包");
        let probe = ClockProbe::from_datagram(&decode(&probe_wire)).expect("解探测包");

        let half = i64::try_from(rtt_us / 2).expect("半程");
        let t2 = probe.t1 + offset_us + half;
        let reply_wire = wire_reply(probe.probe_seq, probe.t1, t2, t2);

        let sample = state
            .on_reply(&decode(&reply_wire), now_us + rtt_us)
            .expect("解应答包")
            .expect("应答必须与未决探测配对成功");
        publish_clock(telemetry, state.estimate());
        sample
    }

    #[test]
    fn responder_echoes_probe_with_28_byte_payload_and_t2_equal_t3() {
        let mut state = ClockProbeState::new();
        let probe_wire = wire_probe(4242, 1_234_567);
        assert_eq!(
            probe_wire.len(),
            DATAGRAM_HEADER_LEN + ClockProbe::LEN,
            "探测整包 = 24 B 帧头 + 12 B 载荷"
        );

        let reply_wire = state
            .on_probe(&decode(&probe_wire), 9_000_111)
            .expect("响应方必须产出应答");

        assert_eq!(
            reply_wire.len(),
            DATAGRAM_HEADER_LEN + ClockReply::LEN,
            "应答整包 = 24 B 帧头 + 28 B 载荷"
        );
        let datagram = decode(&reply_wire);
        assert_eq!(datagram.header.ptype, Ptype::ClockReply);
        assert_eq!(datagram.payload.len(), 28, "CLOCK_REPLY 载荷必须恰好 28 B");

        let reply = ClockReply::from_datagram(&datagram).expect("解应答载荷");
        assert_eq!(reply.probe_seq, 4242, "probe_seq 必须原样回填");
        assert_eq!(reply.t1, 1_234_567, "t1 必须原样回填（发起方据此配对）");
        assert_eq!(reply.t2, 9_000_111, "t2 = 收到探测的本机时刻");
        assert_eq!(reply.t3, reply.t2, "§6：t3 = t2，不引入额外处理延迟");

        let stats = state.stats();
        assert_eq!(stats.received, 1);
        assert_eq!(stats.sent, 0, "响应方没有发出探测");
    }

    #[test]
    fn telemetry_clock_stays_zero_until_eight_samples_then_follows_the_estimate() {
        let mut state = ClockProbeState::new();
        let mut telemetry = TelemetryAggregator::new(1, codec());
        // 哨兵：`rtt_us` 的唯一来源是 QUIC 平滑 RTT，时钟路径**不得**改写它（task-9 裁决）。
        telemetry.set_rtt_us(9_999);

        for index in 0..7u64 {
            let sample = round_trip(
                &mut state,
                &mut telemetry,
                1_000_000 + index * 100_000,
                2_000,
                OFFSET_US,
            );
            assert_eq!(
                sample.offset_us, OFFSET_US,
                "单次样本的偏移必须精确复原真值"
            );
            assert!(
                state.estimate().is_none(),
                "{} 个样本时不得返回退化估计",
                index + 1
            );

            let stats = telemetry.snapshot();
            assert_eq!(
                stats.clock_offset_us, 0,
                "样本 < 8 时遥测必须保持 0，不得假装收敛"
            );
            assert_eq!(stats.drift_ppm, 0);
            assert_eq!(
                stats.rtt_us, 9_999,
                "时钟路径不得改写 rtt_us（未收敛时也一样）"
            );
        }

        round_trip(
            &mut state,
            &mut telemetry,
            1_000_000 + 7 * 100_000,
            2_000,
            OFFSET_US,
        );

        let estimate = state.estimate().expect("第 8 个样本起必须给出估计");
        assert_eq!(estimate.samples, 8);
        assert_eq!(estimate.offset_us, OFFSET_US);

        let stats = telemetry.snapshot();
        assert_eq!(stats.clock_offset_us, i32::try_from(OFFSET_US).unwrap());
        assert_eq!(stats.drift_ppm, 0, "无漂移样本的回归斜率应为 0");
        assert_eq!(
            stats.rtt_us, 9_999,
            "收敛后 rtt_us 仍是 QUIC 平滑 RTT：§6 的代表 RTT 走 clock_estimate() 出口"
        );
        // 反过来确认时钟侧的 RTT 环**不受** rtt_us 影响：它由样本自己喂，与遥测那个字段无关。
        let probe_stats = state.stats();
        assert_eq!(probe_stats.rtt_samples, 8);
        assert_eq!(
            probe_stats.rtt_p50_us,
            Some(2_000),
            "clock_probe_stats() 的 RTT 环独立于 StreamStats.rtt_us"
        );
    }

    #[test]
    fn rtt_percentiles_are_none_until_eight_samples() {
        let mut state = ClockProbeState::new();
        let mut telemetry = TelemetryAggregator::new(1, codec());

        for index in 0..7u64 {
            round_trip(
                &mut state,
                &mut telemetry,
                1_000_000 + index * 100_000,
                2_000,
                OFFSET_US,
            );
            let stats = state.stats();
            assert_eq!(stats.rtt_samples, u32::try_from(index + 1).unwrap());
            assert_eq!(
                stats.rtt_p50_us, None,
                "样本 < 8 时不得给分位数（不是 Some(0)）"
            );
            assert_eq!(stats.rtt_p95_us, None);
        }

        round_trip(&mut state, &mut telemetry, 1_800_000, 2_000, OFFSET_US);
        let stats = state.stats();
        assert_eq!(stats.rtt_samples, 8);
        assert_eq!(stats.rtt_p50_us, Some(2_000));
        assert_eq!(stats.rtt_p95_us, Some(2_000));
    }

    #[test]
    fn rtt_ring_keeps_only_the_last_two_hundred_samples() {
        let mut state = ClockProbeState::new();
        let mut telemetry = TelemetryAggregator::new(1, codec());

        // 先喂 100 个 1 ms，再喂 200 个 9 ms：环里只剩后 200 个（全是 9 ms）。
        for index in 0..100u64 {
            round_trip(
                &mut state,
                &mut telemetry,
                1_000_000 + index * 1_000,
                1_000,
                OFFSET_US,
            );
        }
        for index in 0..200u64 {
            round_trip(
                &mut state,
                &mut telemetry,
                2_000_000 + index * 1_000,
                9_000,
                OFFSET_US,
            );
        }
        let stats = state.stats();
        assert_eq!(
            stats.rtt_samples,
            u32::try_from(RTT_WINDOW).unwrap(),
            "环必须封顶 200"
        );
        assert_eq!(stats.rtt_p50_us, Some(9_000), "旧的 1 ms 样本必须已被覆盖");
        assert_eq!(stats.replies, 300);

        // 再喂 30 个 41 ms：P95 必须跟着尾部走（170 个 9 ms + 30 个 41 ms）。
        for index in 0..30u64 {
            round_trip(
                &mut state,
                &mut telemetry,
                3_000_000 + index * 1_000,
                41_000,
                OFFSET_US,
            );
        }
        let stats = state.stats();
        assert_eq!(stats.rtt_samples, u32::try_from(RTT_WINDOW).unwrap());
        assert_eq!(stats.rtt_p50_us, Some(9_000));
        assert_eq!(stats.rtt_p95_us, Some(41_000), "尾部尖刺必须反映在 P95 上");
    }

    #[test]
    fn an_anomalous_sample_counts_as_a_reply_but_never_enters_the_rtt_ring() {
        let mut state = ClockProbeState::new();
        let probe_wire = state.build_probe(5_000_000).expect("组探测包");
        let probe = ClockProbe::from_datagram(&decode(&probe_wire)).expect("解探测包");

        // 对端把 t2 = t3 回填成 t1，本机却在 t1 之前"收到"（t4 < t1）→ RTT 为负。
        let reply_wire = wire_reply(probe.probe_seq, probe.t1, probe.t1, probe.t1);
        let sample = state
            .on_reply(&decode(&reply_wire), 4_000_000)
            .expect("包本身合法")
            .expect("序号与 t1 都对，必须配对成功");

        assert!(sample.anomalous, "负 RTT 必须判为异常样本");
        assert_eq!(state.stats().replies, 1, "配对成功仍计 replies");
        assert_eq!(state.stats().rtt_samples, 0, "异常样本不得进 RTT 环");
        assert_eq!(state.stats().rtt_p50_us, None);
        assert_eq!(state.estimator.samples(), 0, "异常样本同样不入估计窗口");
    }

    #[test]
    fn reply_without_matching_probe_is_unmatched_and_never_enters_the_window() {
        let mut state = ClockProbeState::new();

        // 一个探测都没发过就收到应答：probe_seq 无从配对。
        let wire = wire_reply(7, 100, 200, 200);
        assert!(
            state
                .on_reply(&decode(&wire), 300)
                .expect("包本身合法")
                .is_none()
        );

        let stats = state.stats();
        assert_eq!(stats.unmatched, 1);
        assert_eq!(stats.replies, 0);
        assert_eq!(stats.sent, 0);
        assert_eq!(stats.rtt_samples, 0);
        assert_eq!(state.estimator.samples(), 0, "配不上的应答不得入窗");
    }

    #[test]
    fn echoed_t1_that_differs_from_ours_is_unmatched() {
        let mut state = ClockProbeState::new();
        let _ = state.build_probe(1_000).expect("组探测包");

        // 序号对得上（0），但回填的 t1 不是我们发出的 1000。
        let wire = wire_reply(0, 4_242, 5_000, 5_000);
        assert!(
            state
                .on_reply(&decode(&wire), 6_000)
                .expect("包本身合法")
                .is_none()
        );

        assert_eq!(state.stats().unmatched, 1);
        assert_eq!(state.stats().replies, 0);
        assert_eq!(state.estimator.samples(), 0);
    }

    #[test]
    fn a_replayed_reply_pairs_only_once() {
        let mut state = ClockProbeState::new();
        let mut telemetry = TelemetryAggregator::new(1, codec());
        let probe_wire = state.build_probe(1_000_000).expect("组探测包");
        let probe = ClockProbe::from_datagram(&decode(&probe_wire)).expect("解探测包");
        let reply_wire = wire_reply(probe.probe_seq, probe.t1, 1_000_500, 1_000_500);
        let t4 = 1_001_000;

        assert!(
            state
                .on_reply(&decode(&reply_wire), t4)
                .expect("包本身合法")
                .is_some()
        );
        // 同一份应答重放：必须被挡住，否则坏对端能靠重放把窗口塞满同一时刻的样本。
        assert!(
            state
                .on_reply(&decode(&reply_wire), t4)
                .expect("包本身合法")
                .is_none()
        );

        assert_eq!(state.stats().replies, 1);
        assert_eq!(state.stats().unmatched, 1);
        assert_eq!(state.estimator.samples(), 1);
        publish_clock(&mut telemetry, state.estimate());
        assert_eq!(telemetry.snapshot().clock_offset_us, 0, "1 个样本仍未收敛");
    }

    #[test]
    fn probe_rhythm_is_100ms_for_the_first_50_then_one_second() {
        let mut state = ClockProbeState::new();

        for index in 0..u64::from(FAST_PROBES) {
            assert_eq!(
                state.probe_interval(),
                Duration::from_millis(FAST_INTERVAL_MS),
                "第 {} 次探测仍属首连快速阶段",
                index + 1
            );
            let _ = state.build_probe(index).expect("组探测包");
        }

        assert_eq!(
            state.probe_interval(),
            Duration::from_millis(STEADY_INTERVAL_MS),
            "满 50 次后进入 1 Hz 稳态"
        );
        assert_eq!(state.stats().sent, u64::from(FAST_PROBES));

        let _ = state.build_probe(999_999).expect("组探测包");
        assert_eq!(
            state.probe_interval(),
            Duration::from_millis(STEADY_INTERVAL_MS),
            "进入稳态后不再回到快速阶段"
        );
    }

    #[test]
    fn outstanding_table_is_bounded_and_evicts_the_oldest() {
        let mut state = ClockProbeState::new();
        for index in 0..(MAX_OUTSTANDING as u64 + 8) {
            let _ = state.build_probe(index).expect("组探测包");
        }
        assert_eq!(state.outstanding.len(), MAX_OUTSTANDING, "未决表必须封顶");
        assert_eq!(state.order.len(), MAX_OUTSTANDING);

        // 最旧的一条（seq 0）已被淘汰 → 它的应答只能算「配不上」。
        let wire = wire_reply(0, 0, 1, 1);
        assert!(
            state
                .on_reply(&decode(&wire), 2)
                .expect("包本身合法")
                .is_none()
        );
        assert_eq!(state.stats().unmatched, 1);
        assert_eq!(state.stats().replies, 0);
    }

    #[test]
    fn a_datagram_whose_ptype_is_wrong_is_rejected_without_touching_counters() {
        let mut state = ClockProbeState::new();

        // 把探测包喂给「处理应答」的入口：ptype 不符 → Err，且不污染计数。
        let probe_wire = wire_probe(0, 1);
        assert!(state.on_reply(&decode(&probe_wire), 2).is_err());
        // 反过来同理。
        let reply_wire = wire_reply(0, 1, 2, 2);
        assert!(state.on_probe(&decode(&reply_wire), 3).is_err());

        assert_eq!(state.stats(), ClockProbeStats::default());
        assert_eq!(state.estimator.samples(), 0);
    }

    #[test]
    fn extreme_timestamps_do_not_panic() {
        let mut state = ClockProbeState::new();

        // 没发过探测 + 全 i64 极值：只能得到 unmatched，不得 panic。
        let wire = wire_reply(u32::MAX, i64::MAX, i64::MAX, i64::MAX);
        assert!(
            state
                .on_reply(&decode(&wire), u64::MAX)
                .expect("包本身合法")
                .is_none()
        );
        assert_eq!(state.stats().unmatched, 1);

        // 响应方方向：负时间戳也必须只是一次合法编码的应答，不得 panic。
        let reply = state
            .on_probe(&decode(&wire_probe(u32::MAX, i64::MIN)), u64::MAX)
            .expect("响应方不得 panic");
        let decoded = ClockReply::from_datagram(&decode(&reply)).expect("解应答载荷");
        assert_eq!(decoded.t1, i64::MIN, "负 t1 原样回填");
        assert_eq!(decoded.t2, decoded.t3);
    }

    #[test]
    fn zero_timestamps_are_still_a_sample_not_a_dead_clock() {
        // t1 = 0（本机刚启动那一瞬间）也会走完整条路径：0 是合法时间戳，不是「没测」。
        let mut state = ClockProbeState::new();
        let mut telemetry = TelemetryAggregator::new(1, codec());
        let sample = round_trip(&mut state, &mut telemetry, 0, 1_000, 0);
        assert_eq!(sample.t1, 0);
        assert_eq!(sample.offset_us, 0);
        assert_eq!(state.stats().replies, 1);
        assert_eq!(state.stats().rtt_samples, 1);
    }
}
