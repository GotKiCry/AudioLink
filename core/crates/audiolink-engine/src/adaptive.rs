//! §8 的自适应码率规则表（发送端**单方面**调整，接收端无需重协商）。
//!
//! | 触发（`docs/03-protocol.md` §8） | 动作 | 本模块 |
//! |---|---|---|
//! | 丢包 > 1% 持续 3 s | 码率 −20%（下限 96 kbps） | ✅ 实施 |
//! | 丢包 > 5% 持续 3 s | 叠加：立体声 → 单声道 | 只记账（见下） |
//! | RTT P95 > 30 ms 或延迟预算超 100 ms | 帧长 20 ms → 10 ms | 只记账 |
//! | 连续 10 s 无丢包且延迟允许 | 码率 +10%（上限 320 kbps）逐级恢复 | ✅ 实施 |
//! | 恢复上限 | 每 5 s 最多上调一级（防震荡） | ✅ 实施 |
//!
//! **为什么这一轮只做码率这一档**：码率是 `opus-rs` 里「一改下一帧就生效」的字段；
//! 而「立体声 → 单声道」要动 `audiolink-audio::format` 的编译期常量、
//! 「20 ms → 10 ms」要两端重新对齐采集分帧与解码帧长（那是 `OPEN_STREAM` 已协商过的参数）——
//! 两者都是跨端契约变更，不该塞进这一轮。它们由 [`AdaptiveBitrate::severity`] 记账，留给专门的任务。
//!
//! 本模块是**纯状态机**：输入 1 Hz 的链路样本，输出「要不要改目标码率」；不碰编码器、不碰套接字。
//! 规则表因此可以被完整单测 —— 判定错了不可能怪环境。

use std::time::Duration;

/// 码率下限（§8：下限 96 kbps）。
pub const MIN_BITRATE_BPS: i32 = 96_000;

/// 码率上限（§8：上限 320 kbps）。
pub const MAX_BITRATE_BPS: i32 = 320_000;

/// 降级窗口：丢包 > 1% 持续 3 s（§8）。
pub const LOSS_WINDOW_SECS: u32 = 3;

/// 恢复窗口：连续 10 s 无丢包（§8）。
pub const CLEAN_WINDOW_SECS: u32 = 10;

/// 两次上调之间的最小间隔：每 5 s 最多一级（§8）。
pub const RAISE_INTERVAL_SECS: u64 = 5;

/// 触发降级的丢包率阈值（百分比 ×100）：> 1%。
pub const LOSS_THRESHOLD_X100: u16 = 100;

/// 「严重丢包」阈值（百分比 ×100）：> 5%（§8 里触发单声道降级的那一档，本轮只记账）。
pub const SEVERE_LOSS_THRESHOLD_X100: u16 = 500;

/// 「延迟允许」的 RTT 上限（µs）：§8 的 `RTT P95 > 30 ms` 判据。
///
/// 口径说明：本模块拿到的通常是 QUIC 的**平滑 RTT**（`StreamStats.rtt_us`），
/// 它比 P95 更稳、也更早暴露趋势；两者都远小于 30 ms 时才允许恢复。
pub const RTT_LIMIT_US: u32 = 30_000;

/// 降级步长分子（−20%）。
const DOWN_STEP_NUM: i32 = 4;
/// 降级步长分母。
const DOWN_STEP_DEN: i32 = 5;
/// 恢复步长分子（+10%）。
const UP_STEP_NUM: i32 = 11;
/// 恢复步长分母。
const UP_STEP_DEN: i32 = 10;

/// 链路严重度（本轮只用于记账与事件文案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSeverity {
    /// 丢包在阈值内。
    Ok,
    /// 丢包 > 1%：触发码率降级。
    Loss,
    /// 丢包 > 5%：§8 还要求叠加单声道降级（本轮未实施）。
    SevereLoss,
}

impl LinkSeverity {
    /// 稳定名字（事件 / 日志用）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Loss => "loss",
            Self::SevereLoss => "severe_loss",
        }
    }
}

/// 变更原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitrateReason {
    /// 丢包持续超窗 → 降级。
    PacketLoss {
        /// 触发时的丢包率（百分比 ×100）。
        loss_pct_x100: u16,
    },
    /// 连续稳定 → 恢复一级。
    Recovered,
}

impl BitrateReason {
    /// 人类可读说明（写进事件与验收记录）。
    pub fn describe(self) -> String {
        match self {
            Self::PacketLoss { loss_pct_x100 } => format!(
                "丢包 {}% 持续 {LOSS_WINDOW_SECS} s → 降级",
                f64::from(loss_pct_x100) / 100.0
            ),
            Self::Recovered => format!("连续 {CLEAN_WINDOW_SECS} s 无丢包 → 恢复一级"),
        }
    }
}

/// 一次码率变更。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitrateChange {
    /// 变更前（bps）。
    pub from_bps: i32,
    /// 变更后（bps）。
    pub to_bps: i32,
    /// 原因。
    pub reason: BitrateReason,
}

/// §8 的码率自适应状态机。
#[derive(Debug)]
pub struct AdaptiveBitrate {
    current_bps: i32,
    loss_seconds: u32,
    clean_seconds: u32,
    last_raise_at: Option<Duration>,
    last_loss_pct_x100: u16,
}
impl AdaptiveBitrate {
    /// 以初始目标码率建状态机（会夹到 `MIN_BITRATE_BPS..=MAX_BITRATE_BPS`）。
    pub fn new(initial_bps: i32) -> Self {
        Self {
            current_bps: initial_bps.clamp(MIN_BITRATE_BPS, MAX_BITRATE_BPS),
            loss_seconds: 0,
            clean_seconds: 0,
            last_raise_at: None,
            last_loss_pct_x100: 0,
        }
    }

    /// 当前目标码率（bps）。
    pub const fn current_bps(&self) -> i32 {
        self.current_bps
    }

    /// 最近一次采样的链路严重度。
    pub const fn severity(&self) -> LinkSeverity {
        if self.last_loss_pct_x100 > SEVERE_LOSS_THRESHOLD_X100 {
            LinkSeverity::SevereLoss
        } else if self.last_loss_pct_x100 > LOSS_THRESHOLD_X100 {
            LinkSeverity::Loss
        } else {
            LinkSeverity::Ok
        }
    }

    /// 吃一个 1 Hz 的链路样本；需要改码率时返回这次变更。
    ///
    /// `at` 是**单调递增**的运行时长（调用方给，便于单测喂假时钟）。
    /// 降级优先于恢复：同一条样本不可能既降又升。
    pub fn observe(
        &mut self,
        at: Duration,
        loss_pct_x100: u16,
        rtt_us: u32,
    ) -> Option<BitrateChange> {
        self.last_loss_pct_x100 = loss_pct_x100;

        if loss_pct_x100 > LOSS_THRESHOLD_X100 {
            self.loss_seconds = self.loss_seconds.saturating_add(1);
            self.clean_seconds = 0;
        } else {
            self.loss_seconds = 0;
            self.clean_seconds = self.clean_seconds.saturating_add(1);
        }

        // 降级：丢包 > 1% 且已经持续满 3 s，步长 −20%，下限 96 kbps。
        if self.loss_seconds >= LOSS_WINDOW_SECS {
            let next = (self.current_bps * DOWN_STEP_NUM / DOWN_STEP_DEN).max(MIN_BITRATE_BPS);
            if next < self.current_bps {
                let change = BitrateChange {
                    from_bps: self.current_bps,
                    to_bps: next,
                    reason: BitrateReason::PacketLoss { loss_pct_x100 },
                };
                self.current_bps = next;
                // 降完重新攒窗口：持续丢包时每 3 s 降一级，而不是每拍都降。
                self.loss_seconds = 0;
                self.clean_seconds = 0;
                return Some(change);
            }
            return None;
        }

        // 恢复：连续 10 s 无丢包 + 延迟允许，步长 +10%，上限 320 kbps，每 5 s 最多一级。
        if self.clean_seconds >= CLEAN_WINDOW_SECS && rtt_us <= RTT_LIMIT_US {
            let ready = self.last_raise_at.is_none_or(|last| {
                at.saturating_sub(last) >= Duration::from_secs(RAISE_INTERVAL_SECS)
            });
            if ready {
                let next = (self.current_bps * UP_STEP_NUM / UP_STEP_DEN).min(MAX_BITRATE_BPS);
                if next > self.current_bps {
                    let change = BitrateChange {
                        from_bps: self.current_bps,
                        to_bps: next,
                        reason: BitrateReason::Recovered,
                    };
                    self.current_bps = next;
                    self.last_raise_at = Some(at);
                    // 升一级后只需再等 5 s 就能再升一级（§8：每 5 s 最多一级）。
                    self.clean_seconds =
                        CLEAN_WINDOW_SECS.saturating_sub(RAISE_INTERVAL_SECS as u32);
                    return Some(change);
                }
            }
        }

        None
    }
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// 以「第 n 秒」为时刻喂样本。
    fn feed(model: &mut AdaptiveBitrate, secs: u64, loss_pct_x100: u16) -> Option<BitrateChange> {
        model.observe(Duration::from_secs(secs), loss_pct_x100, 2_000)
    }

    #[test]
    fn loss_below_threshold_never_drops() {
        let mut model = AdaptiveBitrate::new(160_000);
        let mut lowest = model.current_bps();
        for secs in 0..120 {
            if let Some(change) = feed(&mut model, secs, 100) {
                assert!(
                    change.to_bps > change.from_bps,
                    "恰好 1% 不该触发降级（只允许恢复）：{change:?}"
                );
            }
            lowest = lowest.min(model.current_bps());
        }
        assert_eq!(lowest, 160_000, "全程没有降过级");
    }

    #[test]
    fn three_seconds_above_one_percent_drops_twenty_percent() {
        let mut model = AdaptiveBitrate::new(160_000);
        assert_eq!(feed(&mut model, 0, 200), None, "第 1 s 只开始计时");
        assert_eq!(feed(&mut model, 1, 200), None, "第 2 s 还不够");
        let change = feed(&mut model, 2, 200).expect("第 3 s 必须降级");
        assert_eq!(change.from_bps, 160_000);
        assert_eq!(change.to_bps, 128_000, "−20%");
        assert_eq!(
            change.reason,
            BitrateReason::PacketLoss { loss_pct_x100: 200 }
        );
        assert_eq!(model.current_bps(), 128_000);
    }

    #[test]
    fn sustained_loss_steps_down_every_three_seconds_until_the_floor() {
        let mut model = AdaptiveBitrate::new(160_000);
        let mut steps = Vec::new();
        for secs in 0..30 {
            if let Some(change) = feed(&mut model, secs, 300) {
                steps.push(change.to_bps);
            }
        }
        assert_eq!(
            steps,
            vec![128_000, 102_400, 96_000],
            "每 3 s 一级，到 96 kbps 触底"
        );
        assert_eq!(model.current_bps(), MIN_BITRATE_BPS);
        assert_eq!(model.severity(), LinkSeverity::Loss);
    }

    #[test]
    fn recovery_needs_ten_clean_seconds_and_then_steps_every_five() {
        let mut model = AdaptiveBitrate::new(160_000);
        for secs in 0..9 {
            assert_eq!(
                feed(&mut model, secs, 0),
                None,
                "第 {secs} 个样本还不该恢复"
            );
        }
        let first = feed(&mut model, 9, 0).expect("满 10 个样本（10 s）无丢包，开始恢复");
        assert_eq!(first.to_bps, 176_000, "+10%");
        assert_eq!(first.reason, BitrateReason::Recovered);

        // 升一级后还要再攒 5 个无丢包样本（窗口 10 s，但距上次升级必须满 5 s）
        for secs in 10..14 {
            assert_eq!(feed(&mut model, secs, 0), None, "距上次升级不满 5 s");
        }
        let second = feed(&mut model, 14, 0).expect("距上次升级 5 s，可再升一级");
        assert_eq!(second.from_bps, 176_000);
        assert_eq!(second.to_bps, 193_600);
    }

    #[test]
    fn recovery_stops_at_the_ceiling() {
        let mut model = AdaptiveBitrate::new(304_000);
        let mut now = 0_u64;
        let mut last = None;
        for _ in 0..20 {
            now += 6;
            last = feed(&mut model, now, 0);
        }
        assert_eq!(model.current_bps(), MAX_BITRATE_BPS, "封顶 320 kbps");
        assert!(last.is_none(), "到顶之后不再产生变更");
    }

    #[test]
    fn high_rtt_blocks_recovery() {
        let mut model = AdaptiveBitrate::new(160_000);
        for secs in 0..30 {
            let change = model.observe(Duration::from_secs(secs), 0, RTT_LIMIT_US + 1);
            assert_eq!(change, None, "RTT 超限时不允许恢复");
        }
        assert_eq!(model.current_bps(), 160_000);
    }

    #[test]
    fn loss_after_recovery_drops_again() {
        let mut model = AdaptiveBitrate::new(160_000);
        for secs in 0..12 {
            feed(&mut model, secs, 0);
        }
        assert!(model.current_bps() > 160_000, "先恢复过一级");
        let before = model.current_bps();
        let mut dropped = None;
        for secs in 12..18 {
            // 只记第一次降级：持续丢包会每 3 s 再降一级，那些是另一条规则的事。
            if dropped.is_none() {
                dropped = feed(&mut model, secs, 900);
            } else {
                let _ = feed(&mut model, secs, 900);
            }
        }
        let dropped = dropped.expect("丢包回来必须再次降级");
        assert_eq!(dropped.from_bps, before);
        assert!(dropped.to_bps < before);
        assert_eq!(
            model.severity(),
            LinkSeverity::SevereLoss,
            "> 5% 记 severe_loss"
        );
    }

    #[test]
    fn initial_bitrate_is_clamped_into_the_documented_range() {
        assert_eq!(AdaptiveBitrate::new(10_000).current_bps(), MIN_BITRATE_BPS);
        assert_eq!(AdaptiveBitrate::new(900_000).current_bps(), MAX_BITRATE_BPS);
    }
}
