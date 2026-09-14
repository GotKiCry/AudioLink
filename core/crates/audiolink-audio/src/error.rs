//! 统一音频错误类型（`docs/02-architecture.md` §11）
//!
//! 所有音频路径的错误都收敛到 [`AudioError`]，并可用 [`AudioError::code`] 映射到协议错误码
//! （`audiolink_types::ErrorCode`）—— 这样「错误码表」始终只有一份来源（`docs/03-protocol.md` §11）。
//!
//! 纪律（`docs/02-architecture.md` §4 实时铁律 1）：本 crate 的任何函数**都不 panic**，
//! 失败一律显式返回 `Err` 并把上下文写清（便于遥测与日志，不丢信息）。

use std::borrow::Cow;
use std::fmt;

use audiolink_types::ErrorCode;

/// 音频子系统错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    /// 端点不存在 / 打不开 / 不支持所需格式 → `2003 CAPTURE_LOST`（用户可见：设备被拔掉或改了配置）。
    DeviceUnavailable {
        /// 失败细节（静态字面量或错误路径上格式化的动态串）。
        context: Cow<'static, str>,
    },
    /// 流在运行中失败（采集掉线、播放器失效）→ `2003 CAPTURE_LOST`。
    StreamFailed {
        /// 失败细节。
        context: Cow<'static, str>,
    },
    /// 播放欠载 → `2001 PLAYOUT_UNDERRUN`（**统计用途，不致命**：不得据此断流）。
    Underrun {
        /// 失败细节。
        context: Cow<'static, str>,
    },
    /// 编解码失败 → `1007 CODEC_UNSUPPORTED`。
    Codec {
        /// 失败细节。
        context: Cow<'static, str>,
    },
    /// 配置非法（帧长 / 声道 / 采样率 / 缓冲尺寸不受支持）→ `1008 BAD_REQUEST`。
    InvalidConfig {
        /// 失败细节。
        context: Cow<'static, str>,
    },
}

impl AudioError {
    /// 静态上下文的「设备不可用」。
    pub const fn device_unavailable(context: &'static str) -> Self {
        Self::DeviceUnavailable {
            context: Cow::Borrowed(context),
        }
    }

    /// 动态上下文的「设备不可用」（仅错误路径使用，允许分配）。
    pub fn device_unavailable_owned(context: String) -> Self {
        Self::DeviceUnavailable {
            context: Cow::Owned(context),
        }
    }

    /// 静态上下文的「流失败」。
    pub const fn stream_failed(context: &'static str) -> Self {
        Self::StreamFailed {
            context: Cow::Borrowed(context),
        }
    }

    /// 动态上下文的「流失败」。
    pub fn stream_failed_owned(context: String) -> Self {
        Self::StreamFailed {
            context: Cow::Owned(context),
        }
    }

    /// 静态上下文的「播放欠载」。
    pub const fn underrun(context: &'static str) -> Self {
        Self::Underrun {
            context: Cow::Borrowed(context),
        }
    }

    /// 动态上下文的「播放欠载」。
    pub fn underrun_owned(context: String) -> Self {
        Self::Underrun {
            context: Cow::Owned(context),
        }
    }

    /// 静态上下文的「编解码失败」。
    pub const fn codec(context: &'static str) -> Self {
        Self::Codec {
            context: Cow::Borrowed(context),
        }
    }

    /// 动态上下文的「编解码失败」。
    pub fn codec_owned(context: String) -> Self {
        Self::Codec {
            context: Cow::Owned(context),
        }
    }

    /// 静态上下文的「配置非法」。
    pub const fn invalid_config(context: &'static str) -> Self {
        Self::InvalidConfig {
            context: Cow::Borrowed(context),
        }
    }

    /// 动态上下文的「配置非法」。
    pub fn invalid_config_owned(context: String) -> Self {
        Self::InvalidConfig {
            context: Cow::Owned(context),
        }
    }

    /// 映射到协议错误码（§11 表）。
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::DeviceUnavailable { .. } | Self::StreamFailed { .. } => ErrorCode::CaptureLost,
            Self::Underrun { .. } => ErrorCode::PlayoutUnderrun,
            Self::Codec { .. } => ErrorCode::CodecUnsupported,
            Self::InvalidConfig { .. } => ErrorCode::BadRequest,
        }
    }

    /// 失败细节。
    pub fn context(&self) -> &str {
        match self {
            Self::DeviceUnavailable { context }
            | Self::StreamFailed { context }
            | Self::Underrun { context }
            | Self::Codec { context }
            | Self::InvalidConfig { context } => context.as_ref(),
        }
    }

    /// 是否为「可恢复、仅供统计」的错误（欠载属于此类：必须计数，但不得断流）。
    pub fn is_statistical(&self) -> bool {
        matches!(self, Self::Underrun { .. })
    }
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}: {}",
            self.code().as_u16(),
            self.code().name(),
            self.context()
        )
    }
}

impl std::error::Error for AudioError {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 错误码映射符合协议表() {
        assert_eq!(
            AudioError::device_unavailable("no endpoint").code(),
            ErrorCode::CaptureLost
        );
        assert_eq!(
            AudioError::stream_failed("stream died").code(),
            ErrorCode::CaptureLost
        );
        assert_eq!(
            AudioError::underrun("sink starved").code(),
            ErrorCode::PlayoutUnderrun
        );
        assert_eq!(
            AudioError::codec("bad frame").code(),
            ErrorCode::CodecUnsupported
        );
        assert_eq!(
            AudioError::invalid_config("frame_ms").code(),
            ErrorCode::BadRequest
        );
    }

    #[test]
    fn 显示携带错误码与上下文() {
        let text = AudioError::codec("opus 编码失败").to_string();
        assert!(text.starts_with("1007 CODEC_UNSUPPORTED"), "{text}");
        assert!(text.ends_with("opus 编码失败"), "{text}");
    }

    #[test]
    fn 只有欠载是统计性的() {
        assert!(AudioError::underrun("x").is_statistical());
        assert!(!AudioError::codec("x").is_statistical());
    }
}
