//! 回收持续积压，覆盖内核队列和 Android 输出链；不把一次突发到包当成过载。

use std::time::{Duration, Instant};

use audiolink_audio::PlayoutBufferState;

const EXCESS_WINDOW: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BacklogAction {
    Play,
    /// 丢掉一帧旧音频并正常写下一帧，降低内核队列水位。
    DropQueued,
    /// 消费一帧旧音频但本拍不写设备，让已有的下游积压自然播放。
    DrainOutput,
}

#[derive(Default)]
pub(super) struct PlayoutBacklog {
    excess_since: Option<Instant>,
}

impl PlayoutBacklog {
    pub(super) fn reset(&mut self) {
        self.excess_since = None;
    }

    /// 只供单源、非预约播放使用。未知输出水位不能参与总量调节。
    pub(super) fn observe(
        &mut self,
        now: Instant,
        queued_packets: usize,
        target_packets: usize,
        packet_frames: u32,
        output: Option<PlayoutBufferState>,
    ) -> BacklogAction {
        let Some(output) = output.filter(|state| state.target_frames > 0) else {
            self.reset();
            return BacklogAction::Play;
        };
        let total = (queued_packets as u64)
            .saturating_mul(u64::from(packet_frames))
            .saturating_add(u64::from(output.queued_frames));
        let budget = (target_packets as u64)
            .saturating_mul(u64::from(packet_frames))
            .saturating_add(u64::from(output.target_frames));
        // 保留一整帧摆幅；500 ms 内任一拍回落就取消回收。下游至少还有
        // 一帧 + 目标余量才允许停写，不能把已有余量抽干制造新的欠载。
        let action = if output.queued_frames >= output.target_frames.saturating_add(packet_frames)
            && queued_packets > 0
        {
            BacklogAction::DrainOutput
        } else if queued_packets > target_packets {
            BacklogAction::DropQueued
        } else {
            BacklogAction::Play
        };
        if packet_frames == 0
            || total < budget.saturating_add(u64::from(packet_frames))
            || action == BacklogAction::Play
        {
            self.reset();
            return BacklogAction::Play;
        }
        let since = *self.excess_since.get_or_insert(now);
        if now.saturating_duration_since(since) < EXCESS_WINDOW {
            return BacklogAction::Play;
        }
        self.reset();
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(queued_frames: u32) -> Option<PlayoutBufferState> {
        Some(PlayoutBufferState {
            queued_frames,
            target_frames: 1_440,
        })
    }

    #[test]
    fn eighty_ms_bursts_are_not_mistaken_for_sustained_backlog() {
        let mut control = PlayoutBacklog::default();
        let start = Instant::now();
        for tick in 0..500 {
            let queued = 6 - tick % 4;
            assert_eq!(
                control.observe(
                    start + Duration::from_millis(tick * 20),
                    queued as usize,
                    3,
                    960,
                    output(960)
                ),
                BacklogAction::Play
            );
        }
    }

    #[test]
    fn backlog_cannot_hide_by_moving_from_device_to_engine() {
        let mut control = PlayoutBacklog::default();
        let start = Instant::now();
        // 总量始终 110 ms：从 40+70 移到 80+30，仍应回收。
        assert_eq!(
            control.observe(start, 2, 1, 960, output(3_360)),
            BacklogAction::Play
        );
        assert_eq!(
            control.observe(start + EXCESS_WINDOW, 4, 1, 960, output(1_440)),
            BacklogAction::DropQueued
        );
        // 回收每次只有一帧，不能同一拍连续抽干。
        assert_eq!(
            control.observe(start + EXCESS_WINDOW, 3, 1, 960, output(1_440)),
            BacklogAction::Play
        );
    }

    #[test]
    fn output_backlog_is_drained_without_refilling_it_with_silence() {
        let mut control = PlayoutBacklog::default();
        let start = Instant::now();
        control.observe(start, 1, 1, 960, output(2_880));
        assert_eq!(
            control.observe(start + EXCESS_WINDOW, 1, 1, 960, output(2_880)),
            BacklogAction::DrainOutput
        );
    }

    #[test]
    fn transient_recovery_unknown_feedback_and_target_raise_reset_the_window() {
        let start = Instant::now();
        for interrupted in [None, output(960)] {
            let mut control = PlayoutBacklog::default();
            control.observe(start, 4, 1, 960, output(1_440));
            control.observe(start + Duration::from_millis(400), 1, 1, 960, interrupted);
            assert_eq!(
                control.observe(start + EXCESS_WINDOW, 4, 1, 960, output(1_440)),
                BacklogAction::Play
            );
        }
        let mut control = PlayoutBacklog::default();
        control.observe(start, 4, 1, 960, output(1_440));
        assert_eq!(
            control.observe(start + EXCESS_WINDOW, 4, 6, 960, output(1_440)),
            BacklogAction::Play
        );
    }
}
