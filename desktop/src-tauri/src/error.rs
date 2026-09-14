//! 命令失败的**前端可读**形状。
//!
//! 为什么不让命令直接返回 `audiolink_types::AudioLinkError`：那个类型是内核内部错误，
//! 形状会随内核演进；而前端需要的是稳定的三件事 —— 错误码（给日志/诊断）、
//! 人话原因（给用户，`docs/08-ui-spec.md` §4 明令"错误必说人话，不允许未知错误"）、
//! 上下文（给排查，`docs/02-architecture.md` §11 的 `code + context` 约定）。
//!
//! 错误码沿用 `audiolink_types::ErrorCode`（`1001`–`3001`），不自己发明数字。

use std::fmt;

use audiolink_types::ErrorCode;
use serde::Serialize;

/// 命令失败 → 前端（`invoke` 的 Promise rejection 就是这个 JSON）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    /// `ErrorCode` 的数值形式（如 `1003` = `PAIR_REJECTED`）。
    pub code: u16,
    /// 给用户看的整句人话（短句、动词开头、不出现 socket/RTT 之类黑话）。
    pub message: String,
    /// 给排查用的机械上下文（命令名 / 地址 / 对端 id 等）。
    pub context: String,
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>, context: impl Into<String>) -> Self {
        Self {
            code: code.as_u16(),
            message: message.into(),
            context: context.into(),
        }
    }

    /// `1008 BAD_REQUEST`：入参本身不合法（地址格式错、目标不存在…）。
    pub fn bad_request(message: impl Into<String>, context: impl Into<String>) -> Self {
        Self::new(ErrorCode::BadRequest, message, context)
    }

    /// `1009 BUSY`：当前状态不允许该操作（重复连接、已有推流…）。
    pub fn busy(message: impl Into<String>, context: impl Into<String>) -> Self {
        Self::new(ErrorCode::Busy, message, context)
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {} ({})", self.code, self.message, self.context)
    }
}

impl std::error::Error for CommandError {}
