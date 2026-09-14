//! 跨 FFI 错误：契约 §1 的 `{code, message, context}` 形状。
//!
//! 上游（engine / net / identity / audio）已经在 engine 边界收敛成
//! [`audiolink_types::AudioLinkError`]（`code() -> ErrorCode` + `context() -> &str`），
//! 这里只做**形状转换**，不发明新错误码 —— 否则 Kotlin 侧就没法与 `docs/03-protocol.md` §11 的错误码表对照。
//!
//! # 写这个文件的文档注释时的硬约束（踩过一次，别再踩）
//!
//! UniFFI 把这里的 rustdoc **原样**搬进生成的 Kotlin KDoc，而 **Kotlin 的块注释可以嵌套**。
//! 于是注释里出现一次「斜杠紧跟星号」的字符序列，就会在 KDoc 里开一个**内层**注释，
//! 需要两个结束标记才能闭合 —— 一个路径通配符足以让**整个生成文件被吞到文件尾**，
//! 编译器只报 `Unclosed comment`，而 review 完全看不出来（实测：`bindings/kotlin/` 下那份
//! `.kt` 的第 1779–2343 行全被吞掉，`PcmFeed` / `PcmPull` 的 callback object 与全部顶层函数一起消失）。
//!
//! 规矩：**文档注释里不许出现「`/` 紧跟 `*`」或「`*` 紧跟 `/`」的字符序列**；
//! 要写路径通配就用文字绕开（例如「`bindings/kotlin/` 目录下的 `.kt` 文件」）。
//! 这条规矩由 `crate::kotlin_guard` 的用例机械检查（源码侧 + 生成物侧各一条），不靠自觉。

use audiolink_types::{AudioLinkError, ErrorCode};

/// FFI 失败。
///
/// 用 `enum` 而不是 struct：UniFFI 0.29 的 `#[derive(uniffi::Error)]` 只支持枚举
/// （`uniffi_macros::error::expand_error` 内部构造的是 `EnumItem`）。
/// 只有一个变体 `Failure`；Kotlin 侧生成的是 `FfiException.Failure(code, messageText, context)`
/// —— 类型名带 `Exception` 后缀是 UniFFI 的 Kotlin 命名规则。
///
/// # 为什么字段是 `message_text` 而不是 `message`
///
/// Kotlin 生成的 `Failure` 继承 `FfiException`，而它继承 `kotlin.Exception` → `Throwable`；
/// UniFFI 同时会给异常生成一个 `override val message` getter，那个 getter 又要引用记录里的同名字段
/// —— 同名冲突，实测编译器报：
///
/// ```text
/// e: 'message' hides member of supertype 'Throwable' and needs an 'override' modifier
/// e: Conflicting declarations: val message: String
/// ```
///
/// 所以字段改名为 `message_text`（Kotlin 侧自动转成 `messageText`）。**语义不变**：
/// 契约 §1 里那一格还是「§11 错误码短名」（如 `NOT_PAIRED`），
/// 完整人话串由 `Throwable.message` 给出（`1002 NOT_PAIRED: peer requires pin pairing`）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    /// 统一失败形态。
    ///
    /// - `code`：`docs/03-protocol.md` §11 错误码的数值形式（如 `1002` = `NOT_PAIRED`）；
    /// - `message_text`：错误码短名（如 `NOT_PAIRED`），供日志 / UI 查找与展示；
    /// - `context`：出错细节（谁、在哪、什么值）。
    #[error("{code} {message_text}: {context}")]
    Failure {
        /// §11 错误码数值。
        code: u16,
        /// §11 错误码短名。
        message_text: String,
        /// 失败细节。
        context: String,
    },
}

impl FfiError {
    /// 由 §11 错误码 + 细节构造。
    pub fn from_code(code: ErrorCode, context: impl Into<String>) -> Self {
        Self::Failure {
            code: code.as_u16(),
            message_text: code.name().to_string(),
            context: context.into(),
        }
    }

    /// 由内核错误构造（engine 边界的统一错误类型）。
    pub fn from_audio_link(error: &AudioLinkError) -> Self {
        Self::from_code(error.code(), error.context())
    }

    /// FFI 层前置条件失败：引擎没启动。
    ///
    /// 用 `1008 BAD_REQUEST`：§11 里没有「引擎未启动」这一类 —— 那不是一个协议事件，
    /// 而是**调用顺序错误**，与「载荷 / 请求非法」同属契约级失败。
    pub fn engine_not_started(operation: &str) -> Self {
        Self::from_code(
            ErrorCode::BadRequest,
            format!("engine is not started; {operation}() requires a running engine"),
        )
    }

    /// FFI 层前置条件失败：参数非法。
    pub fn invalid_argument(context: impl Into<String>) -> Self {
        Self::from_code(ErrorCode::BadRequest, context)
    }

    /// §11 错误码数值。
    pub fn code(&self) -> u16 {
        match self {
            Self::Failure { code, .. } => *code,
        }
    }

    /// §11 错误码短名（契约 §1 的 `message` 一格）。
    pub fn message_text(&self) -> &str {
        match self {
            Self::Failure { message_text, .. } => message_text.as_str(),
        }
    }

    /// 失败细节。
    pub fn context(&self) -> &str {
        match self {
            Self::Failure { context, .. } => context.as_str(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 内核错误转换保留错误码与上下文() {
        let error = AudioLinkError::not_paired("peer requires pin pairing");
        let ffi = FfiError::from_audio_link(&error);

        assert_eq!(ffi.code(), 1002);
        assert_eq!(ffi.message_text(), "NOT_PAIRED");
        assert_eq!(ffi.context(), "peer requires pin pairing");
        // 展示形式同时带齐三要素（Kotlin 侧 `Throwable.message` 就是这一串）
        assert_eq!(
            ffi.to_string(),
            "1002 NOT_PAIRED: peer requires pin pairing"
        );
    }

    #[test]
    fn 未启动引擎是_1008_契约级失败() {
        let ffi = FfiError::engine_not_started("connect");
        assert_eq!(ffi.code(), ErrorCode::BadRequest.as_u16());
        assert!(ffi.context().contains("connect()"));
    }
}
