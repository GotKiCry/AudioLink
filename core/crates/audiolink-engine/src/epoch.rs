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

/// 「可以提前播」的容差（µs）：**必须远小于一帧、又远大于线程唤醒抖动**。
///
/// # 为什么需要它（2026-09-17 第四次定位）
///
/// 拍点被锚到 target 本身之后，播放线程仍可能在 target **之前几微秒**醒来（`Instant` 与
/// 单调时钟两次读取之间的差、`sleep` 的粒度、调度抖动）。判据若写死 `now >= target`，
/// 这一端就会「还没到 → 这一拍补静音、下一拍再播」，另一端恰好按时醒来 → 立刻播 ——
/// 首拍错开一拍，整段音频就差一帧（实测失败样本恒为 P50 ≈ 20 ms、扣帧后 0.00 ms）。
///
/// 2 ms 的提前量在这个问题上是一把合适的尺子：它比 µs 级抖动大三个数量级，又只占一帧的十分之一，
/// 听感上完全等价；同时它把「靠阈值二值化」这件事从**抖动中心**挪到了安全区。
pub const EARLY_TOLERANCE_US: i64 = 2_000;

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

    /// 该帧在**本机**时钟轴上的目标时刻（µs），不做任何网格对齐。
    ///
    /// # 为什么不再对齐到「全局帧网格」（2026-09-17 第三层修复）
    ///
    /// 第二层修复把 target **向上取整到全局帧网格**（k × frame 的整数倍），理由是「让两端比较同一个边界」。
    /// 但那选错了参考系：两端各有自己的时钟估计（误差 ε，回环实测几毫秒），把同一个 epoch 换算到本机轴后
    /// 得到的 target 本来就差 ε —— 再拿**同一个全局网格**去取整，只要这两个 target 之间夹着一个网格边界，
    /// 一端就被抬到第 k 格、另一端第 k+1 格，**整段音频差一帧（20 ms）**；这与实测「绝对偏差 P50 19.98 ms、
    /// 扣掉一帧后 0.01 ms」完全一致（见 docs/49-m3-group-start-phase.md）。
    ///
    /// 正解是让 target 与**拍点**同源：两者都用「本端换算后的 epoch 网格」（见 `phase_us`），于是每一帧的
    /// target 恰好落在本端的某一拍上，判定在那一拍必然成立 —— 两端的播放时刻之差退化成时钟估计误差 ε 本身，
    /// 而不是「0 或一整帧」的二值结果。
    #[must_use]
    pub fn target_us(&self, sample_index: u32, offset_us: i64) -> i64 {
        self.local_target_us(sample_index, offset_us)
    }

    /// 拍点相位（µs）：**某一帧**的 target 在帧网格上的相位。
    ///
    /// 播放线程把拍点锚到这个相位，而不是锚到全局网格的 0 相位。两端的相位各自由自己的时钟估计算出、
    /// 相差 ε，而各自的 target 也相差同一个 ε —— 同源，所以该帧的 target 恰好落在本端的某一拍上。
    ///
    /// **必须传入真实的帧序号**，不能假设「首帧样本序号是 0」：实际流的样本序号由首包给出
    /// （`PlayoutSync::sample_index_of` 里那个 base），offset 任意；用错序号会让拍点整体错相。
    #[must_use]
    pub fn phase_us(&self, sample_index: u32, offset_us: i64, frame_samples: u32) -> i64 {
        let frame = frame_us(frame_samples).max(1);
        self.target_us(sample_index, offset_us).rem_euclid(frame)
    }

    /// §7 的排播判定：等待 / 播放 / 丢弃。
    ///
    /// **等待判定用与拍点同源的 target**（见 `target_us` / `phase_us`）；迟到门限仍按微秒算 ——
    /// §7 说的是「超过 20 ms」，那是与帧长无关的固定窗口。
    pub fn action(&self, now_local_us: i64, sample_index: u32, offset_us: i64) -> PlayoutAction {
        let target = self.target_us(sample_index, offset_us);
        // 提前量：见 `EARLY_TOLERANCE_US` —— 判据贴着 target 会因为几微秒的唤醒抖动而二值化。
        let now_within_tolerance = now_local_us.saturating_add(EARLY_TOLERANCE_US);
        if now_within_tolerance < target {
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
        let schedule = schedule(); // epoch = 1_000_000 µs，无提前量
        // 距目标 1 ms：落在 2 ms 的提前量之内 → 已经算赶上（见 EARLY_TOLERANCE_US）。
        assert_eq!(schedule.action(999_000, 0, 0), PlayoutAction::Play);
        // 距目标 3 ms：超出提前量 → 继续等，wait_us 给出真实差值。
        assert_eq!(
            schedule.action(997_000, 0, 0),
            PlayoutAction::Wait { wait_us: 3_000 }
        );
        assert_eq!(
            schedule.action(0, 0, 0),
            PlayoutAction::Wait { wait_us: 1_000_000 }
        );
    }

    #[test]
    fn action_plays_inside_the_late_window() {
        let schedule = schedule();
        assert_eq!(schedule.action(1_000_000, 0, 0), PlayoutAction::Play);
        assert_eq!(schedule.action(1_000_001, 0, 0), PlayoutAction::Play);
        // 恰好 20 ms：仍算赶上（§7 说的是「超过」20 ms 才丢）
        assert_eq!(schedule.action(1_020_000, 0, 0), PlayoutAction::Play);
    }

    #[test]
    fn action_drops_when_too_late() {
        let schedule = schedule();
        assert_eq!(
            schedule.action(1_020_001, 0, 0),
            PlayoutAction::Drop { late_us: 20_001 }
        );
        assert_eq!(
            schedule.action(1_500_000, 0, 0),
            PlayoutAction::Drop { late_us: 500_000 }
        );
    }

    /// 未到目标就等、到点就播：target 就是 epoch + lead 本身，不再被推到全局网格。
    #[test]
    fn target_is_played_on_the_tick_that_reaches_it() {
        // epoch 在 1_010_000 µs —— 刻意不是 20 ms 的整数倍。
        let schedule = EpochSchedule::new(7, 1_010_000, 0);
        assert_eq!(schedule.target_us(0, 0), 1_010_000);
        // 距目标还差 3 ms（超出 2 ms 的提前量）→ 必须等。
        assert!(matches!(
            schedule.action(1_007_000, 0, 0),
            PlayoutAction::Wait { .. }
        ));
        assert_eq!(schedule.action(1_010_000, 0, 0), PlayoutAction::Play);
        assert_eq!(schedule.action(1_010_001, 0, 0), PlayoutAction::Play);
    }

    /// 第四层回归：拍点抖到 target **之前**几微秒时，必须仍然判「播」——
    /// 否则这一端会白等一拍，首拍就与对端错开整整一帧（实测失败样本 P50 ≈ 20 ms、扣帧后 0.00 ms）。
    #[test]
    fn early_tolerance_absorbs_tick_jitter() {
        let schedule = EpochSchedule::new(7, 1_010_000, 0);
        // 提前 1 µs / 1.999 ms：都算赶上（线程唤醒抖动的量级）。
        assert_eq!(schedule.action(1_010_000 - 1, 0, 0), PlayoutAction::Play);
        assert_eq!(
            schedule.action(1_010_000 - 1_999, 0, 0),
            PlayoutAction::Play
        );
        // 提前 2.001 ms：超出容差，老老实实等。
        assert!(matches!(
            schedule.action(1_010_000 - 2_001, 0, 0),
            PlayoutAction::Wait { .. }
        ));
        // 迟到窗口不受影响（20 ms 仍在 µs 口径上）。
        assert!(matches!(
            schedule.action(1_010_000 + 20_001, 0, 0),
            PlayoutAction::Drop { .. }
        ));
    }

    /// 第三层回归（2026-09-17）：**target 必须落在本端拍点网格上**，且对两端的时钟估计误差免疫。
    ///
    /// 这正是「差 0 或差一整帧」那个二值结果的根：只要 target 与拍点同源，两端各自的拍点就一定是
    /// 「本端 target 所在的那一拍」，两端的播放时刻之差退化成 ε 本身。
    #[test]
    fn target_sits_exactly_on_the_epoch_phase_grid() {
        let schedule = EpochSchedule::new(7, 1_010_000, 0);
        let frame_samples = 960_u32;
        let frame = frame_us(frame_samples);
        // ε 取回环实测量级：±2 ms（两端时钟估计之差）。
        // 序号刻意包含「非帧整数倍」（44_640 = 46.5 帧）与「帧整数倍」两类 —— 相位由同一序号算出，
        // 所以两类都必须成立（实际流里 base 是首包给的，不能假设它对齐）。
        for offset in [0_i64, 2_000, -2_000, 1_337] {
            for index in [0_u32, 960, 1_920, 48_000, 44_640] {
                let phase = schedule.phase_us(index, offset, frame_samples);
                let target = schedule.target_us(index, offset);
                assert_eq!(
                    (target - phase).rem_euclid(frame),
                    0,
                    "offset={offset} index={index}：target 必须恰好落在本端拍点上"
                );
            }
        }
    }

    /// 第三层回归：两端在同一拍号上判定时必须都播，且播放时刻之差就是 ε（远小于一帧）。
    #[test]
    fn both_ends_play_the_same_frame_within_the_clock_estimate_error() {
        let schedule = EpochSchedule::new(7, 1_010_000, 0);
        let frame_samples = 960_u32;
        let frame = frame_us(frame_samples);
        let (offset_a, offset_b) = (0_i64, 2_000_i64); // 两端时钟估计相差 2 ms
        // 两端的拍点序列起点 = 本端对第 0 帧算出的 target（相差 ε），此后每帧一格。
        let (base_a, base_b) = (
            schedule.target_us(0, offset_a),
            schedule.target_us(0, offset_b),
        );
        for tick in 0..20_i64 {
            let index = 960_u32 * (tick as u32 + 1);
            let now_a = base_a + (tick + 1) * frame;
            let now_b = base_b + (tick + 1) * frame;
            let act_a = schedule.action(now_a, index, offset_a);
            let act_b = schedule.action(now_b, index, offset_b);
            assert_eq!(act_a, PlayoutAction::Play, "A 端第 {tick} 拍该播");
            assert_eq!(act_b, PlayoutAction::Play, "B 端第 {tick} 拍该播");
            assert!(
                (now_a - now_b).abs() <= 2_000,
                "两端的播放时刻之差只能是时钟估计误差 ε，而不是一整帧"
            );
        }
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
        let action = schedule.action(5_000_000, 0, 10_000_000);
        assert!(matches!(
            action,
            PlayoutAction::Play | PlayoutAction::Drop { .. }
        ));
    }
}
