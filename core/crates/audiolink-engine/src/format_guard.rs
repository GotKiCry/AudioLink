//! 「零重采样」硬断言（NFR-13，M1 的两条硬指标之一）
//!
//! `docs/05-roadmap.md` M1 写着：**「零重采样」与「低延迟模式」两项若未达标，不得进入 M2**。
//! 这两条之所以是硬指标，是因为它们是延迟预算里最大的单点杠杆（Oboe 实测分别值 ~160 ms 与 ~185 ms）。
//! 而它们有一个共同特点：**不达标时链路照样「能出声」**。既然能出声，就没人会去查 ——
//! 于是它会一直藏着，直到某天有人问「为什么延迟比宣传的高一倍」。
//!
//! 所以这里不做「尽力而为」，而是**在链路启动的必经路径上设一道会拒绝启动的闸**：
//! 设备格式不是 48 kHz / 2ch / f32，就直接返回错误，绝不静默重采样。
//!
//! # 为什么坚决不做静默 SRC
//!
//! 需求 `docs/01-requirements.md` 与 `docs/04-tech-stack.md` 都写死了：设备不是 48 kHz 时
//! **引导用户改系统设置**，而不是让内核偷偷重采样。理由有三条，都不是洁癖：
//!
//! 1. **隐式延迟**：重采样器要么引入算法延迟，要么引入缓冲延迟，两个都会直接吃掉延迟预算；
//! 2. **不可自证**：一旦允许 SRC，「全链路零重采样」这条验收指标就永远无法被验证 ——
//!    你无法从遥测里区分「没做 SRC」与「做了 SRC 但没上报」；
//! 3. **音质**：44.1 → 48 kHz 是非整数倍转换，实现差一点就能听出混叠。

use audiolink_audio::{DeviceFormat, SAMPLE_RATE_HZ, SampleFormat};
use audiolink_types::{AudioLinkError, ErrorCode};

/// 构造 `1005 CAP_UNSUPPORTED` 错误（动态上下文）。
///
/// `AudioLinkError` 的 `const` 构造器只吃 `&'static str`，而这里的失败信息必须带上
/// **实测到的**采样率 / 声道数 / 样本格式 —— 用户要照着它去改设备设置，所以不能用静态串糊过去。
fn cap_unsupported(context: String) -> AudioLinkError {
    AudioLinkError::owned(ErrorCode::CapUnsupported, context)
}

/// 断言设备格式就是内核统一格式：`48_000 Hz / 2ch / f32`。
///
/// `role` 用于把失败信息定位到具体环节（`"capture"` / `"playout"`），
/// `backend` 是实现名（如 `"wasapi-loopback"`），让用户知道该去改哪个设备的设置。
///
/// # Errors
///
/// 采样率、声道数或样本格式任一不符即返回 [`AudioLinkError`]（`1005 CAP_UNSUPPORTED`）——
/// **不尝试转换**，也**不降级继续**。
pub fn require_unified_format(
    role: &'static str,
    backend: &str,
    format: DeviceFormat,
) -> Result<(), AudioLinkError> {
    // 采样率单独判：它是用户唯一能通过系统设置修好的那一项，所以要给出可操作的提示。
    if format.sample_rate != SAMPLE_RATE_HZ {
        return Err(cap_unsupported(format!(
            "{role} device '{backend}' runs at {} Hz but AudioLink requires {} Hz; \
             change the device's sample rate in the OS sound settings \
             (AudioLink never resamples silently)",
            format.sample_rate, SAMPLE_RATE_HZ
        )));
    }

    if format.channels != 2 {
        return Err(cap_unsupported(format!(
            "{role} device '{backend}' exposes {} channel(s) but AudioLink requires 2",
            format.channels
        )));
    }

    if format.sample_format != SampleFormat::F32 {
        return Err(cap_unsupported(format!(
            "{role} device '{backend}' delivers {:?} samples but AudioLink requires f32",
            format.sample_format
        )));
    }

    Ok(())
}

/// 一次性断言采集与播放两端（任一端不合格即失败）。
///
/// 分开传参而不是传一个 Vec，是为了让错误信息里能明确指出是采集端还是播放端 ——
/// 这两端的修复动作完全不同（改录音设备 vs 改播放设备）。
pub fn require_unified_link(
    capture: Option<(&'static str, DeviceFormat)>,
    playout: Option<(&'static str, DeviceFormat)>,
) -> Result<(), AudioLinkError> {
    if let Some((backend, format)) = capture {
        require_unified_format("capture", backend, format)?;
    }
    if let Some((backend, format)) = playout {
        require_unified_format("playout", backend, format)?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use audiolink_types::ErrorCode;

    fn unified() -> DeviceFormat {
        DeviceFormat::new(SAMPLE_RATE_HZ, 2, SampleFormat::F32)
    }

    #[test]
    fn accepts_exactly_the_unified_format() {
        assert!(require_unified_format("capture", "wasapi-loopback", unified()).is_ok());
    }

    #[test]
    fn rejects_forty_four_one_kilohertz_and_tells_the_user_what_to_do() {
        // 44.1 kHz 是最常见的「差一点点」：不拦下来的话，内核会静默 SRC 到 48 kHz，
        // 延迟与音质都被悄悄改掉，而验收指标再也无法自证。
        let format = DeviceFormat::new(44_100, 2, SampleFormat::F32);
        let error = require_unified_format("capture", "wasapi-loopback", format).unwrap_err();

        assert_eq!(error.code(), ErrorCode::CapUnsupported);
        let context = error.context();
        assert!(context.contains("44100"), "要说清实际值：{context}");
        assert!(context.contains("48000"), "要说清期望值：{context}");
        assert!(
            context.contains("sound settings"),
            "要给用户可操作的修复方向，而不是只说「不行」：{context}"
        );
        assert!(
            context.contains("never resamples"),
            "要明确承诺不做隐式重采样，否则用户会以为内核会兜底：{context}"
        );
    }

    #[test]
    fn rejects_wrong_channel_count() {
        let mono = DeviceFormat::new(SAMPLE_RATE_HZ, 1, SampleFormat::F32);
        let error = require_unified_format("playout", "wasapi-render", mono).unwrap_err();
        assert_eq!(error.code(), ErrorCode::CapUnsupported);
        assert!(error.context().contains("1 channel"));

        let surround = DeviceFormat::new(SAMPLE_RATE_HZ, 6, SampleFormat::F32);
        assert!(require_unified_format("playout", "wasapi-render", surround).is_err());
    }

    #[test]
    fn rejects_non_f32_sample_formats() {
        for (format, needle) in [
            (SampleFormat::I16, "I16"),
            (SampleFormat::I24, "I24"),
            (SampleFormat::I32, "I32"),
        ] {
            let device = DeviceFormat::new(SAMPLE_RATE_HZ, 2, format);
            let error = require_unified_format("capture", "wasapi-loopback", device).unwrap_err();
            assert_eq!(error.code(), ErrorCode::CapUnsupported);
            assert!(
                error.context().contains(needle),
                "错误信息里要能看出是哪种格式：{}",
                error.context()
            );
        }
    }

    #[test]
    fn link_assertion_names_the_failing_side() {
        let bad = DeviceFormat::new(44_100, 2, SampleFormat::F32);

        // 采集端坏：信息里必须是 capture，不能是 playout（否则用户会去改错设备的设置）
        let error = require_unified_link(
            Some(("wasapi-loopback", bad)),
            Some(("wasapi-render", unified())),
        )
        .unwrap_err();
        assert!(error.context().starts_with("capture device"));

        let error = require_unified_link(
            Some(("wasapi-loopback", unified())),
            Some(("wasapi-render", bad)),
        )
        .unwrap_err();
        assert!(error.context().starts_with("playout device"));
    }

    #[test]
    fn link_assertion_accepts_a_one_sided_link() {
        // 纯发送端（无播放设备）与纯接收端（无采集设备）都必须能启动 ——
        // 否则「只推不播」的用法会被这道闸误伤。
        assert!(require_unified_link(Some(("wasapi-loopback", unified())), None).is_ok());
        assert!(require_unified_link(None, Some(("wasapi-render", unified()))).is_ok());
        assert!(require_unified_link(None, None).is_ok());
    }

    #[test]
    fn error_is_not_statistical_so_it_actually_stops_the_startup() {
        // 关键：这条错误必须属于「契约级失败」。如果误标成统计类，
        // 上层就会「计数后继续」，闸门形同虚设 —— 链路会带着重采样跑起来。
        let error = require_unified_format(
            "capture",
            "wasapi-loopback",
            DeviceFormat::new(44_100, 2, SampleFormat::F32),
        )
        .unwrap_err();

        assert!(
            !error.is_statistical(),
            "零重采样失败必须阻断启动，不能只计数"
        );
        assert!(error.is_fatal());
    }
}
