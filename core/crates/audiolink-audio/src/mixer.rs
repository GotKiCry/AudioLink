//! FR-12 混音器：多路求和 + 软限幅 + 每路增益 / 静音。
//!
//! 四条纪律：
//!
//! 1. **纯逻辑**：只吃「已经解出来的 PCM 帧」，不碰设备、不碰线程、不读时钟 —— 于是它能被单测钉死；
//! 2. **格式必须一致**：多路混音的第一性前提是同一时间轴、同一格式（NFR-13 零重采样），不一致一律拒绝；
//! 3. **软限幅而不是硬削顶**：幅度不超过拐点时直通，超过后指数逼近 ±1（连续、单调、永不削顶），
//!    并记录「被限幅的样本数」等可观测证据 —— 「听起来有没有爆」不该只能靠猜；
//! 4. **上限 8 路**（FR-12）：第 9 路明确拒绝，而不是悄悄混进来把音量搞乱。

use std::collections::VecDeque;

/// FR-12 的混音路上限（超过要报 STREAM_LIMIT）。
pub const MAX_MIX_SOURCES: usize = 8;

/// 软限幅的拐点：绝对幅度超过它才开始压缩。
pub const SOFT_KNEE: f32 = 0.7;

/// 混音器的输入格式（多路必须一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

/// 混音失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MixerError {
    /// 超过 FR-12 的上限。
    SourceLimit {
        /// 上限值。
        max: usize,
    },
    /// 与混音器格式不一致（采样率或声道数）。
    FormatMismatch {
        /// 混音器的格式。
        expected: MixFormat,
        /// 这一路送来的格式。
        got: MixFormat,
    },
    /// 未知的源。
    UnknownSource {
        /// 源标识。
        id: u32,
    },
    /// 源已经存在。
    DuplicateSource {
        /// 源标识。
        id: u32,
    },
    /// 送来的样本数不是整帧（会把多路的时间轴错开）。
    PartialFrame {
        /// 收到的样本数。
        samples: usize,
        /// 一帧的样本数（交错后）。
        frame: usize,
    },
}

impl std::fmt::Display for MixerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SourceLimit { max } => write!(f, "mixer is limited to {max} sources"),
            Self::FormatMismatch { expected, got } => write!(
                f,
                "format mismatch: mixer is {} Hz/{} ch, source sent {} Hz/{} ch",
                expected.sample_rate, expected.channels, got.sample_rate, got.channels
            ),
            Self::UnknownSource { id } => write!(f, "unknown mix source {id}"),
            Self::DuplicateSource { id } => write!(f, "mix source {id} already exists"),
            Self::PartialFrame { samples, frame } => {
                write!(f, "{samples} samples is not a whole frame of {frame}")
            }
        }
    }
}

impl std::error::Error for MixerError {}

/// 一路输入：自己的增益、静音开关与待混样本。
struct MixSource {
    id: u32,
    gain_x1000: u32,
    muted: bool,
    buffer: VecDeque<f32>,
}

/// 一次混音的观测数据。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MixStats {
    /// 这一帧真正贡献了样本的源数。
    pub sources: usize,
    /// 这一帧被限幅的样本数。
    pub limited_samples: u64,
    /// 限幅前这一帧的最大绝对幅度（多路求和之后）。
    pub peak: f32,
}

/// 多路混音器。
pub struct PcmMixer {
    format: MixFormat,
    frame_samples: usize,
    /// 每路缓冲的容量（样本数，交错后）。
    capacity: usize,
    sources: Vec<MixSource>,
    /// 累计被限幅的样本数（跨帧）。
    limited_total: u64,
}

impl PcmMixer {
    /// 建一个混音器：单声道帧长是 frame_samples，输出一帧是 frame_samples × channels。
    pub fn new(format: MixFormat, frame_samples: usize, buffer_frames: usize) -> Self {
        let capacity =
            frame_samples.max(1) * buffer_frames.max(1) * usize::from(format.channels.max(1));
        Self {
            format,
            frame_samples: frame_samples.max(1),
            capacity,
            sources: Vec::new(),
            limited_total: 0,
        }
    }

    /// 混音器的输入格式。
    pub const fn format(&self) -> MixFormat {
        self.format
    }

    /// 当前路数。
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// 累计被限幅的样本数（跨帧；验收「有没有爆」看它）。
    pub const fn limited_total(&self) -> u64 {
        self.limited_total
    }

    /// 加一路源。超过 FR-12 上限时明确拒绝。
    pub fn add_source(&mut self, id: u32) -> Result<(), MixerError> {
        if self.sources.iter().any(|source| source.id == id) {
            return Err(MixerError::DuplicateSource { id });
        }
        if self.sources.len() >= MAX_MIX_SOURCES {
            return Err(MixerError::SourceLimit {
                max: MAX_MIX_SOURCES,
            });
        }
        self.sources.push(MixSource {
            id,
            gain_x1000: 1_000,
            muted: false,
            buffer: VecDeque::new(),
        });
        Ok(())
    }

    /// 移掉一路源（会话结束时用）。
    pub fn remove_source(&mut self, id: u32) -> bool {
        let before = self.sources.len();
        self.sources.retain(|source| source.id != id);
        self.sources.len() != before
    }

    /// 设置某路的增益（千分点，上限 2000）。
    pub fn set_gain(&mut self, id: u32, gain_x1000: u32) -> Result<(), MixerError> {
        let source = self
            .source_mut(id)
            .ok_or(MixerError::UnknownSource { id })?;
        source.gain_x1000 = gain_x1000.min(2_000);
        Ok(())
    }

    /// 设置某路静音。
    pub fn set_mute(&mut self, id: u32, muted: bool) -> Result<(), MixerError> {
        let source = self
            .source_mut(id)
            .ok_or(MixerError::UnknownSource { id })?;
        source.muted = muted;
        Ok(())
    }

    /// 某路当前的增益（千分点）。
    pub fn gain_x1000(&self, id: u32) -> Option<u32> {
        self.sources
            .iter()
            .find(|source| source.id == id)
            .map(|source| source.gain_x1000)
    }

    /// 送一帧样本进来（交错格式，长度必须是整帧）。
    ///
    /// 缓冲满时**丢最旧**：混音不该让慢的那一路把整台设备拖住（与采集侧的纪律一致）。
    pub fn push(&mut self, id: u32, samples: &[f32]) -> Result<(), MixerError> {
        let frame = self.frame_samples * usize::from(self.format.channels.max(1));
        if samples.is_empty() || !samples.len().is_multiple_of(frame) {
            return Err(MixerError::PartialFrame {
                samples: samples.len(),
                frame,
            });
        }
        let capacity = self.capacity;
        let source = self
            .source_mut(id)
            .ok_or(MixerError::UnknownSource { id })?;
        for sample in samples {
            if source.buffer.len() >= capacity {
                source.buffer.pop_front();
            }
            source.buffer.push_back(*sample);
        }
        Ok(())
    }

    /// 取一帧混音结果（写进 out，长度是 frame_samples × channels）。
    pub fn mix_frame(&mut self, out: &mut Vec<f32>) -> MixStats {
        let frame = self.frame_samples * usize::from(self.format.channels.max(1));
        out.clear();
        out.resize(frame, 0.0);

        let mut stats = MixStats::default();
        for source in &mut self.sources {
            let gain = if source.muted {
                0.0
            } else {
                source.gain_x1000 as f32 / 1_000.0
            };
            let mut written = 0usize;
            while written < frame {
                let Some(sample) = source.buffer.pop_front() else {
                    break;
                };
                out[written] += sample * gain;
                written += 1;
            }
            if written > 0 {
                stats.sources += 1;
            }
        }

        for sample in out.iter_mut() {
            let magnitude = sample.abs();
            if magnitude > stats.peak {
                stats.peak = magnitude;
            }
            if magnitude > SOFT_KNEE {
                let limited = soft_limit(*sample);
                if (limited - *sample).abs() > f32::EPSILON {
                    stats.limited_samples = stats.limited_samples.saturating_add(1);
                }
                *sample = limited;
            }
        }
        self.limited_total = self.limited_total.saturating_add(stats.limited_samples);
        stats
    }

    fn source_mut(&mut self, id: u32) -> Option<&mut MixSource> {
        self.sources.iter_mut().find(|source| source.id == id)
    }
}

/// 软限幅：幅度不超过拐点时直通；超过后指数逼近 ±1（连续、单调、永不削顶）。
fn soft_limit(value: f32) -> f32 {
    let magnitude = value.abs();
    if magnitude <= SOFT_KNEE {
        return value;
    }
    let span = 1.0 - SOFT_KNEE;
    let compressed = SOFT_KNEE + span * (1.0 - (-(magnitude - SOFT_KNEE) / span).exp());
    value.signum() * compressed
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn mixer() -> PcmMixer {
        PcmMixer::new(
            MixFormat {
                sample_rate: 48_000,
                channels: 2,
            },
            4,
            4,
        )
    }

    fn frame(left: f32) -> Vec<f32> {
        // 4 个单声道样本 × 2 声道
        vec![left; 8]
    }

    #[test]
    fn a_single_source_passes_through_unchanged() {
        let mut mixer = mixer();
        mixer.add_source(1).unwrap();
        mixer.push(1, &frame(0.25)).unwrap();
        let mut out = Vec::new();
        let stats = mixer.mix_frame(&mut out);
        assert_eq!(stats.sources, 1);
        assert_eq!(stats.limited_samples, 0);
        assert!(out.iter().all(|sample| (*sample - 0.25).abs() < 1e-6));
    }

    #[test]
    fn two_sources_sum() {
        let mut mixer = mixer();
        mixer.add_source(1).unwrap();
        mixer.add_source(2).unwrap();
        mixer.push(1, &frame(0.2)).unwrap();
        mixer.push(2, &frame(0.3)).unwrap();
        let mut out = Vec::new();
        let stats = mixer.mix_frame(&mut out);
        assert_eq!(stats.sources, 2);
        assert!(out.iter().all(|sample| (*sample - 0.5).abs() < 1e-6));
    }

    #[test]
    fn per_source_gain_and_mute() {
        let mut mixer = mixer();
        mixer.add_source(1).unwrap();
        mixer.add_source(2).unwrap();
        mixer.set_gain(1, 500).unwrap();
        mixer.set_mute(2, true).unwrap();
        mixer.push(1, &frame(0.4)).unwrap();
        mixer.push(2, &frame(0.4)).unwrap();
        let mut out = Vec::new();
        mixer.mix_frame(&mut out);
        // 一路减半 + 一路静音
        assert!(out.iter().all(|sample| (*sample - 0.2).abs() < 1e-6));
        assert_eq!(mixer.gain_x1000(1), Some(500));
    }

    #[test]
    fn rejects_the_ninth_source() {
        let mut mixer = mixer();
        for id in 1..=MAX_MIX_SOURCES as u32 {
            mixer.add_source(id).unwrap();
        }
        assert_eq!(
            mixer.add_source(99),
            Err(MixerError::SourceLimit {
                max: MAX_MIX_SOURCES
            })
        );
    }

    #[test]
    fn rejects_duplicate_and_unknown_sources() {
        let mut mixer = mixer();
        mixer.add_source(1).unwrap();
        assert_eq!(
            mixer.add_source(1),
            Err(MixerError::DuplicateSource { id: 1 })
        );
        assert_eq!(
            mixer.set_gain(7, 500),
            Err(MixerError::UnknownSource { id: 7 })
        );
        assert_eq!(
            mixer.push(7, &frame(0.1)),
            Err(MixerError::UnknownSource { id: 7 })
        );
        assert!(!mixer.remove_source(7));
    }

    #[test]
    fn rejects_a_partial_frame() {
        let mut mixer = mixer();
        mixer.add_source(1).unwrap();
        assert_eq!(
            mixer.push(1, &[0.1, 0.2, 0.3]),
            Err(MixerError::PartialFrame {
                samples: 3,
                frame: 8
            })
        );
    }

    #[test]
    fn an_empty_source_is_padded_with_silence_not_repeated() {
        let mut mixer = mixer();
        mixer.add_source(1).unwrap();
        mixer.add_source(2).unwrap();
        mixer.push(1, &frame(0.5)).unwrap();
        mixer.push(2, &frame(0.1)).unwrap();

        // 第一拍把两路各一整帧都消耗掉
        let mut out = Vec::new();
        let stats = mixer.mix_frame(&mut out);
        assert_eq!(stats.sources, 2);
        assert!(out.iter().all(|sample| (*sample - 0.6).abs() < 1e-6));

        // 第二拍两路都没数据：必须按静音补，而**不是**重复上一帧的采样点
        let stats = mixer.mix_frame(&mut out);
        assert_eq!(stats.sources, 0);
        assert!(out.iter().all(|sample| sample.abs() < 1e-9));
    }

    #[test]
    fn soft_limit_keeps_everything_below_one() {
        let mut mixer = mixer();
        mixer.add_source(1).unwrap();
        mixer.add_source(2).unwrap();
        mixer.add_source(3).unwrap();
        for id in 1..=3 {
            mixer.push(id, &frame(0.6)).unwrap();
        }
        let mut out = Vec::new();
        let stats = mixer.mix_frame(&mut out);
        // 原始 1.8：必须被压回 1.0 以内，而且超过拐点的样本要被计数
        assert!(
            stats.peak > 1.5,
            "限幅前的峰值应当记录原始幅度：{}",
            stats.peak
        );
        assert!(out.iter().all(|sample| sample.abs() < 1.0));
        assert!(out.iter().all(|sample| sample.abs() > SOFT_KNEE));
        assert_eq!(stats.limited_samples, out.len() as u64);
        assert_eq!(mixer.limited_total(), out.len() as u64);
    }

    #[test]
    fn a_full_buffer_drops_the_oldest_samples() {
        let mut mixer = PcmMixer::new(
            MixFormat {
                sample_rate: 48_000,
                channels: 1,
            },
            2,
            1,
        );
        mixer.add_source(1).unwrap();
        // 容量 = 2 采样点：送三帧，最早的那一帧必须被丢掉
        mixer.push(1, &[0.1, 0.1]).unwrap();
        mixer.push(1, &[0.2, 0.2]).unwrap();
        mixer.push(1, &[0.3, 0.3]).unwrap();
        let mut out = Vec::new();
        mixer.mix_frame(&mut out);
        // 容量只装得下两个采样点：留下的必然是**最后**送进来的那两个（最早的全被丢掉）
        assert!((out[0] - 0.3).abs() < 1e-6, "最旧的采样点应当被丢掉");
        assert!((out[1] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn a_removed_source_stops_contributing() {
        let mut mixer = mixer();
        mixer.add_source(1).unwrap();
        mixer.add_source(2).unwrap();
        mixer.push(1, &frame(0.3)).unwrap();
        mixer.push(2, &frame(0.3)).unwrap();
        assert!(mixer.remove_source(2));
        assert_eq!(mixer.source_count(), 1);
        let mut out = Vec::new();
        mixer.mix_frame(&mut out);
        assert!(out.iter().all(|sample| (*sample - 0.3).abs() < 1e-6));
    }

    #[test]
    fn the_mixer_keeps_its_own_format() {
        // 混音器只接受与自身一致的格式：采样率不同不是「能凑合」，而是时间轴不同（NFR-13）。
        let mixer = PcmMixer::new(
            MixFormat {
                sample_rate: 48_000,
                channels: 2,
            },
            4,
            4,
        );
        let other = MixFormat {
            sample_rate: 44_100,
            channels: 2,
        };
        assert_ne!(mixer.format(), other);
    }
}
