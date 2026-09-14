//! 统一内部音频格式与设备样本转换
//!
//! 依据 `docs/02-architecture.md` §3：**统一内部格式 = 48 000 Hz / f32 / 2ch（交错）**。
//! 任何采集源（44.1 k / 16 bit / 单声道 / 24 bit …）进入内核前都必须转成该格式；
//! 编码器、抖动缓冲、混音器只处理这一种格式 —— 这样混音与时钟同步只需一套算术。
//!
//! # 采样率策略（M1 明确取舍）
//!
//! WASAPI 共享模式若打开 `autoconvert`，驱动会**静默重采样**（实测：请求 44.1 kHz 也会「成功」，
//! 但拿到的字节已经是 SRC 之后的样本，内核按原采样率解读就会错）。所以「零重采样」不是靠内核
//! 强制 48 k 得到的，而是靠**把 Windows 播放设备设为 48 kHz**（`docs/05-roadmap.md` M1 验收：
//! 「遥测显示采集/内核/播放采样率均为 48000」）。遇到 44.1 k 设备时不静默重采样，
//! 而是返回 [`AudioError::InvalidConfig`] 并让调用方给出可操作提示 —— 静默 SRC 会让
//! 「零重采样」这条验收失去意义，也会引入不可控的延迟与音质损失。

use crate::error::AudioError;

/// 内部统一采样率（Hz）。
pub const SAMPLE_RATE_HZ: u32 = 48_000;

/// 内部统一声道数（交错）。
pub const CHANNELS: u16 = 2;

/// 每毫秒的每声道样本数（48 000 / 1000）。
pub const SAMPLES_PER_MS: usize = 48;

/// 默认帧长（ms）：Opus 基线 20 ms（`docs/04-tech-stack.md` ADR-003）。
pub const DEFAULT_FRAME_MS: u32 = 20;

/// 默认帧长下每声道样本数（960 = 20 × 48）。
pub const DEFAULT_FRAME_SAMPLES: usize = 960;

/// 默认帧长下的交错样本数（1 920 = 960 × 2）。
pub const DEFAULT_FRAME_INTERLEAVED: usize = DEFAULT_FRAME_SAMPLES * 2;

/// 协议允许的帧长（`03-protocol.md` §10 `CodecStats.frame_ms`：10 / 20 / 40 / 60）。
pub const FRAME_MS_OPTIONS: [u32; 4] = [10, 20, 40, 60];

/// 帧长是否受支持。
pub const fn is_supported_frame_ms(frame_ms: u32) -> bool {
    matches!(frame_ms, 10 | 20 | 40 | 60)
}

/// 给定帧长下「每声道」的样本数（内部 48 kHz 口径）。
pub const fn frame_samples_per_channel(frame_ms: u32) -> usize {
    frame_ms as usize * SAMPLES_PER_MS
}

/// 给定帧长下的交错样本数（= 每声道样本数 × 2）。
pub const fn frame_interleaved(frame_ms: u32) -> usize {
    frame_samples_per_channel(frame_ms) * CHANNELS as usize
}

/// 设备样本在内存中的存储格式（WASAPI 混音格式的 `wBitsPerSample` / `SubFormat`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    /// 无符号 8 位（128 为中心）。
    U8,
    /// 有符号 16 位小端。
    I16,
    /// 有符号 24 位小端（**紧凑 3 字节**；24-in-32 用 [`SampleFormat::I32`] + `valid_bits = 24`）。
    I24,
    /// 有符号 32 位小端（含「24 位有效位左对齐装在 4 字节容器」）。
    I32,
    /// IEEE-754 单精度小端。
    F32,
}

impl SampleFormat {
    /// 每个样本占用的字节数。
    pub const fn bytes_per_sample(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::I16 => 2,
            Self::I24 => 3,
            Self::I32 | Self::F32 => 4,
        }
    }
}

/// 设备（或任意外部来源）的音频格式描述。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceFormat {
    /// 采样率（Hz）。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: u16,
    /// 样本存储格式。
    pub sample_format: SampleFormat,
    /// 容器内有效位（`WAVEFORMATEXTENSIBLE.wValidBitsPerSample`；0 = 等于容器位宽）。
    ///
    /// 仅用于**校验与展示**：左对齐装在更宽容器里的情况（如 24-in-32）按容器满量程折算即可，
    /// 无需额外缩放。
    pub valid_bits: u16,
}

impl DeviceFormat {
    /// 构造（`valid_bits` 取容器位宽）。
    pub const fn new(sample_rate: u32, channels: u16, sample_format: SampleFormat) -> Self {
        Self {
            sample_rate,
            channels,
            sample_format,
            valid_bits: 0,
        }
    }

    /// 每个样本的字节数。
    pub const fn bytes_per_sample(&self) -> usize {
        self.sample_format.bytes_per_sample()
    }

    /// 每帧（全声道）的字节数。
    pub const fn bytes_per_frame(&self) -> usize {
        self.bytes_per_sample() * self.channels as usize
    }

    /// 字节数换算成帧数（向下取整；不足一帧的尾部字节被忽略；块对齐为 0 时返回 0）。
    pub fn frames_in(&self, bytes: usize) -> usize {
        bytes.checked_div(self.bytes_per_frame()).unwrap_or(0)
    }

    /// 是否已经是统一内部格式（48 kHz / 2ch / f32）。
    pub const fn is_unified(&self) -> bool {
        self.sample_rate == SAMPLE_RATE_HZ
            && self.channels == CHANNELS
            && matches!(self.sample_format, SampleFormat::F32)
    }
}

/// 校验采样率必须是内部统一采样率（不做静默重采样，见模块文档）。
pub fn require_unified_sample_rate(sample_rate: u32) -> Result<(), AudioError> {
    if sample_rate == SAMPLE_RATE_HZ {
        Ok(())
    } else {
        Err(AudioError::invalid_config_owned(format!(
            "设备采样率 {sample_rate} Hz ≠ 内核统一 {SAMPLE_RATE_HZ} Hz；请在系统声音设置里把设备设为 48 kHz（本项目不做静默重采样）"
        )))
    }
}

/// 从字节缓冲读一个样本并折算到 `[-1.0, 1.0]`（非有限值 → 0.0，实时路径不接受 NaN/Inf）。
#[inline]
fn read_sample(bytes: &[u8], format: SampleFormat) -> Option<f32> {
    let value = match format {
        SampleFormat::U8 => (f32::from(*bytes.first()?) - 128.0) / 128.0,
        SampleFormat::I16 => {
            let raw: [u8; 2] = bytes.get(0..2)?.try_into().ok()?;
            f32::from(i16::from_le_bytes(raw)) / 32_768.0
        }
        SampleFormat::I24 => {
            let raw: [u8; 3] = bytes.get(0..3)?.try_into().ok()?;
            // 小端 24 位 → 符号扩展成 i32
            let signed = (i32::from_le_bytes([raw[0], raw[1], raw[2], 0]) << 8) >> 8;
            signed as f32 / 8_388_608.0
        }
        SampleFormat::I32 => {
            let raw: [u8; 4] = bytes.get(0..4)?.try_into().ok()?;
            i32::from_le_bytes(raw) as f32 / 2_147_483_648.0
        }
        SampleFormat::F32 => {
            let raw: [u8; 4] = bytes.get(0..4)?.try_into().ok()?;
            f32::from_le_bytes(raw)
        }
    };
    Some(if value.is_finite() { value } else { 0.0 })
}

/// 把一个 `[-1.0, 1.0]` 的样本写成设备字节（写满 `bytes_per_sample` 个字节，返回是否成功）。
#[inline]
fn write_sample(value: f32, format: SampleFormat, out: &mut [u8]) -> bool {
    let clamped = if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    match format {
        SampleFormat::U8 => match out.first_mut() {
            Some(slot) => {
                *slot = (clamped * 127.0 + 128.0) as u8;
                true
            }
            None => false,
        },
        SampleFormat::I16 => match out.get_mut(0..2) {
            Some([a, b]) => {
                let raw = ((clamped * 32_767.0).round() as i16).to_le_bytes();
                *a = raw[0];
                *b = raw[1];
                true
            }
            _ => false,
        },
        SampleFormat::I24 => match out.get_mut(0..3) {
            Some([a, b, c]) => {
                let raw = ((clamped * 8_388_607.0).round() as i32).to_le_bytes();
                *a = raw[0];
                *b = raw[1];
                *c = raw[2];
                true
            }
            _ => false,
        },
        SampleFormat::I32 => match out.get_mut(0..4) {
            Some([a, b, c, d]) => {
                let raw = ((clamped * 2_147_483_647.0).round() as i32).to_le_bytes();
                *a = raw[0];
                *b = raw[1];
                *c = raw[2];
                *d = raw[3];
                true
            }
            _ => false,
        },
        SampleFormat::F32 => match out.get_mut(0..4) {
            Some([a, b, c, d]) => {
                let raw = clamped.to_le_bytes();
                *a = raw[0];
                *b = raw[1];
                *c = raw[2];
                *d = raw[3];
                true
            }
            _ => false,
        },
    }
}

/// 把设备字节缓冲转换成**内部统一格式**（48 kHz / f32 / 2ch 交错）。
///
/// - 位深：按上表折算到 `[-1.0, 1.0]`；
/// - 声道：1 ch → 复制成双声道；2 ch → 直通；> 2 ch → 取**前两个声道**（WAVEFORMATEXTENSIBLE 下即 FL/FR）；
/// - 采样率：不做转换（调用方必须先过 [`require_unified_sample_rate`]，避免静默 SRC）。
///
/// 返回写入 `dst` 的 **f32 个数**（= 帧数 × 2）。`dst` 不足时报 [`AudioError::InvalidConfig`]，不 panic。
pub fn convert_to_internal(
    src: &[u8],
    format: DeviceFormat,
    dst: &mut [f32],
) -> Result<usize, AudioError> {
    let stride = format.bytes_per_frame();
    if stride == 0 {
        return Err(AudioError::invalid_config("设备格式声道数为 0"));
    }
    if format.channels == 0 {
        return Err(AudioError::invalid_config("设备格式声道数为 0"));
    }
    let frames = src.len() / stride;
    let needed = frames * CHANNELS as usize;
    if dst.len() < needed {
        return Err(AudioError::invalid_config_owned(format!(
            "目标缓冲过小：需要 {needed} 个 f32，实际 {}",
            dst.len()
        )));
    }

    let bytes_per_sample = format.bytes_per_sample();
    let mut written = 0usize;
    for (frame_bytes, frame_out) in src
        .chunks_exact(stride)
        .zip(dst.chunks_exact_mut(CHANNELS as usize))
    {
        let left_bytes = frame_bytes.get(0..bytes_per_sample);
        let left = match left_bytes.and_then(|b| read_sample(b, format.sample_format)) {
            Some(value) => value,
            None => return Err(AudioError::invalid_config("样本字节不足")),
        };
        let right = if format.channels == 1 {
            left
        } else {
            let offset = bytes_per_sample;
            let right_bytes = frame_bytes.get(offset..offset + bytes_per_sample);
            match right_bytes.and_then(|b| read_sample(b, format.sample_format)) {
                Some(value) => value,
                None => return Err(AudioError::invalid_config("样本字节不足")),
            }
        };
        if let [l, r] = frame_out {
            *l = left;
            *r = right;
            written += CHANNELS as usize;
        }
    }
    Ok(written)
}

/// 把**内部统一格式**（f32 / 2ch 交错）写回设备字节缓冲。
///
/// - 设备 2 ch：直通；设备 1 ch：取 `(L + R) / 2`；设备 > 2 ch：前两个声道写 L/R，**其余声道补 0**
///   （v1 不做上混矩阵，避免制造虚假环绕内容）；
/// - 采样率不做转换（同 [`convert_to_internal`]）。
///
/// 返回写入的**字节数**。
pub fn convert_from_internal(
    src: &[f32],
    format: DeviceFormat,
    dst: &mut [u8],
) -> Result<usize, AudioError> {
    let stride = format.bytes_per_frame();
    if stride == 0 {
        return Err(AudioError::invalid_config("设备格式声道数为 0"));
    }
    let channels = format.channels as usize;
    let frames = src.len() / CHANNELS as usize;
    let needed = frames * stride;
    if dst.len() < needed {
        return Err(AudioError::invalid_config_owned(format!(
            "目标缓冲过小：需要 {needed} 字节，实际 {}",
            dst.len()
        )));
    }

    let bytes_per_sample = format.bytes_per_sample();
    let mut written = 0usize;
    for (frame_in, frame_out) in src
        .chunks_exact(CHANNELS as usize)
        .zip(dst.chunks_exact_mut(stride))
    {
        let (left, right) = match frame_in {
            [l, r] => (*l, *r),
            _ => return Err(AudioError::invalid_config("内部帧不是双声道交错")),
        };
        for channel in 0..channels {
            let value = match (channels, channel) {
                (1, _) => (left + right) * 0.5,
                (_, 0) => left,
                (_, 1) => right,
                _ => 0.0,
            };
            let offset = channel * bytes_per_sample;
            match frame_out.get_mut(offset..offset + bytes_per_sample) {
                Some(slot) => {
                    if !write_sample(value, format.sample_format, slot) {
                        return Err(AudioError::invalid_config("样本写入失败"));
                    }
                }
                None => return Err(AudioError::invalid_config("样本写入越界")),
            }
        }
        written += stride;
    }
    Ok(written)
}

/// f32 → i16（PCM16 回退路径、测试与夹具用）。
pub fn f32_to_i16(src: &[f32], dst: &mut [i16]) -> usize {
    let mut written = 0usize;
    for (value, slot) in src.iter().zip(dst.iter_mut()) {
        let clamped = if value.is_finite() {
            value.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        *slot = (clamped * 32_767.0).round() as i16;
        written += 1;
    }
    written
}

/// i16 → f32。
pub fn i16_to_f32(src: &[i16], dst: &mut [f32]) -> usize {
    let mut written = 0usize;
    for (value, slot) in src.iter().zip(dst.iter_mut()) {
        *slot = f32::from(*value) / 32_768.0;
        written += 1;
    }
    written
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 帧长换算符合_m1_口径() {
        assert_eq!(frame_samples_per_channel(20), 960);
        assert_eq!(frame_interleaved(20), 1_920);
        assert_eq!(frame_interleaved(10), 960);
        assert_eq!(frame_interleaved(60), 5_760);
        assert!(is_supported_frame_ms(20));
        assert!(!is_supported_frame_ms(15));
    }

    #[test]
    fn f32_立体声直通() {
        let src: Vec<u8> = [0.0f32, 0.5, -0.5, 1.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let format = DeviceFormat::new(48_000, 2, SampleFormat::F32);
        assert!(format.is_unified());
        let mut dst = [0.0f32; 4];
        let written = convert_to_internal(&src, format, &mut dst).unwrap();
        assert_eq!(written, 4);
        assert_eq!(dst, [0.0, 0.5, -0.5, 1.0]);
    }

    #[test]
    fn 单声道复制成双声道() {
        let src: Vec<u8> = [0.25f32, -0.25]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let format = DeviceFormat::new(48_000, 1, SampleFormat::F32);
        let mut dst = [0.0f32; 4];
        assert_eq!(convert_to_internal(&src, format, &mut dst).unwrap(), 4);
        assert_eq!(dst, [0.25, 0.25, -0.25, -0.25]);
    }

    #[test]
    fn 多声道取前两声道() {
        // 4 ch：FL FR BL BR
        let frame: [i16; 4] = [16_384, -16_384, 3_000, 3_000];
        let src: Vec<u8> = frame.iter().flat_map(|v| v.to_le_bytes()).collect();
        let format = DeviceFormat::new(48_000, 4, SampleFormat::I16);
        let mut dst = [0.0f32; 2];
        assert_eq!(convert_to_internal(&src, format, &mut dst).unwrap(), 2);
        assert!((dst[0] - 0.5).abs() < 1e-6, "{dst:?}");
        assert!((dst[1] + 0.5).abs() < 1e-6, "{dst:?}");
    }

    #[test]
    fn i16_与_i24_折算正确() {
        let i16_src = (-32_768i16).to_le_bytes();
        let mut dst = [1.0f32; 2];
        convert_to_internal(
            &i16_src,
            DeviceFormat::new(48_000, 1, SampleFormat::I16),
            &mut dst,
        )
        .unwrap();
        assert_eq!(dst, [-1.0, -1.0]);

        // 24 位紧凑：-8388608 是最小值（-1.0）
        let i24_src = [0x00u8, 0x00, 0x80];
        let mut dst24 = [1.0f32; 2];
        convert_to_internal(
            &i24_src,
            DeviceFormat::new(48_000, 1, SampleFormat::I24),
            &mut dst24,
        )
        .unwrap();
        assert_eq!(dst24, [-1.0, -1.0]);
    }

    #[test]
    fn 非有限值被抑制为静音() {
        // 单声道 2 帧 → 转成双声道后是 4 个 f32
        let src: Vec<u8> = [f32::NAN, f32::INFINITY]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let mut dst = [1.0f32; 4];
        convert_to_internal(
            &src,
            DeviceFormat::new(48_000, 1, SampleFormat::F32),
            &mut dst,
        )
        .unwrap();
        assert_eq!(dst, [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn 目标缓冲过小返回错误而不_panic() {
        let format = DeviceFormat::new(48_000, 2, SampleFormat::F32);
        let mut dst = [0.0f32; 2];
        let err = convert_to_internal(&[0u8; 32], format, &mut dst).unwrap_err();
        assert_eq!(err.code(), audiolink_types::ErrorCode::BadRequest);
    }

    #[test]
    fn 反向转换_立体声与单声道设备() {
        let src = [1.0f32, 0.0, 0.0, 1.0];

        let mut stereo = [0u8; 16];
        let written = convert_from_internal(
            &src,
            DeviceFormat::new(48_000, 2, SampleFormat::F32),
            &mut stereo,
        )
        .unwrap();
        assert_eq!(written, 16);
        let mut back = [0.0f32; 4];
        convert_to_internal(
            &stereo,
            DeviceFormat::new(48_000, 2, SampleFormat::F32),
            &mut back,
        )
        .unwrap();
        assert_eq!(back, src);

        // 单声道设备：L/R 取平均 → 两帧都是 0.5
        let mut mono = [0u8; 8];
        convert_from_internal(
            &src,
            DeviceFormat::new(48_000, 1, SampleFormat::F32),
            &mut mono,
        )
        .unwrap();
        let mut back_mono = [0.0f32; 4];
        convert_to_internal(
            &mono,
            DeviceFormat::new(48_000, 1, SampleFormat::F32),
            &mut back_mono,
        )
        .unwrap();
        assert_eq!(back_mono, [0.5, 0.5, 0.5, 0.5]);
    }

    #[test]
    fn 多声道设备额外声道补零() {
        let src = [0.5f32, -0.5];
        let mut dst = [0xffu8; 16];
        let written = convert_from_internal(
            &src,
            DeviceFormat::new(48_000, 4, SampleFormat::F32),
            &mut dst,
        )
        .unwrap();
        assert_eq!(written, 16);
        let mut back = [0.0f32; 8];
        let samples = convert_to_internal(
            &dst,
            DeviceFormat::new(48_000, 4, SampleFormat::F32),
            &mut back,
        )
        .unwrap();
        assert_eq!(samples, 2);
        assert_eq!(&back[..2], &[0.5, -0.5]);
    }

    #[test]
    fn 采样率非_48k_被显式拒绝() {
        assert!(require_unified_sample_rate(48_000).is_ok());
        let err = require_unified_sample_rate(44_100).unwrap_err();
        assert!(err.context().contains("44100"), "{}", err.context());
    }
}
