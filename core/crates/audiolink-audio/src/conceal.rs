//! CELT-only 的 PCM 丢包掩盖。
//!
//! `opus-rs` 在项目默认的 `RESTRICTED_LOWDELAY` 档下，第 2 个连续丢失帧起会退化为硬静音。
//! 本模块保存上一帧已解码 PCM：丢包时重复并逐渐淡出，真实音频恢复时做短交叉淡化。
//! 所有缓冲只在构造时分配，`conceal_into` / `process_good` 稳态路径不分配。

use crate::error::AudioError;
use crate::format::{CHANNELS, SAMPLES_PER_MS, frame_interleaved, is_supported_frame_ms};

/// 完全淡出到静音的最长连续丢包时长。
pub const CONCEAL_FADE_MS: usize = 120;
/// 丢包开始和恢复时的边界平滑时长；与 CELT 的 2.5 ms overlap 一致。
pub const CONCEAL_CROSSFADE_SAMPLES: usize = 120;

/// 重复上一帧、淡出并在恢复时交叉淡化的 PCM 掩盖器。
#[derive(Debug)]
pub struct PcmConcealer {
    history: Vec<f32>,
    last_output: [f32; CHANNELS as usize],
    frame_samples: usize,
    lost_samples: usize,
    has_history: bool,
}

impl PcmConcealer {
    /// 按协议帧长建立掩盖器。
    pub fn new(frame_ms: u32) -> Result<Self, AudioError> {
        if !is_supported_frame_ms(frame_ms) {
            return Err(AudioError::invalid_config_owned(format!(
                "丢包掩盖不支持 {frame_ms} ms 帧"
            )));
        }
        let frame_samples = frame_interleaved(frame_ms);
        Ok(Self {
            history: vec![0.0; frame_samples],
            last_output: [0.0; CHANNELS as usize],
            frame_samples,
            lost_samples: 0,
            has_history: false,
        })
    }

    /// 处理一帧成功解码的 PCM。若刚从丢包恢复，会原地交叉淡化帧首。
    pub fn process_good(&mut self, samples: &mut [f32]) -> Result<(), AudioError> {
        self.require_frame(samples)?;
        if self.lost_samples > 0 {
            let overlap = CONCEAL_CROSSFADE_SAMPLES.min(self.frame_samples / CHANNELS as usize);
            for frame in 0..overlap {
                let mix = (frame + 1) as f32 / overlap as f32;
                for channel in 0..CHANNELS as usize {
                    let index = frame * CHANNELS as usize + channel;
                    samples[index] =
                        self.last_output[channel].mul_add(1.0 - mix, samples[index] * mix);
                }
            }
        }

        self.history.copy_from_slice(samples);
        let last = self.frame_samples - CHANNELS as usize;
        self.last_output.copy_from_slice(&samples[last..]);
        self.lost_samples = 0;
        self.has_history = true;
        Ok(())
    }

    /// 生成一帧掩盖 PCM：重复上一帧，在 120 ms 内线性淡出到静音。
    pub fn conceal_into(&mut self, out: &mut [f32]) -> Result<(), AudioError> {
        self.require_frame(out)?;
        if !self.has_history {
            return Err(AudioError::codec("尚无成功解码帧，无法生成 PCM 丢包掩盖"));
        }

        let channels = CHANNELS as usize;
        let frames = self.frame_samples / channels;
        let overlap = CONCEAL_CROSSFADE_SAMPLES.min(frames);
        let fade_samples = CONCEAL_FADE_MS * SAMPLES_PER_MS;
        for frame in 0..frames {
            let elapsed = self.lost_samples.saturating_add(frame);
            let gain = 1.0 - elapsed.min(fade_samples) as f32 / fade_samples as f32;
            let edge_mix = if frame < overlap {
                (frame + 1) as f32 / overlap as f32
            } else {
                1.0
            };
            for channel in 0..channels {
                let index = frame * channels + channel;
                let repeated = self.history[index];
                let smoothed =
                    self.last_output[channel].mul_add(1.0 - edge_mix, repeated * edge_mix);
                out[index] = smoothed * gain;
            }
        }

        let last = self.frame_samples - channels;
        self.last_output.copy_from_slice(&out[last..]);
        self.lost_samples = self.lost_samples.saturating_add(frames);
        Ok(())
    }

    /// 当前连续丢失的每声道样本数。
    pub const fn lost_samples(&self) -> usize {
        self.lost_samples
    }

    /// 丢弃历史（新流或长时间序号跳变时调用）。
    pub fn reset(&mut self) {
        self.history.fill(0.0);
        self.last_output.fill(0.0);
        self.lost_samples = 0;
        self.has_history = false;
    }

    fn require_frame(&self, samples: &[f32]) -> Result<(), AudioError> {
        if samples.len() == self.frame_samples {
            Ok(())
        } else {
            Err(AudioError::invalid_config_owned(format!(
                "丢包掩盖需要 {} 个交错样本，实际 {}",
                self.frame_samples,
                samples.len()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|value| value * value).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn 无历史与错误帧长被明确拒绝() {
        let mut concealer = PcmConcealer::new(20).unwrap();
        assert!(concealer.conceal_into(&mut [0.0; 4]).is_err());
        assert!(concealer.process_good(&mut [0.0; 4]).is_err());
        let mut frame = vec![0.0; frame_interleaved(20)];
        assert!(concealer.conceal_into(&mut frame).is_err());
    }

    #[test]
    fn 连续丢包重复上一帧并在限定时间内淡出() {
        let mut concealer = PcmConcealer::new(20).unwrap();
        let mut good = vec![0.5; frame_interleaved(20)];
        concealer.process_good(&mut good).unwrap();
        let mut concealed = vec![0.0; good.len()];
        let mut levels = Vec::new();
        for _ in 0..7 {
            concealer.conceal_into(&mut concealed).unwrap();
            levels.push(rms(&concealed));
        }
        assert!(levels[0] > 0.4, "首个掩盖帧应保留主要能量：{levels:?}");
        assert!(levels.windows(2).all(|pair| pair[1] <= pair[0]));
        assert!(levels[6] < 1e-6, "120 ms 后应平滑进入静音：{levels:?}");
        assert_eq!(concealer.lost_samples(), 7 * 960);
    }

    #[test]
    fn 恢复帧首交叉淡化并在_overlap_后回到真实音频() {
        let mut concealer = PcmConcealer::new(20).unwrap();
        let mut good = vec![1.0; frame_interleaved(20)];
        concealer.process_good(&mut good).unwrap();
        let mut concealed = vec![0.0; good.len()];
        concealer.conceal_into(&mut concealed).unwrap();
        let previous = *concealed.last().unwrap();

        let mut recovered = vec![-1.0; good.len()];
        concealer.process_good(&mut recovered).unwrap();
        assert!((recovered[0] - previous).abs() < 0.05);
        let after_overlap = CONCEAL_CROSSFADE_SAMPLES * CHANNELS as usize;
        assert_eq!(recovered[after_overlap], -1.0);
        assert_eq!(concealer.lost_samples(), 0);
    }

    #[test]
    fn 立体声通道独立且复位后不泄露旧历史() {
        let mut concealer = PcmConcealer::new(10).unwrap();
        let mut good = vec![0.0; frame_interleaved(10)];
        for pair in good.chunks_exact_mut(2) {
            pair[0] = 0.75;
            pair[1] = -0.25;
        }
        concealer.process_good(&mut good).unwrap();
        let mut concealed = vec![0.0; good.len()];
        concealer.conceal_into(&mut concealed).unwrap();
        assert!(
            concealed
                .chunks_exact(2)
                .all(|pair| pair[0] > 0.0 && pair[1] < 0.0)
        );
        concealer.reset();
        assert!(concealer.conceal_into(&mut concealed).is_err());
    }
}
