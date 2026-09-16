//! §7 预约播放的接收端排播（纯逻辑）。
//!
//! 协议 §7 的换算与调度规则，落到这里只有两个函数：
//!
//! 1. 时间轴换算：设 epoch 在**发送端**时钟轴上的时刻为 epoch_local_us、时钟估计给出的偏移为
//!    offset_us（对端 − 本机），则 epoch 在本机时钟轴上的对应时刻是
//!    local(epoch) = epoch_local_us − offset_us；某个包首样本的目标时刻则是
//!    local(epoch) + sample_index / 48000 + lead_ms；
//! 2. 排播判定（§7 第 1 / 3 条）：早于目标 → 等待（这一拍补静音）；迟到不超过 20 ms → 照常播；
//!    超过 20 ms → 丢弃并计入 late_drops。
//!
//! 纪律：
//!
//! - **纯逻辑**：不碰线程、不碰套接字、不读时钟。调用方把「现在的本地时刻」算好传进来，
//!   于是这套换算可以被单测钉死在 µs 上；
//! - **不做听感猜测**：没有 epoch、没有时钟估计时，调用方就该回到 M1 / M2 的本地游标路径，
//!   而不是拿一个半成品公式硬算。
//!
//! 尚未接入的部分（记账，不假装）：§7 公式里的 buffer_ms / dac_latency_ms 补偿项需要设备
//! 出厂 DAC 延迟估计，目前只用 20 ms 迟到门限；epoch_id 变更让旧排播立刻失效，这件事由调用方负责。

/// 协议锁定的音频采样率（§3）：48 kHz。
pub const SAMPLE_RATE_HZ: u64 = 48_000;

/// 迟到门限（§7 第 3 条：超过目标时刻 20 ms 直接跳过）。
pub const LATE_LIMIT_MS: u64 = 20;

/// 一帧有多少微秒（`frame_samples / 48000 s`）。
pub const fn frame_us(frame_samples: u32) -> i64 {
    let samples = if frame_samples == 0 { 1 } else { frame_samples };
    (samples as i64) * 1_000_000 / SAMPLE_RATE_HZ as i64
}

/// 发送端给出的组基准（来自 GROUP_EPOCH 控制帧，或会话默认）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpochSchedule {
    /// 组基准标识；会话重建后换新值。
    pub epoch_id: u64,
    /// epoch 在**发送端**单调时钟上的对应时刻（µs）。
    pub epoch_local_us: u64,
    /// 预约提前量（ms）：越大越安全，代价是起播更晚。
    pub lead_ms: u32,
}

impl EpochSchedule {
    pub const fn new(epoch_id: u64, epoch_local_us: u64, lead_ms: u32) -> Self {
        Self {
            epoch_id,
            epoch_local_us,
            lead_ms,
        }
    }

    /// 该包首样本在**本机**时钟轴上的目标时刻（µs）。
    ///
    /// offset_us 是时钟估计给出的「对端 − 本机」：对端比本机快 1 s 时 offset = +1_000_000，
    /// 于是 local(epoch) = epoch_local_us − 1 s —— 符号错了整组同步就整体偏掉两倍偏移量。
    pub fn local_target_us(&self, sample_index: u32, offset_us: i64) -> i64 {
        let local_epoch_us = self.epoch_local_us as i64 - offset_us;
        let sample_offset_us =
            (sample_index as i64).saturating_mul(1_000_000) / SAMPLE_RATE_HZ as i64;
        local_epoch_us
            .saturating_add(sample_offset_us)
            .saturating_add(self.lead_ms as i64 * 1_000)
    }

    /// 目标时刻**对齐到帧边界**（向上取整）。
    ///
    /// 为什么要对齐 —— 2026-09-17 定位到的真缺陷：`action` 原本直接比较「现在 ≥ target」，
    /// 而 target 是**任意**微秒时刻。两个接收端各自的判定时刻只要分别落在它前后 1 µs，
    /// 就会一头「还没到、这一拍补静音」、一头「到了、立刻播」—— **组内偏差整整一帧（20 ms）**，
    /// 直接破掉 M3 的 ±10 ms 验收线（回环实测 5 次踩中 2 次）。
    /// 对齐到帧边界后，两端比较的是**同一个帧边界**，判定必然一致（时钟估计误差 ≤ 2 ms，远小于一帧）。
    #[must_use]
    pub fn frame_aligned_target_us(
        &self,
        sample_index: u32,
        offset_us: i64,
        frame_samples: u32,
    ) -> i64 {
        let frame = frame_us(frame_samples);
        let target = self.local_target_us(sample_index, offset_us);
        let frames = target.div_euclid(frame);
        let extra = i64::from(target.rem_euclid(frame) != 0);
        frames.saturating_add(extra).saturating_mul(frame)
    }

    /// §7 的排播判定：等待 / 播放 / 丢弃。
    ///
    /// **等待判定在「帧边界」上做**（见 `frame_aligned_target_us`）；迟到门限仍按微秒算 ——
    /// §7 说的是「超过 20 ms」，那是与帧长无关的固定窗口。
    pub fn action(
        &self,
        now_local_us: i64,
        sample_index: u32,
        offset_us: i64,
        frame_samples: u32,
    ) -> PlayoutAction {
        let target = self.frame_aligned_target_us(sample_index, offset_us, frame_samples);
        if now_local_us < target {
            return PlayoutAction::Wait {
                wait_us: (target - now_local_us) as u64,
            };
        }
        let behind_us = now_local_us - target;
        if behind_us > LATE_LIMIT_MS as i64 * 1_000 {
            return PlayoutAction::Drop {
                late_us: behind_us.unsigned_abs(),
            };
        }
        PlayoutAction::Play
    }
}

/// 这一拍该怎么处理这帧。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayoutAction {
    /// 还没到目标时刻：这一拍补静音、不推进播放游标（等待 wait_us 到位）。
    Wait {
        /// 距目标时刻还有多久（µs）。
        wait_us: u64,
    },
    /// 到点了，正常交给播放器。
    Play,
    /// 已经过期太久（超过目标 + 20 ms）：丢弃并计入 late_drops。
    Drop {
        /// 晚了多久（µs）。
        late_us: u64,
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn schedule() -> EpochSchedule {
        // epoch 在发送端 1 000 000 µs（= 1 s）处，无提前量
        EpochSchedule::new(0x1122_3344, 1_000_000, 0)
    }

    #[test]
    fn maps_sample_index_along_the_epoch_axis() {
        let schedule = schedule();
        assert_eq!(schedule.local_target_us(0, 0), 1_000_000);
        assert_eq!(schedule.local_target_us(48_000, 0), 2_000_000);
        assert_eq!(schedule.local_target_us(960, 0), 1_020_000);
        assert_eq!(schedule.local_target_us(1, 0), 1_000_020);
    }

    #[test]
    fn offset_shifts_the_epoch_the_right_way() {
        let schedule = schedule();
        // 对端比本机快 1 s：同一个 epoch 时刻，本机钟面上要减掉这 1 s
        assert_eq!(schedule.local_target_us(0, 1_000_000), 0);
        // 对端比本机慢 0.5 s（offset 为负）：本机钟面上要加回去
        assert_eq!(schedule.local_target_us(0, -500_000), 1_500_000);
    }

    #[test]
    fn lead_ms_is_added_on_top() {
        let schedule = EpochSchedule::new(1, 1_000_000, 50);
        assert_eq!(schedule.local_target_us(0, 0), 1_050_000);
        assert_eq!(schedule.local_target_us(48_000, 0), 2_050_000);
    }

    #[test]
    fn action_waits_before_the_target() {
        let schedule = schedule();
        assert_eq!(
            schedule.action(999_000, 0, 0, 960),
            PlayoutAction::Wait { wait_us: 1_000 }
        );
        assert_eq!(
            schedule.action(0, 0, 0, 960),
            PlayoutAction::Wait { wait_us: 1_000_000 }
        );
    }

    #[test]
    fn action_plays_inside_the_late_window() {
        let schedule = schedule();
        assert_eq!(schedule.action(1_000_000, 0, 0, 960), PlayoutAction::Play);
        assert_eq!(schedule.action(1_000_001, 0, 0, 960), PlayoutAction::Play);
        // 恰好 20 ms：仍算赶上（§7 说的是「超过」20 ms 才丢）
        assert_eq!(schedule.action(1_020_000, 0, 0, 960), PlayoutAction::Play);
    }

    #[test]
    fn action_drops_when_too_late() {
        let schedule = schedule();
        assert_eq!(
            schedule.action(1_020_001, 0, 0, 960),
            PlayoutAction::Drop { late_us: 20_001 }
        );
        assert_eq!(
            schedule.action(1_500_000, 0, 0, 960),
            PlayoutAction::Drop { late_us: 500_000 }
        );
    }

    /// 2026-09-17 那个真缺陷的回归测试：target 落在**帧中间**时，同一帧内的两端必须做同一个决定。
    #[test]
    fn both_ends_decide_the_same_within_one_frame() {
        // epoch 在 1_010_000 µs —— 刻意不是 20 ms 的整数倍，落在帧中间。
        let schedule = EpochSchedule::new(7, 1_010_000, 0);
        let frame_samples = 960_u32; // 20 ms
        let just_before = schedule.action(1_009_999, 0, 0, frame_samples);
        let just_after = schedule.action(1_010_001, 0, 0, frame_samples);
        assert!(
            matches!(just_before, PlayoutAction::Wait { .. })
                && matches!(just_after, PlayoutAction::Wait { .. }),
            "同一帧内的两端必须都等：{just_before:?} vs {just_after:?}（修正前这里会一头等一头播）"
        );
        // 目标被抬到下一个帧边界 1_020_000，于是两端一起在这一刻播。
        assert_eq!(
            schedule.action(1_020_000, 0, 0, frame_samples),
            PlayoutAction::Play
        );
        assert_eq!(
            schedule.frame_aligned_target_us(0, 0, frame_samples),
            1_020_000,
            "目标必须被对齐到帧边界"
        );
        // 恰好落在帧边界上的目标不该被多推一帧。
        let aligned = EpochSchedule::new(8, 1_020_000, 0);
        assert_eq!(
            aligned.frame_aligned_target_us(0, 0, frame_samples),
            1_020_000
        );
    }

    #[test]
    fn huge_sample_index_does_not_overflow() {
        let schedule = EpochSchedule::new(1, 0, 0);
        let target = schedule.local_target_us(u32::MAX, 0);
        // u32::MAX 个样本约 24.8 小时；目标必须是有限且单调的
        assert!(target > 0);
        assert!(target > schedule.local_target_us(u32::MAX - 1, 0));
    }

    #[test]
    fn epoch_before_boot_still_decides_sanely() {
        // epoch 早于本机开机（换算到本机轴上是负数）：现在当然已经过点 —— 要么播要么丢，不许不置可否
        let schedule = EpochSchedule::new(1, 1_000, 0);
        let action = schedule.action(5_000_000, 0, 10_000_000, 960);
        assert!(matches!(
            action,
            PlayoutAction::Play | PlayoutAction::Drop { .. }
        ));
    }
}
