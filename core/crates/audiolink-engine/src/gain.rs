//! §4.1 的音量控制（SET_GAIN / SET_MUTE）在**播放侧**的落地。
//!
//! 三条纪律：
//!
//! 1. **纯逻辑**：这里只算「这一帧该乘多少」，不碰播放器、不碰线程；
//! 2. **渐变不是可选项**：硬切增益就是爆音，所以每次变更都按 ramp_ms 线性走，每帧最多一个步长；
//! 3. **非法值明确拒绝**：NaN / inf / 负数 / 超上限一律 Err，绝不静默夹到合法值再假装成功 ——
//!    runtime 里那句「假装接受了 SET_GAIN，用户会以为音量真的变了」现在兑现了。

/// 增益上限（§4.1：0.0–2.0）。
pub const MAX_GAIN_X1000: u32 = 2_000;

/// 增益参数不合法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GainError {
    /// 不是有限数（NaN / inf）。
    NotFinite,
    /// 超出 §4.1 的 0.0–2.0。
    OutOfRange,
}

impl std::fmt::Display for GainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFinite => write!(f, "gain is not a finite number"),
            Self::OutOfRange => write!(f, "gain is outside 0.0..=2.0"),
        }
    }
}

impl std::error::Error for GainError {}

/// f32 增益 → 千分点整数。
///
/// 协议给的是 f32，但内部一律用整数比较：浮点参数会让「有没有变化」这件事不可复现，
/// 而音量变更的验收恰恰要看这条线。
pub fn gain_x1000_from_f32(gain: f32) -> Result<u32, GainError> {
    if !gain.is_finite() {
        return Err(GainError::NotFinite);
    }
    if !(0.0..=2.0).contains(&gain) {
        return Err(GainError::OutOfRange);
    }
    Ok((gain * 1000.0).round() as u32)
}

/// §4.1 + FR-12：两层增益的合成 —— **本地 × 对端下发**。
///
/// # 为什么是乘法，而不是「本地覆盖对端」
///
/// 两层的**来源与语义都不同**：对端下发的是「对方希望我用多大声听」；本地是「这台设备自己
/// 还要再调多少」。覆盖语义会让任一侧的动作把另一侧的意图悄悄抹掉 —— 用户调完本地，对端一改
/// 音量就全丢。相乘则互不覆盖：任一层变化都仍然听得见影响，而两层都为 1.0 时与不分层完全一样。
///
/// # 为什么是两层状态、一个乘法点
///
/// 两个 [`GainState`] 各自走自己的 ramp（各自响应各自的变更），只在**播放前合成为一次乘法**
/// （见 `runtime.rs` 里 `playout_main` 的取帧分支）。合成放在乘法点而不是写回状态机，是为了
/// 避免「本地改一次就把对端的 target 覆盖掉」这类隐式耦合。
///
/// # 上界与削顶
///
/// 两层各自被 §4.1 的 0.0–2.0 校验，因此乘积最大 **4.0**（+12 dB）。这里**不做二次夹取**：
/// 夹了就等于偷偷改掉「相乘」这条拍板语义。真正会削顶的情形由混音器的软限幅兜住（它永不削平顶）。
#[must_use]
pub fn combine_local_and_remote(remote: f32, local: f32) -> f32 {
    remote * local
}

/// 音量状态机：当前值 → 目标值，按步长逐帧逼近。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GainState {
    current_x1000: u32,
    target_x1000: u32,
    /// 每帧最多变化的千分点（1 是下限：步长 0 会让渐变永远走不到）。
    step_x1000: u32,
}

impl GainState {
    /// 以某个增益起步（不做渐变）。
    pub const fn new(gain_x1000: u32) -> Self {
        let clamped = if gain_x1000 > MAX_GAIN_X1000 {
            MAX_GAIN_X1000
        } else {
            gain_x1000
        };
        Self {
            current_x1000: clamped,
            target_x1000: clamped,
            step_x1000: 1,
        }
    }

    /// 设定新目标：ramp_ms 内线性到位（不足一帧的提前量按一帧处理）。
    pub fn set_target(
        &mut self,
        gain_x1000: u32,
        ramp_ms: u32,
        frame_ms: u32,
    ) -> Result<(), GainError> {
        if gain_x1000 > MAX_GAIN_X1000 {
            return Err(GainError::OutOfRange);
        }
        self.target_x1000 = gain_x1000;
        let frames = (ramp_ms / frame_ms.max(1)).max(1);
        let delta = self.current_x1000.abs_diff(gain_x1000);
        self.step_x1000 = delta.div_ceil(frames).max(1);
        Ok(())
    }

    /// 本帧该乘多少，并按步长推进（每帧调一次）。
    pub fn next_frame_gain(&mut self) -> f32 {
        if self.current_x1000 < self.target_x1000 {
            self.current_x1000 = self
                .current_x1000
                .saturating_add(self.step_x1000)
                .min(self.target_x1000);
        } else if self.current_x1000 > self.target_x1000 {
            self.current_x1000 = self
                .current_x1000
                .saturating_sub(self.step_x1000)
                .max(self.target_x1000);
        }
        self.current_x1000 as f32 / 1000.0
    }

    pub const fn current_x1000(&self) -> u32 {
        self.current_x1000
    }

    pub const fn target_x1000(&self) -> u32 {
        self.target_x1000
    }

    pub const fn is_settled(&self) -> bool {
        self.current_x1000 == self.target_x1000
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// 两层增益**相乘**：本地 0 永远压过对端任何值（本地静音就是静音），两层都 1.0 等于原声。
    ///
    /// 改坏（例如写成覆盖或相加）：用户会看到「对端调完音量，我这边设的又没了」，
    /// 而这正是 FR-12 要避免的那种互相打架。
    #[test]
    fn local_and_remote_gains_multiply() {
        assert_eq!(combine_local_and_remote(1.0, 1.0), 1.0, "两层都原声 = 原声");
        assert_eq!(combine_local_and_remote(0.5, 1.0), 0.5, "只动对端那层");
        assert_eq!(combine_local_and_remote(1.0, 0.5), 0.5, "只动本地那层");
        assert_eq!(
            combine_local_and_remote(0.5, 0.5),
            0.25,
            "两层各减半 → 四分之一，这才是「相乘」的判别性证据"
        );
        assert_eq!(
            combine_local_and_remote(2.0, 2.0),
            4.0,
            "上界是乘积而非夹取"
        );
        assert_eq!(combine_local_and_remote(2.0, 0.0), 0.0, "本地静音压过一切");
    }

    #[test]
    fn starts_settled_and_clamps_above_the_ceiling() {
        let state = GainState::new(1_000);
        assert_eq!(state.current_x1000(), 1_000);
        assert!(state.is_settled());

        // 起步值超上限：夹到 §4.1 的上限（这是构造期，不是「接受一个非法变更」）
        assert_eq!(GainState::new(9_999).current_x1000(), MAX_GAIN_X1000);
    }

    #[test]
    fn ramps_across_the_requested_window() {
        let mut state = GainState::new(1_000);
        // 20 ms 帧、100 ms 渐变 = 5 帧走完 1000 个千分点 → 每帧 200
        state.set_target(0, 100, 20).unwrap();
        assert_eq!(state.next_frame_gain(), 0.8);
        assert_eq!(state.next_frame_gain(), 0.6);
        assert_eq!(state.next_frame_gain(), 0.4);
        assert_eq!(state.next_frame_gain(), 0.2);
        assert_eq!(state.next_frame_gain(), 0.0);
        assert!(state.is_settled());
        // 到位后不再变
        assert_eq!(state.next_frame_gain(), 0.0);
    }

    #[test]
    fn zero_ramp_lands_in_one_frame() {
        let mut state = GainState::new(1_000);
        state.set_target(500, 0, 20).unwrap();
        assert_eq!(state.next_frame_gain(), 0.5);
        assert!(state.is_settled());
    }

    #[test]
    fn tiny_delta_still_moves_every_frame() {
        // 步长下限是 1：即使 ramp 很长，也不会算出 0 步长把自己卡住
        let mut state = GainState::new(1_000);
        state.set_target(1_001, 10_000, 20).unwrap();
        assert_eq!(state.next_frame_gain(), 1.001);
        assert!(state.is_settled());
    }

    #[test]
    fn mute_then_restore_is_a_round_trip() {
        let mut state = GainState::new(1_000);
        state.set_target(0, 20, 20).unwrap();
        assert_eq!(state.next_frame_gain(), 0.0);
        state.set_target(1_000, 20, 20).unwrap();
        assert_eq!(state.next_frame_gain(), 1.0);
        assert!(state.is_settled());
    }

    #[test]
    fn rejects_gain_above_the_ceiling() {
        let mut state = GainState::new(1_000);
        assert_eq!(state.set_target(2_001, 20, 20), Err(GainError::OutOfRange));
        // 拒绝之后状态不变：目标仍是原来的值
        assert_eq!(state.target_x1000(), 1_000);
    }

    #[test]
    fn converts_protocol_floats_strictly() {
        assert_eq!(gain_x1000_from_f32(0.0).unwrap(), 0);
        assert_eq!(gain_x1000_from_f32(0.5).unwrap(), 500);
        assert_eq!(gain_x1000_from_f32(2.0).unwrap(), 2_000);
        assert_eq!(gain_x1000_from_f32(f32::NAN), Err(GainError::NotFinite));
        assert_eq!(
            gain_x1000_from_f32(f32::INFINITY),
            Err(GainError::NotFinite)
        );
        assert_eq!(gain_x1000_from_f32(-0.5), Err(GainError::OutOfRange));
        assert_eq!(gain_x1000_from_f32(2.5), Err(GainError::OutOfRange));
    }

    #[test]
    fn amplification_is_allowed_up_to_two() {
        let mut state = GainState::new(1_000);
        state.set_target(2_000, 20, 20).unwrap();
        assert_eq!(state.next_frame_gain(), 2.0);
        assert!(state.is_settled());
    }
}
