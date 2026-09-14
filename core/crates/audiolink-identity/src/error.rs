//! 身份子系统错误（`docs/02-architecture.md` §11、`docs/03-protocol.md` §11）
//!
//! 风格对齐 `audiolink-audio::AudioError`：`code() -> ErrorCode` + `context() -> &str` +
//! `is_statistical()`；`Display` 由 `thiserror` 生成，格式与 `audiolink_types::AudioLinkError`
//! 完全一致（`<错误码> <名称>: <上下文>`），两端日志可直接对齐比对。
//!
//! **错误码取舍（一处刻意的映射决定）**：§11 的表里没有「本地持久化 / 证书解析」这类码。
//! 它们不是对端发了坏包，而是本机自身的问题，因此统一收敛到 `1008 BAD_REQUEST` —— 表里唯一
//! 中性的「本端拒绝」码；engine 侧不得据此判定对端异常。真正的协议级失败只有
//! [`IdentityError::AuthFailed`]（`1004`，§5「认证强度」：对端证书与签名不匹配，可能是中间人）。

use std::borrow::Cow;

use audiolink_types::ErrorCode;

/// 身份 / 证书 / 信任库 / 配对路径上的错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    /// 证书生成、PEM/DER 解析失败，或证书与私钥不匹配。
    #[error("{} {}: {context}", self.code().as_u16(), self.code().name())]
    Certificate {
        /// 失败细节。
        context: Cow<'static, str>,
    },
    /// 私钥解析或签名运算失败。
    #[error("{} {}: {context}", self.code().as_u16(), self.code().name())]
    Key {
        /// 失败细节。
        context: Cow<'static, str>,
    },
    /// 文件系统交互失败（读取身份文件、原子落盘、建目录）。
    #[error("{} {}: {context}", self.code().as_u16(), self.code().name())]
    Io {
        /// 失败细节。
        context: Cow<'static, str>,
    },
    /// 信任库文件损坏或结构非法。
    ///
    /// 这是**必须暴露**的错误：静默重置信任库等于悄悄清空信任边界，比拒绝启动危险得多。
    #[error("{} {}: {context}", self.code().as_u16(), self.code().name())]
    TrustStore {
        /// 失败细节。
        context: Cow<'static, str>,
    },
    /// 对端签名校验失败 → `1004 AUTH_FAILED`（§5：可能是中间人）。
    #[error("{} {}: {context}", self.code().as_u16(), self.code().name())]
    AuthFailed {
        /// 失败细节。
        context: Cow<'static, str>,
    },
    /// 调用参数非法（空节点名、路径没有文件名等）。
    #[error("{} {}: {context}", self.code().as_u16(), self.code().name())]
    InvalidConfig {
        /// 失败细节。
        context: Cow<'static, str>,
    },
}

impl IdentityError {
    /// 静态上下文的「证书错误」。
    pub const fn certificate(context: &'static str) -> Self {
        Self::Certificate {
            context: Cow::Borrowed(context),
        }
    }

    /// 动态上下文的「证书错误」（仅错误路径使用，允许分配）。
    pub fn certificate_owned(context: String) -> Self {
        Self::Certificate {
            context: Cow::Owned(context),
        }
    }

    /// 静态上下文的「私钥错误」。
    pub const fn key(context: &'static str) -> Self {
        Self::Key {
            context: Cow::Borrowed(context),
        }
    }

    /// 动态上下文的「私钥错误」。
    pub fn key_owned(context: String) -> Self {
        Self::Key {
            context: Cow::Owned(context),
        }
    }

    /// 静态上下文的「IO 错误」。
    pub const fn io(context: &'static str) -> Self {
        Self::Io {
            context: Cow::Borrowed(context),
        }
    }

    /// 动态上下文的「IO 错误」。
    pub fn io_owned(context: String) -> Self {
        Self::Io {
            context: Cow::Owned(context),
        }
    }

    /// 静态上下文的「信任库损坏」。
    pub const fn trust_store(context: &'static str) -> Self {
        Self::TrustStore {
            context: Cow::Borrowed(context),
        }
    }

    /// 动态上下文的「信任库损坏」。
    pub fn trust_store_owned(context: String) -> Self {
        Self::TrustStore {
            context: Cow::Owned(context),
        }
    }

    /// 静态上下文的「签名校验失败」。
    pub const fn auth_failed(context: &'static str) -> Self {
        Self::AuthFailed {
            context: Cow::Borrowed(context),
        }
    }

    /// 动态上下文的「签名校验失败」。
    pub fn auth_failed_owned(context: String) -> Self {
        Self::AuthFailed {
            context: Cow::Owned(context),
        }
    }

    /// 静态上下文的「参数非法」。
    pub const fn invalid_config(context: &'static str) -> Self {
        Self::InvalidConfig {
            context: Cow::Borrowed(context),
        }
    }

    /// 动态上下文的「参数非法」。
    pub fn invalid_config_owned(context: String) -> Self {
        Self::InvalidConfig {
            context: Cow::Owned(context),
        }
    }

    /// 映射到协议错误码（§11 表）；只有验签失败是协议级失败。
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::AuthFailed { .. } => ErrorCode::AuthFailed,
            Self::Certificate { .. }
            | Self::Key { .. }
            | Self::Io { .. }
            | Self::TrustStore { .. }
            | Self::InvalidConfig { .. } => ErrorCode::BadRequest,
        }
    }

    /// 失败细节。
    pub fn context(&self) -> &str {
        match self {
            Self::Certificate { context }
            | Self::Key { context }
            | Self::Io { context }
            | Self::TrustStore { context }
            | Self::AuthFailed { context }
            | Self::InvalidConfig { context } => context.as_ref(),
        }
    }

    /// 是否**统计类**（不致命）。
    ///
    /// 恒为 `false`：身份与认证失败绝不能「计数后继续」。签名不匹配意味着对端身份不可信
    /// （§5「认证强度」明确指向中间人），信任库损坏意味着信任边界状态不可知 —— 二者都必须
    /// 停下来，而不是像欠载那样记一笔就接着跑。
    pub fn is_statistical(&self) -> bool {
        match self {
            Self::Certificate { .. }
            | Self::Key { .. }
            | Self::Io { .. }
            | Self::TrustStore { .. }
            | Self::AuthFailed { .. }
            | Self::InvalidConfig { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 只有验签失败是协议级错误码() {
        // §11：1004 是「签名校验失败（可能是中间人）」；其余本机问题收敛到 1008。
        assert_eq!(
            IdentityError::auth_failed("x").code(),
            ErrorCode::AuthFailed
        );
        assert_eq!(
            IdentityError::certificate("x").code(),
            ErrorCode::BadRequest
        );
        assert_eq!(IdentityError::key("x").code(), ErrorCode::BadRequest);
        assert_eq!(IdentityError::io("x").code(), ErrorCode::BadRequest);
        assert_eq!(
            IdentityError::trust_store("x").code(),
            ErrorCode::BadRequest
        );
        assert_eq!(
            IdentityError::invalid_config("x").code(),
            ErrorCode::BadRequest
        );
    }

    #[test]
    fn 显示格式与统一错误类型一致() {
        let text = IdentityError::auth_failed("对端签名校验失败").to_string();
        assert!(text.starts_with("1004 AUTH_FAILED"), "{text}");
        assert!(text.ends_with("对端签名校验失败"), "{text}");
    }

    #[test]
    fn 身份错误一律不统计() {
        assert!(!IdentityError::certificate("x").is_statistical());
        assert!(!IdentityError::key("x").is_statistical());
        assert!(!IdentityError::io("x").is_statistical());
        assert!(!IdentityError::trust_store("x").is_statistical());
        assert!(!IdentityError::auth_failed("x").is_statistical());
        assert!(!IdentityError::invalid_config("x").is_statistical());
    }
}
