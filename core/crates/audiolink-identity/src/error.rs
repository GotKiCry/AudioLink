//! 身份子系统错误（`docs/02-architecture.md` §11、`docs/03-protocol.md` §11）
//!
//! 风格对齐 `audiolink-audio::AudioError`：`code() -> ErrorCode` + `context() -> &str` +
//! `is_statistical()`；`Display` 由 `thiserror` 生成，格式与 `audiolink_types::AudioLinkError`
//! 完全一致（`<错误码> <名称>: <上下文>`），两端日志可直接对齐比对。
//!
//! **错误码取舍（一处刻意的映射决定）**：§11 的表里没有「本地持久化 / 证书解析」这类码。
//! 它们不是对端发了坏包，而是本机自身的问题，因此统一收敛到 `1008 BAD_REQUEST` —— 表里唯一
//! 中性的「本端拒绝」码；engine 侧不得据此判定对端异常。协议级的 `1004 AUTH_FAILED` 已随挑战
//! 应答一起删除（`docs/71-remove-pairing.md` §4），本 crate 不再产出任何协议级失败码。

use std::borrow::Cow;

use audiolink_types::ErrorCode;

/// 身份 / 证书路径上的错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    /// 证书生成、PEM/DER 解析失败，或证书与私钥不匹配。
    #[error("{} {}: {context}", self.code().as_u16(), self.code().name())]
    Certificate {
        /// 失败细节。
        context: Cow<'static, str>,
    },
    /// 私钥解析失败。
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

    /// 映射到协议错误码（§11 表）：本 crate 的失败都是本机侧问题 → 一律 `1008`。
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Certificate { .. }
            | Self::Key { .. }
            | Self::Io { .. }
            | Self::InvalidConfig { .. } => ErrorCode::BadRequest,
        }
    }

    /// 失败细节。
    pub fn context(&self) -> &str {
        match self {
            Self::Certificate { context }
            | Self::Key { context }
            | Self::Io { context }
            | Self::InvalidConfig { context } => context.as_ref(),
        }
    }

    /// 是否**统计类**（不致命）。
    ///
    /// 恒为 `false`：身份失败绝不能「计数后继续」。证书与私钥不匹配、证书解析不出公钥，
    /// 都意味着本机身份立不住（对端在 TLS 握手里就会把本机拒掉），必须停下来而不是记一笔就接着跑。
    pub fn is_statistical(&self) -> bool {
        match self {
            Self::Certificate { .. }
            | Self::Key { .. }
            | Self::Io { .. }
            | Self::InvalidConfig { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 身份错误一律映射到_1008() {
        // §11：表里唯一中性的「本端拒绝」码；本 crate 不再有协议级失败码。
        assert_eq!(
            IdentityError::certificate("x").code(),
            ErrorCode::BadRequest
        );
        assert_eq!(IdentityError::key("x").code(), ErrorCode::BadRequest);
        assert_eq!(IdentityError::io("x").code(), ErrorCode::BadRequest);
        assert_eq!(
            IdentityError::invalid_config("x").code(),
            ErrorCode::BadRequest
        );
    }

    #[test]
    fn 显示格式与统一错误类型一致() {
        let text = IdentityError::certificate("证书与私钥不匹配").to_string();
        assert!(text.starts_with("1008 BAD_REQUEST"), "{text}");
        assert!(text.ends_with("证书与私钥不匹配"), "{text}");
    }

    #[test]
    fn 身份错误一律不统计() {
        assert!(!IdentityError::certificate("x").is_statistical());
        assert!(!IdentityError::key("x").is_statistical());
        assert!(!IdentityError::io("x").is_statistical());
        assert!(!IdentityError::invalid_config("x").is_statistical());
    }
}
