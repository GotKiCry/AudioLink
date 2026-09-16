//! Opus 编解码封装（ADR-003：`opus-rs` 纯 Rust，零 C 依赖）
//!
//! # 参数基线（`docs/04-tech-stack.md` ADR-003）
//!
//! 48 kHz / 立体声 / 20 ms 帧 / 160 kbps VBR / complexity 7 / `RESTRICTED_LOWDELAY`；
//! 音乐场景**关闭 in-band FEC**（主防线是冗余双发 + **自建丢包掩盖**，见下）。
//!
//! # 与 `opus-rs` 的边界（为什么要有这一层）
//!
//! `opus-rs` 0.1.33 的公开面很窄：`OpusEncoder::{new, encode, enable_hybrid_mode}`、
//! `OpusDecoder::{new, decode}` 加若干 `pub` 控制字段。本 crate 把「帧长校验、缓冲复用、PLC 触发、
//! 遥测计数、错误收敛」都收在这里，让上层（engine / tools）只看见 [`OpusEncoder`] / [`OpusDecoder`]。
//!
//! # 源码级核实 + 实测事实（2026-09-14，i7-10700 / release）
//!
//! 每条都有 `target/notes/opus-rs-0.1.33-api.md` 的原始输出支撑；**改本文件前先读那张表**。
//!
//! | 事实 | 数值 / 结论 | 影响 |
//! |---|---|---|
//! | 48 kHz 合法帧长 | **只有 240 / 480 / 960 样本**（5/10/20 ms）；2.5 ms（120）会**直接 panic**（`celt.rs:2252` 的 assert 写错） | 必须白名单校验帧长，见 [`CodecConfig::validate`] |
//! | `encode` 输入 | f32 **交错**，长度必须 ≥ `frame_size × channels`；返回**含 1 B ToC** 的包长 | 已封装，调用方不用管 |
//! | `decode` 返回值 | **每声道**样本数（960），不是交错样本数（1920） | 本封装统一换算成**交错样本数**返回 |
//! | `decode` 鲁棒性 | 20000 个畸形包 → Err 17131 / **panic 0** | 可安全放在网络入站路径 |
//! | 编解码耗时 | encode P50 **75 μs**、decode P50 **57 μs**（占 20 ms 帧预算 0.7%，≈151× 实时） | 单核足够撑 8 路 |
//! | 热路径分配 | 连续 200 帧 encode+decode = **0 次分配** | 满足实时纪律 |
//! | 实际码率 | 160 kbps 目标 → **160.4 kbps**（401 B/帧） | 远低于数据报载荷上限 1176 B |
//! | 管线延迟 | `RESTRICTED_LOWDELAY` **120 样本 = 2.5 ms**；`Audio` **312 样本 = 6.5 ms**（= 192 lookahead + 120 CELT overlap） | 见 [`CodecConfig::nominal_pipeline_delay_us`] |
//! | `packet_loss_perc` / `use_inband_fec` | **在 CELT-only 帧里是空设置**（逐字节比对无变化；只在 SILK/Hybrid 帧生效） | ADR-004 的「必须显式设置」在 48k/160k 下不成立，M2 不能依赖它 |
//! | `enable_hybrid_mode()` | 死 API（只写一个从不被读的字段） | 不要用 |
//! | 低码率 Hybrid | 目标 < 16 kbps 会严重超码率（目标 6 kbps → 实测 42 kbps） | 自适应码率下限取 16 kbps |
//!
//! # PLC 的确切触发方式与**已知缺陷**
//!
//! 触发方式（源码级核实）：`opus-rs` 把 **1 字节报文**视为「丢帧」，走上一帧模式的隐藏重建
//! （`lib.rs` 1177–1180：「A packet of 0 or 1 bytes (ToC only) is a lost/DTX frame … using the
//! previous mode's concealment」）。因此本封装记住**上一次成功解码报文的 ToC 字节**，
//! 丢包时用 `&[last_toc]` 触发 —— 这样模式 / 带宽 / 声道都与该流一致。
//!
//! **但 CELT-only（M1 的默认档）没有真正的 PLC**（实测）：第 1 个丢失帧只剩 MDCT 重叠余响
//! （rms 0.08，与真实帧互相关 −0.15），**第 2 帧起是硬静音**；接回真实包时还有 −45 dB 瞬态凹陷。
//! ⇒「丢包靠 PLC 兜底」在 CELT-only 下不成立，M2 必须自建掩盖（重复上一包 + 淡出 + 交叉淡化）。
//! [`OpusDecoder::concealment_is_real`] 返回 `false` 就是在提醒上层：别把它当主防线。

use opus_rs::{Application, OpusDecoder as RawDecoder, OpusEncoder as RawEncoder};

use crate::error::AudioError;
use crate::format::{CHANNELS, SAMPLE_RATE_HZ, is_supported_frame_ms};

/// `RESTRICTED_LOWDELAY` 下 CELT-only 的帧长上限（ms）：CELT 单帧最长 20 ms @ 48 kHz。
pub const CELT_MAX_FRAME_MS: u32 = 20;

/// 编码参数档位（对应协议 §8 的自适应码率表；M2 会在这些档位间切换）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodecConfig {
    /// 帧长（ms）：10 / 20（`RESTRICTED_LOWDELAY` 下不允许 40/60）。
    pub frame_ms: u32,
    /// 目标码率（bps）。
    pub bitrate_bps: i32,
    /// 复杂度 0..=10。
    pub complexity: i32,
    /// 应用模式。
    pub application: Application,
    /// 是否 CBR（默认 VBR）。
    pub use_cbr: bool,
    /// 是否开 in-band FEC（音乐场景默认关闭，ADR-004）。
    pub use_inband_fec: bool,
    /// 期望丢包率（%）：**必须显式设置**，否则编码器内部的抗丢包策略形同虚设（ADR-004）。
    pub packet_loss_perc: i32,
}

impl CodecConfig {
    /// M1 基线：20 ms / 160 kbps / complexity 7 / `RESTRICTED_LOWDELAY` / VBR。
    pub const fn m1_default() -> Self {
        Self {
            frame_ms: 20,
            bitrate_bps: 160_000,
            complexity: 7,
            application: Application::RestrictedLowDelay,
            use_cbr: false,
            use_inband_fec: false,
            packet_loss_perc: 0,
        }
    }

    /// 10 ms 档（包率翻倍，弱网下的备选；`docs/02-architecture.md` §5）。
    pub const fn m1_low_delay_tight() -> Self {
        Self {
            frame_ms: 10,
            ..Self::m1_default()
        }
    }

    /// 每声道帧样本数。
    pub const fn frame_samples(&self) -> usize {
        crate::format::frame_samples_per_channel(self.frame_ms)
    }

    /// 一帧的交错样本数（编码器输入长度）。
    pub const fn interleaved_frame(&self) -> usize {
        crate::format::frame_interleaved(self.frame_ms)
    }

    /// 校验（帧长 / 码率 / 复杂度 / 应用模式与帧长的相容性）。
    pub fn validate(&self) -> Result<(), AudioError> {
        if !is_supported_frame_ms(self.frame_ms) {
            return Err(AudioError::invalid_config_owned(format!(
                "不支持的帧长 {} ms（只允许 10/20/40/60）",
                self.frame_ms
            )));
        }
        if self.application == Application::RestrictedLowDelay && self.frame_ms > CELT_MAX_FRAME_MS
        {
            return Err(AudioError::invalid_config_owned(format!(
                "RESTRICTED_LOWDELAY 是 CELT-only 模式，帧长上限 {} ms（当前 {} ms）；需要 40/60 ms 请改用 Application::Audio",
                CELT_MAX_FRAME_MS, self.frame_ms
            )));
        }
        if !(500..=3_000_000).contains(&self.bitrate_bps) {
            return Err(AudioError::invalid_config_owned(format!(
                "码率 {} bps 超出 Opus 允许范围 500..=3_000_000",
                self.bitrate_bps
            )));
        }
        if !(0..=10).contains(&self.complexity) {
            return Err(AudioError::invalid_config_owned(format!(
                "复杂度 {} 超出 0..=10",
                self.complexity
            )));
        }
        if !(0..=100).contains(&self.packet_loss_perc) {
            return Err(AudioError::invalid_config_owned(format!(
                "期望丢包率 {}% 超出 0..=100",
                self.packet_loss_perc
            )));
        }
        Ok(())
    }

    /// 遥测快照（协议 §10 的 `CodecStats`）。
    pub fn telemetry(&self) -> audiolink_types::CodecStats {
        audiolink_types::CodecStats {
            frame_ms: self.frame_ms as u8,
            channels: CHANNELS as u8,
            complexity: self.complexity as u8,
        }
    }

    /// 编解码**管线延迟**的标称值（μs）——用于延迟分解里的「编码段」。
    ///
    /// 口径来自实测互相关（`target/notes/opus-rs-0.1.33-api.md`，300 帧噪声，峰值 ±1 样本内可信）：
    ///
    /// - `RESTRICTED_LOWDELAY`：**120 样本 = 2 500 μs**。其中编码器 lookahead
    ///   （`delay_compensation`）为 **0**（源码 `lib.rs` 442–446 硬编码），这 120 样本全部来自
    ///   **解码端** CELT 的 MDCT 窗重叠；
    /// - `Audio`：**312 样本 = 6 500 μs**（= 192 样本 lookahead + 120 样本重叠）。
    ///
    /// **这是标称值而非实时测量值**：`opus-rs` 没有任何公开 delay/lookahead getter，
    /// 所以只能按上表硬编码；真实链路值由 `tools/self-loop` 的标记往返测量给出，
    /// 两者在报告里分行呈现，不混在一起。
    pub const fn nominal_pipeline_delay_us(&self) -> u32 {
        match self.application {
            Application::RestrictedLowDelay => 2_500,
            _ => 6_500,
        }
    }
}

impl Default for CodecConfig {
    fn default() -> Self {
        Self::m1_default()
    }
}

/// Opus 编码器（编码线程独占）。
pub struct OpusEncoder {
    inner: RawEncoder,
    config: CodecConfig,
    encoded_bytes: u64,
    frames_encoded: u64,
}

impl OpusEncoder {
    /// 建编码器（参数经 [`CodecConfig::validate`] 校验）。
    pub fn new(config: CodecConfig) -> Result<Self, AudioError> {
        config.validate()?;
        let mut inner =
            RawEncoder::new(SAMPLE_RATE_HZ as i32, CHANNELS as usize, config.application).map_err(
                |message| AudioError::codec_owned(format!("Opus 编码器初始化失败：{message}")),
            )?;
        inner.bitrate_bps = config.bitrate_bps;
        inner.complexity = config.complexity;
        inner.use_cbr = config.use_cbr;
        inner.use_inband_fec = config.use_inband_fec;
        inner.packet_loss_perc = config.packet_loss_perc;
        Ok(Self {
            inner,
            config,
            encoded_bytes: 0,
            frames_encoded: 0,
        })
    }

    /// 参数快照。
    pub fn config(&self) -> CodecConfig {
        self.config
    }

    /// 编码一帧（`frame` 为交错 f32，长度必须 ≥ [`CodecConfig::interleaved_frame`]）。
    ///
    /// 返回写入 `out` 的字节数。建议 `out` 长度 ≥ `frame_ms × 码率`（20 ms/160 kbps ≈ 400 B），
    /// 协议层上限见 [`crate::proto_budget`]。
    pub fn encode_into(&mut self, frame: &[f32], out: &mut [u8]) -> Result<usize, AudioError> {
        let needed = self.config.interleaved_frame();
        if frame.len() < needed {
            return Err(AudioError::invalid_config_owned(format!(
                "编码输入不足一帧：需要 {needed} 个交错样本，实际 {}",
                frame.len()
            )));
        }
        let written = self
            .inner
            .encode(frame, self.config.frame_samples(), out)
            .map_err(|message| AudioError::codec_owned(format!("Opus 编码失败：{message}")))?;
        self.encoded_bytes += written as u64;
        self.frames_encoded += 1;
        Ok(written)
    }

    /// 累计编码帧数。
    pub fn frames_encoded(&self) -> u64 {
        self.frames_encoded
    }

    /// 累计编码字节数（用于实测码率）。
    pub fn encoded_bytes(&self) -> u64 {
        self.encoded_bytes
    }

    /// 实测平均码率（bps）；没有帧时返回 0。
    pub fn measured_bitrate_bps(&self) -> u32 {
        if self.frames_encoded == 0 {
            return 0;
        }
        let bits = self.encoded_bytes * 8;
        let millis = self.frames_encoded * u64::from(self.config.frame_ms);
        if millis == 0 {
            return 0;
        }
        (bits * 1_000 / millis) as u32
    }

    /// 编解码管线延迟标称值（μs）。
    pub fn nominal_pipeline_delay_us(&self) -> u32 {
        self.config.nominal_pipeline_delay_us()
    }
}

/// Opus 解码器（播放线程独占）。
pub struct OpusDecoder {
    inner: RawDecoder,
    config: CodecConfig,
    /// 上一次成功解码报文的 ToC 字节 —— PLC 时用它构造 1 字节「丢帧」报文。
    last_toc: Option<u8>,
    plc_frames: u64,
    frames_decoded: u64,
}

impl OpusDecoder {
    /// 建解码器。
    pub fn new(config: CodecConfig) -> Result<Self, AudioError> {
        config.validate()?;
        let inner =
            RawDecoder::new(SAMPLE_RATE_HZ as i32, CHANNELS as usize).map_err(|message| {
                AudioError::codec_owned(format!("Opus 解码器初始化失败：{message}"))
            })?;
        Ok(Self {
            inner,
            config,
            last_toc: None,
            plc_frames: 0,
            frames_decoded: 0,
        })
    }

    /// 参数快照。
    pub fn config(&self) -> CodecConfig {
        self.config
    }

    /// 解码一个报文到 `out`（交错 f32；长度必须 ≥ 该报文的实际样本数 × 2）。
    ///
    /// 返回写入的**交错样本数**。
    pub fn decode_into(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, AudioError> {
        let first = packet
            .first()
            .copied()
            .ok_or_else(|| AudioError::codec("空报文无法解码（丢包请走 conceal_into）"))?;
        let written = self
            .inner
            .decode(packet, self.config.frame_samples(), out)
            .map_err(|message| AudioError::codec_owned(format!("Opus 解码失败：{message}")))?;
        self.last_toc = Some(first);
        self.frames_decoded += 1;
        Ok(written * CHANNELS as usize)
    }

    /// 丢包隐藏：按上一帧的模式重建一帧（**不 panic**，但见下面的警告）。
    ///
    /// # ⚠ 在 CELT-only 档（M1 默认）下这不是真正的 PLC —— 实测结论
    ///
    /// 第一帧丢失只剩 MDCT 重叠余响（rms ≈ 0.08，与真实帧互相关 −0.15），**第二帧起是硬静音**；
    /// 接回真实包时还有约 −45 dB 的瞬态凹陷。也就是说本方法能保证「不 panic、不断流、时间轴正确」，
    /// **不能保证可听性**。M2 的弱网验收必须自建掩盖（重复上一包 + 淡出 + 交叉淡化），
    /// 见 [`OpusDecoder::concealment_is_real`] 与 `docs/04-tech-stack.md` ADR-003 注记。
    ///
    /// 没有任何历史报文时返回 [`AudioError::Codec`]（调用方应改为输出静音并计数）。
    pub fn conceal_into(&mut self, out: &mut [f32]) -> Result<usize, AudioError> {
        let toc = self
            .last_toc
            .ok_or_else(|| AudioError::codec("尚无历史报文，无法做丢包隐藏"))?;
        let packet = [toc];
        let written = self
            .inner
            .decode(&packet, self.config.frame_samples(), out)
            .map_err(|message| AudioError::codec_owned(format!("Opus 丢包隐藏失败：{message}")))?;
        self.plc_frames += 1;
        Ok(written * CHANNELS as usize)
    }

    /// 当前档位下 [`OpusDecoder::conceal_into`] 是否能被证明提供**真实**的丢包掩盖。
    ///
    /// `opus-rs` 不公开每包最终选择的 SILK / Hybrid / CELT 模式；`Application::Audio` 在本项目
    /// 160 kbps 档同样会走 CELT，不能仅凭 application 猜测。为了不让一次错误猜测污染恢复帧，
    /// 当前所有支持档位都保守返回 `false`，上层使用自建 PCM 掩盖。将来只有拿到可靠模式 getter
    /// 或逐包 ToC 判定后，才可对已证明的 SILK / Hybrid 路径返回 `true`。
    pub const fn concealment_is_real(&self) -> bool {
        false
    }

    /// 累计丢包隐藏帧数（遥测 §10 的 `plc_count`）。
    pub fn plc_count(&self) -> u64 {
        self.plc_frames
    }

    /// 累计成功解码帧数。
    pub fn frames_decoded(&self) -> u64 {
        self.frames_decoded
    }

    /// 复位（重建流时用）：丢弃历史，PLC 计数保留在外部账本里。
    pub fn reset(&mut self) {
        self.last_toc = None;
    }
}

/// 协议层给 Opus 载荷留下的预算（`docs/03-protocol.md` §3 载荷表）。
pub mod proto_budget {
    use audiolink_types::{DATAGRAM_HEADER_LEN, DATAGRAM_MAX_LEN};

    /// 单个数据报的载荷上限（默认 MTU 约束下 1176 B）。
    pub const MAX_OPUS_PAYLOAD: usize = DATAGRAM_MAX_LEN - DATAGRAM_HEADER_LEN;

    /// 编码输出缓冲的建议长度（20 ms / 160 kbps ≈ 400 B，留足余量后仍远小于上限）。
    pub const SUGGESTED_OUTPUT: usize = 1_024;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::format::DEFAULT_FRAME_INTERLEAVED;

    fn tone(frames: usize, phase: &mut f64, freq: f64) -> Vec<f32> {
        let step = freq / f64::from(SAMPLE_RATE_HZ);
        let mut out = Vec::with_capacity(frames * 2);
        for _ in 0..frames {
            let value = ((*phase) * std::f64::consts::TAU).sin() as f32 * 0.3;
            *phase += step;
            out.push(value);
            out.push(value);
        }
        out
    }

    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum: f32 = samples.iter().map(|value| value * value).sum();
        (sum / samples.len() as f32).sqrt()
    }

    #[test]
    fn 默认档位符合_adr_003_基线() {
        let config = CodecConfig::m1_default();
        assert_eq!(config.frame_ms, 20);
        assert_eq!(config.bitrate_bps, 160_000);
        assert_eq!(config.complexity, 7);
        assert_eq!(config.application, Application::RestrictedLowDelay);
        assert!(!config.use_cbr, "默认必须是 VBR");
        assert!(!config.use_inband_fec, "音乐场景默认关闭 in-band FEC");
        assert_eq!(config.frame_samples(), 960);
        assert_eq!(config.interleaved_frame(), DEFAULT_FRAME_INTERLEAVED);
        config.validate().unwrap();
        assert_eq!(config.telemetry().frame_ms, 20);
    }

    #[test]
    fn 编码解码往返保持能量与样本数() {
        let config = CodecConfig::m1_default();
        let mut encoder = OpusEncoder::new(config).unwrap();
        let mut decoder = OpusDecoder::new(config).unwrap();
        let mut phase = 0.0f64;
        let mut out = vec![0u8; proto_budget::SUGGESTED_OUTPUT];
        let mut pcm = vec![0.0f32; config.interleaved_frame()];

        let mut max_rms = 0.0f32;
        for index in 0..50 {
            let frame = tone(config.frame_samples(), &mut phase, 440.0);
            let written = encoder.encode_into(&frame, &mut out).unwrap();
            assert!(written > 0, "第 {index} 帧编码为空");
            assert!(
                written <= proto_budget::MAX_OPUS_PAYLOAD,
                "帧 {written} B 超出数据报载荷上限"
            );
            // 头 5 帧让编解码器收敛（CELT 前几帧能量尚未建立），从第 5 帧起检查能量
            let decoded = decoder.decode_into(&out[..written], &mut pcm).unwrap();
            assert_eq!(decoded, config.interleaved_frame());
            if index >= 5 {
                max_rms = max_rms.max(rms(&pcm));
            }
        }
        assert!(max_rms > 0.1, "解码后能量过低（rms={max_rms}），链路可疑");
        assert_eq!(decoder.plc_count(), 0);
        assert_eq!(encoder.frames_encoded(), 50);
        assert_eq!(decoder.frames_decoded(), 50);
    }

    #[test]
    fn 实测码率落在_vbr_合理区间() {
        let config = CodecConfig::m1_default();
        let mut encoder = OpusEncoder::new(config).unwrap();
        let mut phase = 0.0f64;
        let mut out = vec![0u8; proto_budget::SUGGESTED_OUTPUT];
        for _ in 0..100 {
            let frame = tone(config.frame_samples(), &mut phase, 1_000.0);
            encoder.encode_into(&frame, &mut out).unwrap();
        }
        let bitrate = encoder.measured_bitrate_bps();
        // VBR 的「目标码率」是长期平均目标，单段实测可以略高（这里 162 kbps > 160 kbps），
        // 因此只守住「量级正确」：落在目标的 0.5×～1.5× 区间内。
        let target = config.bitrate_bps as u32;
        assert!(
            bitrate >= target / 2 && bitrate <= target + target / 2,
            "实测码率 {bitrate} bps 偏离目标 {target} bps 过多"
        );
    }

    #[test]
    fn 丢包隐藏产出非静音且计数() {
        let config = CodecConfig::m1_default();
        let mut encoder = OpusEncoder::new(config).unwrap();
        let mut decoder = OpusDecoder::new(config).unwrap();
        let mut phase = 0.0f64;
        let mut out = vec![0u8; proto_budget::SUGGESTED_OUTPUT];
        let mut pcm = vec![0.0f32; config.interleaved_frame()];

        for _ in 0..10 {
            let frame = tone(config.frame_samples(), &mut phase, 300.0);
            let written = encoder.encode_into(&frame, &mut out).unwrap();
            decoder.decode_into(&out[..written], &mut pcm).unwrap();
        }
        let concealed = decoder.conceal_into(&mut pcm).unwrap();
        assert_eq!(concealed, config.interleaved_frame());
        assert_eq!(decoder.plc_count(), 1);
        assert!(rms(&pcm) > 0.01, "PLC 输出不应是静音（rms={}）", rms(&pcm));
    }

    #[test]
    fn 没有历史报文时_plc_返回错误而不是_panic() {
        let mut decoder = OpusDecoder::new(CodecConfig::m1_default()).unwrap();
        let mut pcm = vec![0.0f32; 1_920];
        let err = decoder.conceal_into(&mut pcm).unwrap_err();
        assert_eq!(err.code(), audiolink_types::ErrorCode::CodecUnsupported);
    }

    #[test]
    fn 空报文解码被拒绝() {
        let mut decoder = OpusDecoder::new(CodecConfig::m1_default()).unwrap();
        let mut pcm = vec![0.0f32; 1_920];
        assert!(decoder.decode_into(&[], &mut pcm).is_err());
    }

    #[test]
    fn 非法配置被拒绝() {
        let mut config = CodecConfig::m1_default();
        config.frame_ms = 40;
        assert!(
            config.validate().is_err(),
            "RESTRICTED_LOWDELAY 不允许 40 ms"
        );
        config.application = Application::Audio;
        config.validate().unwrap();
        config.frame_ms = 15;
        assert!(config.validate().is_err());
        config.frame_ms = 20;
        config.bitrate_bps = 100;
        assert!(config.validate().is_err());
        config.bitrate_bps = 160_000;
        config.packet_loss_perc = 101;
        assert!(config.validate().is_err());
    }

    #[test]
    fn 编码输入不足一帧被拒绝() {
        let mut encoder = OpusEncoder::new(CodecConfig::m1_default()).unwrap();
        let mut out = vec![0u8; 1_024];
        let short = vec![0.0f32; 1_919];
        assert!(encoder.encode_into(&short, &mut out).is_err());
    }

    #[test]
    fn 标称管线延迟符合实测分解() {
        let restricted = CodecConfig::m1_default();
        let audio = CodecConfig {
            application: Application::Audio,
            ..restricted
        };
        // 实测：RLD = 120 样本（2.5 ms，全在解码端 MDCT overlap）；Audio = 312 样本（6.5 ms）
        assert_eq!(restricted.nominal_pipeline_delay_us(), 2_500);
        assert_eq!(audio.nominal_pipeline_delay_us(), 6_500);
    }

    #[test]
    fn 无模式_getter_时所有档位都保守使用自建掩盖() {
        let restricted = OpusDecoder::new(CodecConfig::m1_default()).unwrap();
        assert!(
            !restricted.concealment_is_real(),
            "CELT-only（M1 默认档）没有真正的 PLC，必须让上层知道"
        );
        let audio = OpusDecoder::new(CodecConfig {
            application: Application::Audio,
            ..CodecConfig::m1_default()
        })
        .unwrap();
        assert!(
            !audio.concealment_is_real(),
            "Application::Audio 在高码率也会走 CELT，不能只看 application 猜模式"
        );
    }

    /// 这条测试**固定住一个已知缺陷**（不是我们要的行为，但必须先如实记录）：
    /// CELT-only 下连续丢两帧 —— 第 1 帧还有 MDCT 重叠余响，第 2 帧起是硬静音。
    /// 如果将来 `opus-rs` 修好了 PLC，这条会失败，那时应当**改代码 + 改 ADR 注记**，而不是删测试。
    #[test]
    fn celt_only_丢包第二帧起退化为静音_已知缺陷() {
        let config = CodecConfig::m1_default();
        let mut encoder = OpusEncoder::new(config).unwrap();
        let mut decoder = OpusDecoder::new(config).unwrap();
        let mut phase = 0.0f64;
        let mut packet = vec![0u8; proto_budget::SUGGESTED_OUTPUT];
        let mut pcm = vec![0.0f32; config.interleaved_frame()];

        for _ in 0..20 {
            let frame = tone(config.frame_samples(), &mut phase, 440.0);
            let written = encoder.encode_into(&frame, &mut packet).unwrap();
            decoder.decode_into(&packet[..written], &mut pcm).unwrap();
        }
        let reference_rms = rms(&pcm);
        assert!(reference_rms > 0.1, "参考帧能量过低：{reference_rms}");

        decoder.conceal_into(&mut pcm).unwrap();
        let first_loss_rms = rms(&pcm);
        decoder.conceal_into(&mut pcm).unwrap();
        let second_loss_rms = rms(&pcm);

        assert!(
            second_loss_rms < first_loss_rms * 0.1,
            "预期第 2 个丢失帧明显退化（实测硬静音），实际 first={first_loss_rms} second={second_loss_rms}"
        );
        assert_eq!(decoder.plc_count(), 2);
    }
}
