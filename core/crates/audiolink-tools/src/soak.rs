//! soak-runner 的判定与报告逻辑。
//!
//! 「8 h 无故障」这句验收话必须能被机器判定：本模块把 1 Hz 的遥测采样变成
//! **异常快照**（每次越界都留现场）与**汇总报告**（JSON，可作 CI 产物）。
//!
//! 判据口径是「回环稳态不该出现的东西」：欠载、PCM 掩盖、丢包、迟到丢弃、NACK 重传
//! 在一条 127.0.0.1 的链路上都应当是 0 —— 任何非零值都是缺陷信号，不是噪声。
//!
//! # 码率判据：期望值从哪来
//!
//! 接收侧遥测的 `bitrate_bps` 记的是**收到的字节**，而引擎对每个音频帧都主帧 + 冗余帧各发一次
//! （`docs/03-protocol.md` §8.1），所以它天然是发送侧编码器目标码率的 [`REDUNDANT_COPIES`] 倍。
//! 再把期望值钉死成一个数字就是刻舟求剑：干净链路上 §8 的自适应会把目标从 160 kbps 一路推到上限
//! 320 kbps，接收侧于是从 320 kbps 长到 640 kbps —— 拿 320000 当期望值，第 82 轮的 900 s 实测
//! 直接判出 878 条 `bitrate_out_of_range`（其余七类全 0）。
//!
//! 期望值因此有两条来路，语义各自分明：
//!
//! - **跟随**（[`SoakMonitor::follow_encoder_target`]，runner 的默认）：期望值 = 发送侧**当前**目标
//!   码率 × 冗余份数，起点由本次真实使用的 `CodecConfig::bitrate_bps` 推出。它判的是
//!   「收到的码率有没有跟上发送侧实际想发的量」——断流、只收到一份副本、码率塌到下限不恢复，仍是异常。
//! - **钉死**（`--expected-bps N`）：判据钉在给定数字上，用于「故意把目标写错」这类判定链路自检。
//!
//! 关掉码率判据只有两条路：`expected_bitrate_bps == 0`，或 `bitrate_tolerance_pct_x100 == 0`
//! （弱网档，见 [`SoakThresholds::weak_network`]）。两者都在构造时点定，跑动中不会被悄悄打开。
//!
//! 本模块**不碰套接字也不睡觉**：只吃采样、吐判定，因此可以完整单测（编排在 `bin/soak_runner.rs`）。

use std::fmt::Write as _;

use audiolink_types::StreamStats;
use serde_json::{Value, json};

/// 会话处于 streaming 时使用的状态名（与 `SessionState::name()` 一致）。
pub const STREAMING: &str = "streaming";

/// 粗采样粒度（秒）：报告里每分钟留一条，8 h 约 480 条 —— 既看得到趋势，又不至于把报告撑爆。
pub const COARSE_BUCKET_SECS: u64 = 60;

/// 冗余双发的份数：引擎对每个音频帧**主帧 + 冗余帧**各发一次（`docs/03-protocol.md` §8.1）。
///
/// 接收侧遥测的 `bitrate_bps` 按**收到的字节**记账，因此它是发送侧编码器目标码率的两倍。
/// 这不是可以随便挑的口径，而是链路的既有事实：期望值不算上它，就必然差整整一倍
/// （第 82 轮实测：目标 320 kbps、接收侧 640.8 kbps）。
pub const REDUNDANT_COPIES: u32 = 2;

/// 由**发送侧编码器的目标码率**推出接收侧的期望码率（= 目标 × [`REDUNDANT_COPIES`]）。
///
/// 入参应当取自本次运行**真实使用**的配置（`EngineConfig::codec.bitrate_bps`），而不是抄一个常量 ——
/// 默认期望值因此跟着实际配置走，改配置不用改判据。
/// 非正数返回 0，与「不判码率」同义（见 [`SoakMonitor::new`]）。
pub fn expected_bitrate_bps_from_codec(codec_bitrate_bps: i32) -> u32 {
    u32::try_from(codec_bitrate_bps)
        .unwrap_or(0)
        .saturating_mul(REDUNDANT_COPIES)
}

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
    /// **每类**异常留存上限（超出只计数，不再留现场 —— 8 h 跑不该产出无限大的报告）。
    ///
    /// 按类设限而不是对总量设限，是 8 h 弱网实测逼出来的：那份报告里单类
    /// （`bitrate_out_of_range`）就刷了 7055 条，总量配额被它独占，结果留存的 200 条现场
    /// 全落在 t=2692..8293 s，**后面 5.7 h 一条证据都不剩**。分类配额保证每一类都留得下现场，
    /// 长跑之后仍能回答「哪一类、什么时候、还在不在发生」。
    pub max_violations_per_kind: usize,
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
            max_violations_per_kind: 25,
        }
    }
}

impl SoakThresholds {
    /// 弱网档（`--tolerant`）：只钉「会话不断 + 掩盖比例 ≤ 1%」。
    ///
    /// 这份清单是 8 h / 5 Mbps / 2% 丢包 / 15 ms 抖动 的实测钉出来的：
    /// 那一次跑出 7055 条异常，**全部**是 `bitrate_out_of_range` —— 瞬时码率在自适应与重传下
    /// 本就随弱网摆动，把硬阈值留在弱网档里只会制造假警报，与欠载 / 迟到 / NACK / 瞬时丢包同理。
    /// 码率判据用 `bitrate_tolerance_pct_x100 = 0`（= 不判）关掉，而不是塞一个巨大的数字进去：
    /// 语义要能一眼读懂，且与字段文档一致。
    pub fn weak_network(planned_secs: u64, frame_ms: u32) -> Self {
        let total_frames = planned_secs.saturating_mul(1_000) / u64::from(frame_ms.max(1));
        Self {
            max_plc: u32::try_from(total_frames / 100).unwrap_or(u32::MAX),
            max_underruns: u32::MAX,
            max_late_drops: u32::MAX,
            max_nack: u32::MAX,
            max_loss_pct_x100: u16::MAX,
            bitrate_tolerance_pct_x100: 0,
            ..Self::default()
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

/// 异常种类个数（每一类独立计数、独立配额，见 `SoakMonitor`）。
pub const VIOLATION_KINDS: usize = 8;

impl ViolationKind {
    /// 数组下标（每类计数用）。
    pub const fn index(self) -> usize {
        match self {
            Self::NotStreaming => 0,
            Self::Underrun => 1,
            Self::PlayoutConcealment => 2,
            Self::PacketLoss => 3,
            Self::LateDrop => 4,
            Self::NackRetransmit => 5,
            Self::BitrateOutOfRange => 6,
            Self::Stalled => 7,
        }
    }

    /// 全部种类（报告按这个稳定顺序输出）。
    pub const ALL: [Self; VIOLATION_KINDS] = [
        Self::NotStreaming,
        Self::Underrun,
        Self::PlayoutConcealment,
        Self::PacketLoss,
        Self::LateDrop,
        Self::NackRetransmit,
        Self::BitrateOutOfRange,
        Self::Stalled,
    ];

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
    /// 留存下来的异常条数（每类各自配额；总数 = 留存 + 未留存）。
    pub violations: usize,
    /// 按种类计数 —— **完整事实**，不受快照配额影响。
    pub kind_counts: [u64; VIOLATION_KINDS],
    /// 因为**该类配额已满**而未留存的异常条数。
    pub dropped_violations: u64,
    /// 最后一次异常的时刻与种类（配额挡掉快照也不影响它）。
    pub last_violation: Option<(u64, &'static str)>,
    /// 最后一次采样的遥测。
    pub final_stats: Option<StreamStats>,
    /// 判定。
    pub verdict: &'static str,
}

/// 期望码率的**来路**（写进报告：这次到底拿什么在判）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitrateExpectationSource {
    /// 不判码率（`--expected-bps 0`）。
    Off,
    /// 命令行钉死的固定值（`--expected-bps N`）：判据不跟随自适应。
    FixedCli,
    /// 跟随发送侧编码器的**实际**目标码率（runner 的默认）：
    /// 起点由本次真实使用的 `CodecConfig::bitrate_bps` 推出，之后随 `CODEC_ADAPTED` 事件走。
    FollowCodecTarget,
}

impl BitrateExpectationSource {
    /// 稳定名字（报告字段用）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::FixedCli => "fixed-cli",
            Self::FollowCodecTarget => "follow-encoder-target",
        }
    }
}

/// 报告元信息（由调用方填：本次跑的参数）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoakMeta {
    /// 计划时长（秒）。
    pub planned_seconds: u64,
    /// 帧长（ms）。
    pub frame_ms: u32,
    /// **起跑时**的期望码率（bps）；0 = 不判码率。
    ///
    /// 跟随来路（[`BitrateExpectationSource::FollowCodecTarget`]）时它只是起点：
    /// 跑动中的当前值在 [`SoakMonitor::expected_bitrate_bps`]，报告里另有
    /// `summary.expected_bitrate_bps_final`。
    pub expected_bitrate_bps: u32,
    /// 期望码率的来路（决定它会不会跟着发送侧自适应目标走）。
    pub expected_bitrate_source: BitrateExpectationSource,
    /// 开始时刻（Unix 秒）。
    pub started_at_unix: u64,
}
/// 采样监视器。
#[derive(Debug)]
pub struct SoakMonitor {
    thresholds: SoakThresholds,
    /// **当前**期望码率（判据用的就是它）：可以固定，也可以跟着发送侧自适应走。
    expected_bitrate_bps: u32,
    /// **起跑时**的期望码率：跟随的起点，也是「判 / 不判」的开关。
    initial_expected_bitrate_bps: u32,
    /// 期望值被跟随改写了几次（0 = 这次跑没跟随过，判据一直在拿同一个数判）。
    expected_bitrate_updates: u64,
    previous: Option<StreamStats>,
    previous_state: Option<&'static str>,
    stall_run: u32,
    samples: u64,
    violations: Vec<Violation>,
    kind_counts: [u64; VIOLATION_KINDS],
    dropped_violations: u64,
    last_violation: Option<(u64, &'static str)>,
    coarse: Vec<CoarseSample>,
}

impl SoakMonitor {
    /// 新建监视器；`expected_bitrate_bps = 0` 表示不判码率（构造时点定，跑动中不会被打开）。
    ///
    /// 期望值不一定来自命令行：runner 默认把它设成「本次**实际** codec 配置 × 冗余份数」，
    /// 再用 [`Self::follow_encoder_target`] 跟着自适应走 —— 见模块文档「码率判据：期望值从哪来」。
    pub fn new(thresholds: SoakThresholds, expected_bitrate_bps: u32) -> Self {
        Self {
            thresholds,
            expected_bitrate_bps,
            initial_expected_bitrate_bps: expected_bitrate_bps,
            expected_bitrate_updates: 0,
            previous: None,
            previous_state: None,
            stall_run: 0,
            samples: 0,
            violations: Vec::new(),
            kind_counts: [0; VIOLATION_KINDS],
            dropped_violations: 0,
            last_violation: None,
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
                self.thresholds.max_underruns,
                previous.underruns,
                stats.underruns,
                sample.at_secs,
                stats,
            );
            self.check_delta(
                ViolationKind::PlayoutConcealment,
                "PCM 掩盖帧",
                self.thresholds.max_plc,
                previous.plc_count,
                stats.plc_count,
                sample.at_secs,
                stats,
            );
            self.check_delta(
                ViolationKind::LateDrop,
                "迟到丢弃",
                self.thresholds.max_late_drops,
                previous.late_drops,
                stats.late_drops,
                sample.at_secs,
                stats,
            );
            self.check_delta(
                ViolationKind::NackRetransmit,
                "NACK 重传请求",
                self.thresholds.max_nack,
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

        // `bitrate_tolerance_pct_x100 == 0` 就是「不判码率」（弱网档用）。
        // 这条必须显式写出来：下面的 `deviation > tolerance` 在 tolerance 为 0 时
        // 会退化成「任何偏离都算异常」，把「不判」悄悄变成「全判」。
        //
        // 期望值本身可以是固定的（`--expected-bps N`），也可以跟随发送侧的目标码率
        // （`follow_encoder_target`）——但「判 / 不判」只由上面两个条件决定，跟随不会把它打开。
        if self.expected_bitrate_bps > 0
            && stats.bitrate_bps > 0
            && self.thresholds.bitrate_tolerance_pct_x100 > 0
        {
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

    /// **当前**期望码率（bps）；0 = 不判码率。
    pub const fn expected_bitrate_bps(&self) -> u32 {
        self.expected_bitrate_bps
    }

    /// 起跑时的期望码率（bps）。
    pub const fn initial_expected_bitrate_bps(&self) -> u32 {
        self.initial_expected_bitrate_bps
    }

    /// 期望值被跟随改写了几次。
    pub const fn expected_bitrate_updates(&self) -> u64 {
        self.expected_bitrate_updates
    }

    /// 码率判据是否在生效：「期望值 > 0」且「容忍度 > 0」，两个开关都开着才算。
    pub const fn bitrate_judged(&self) -> bool {
        self.expected_bitrate_bps > 0 && self.thresholds.bitrate_tolerance_pct_x100 > 0
    }

    /// 期望码率跟着发送侧编码器的**实际**目标码率走。
    ///
    /// 自适应每升 / 降一级（`EngineEvent::CodecAdapted`）就调用一次：
    /// 期望值 = 新目标 × 冗余份数。返回是否真的改动了期望值（收到同值不算一次跟随）。
    ///
    /// **不会把判据从「不判」打开**：起跑时的期望值为 0（`--expected-bps 0`）时一律不跟随 ——
    /// 判据的开关只在 [`Self::new`] 时由调用方点定，跑动中不偷偷变。
    pub fn follow_encoder_target(&mut self, encoder_target_bps: i32) -> bool {
        if self.initial_expected_bitrate_bps == 0 {
            return false;
        }
        let next = expected_bitrate_bps_from_codec(encoder_target_bps);
        if next == 0 || next == self.expected_bitrate_bps {
            return false;
        }
        self.expected_bitrate_bps = next;
        self.expected_bitrate_updates = self.expected_bitrate_updates.saturating_add(1);
        true
    }

    /// 一行说清码率判据现在是什么状态（摘要用）。
    pub fn bitrate_expectation_text(&self) -> String {
        if !self.bitrate_judged() {
            return if self.expected_bitrate_bps == 0 {
                "关闭（期望值 0 = 不判码率）".to_owned()
            } else {
                "关闭（容忍度 0 = 不判码率，弱网档）".to_owned()
            };
        }
        let tolerance = f64::from(self.thresholds.bitrate_tolerance_pct_x100) / 100.0;
        if self.expected_bitrate_updates == 0 {
            format!(
                "期望 {} bps（固定），容忍 ±{tolerance:.2}%",
                self.expected_bitrate_bps
            )
        } else {
            format!(
                "期望 {} bps（起跑 {} bps，跟随编码器目标 {} 次），容忍 ±{tolerance:.2}%",
                self.expected_bitrate_bps,
                self.initial_expected_bitrate_bps,
                self.expected_bitrate_updates
            )
        }
    }

    /// 汇总。
    pub fn summary(&self) -> SoakSummary {
        SoakSummary {
            samples: self.samples,
            violations: self.violations.len(),
            kind_counts: self.kind_counts,
            dropped_violations: self.dropped_violations,
            last_violation: self.last_violation,
            final_stats: self.previous,
            // 判定看**按类计数**而不是留存条数：配额为 0 时现场一条不留，
            // 但异常确实发生过，判定不能因此变绿。
            verdict: if self.kind_counts.iter().all(|count| *count == 0) {
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

        // 按类计数与「最后一次」进报告，是真实 8 h 跑换来的教训：
        // 快照会被配额截断，于是「哪一类刷了多少条、最后一次发生在什么时候」
        // 成了判断事件**是否仍在发生**的唯一线索（旧报告只有「留存的 200 条」，
        // 看不出后面 5.7 h 到底还有没有异常）。
        let mut by_kind = serde_json::Map::new();
        for kind in ViolationKind::ALL {
            by_kind.insert(
                kind.name().to_owned(),
                Value::from(summary.kind_counts[kind.index()]),
            );
        }
        let total_violations: u64 = summary.kind_counts.iter().sum();

        let report = json!({
            "tool": "soak-runner",
            "started_at_unix": meta.started_at_unix,
            "planned_seconds": meta.planned_seconds,
            "frame_ms": meta.frame_ms,
            "expected_bitrate_bps": meta.expected_bitrate_bps,
            "expected_bitrate_source": meta.expected_bitrate_source.name(),
            "summary": {
                "samples": summary.samples,
                "violations": summary.violations,
                "violations_total": total_violations,
                "dropped_violations": summary.dropped_violations,
                "violations_by_kind": Value::Object(by_kind),
                "last_violation_at_secs": summary.last_violation.map(|(at_secs, _)| at_secs),
                "last_violation_kind": summary.last_violation.map(|(_, kind)| kind),
                "violation_limit_per_kind": self.thresholds.max_violations_per_kind,
                "verdict": summary.verdict,
                "bitrate_judged": self.bitrate_judged(),
                "expected_bitrate_bps_final": self.expected_bitrate_bps,
                "expected_bitrate_updates": self.expected_bitrate_updates,
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
        let _ = writeln!(
            out,
            "码率判据：{}（来路 {}）",
            self.bitrate_expectation_text(),
            meta.expected_bitrate_source.name()
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
        let total: u64 = summary.kind_counts.iter().sum();
        let _ = writeln!(
            out,
            "异常：共 {total} 条（留存 {} 条，未留存 {} 条）→ 判定 {}",
            summary.violations, summary.dropped_violations, summary.verdict
        );
        let by_kind = ViolationKind::ALL
            .iter()
            .filter(|kind| summary.kind_counts[kind.index()] > 0)
            .map(|kind| format!("{}×{}", kind.name(), summary.kind_counts[kind.index()]))
            .collect::<Vec<_>>()
            .join(", ");
        if !by_kind.is_empty() {
            let _ = writeln!(out, "按类：{by_kind}");
        }
        if let Some((at_secs, kind)) = summary.last_violation {
            let _ = writeln!(out, "最后一次异常：t={at_secs}s [{kind}]");
        }
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

    /// 计数器增量的判定。
    ///
    /// `threshold` 是**累计值**的允许上限：只有超过它之后的增量才算异常。
    /// 默认全 0（回环稳态：任何非零增量都是缺陷信号）；弱网档把它放开，
    /// 于是「给定条件造成的欠载」不再被当成故障 —— 否则弱网长跑永远是红的。
    ///
    /// 七个参数不分装结构体：这里就是「一个计数器 + 它的名字 + 它的上限 + 前后值 + 时刻 + 快照」，
    /// 拆成结构体只会让四处调用各自多写一遍字段名。
    #[allow(clippy::too_many_arguments)]
    fn check_delta(
        &mut self,
        kind: ViolationKind,
        label: &str,
        threshold: u32,
        before: u32,
        after: u32,
        at_secs: u64,
        stats: StreamStats,
    ) {
        if after <= threshold {
            return;
        }
        let baseline = before.max(threshold);
        if after > baseline {
            self.push_violation(
                kind,
                at_secs,
                format!("{label} +{}（累计 {after}）", after - baseline),
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
        // 先记账，再决定要不要留现场：快照可能被配额挡掉，
        // 但「这一类总共几条、最后一次在何时」必须是完整事实。
        let slot = kind.index();
        self.kind_counts[slot] = self.kind_counts[slot].saturating_add(1);
        self.last_violation = Some((at_secs, kind.name()));

        let kept_of_kind = self
            .violations
            .iter()
            .filter(|kept| kept.kind == kind)
            .count();
        if kept_of_kind >= self.thresholds.max_violations_per_kind {
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
            expected_bitrate_source: BitrateExpectationSource::FixedCli,
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
            max_violations_per_kind: 2,
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

    #[test]
    fn raised_thresholds_suppress_only_what_they_allow() {
        let thresholds = SoakThresholds {
            max_underruns: 10,
            max_late_drops: u32::MAX,
            ..SoakThresholds::default()
        };
        let mut monitor = SoakMonitor::new(thresholds, 160_000);
        monitor.observe(sample(0, stats(160_000)));

        let mut within = stats(160_000);
        within.underruns = 5;
        within.late_drops = 7;
        monitor.observe(sample(1, within));
        assert!(monitor.violations().is_empty(), "阈值内的增量不该记异常");

        let mut over = stats(160_000);
        over.underruns = 12;
        over.late_drops = 7;
        monitor.observe(sample(2, over));
        assert_eq!(monitor.violations().len(), 1, "只有超过上限的欠载算异常");
        assert_eq!(monitor.violations()[0].detail, "欠载 +2（累计 12）");
    }

    #[test]
    fn bitrate_tolerance_zero_means_dont_judge() {
        // 字段文档写着「0 = 不判」，而判定式 `deviation > tolerance` 在 tolerance 为 0 时
        // 会退化成「任何偏离都算异常」—— 这条测试钉死语义：容忍度 0 就是不判码率。
        let thresholds = SoakThresholds {
            bitrate_tolerance_pct_x100: 0,
            ..SoakThresholds::default()
        };
        let mut monitor = SoakMonitor::new(thresholds, 320_000);
        monitor.observe(sample(0, stats(1)));
        monitor.observe(sample(1, stats(4_000_000)));
        assert!(
            monitor.violations().is_empty(),
            "容忍度 0 = 不判码率，不该留下快照：{:?}",
            monitor.violations()
        );
        assert_eq!(monitor.summary().verdict, "ok");
    }

    #[test]
    fn weak_network_profile_pins_only_session_and_concealment() {
        let thresholds = SoakThresholds::weak_network(28_800, 20);
        assert_eq!(
            thresholds.max_plc,
            28_800 * 1_000 / 20 / 100,
            "掩盖上限 = 总帧数的 1%"
        );
        assert_eq!(thresholds.max_underruns, u32::MAX);
        assert_eq!(thresholds.max_late_drops, u32::MAX);
        assert_eq!(thresholds.max_nack, u32::MAX);
        assert_eq!(thresholds.max_loss_pct_x100, u16::MAX);
        assert_eq!(thresholds.bitrate_tolerance_pct_x100, 0, "弱网档不判码率");

        let mut monitor = SoakMonitor::new(thresholds, 320_000);
        // 码率跑偏 + 欠载 + 迟到 + NACK + 瞬时丢包：弱网档一律不判。
        let mut wild = stats(4_000_000);
        wild.underruns = 9_999;
        wild.late_drops = 9_999;
        wild.nack_count = 9_999;
        wild.loss_pct_x100 = 400;
        monitor.observe(sample(0, wild));
        monitor.observe(sample(1, wild));
        assert!(
            monitor.violations().is_empty(),
            "弱网档只钉会话与掩盖：{:?}",
            monitor.violations()
        );

        // 掩盖越线才判。
        let mut concealed = stats(320_000);
        concealed.plc_count = thresholds.max_plc + 1;
        monitor.observe(sample(2, concealed));
        assert!(
            monitor
                .violations()
                .iter()
                .any(|violation| violation.kind == ViolationKind::PlayoutConcealment),
            "掩盖比例超过 1% 必须判"
        );

        // 会话离开 streaming 仍然判 —— 弱网档也钉这一条。
        let mut down = sample(3, stats(320_000));
        down.state = "reconnecting";
        monitor.observe(down);
        assert!(
            monitor
                .violations()
                .iter()
                .any(|violation| violation.kind == ViolationKind::NotStreaming),
            "会话断了必须判"
        );
    }

    #[test]
    fn violation_quota_is_per_kind_so_every_kind_keeps_evidence() {
        // 实测教训：总量配额会被刷得最凶的那一类独占，其它种类一条现场都留不下。
        let thresholds = SoakThresholds {
            max_violations_per_kind: 2,
            ..SoakThresholds::default()
        };
        let mut monitor = SoakMonitor::new(thresholds, 160_000);
        monitor.observe(sample(0, stats(160_000)));
        for secs in 1..=5u64 {
            let mut bumped = stats(160_000);
            bumped.underruns = secs as u32;
            monitor.observe(sample(secs, bumped));
        }
        let kept_underruns = monitor
            .violations()
            .iter()
            .filter(|violation| violation.kind == ViolationKind::Underrun)
            .count();
        assert_eq!(kept_underruns, 2, "欠载刷满自己的配额就停手");
        assert_eq!(
            monitor.summary().kind_counts[ViolationKind::Underrun.index()],
            5,
            "计数是完整事实，不受配额影响"
        );

        let mut concealed = stats(160_000);
        concealed.plc_count = 3;
        monitor.observe(sample(6, concealed));
        assert!(
            monitor
                .violations()
                .iter()
                .any(|violation| violation.kind == ViolationKind::PlayoutConcealment),
            "另一类不该被欠载挤掉现场"
        );
    }

    #[test]
    fn report_records_kind_counts_and_last_violation() {
        let thresholds = SoakThresholds {
            max_violations_per_kind: 1,
            ..SoakThresholds::default()
        };
        let mut monitor = SoakMonitor::new(thresholds, 160_000);
        monitor.observe(sample(0, stats(160_000)));
        for secs in 1..=4u64 {
            let mut bumped = stats(160_000);
            bumped.underruns = secs as u32;
            monitor.observe(sample(secs, bumped));
        }
        let summary = monitor.summary();
        assert_eq!(summary.kind_counts[ViolationKind::Underrun.index()], 4);
        assert_eq!(summary.violations, 1, "每类配额 1");
        assert_eq!(summary.dropped_violations, 3);
        assert_eq!(summary.last_violation, Some((4, "underrun")));

        let parsed: Value = serde_json::from_str(&monitor.report_json(&meta())).unwrap();
        assert_eq!(parsed["summary"]["violations_total"], 4);
        assert_eq!(parsed["summary"]["violations_by_kind"]["underrun"], 4);
        assert_eq!(parsed["summary"]["violations_by_kind"]["stalled"], 0);
        assert_eq!(parsed["summary"]["last_violation_at_secs"], 4);
        assert_eq!(parsed["summary"]["last_violation_kind"], "underrun");
        assert_eq!(parsed["summary"]["violation_limit_per_kind"], 1);
        assert_eq!(parsed["summary"]["verdict"], "failed");

        let text = monitor.summary_text(&meta());
        assert!(text.contains("按类：underrun×4"), "摘要要按类点名：{text}");
        assert!(text.contains("最后一次异常：t=4s [underrun]"), "{text}");
    }

    #[test]
    fn derived_expectation_counts_the_redundant_copies() {
        // 期望值的默认来路：由**本次实际使用的** codec 配置推出，并且必须算上冗余双发 ——
        // 引擎每帧主帧 + 冗余帧各发一次，接收侧遥测按收到的字节记账，不算就是整整差一倍。
        assert_eq!(REDUNDANT_COPIES, 2);
        assert_eq!(expected_bitrate_bps_from_codec(160_000), 320_000);
        assert_eq!(expected_bitrate_bps_from_codec(0), 0);
        assert_eq!(
            expected_bitrate_bps_from_codec(-1),
            0,
            "非正数（不判码率）不该被当成有效码率"
        );
    }

    #[test]
    fn expectation_follows_the_encoder_target_and_still_has_teeth() {
        // 第 82 轮 900 s 干净回环的实测形态：自适应把编码器目标从 160 kbps 推到上限 320 kbps，
        // 接收侧于是从 320 kbps 长到 640.8 kbps。期望值跟着目标走，那 878 条越界才不该出现。
        let mut monitor = SoakMonitor::new(SoakThresholds::default(), 320_000);
        monitor.observe(sample(0, stats(320_528)));
        assert!(
            monitor.violations().is_empty(),
            "起跑点（目标 160 kbps × 2）不该越界：{:?}",
            monitor.violations()
        );

        // 自适应升一级：期望值跟着挪，接收侧也就继续落在容忍带内。
        assert!(monitor.follow_encoder_target(176_000), "目标变了就该跟随");
        assert_eq!(monitor.expected_bitrate_bps(), 352_000);
        monitor.observe(sample(1, stats(352_000)));
        assert!(monitor.violations().is_empty());

        // 推到上限：640800 bps 落在新期望值的 ±30% 内。
        assert!(monitor.follow_encoder_target(320_000));
        assert_eq!(monitor.expected_bitrate_bps(), 640_000);
        assert_eq!(monitor.expected_bitrate_updates(), 2);
        assert!(!monitor.follow_encoder_target(320_000), "同值不算一次跟随");
        for secs in 2..12 {
            monitor.observe(sample(secs, stats(640_800)));
        }
        assert!(
            monitor.violations().is_empty(),
            "跟随之后不该再有越界：{:?}",
            monitor.violations()
        );
        assert_eq!(monitor.summary().verdict, "ok");

        // 但判据仍然有牙齿：接收到的码率塌到 100 kbps（只剩一份副本 / 链路退化）必须判。
        monitor.observe(sample(12, stats(100_000)));
        assert!(
            monitor
                .violations()
                .iter()
                .any(|violation| violation.kind == ViolationKind::BitrateOutOfRange),
            "期望值跟随≠关掉判据：码率塌掉仍要判：{:?}",
            monitor.violations()
        );
    }

    #[test]
    fn following_never_switches_the_bitrate_criterion_on() {
        // `--expected-bps 0` = 不判码率。跟随不能把它偷偷打开 —— 那等于藏问题。
        let mut monitor = SoakMonitor::new(SoakThresholds::default(), 0);
        assert!(!monitor.follow_encoder_target(320_000));
        assert_eq!(monitor.expected_bitrate_bps(), 0);
        assert!(!monitor.bitrate_judged());
        monitor.observe(sample(0, stats(1)));
        monitor.observe(sample(1, stats(4_000_000)));
        assert!(monitor.violations().is_empty());
        assert_eq!(monitor.summary().verdict, "ok");
    }

    #[test]
    fn bitrate_criterion_state_is_readable_in_one_line() {
        let mut fixed = SoakMonitor::new(SoakThresholds::default(), 320_000);
        assert!(fixed.bitrate_judged());
        assert!(
            fixed
                .bitrate_expectation_text()
                .contains("期望 320000 bps（固定）"),
            "{}",
            fixed.bitrate_expectation_text()
        );

        fixed.follow_encoder_target(320_000);
        let text = fixed.bitrate_expectation_text();
        assert!(
            text.contains("期望 640000 bps") && text.contains("跟随编码器目标 1 次"),
            "{text}"
        );

        let off = SoakMonitor::new(SoakThresholds::default(), 0);
        assert!(!off.bitrate_judged());
        assert!(off.bitrate_expectation_text().contains("期望值 0"));

        let weak = SoakMonitor::new(SoakThresholds::weak_network(60, 20), 320_000);
        assert!(!weak.bitrate_judged(), "弱网档不判码率");
        assert!(weak.bitrate_expectation_text().contains("弱网档"));
    }

    #[test]
    fn report_names_where_the_expectation_came_from() {
        let mut monitor = SoakMonitor::new(
            SoakThresholds::default(),
            expected_bitrate_bps_from_codec(160_000),
        );
        assert!(monitor.follow_encoder_target(320_000));
        monitor.observe(sample(0, stats(640_800)));

        let mut meta = meta();
        meta.expected_bitrate_bps = 320_000;
        meta.expected_bitrate_source = BitrateExpectationSource::FollowCodecTarget;
        let parsed: Value = serde_json::from_str(&monitor.report_json(&meta)).unwrap();
        assert_eq!(
            parsed["expected_bitrate_bps"], 320_000,
            "顶格字段仍是起跑值（tools/soak-report.ps1 在读它）"
        );
        assert_eq!(parsed["expected_bitrate_source"], "follow-encoder-target");
        assert_eq!(parsed["summary"]["expected_bitrate_bps_final"], 640_000);
        assert_eq!(parsed["summary"]["expected_bitrate_updates"], 1);
        assert_eq!(parsed["summary"]["bitrate_judged"], true);
        assert_eq!(parsed["summary"]["verdict"], "ok");
    }
}
