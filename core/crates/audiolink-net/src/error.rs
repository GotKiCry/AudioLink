//! `NetError` —— `audiolink-net` 的唯一错误类型（`docs/11-m1-contract.md` §3）。
//!
//! 风格对齐 `audiolink-audio::AudioError`：
//! - [`NetError::code`] 把失败映射回 `docs/03-protocol.md` §11 错误码表（**只用于展示 / 遥测 / 跨 FFI**）；
//! - [`NetError::context`] 给出可诊断细节（失败在哪一步、上限与实长各是多少）；
//! - [`NetError::is_statistical`] 回答唯一影响处置的问题：**该不该断流**。
//!   按 `audiolink-types` 的纪律，调用方**不得**按 `code()` 自行猜测处置
//!   —— 这是旧版「异常即永久静音」的根治点之一。
//!
//! # 错误码映射口径
//!
//! §11 的错误码表是**协议级**的，没有为「端点绑定失败 / 传输中断」单列码，因此本 crate 的映射是取舍：
//!
//! | 变体 | 码 | 理由 |
//! |---|---|---|
//! | [`NetError::Bind`] / [`NetError::Config`] | `1008` | 本机侧问题：端口绑不上 / 证书私钥 DER 非法 / 参数越界；与对端行为无关 |
//! | [`NetError::Handshake`] / [`NetError::PeerIdentity`] | `1004` | 握手失败或拿不到对端证书 → 身份判定无法完成；§11 对该码的处置正是「断开并告警」 |
//! | [`NetError::DatagramUnsupported`] | `1005` | 对端未协商 RFC 9221 数据报 = 缺一项必需能力 |
//! | [`NetError::DatagramTooLarge`] 等长度类 | `1008` | §11 对 `1008` 的定义明确含「长度越界」；§4 的帧边界违规同属该类 |
//! | [`NetError::Transport`] | `1008` | §11 没有更贴切的码；见下注 |
//!
//! 注：把 [`NetError::Transport`]（连接关闭 / 收发失败）映射到 `1008` 并不贴切，但 §11 没有传输中断码。
//! 由于「是否断流」由 `is_statistical()` 单独承载，这个偏差**不会**造成错误处置。
//!
//! # `is_statistical()` 的口径
//!
//! 只有「丢一个包、会话仍语义完整」才算统计类 —— 即**音频数据报**路径上的丢帧
//! （[`NetError::DatagramTooLarge`] / [`NetError::DatagramBufferTooSmall`]）。
//! 控制帧是会话状态机的输入，丢一帧等于状态机卡住，因此控制面的错误一律非统计类。

use std::borrow::Cow;

use audiolink_types::ErrorCode;

/// 按 §11 错误码表批量生成 [`NetError`]。
///
/// 每个变体生成 2 个构造器（`const` / `_owned`）与 `code()` / `context()` / `is_statistical()`
/// 三个分支，避免 10 份手写样板漂移（与 `audiolink_types` 里 `AudioLinkError` 的生成方式同构）。
///
/// 语法：`#[error("...")] Variant => ctor, ctor_owned, ErrorCode::X, is_statistical;`
macro_rules! define_net_errors {
    ($($(#[$meta:meta])* $variant:ident => $ctor:ident, $ctor_owned:ident, $code:ident, $statistical:literal;)*) => {
        /// 传输层错误。
        #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
        pub enum NetError {
            $(
                $(#[$meta])*
                $variant {
                    /// 失败细节（优先用 `const` 构造器的静态字面量，零分配）。
                    context: Cow<'static, str>,
                },
            )*
        }

        impl NetError {
            $(
                #[doc = concat!("静态上下文的 `", stringify!($variant), "`（`const` 友好，零分配）。")]
                pub const fn $ctor(context: &'static str) -> Self {
                    Self::$variant { context: Cow::Borrowed(context) }
                }

                #[doc = concat!("动态上下文的 `", stringify!($variant), "`（仅错误路径使用，允许分配）。")]
                pub fn $ctor_owned(context: String) -> Self {
                    Self::$variant { context: Cow::Owned(context) }
                }
            )*

            /// 映射到 §11 错误码表（**只用于展示 / 遥测 / 跨 FFI**）。
            pub fn code(&self) -> ErrorCode {
                match self {
                    $(Self::$variant { .. } => ErrorCode::$code,)*
                }
            }

            /// 失败细节。
            pub fn context(&self) -> &str {
                match self {
                    $(Self::$variant { context } => context.as_ref(),)*
                }
            }

            /// 是否**统计类**（计数后继续，绝不因此断流）。
            pub const fn is_statistical(&self) -> bool {
                match self {
                    $(Self::$variant { .. } => $statistical,)*
                }
            }
        }
    };
}

define_net_errors! {
    /// UDP 端口绑定失败（被占用 / 无权限 / 地址不可用）。
    #[error("1008 BAD_REQUEST (endpoint bind): {context}")]
    Bind => bind, bind_owned, BadRequest, false;
    /// 本机配置非法（证书 / 私钥 DER 无法装载，或传输参数越界）。
    #[error("1008 BAD_REQUEST (net config): {context}")]
    Config => config, config_owned, BadRequest, false;
    /// QUIC/TLS 握手失败（对端拒绝、超时、协议不符）。
    #[error("1004 AUTH_FAILED (handshake): {context}")]
    Handshake => handshake, handshake_owned, AuthFailed, false;
    /// 取不到对端证书（未做双向认证 / 证书链为空 / 身份类型不符）。
    #[error("1004 AUTH_FAILED (peer identity): {context}")]
    PeerIdentity => peer_identity, peer_identity_owned, AuthFailed, false;
    /// 连接已关闭，或收发过程中失败。
    #[error("1008 BAD_REQUEST (transport): {context}")]
    Transport => transport, transport_owned, BadRequest, false;
    /// 对端不支持 QUIC 数据报（RFC 9221 未协商成功）。
    #[error("1005 CAP_UNSUPPORTED (datagram): {context}")]
    DatagramUnsupported => datagram_unsupported, datagram_unsupported_owned, CapUnsupported, false;
    /// 数据报超过 `min(DATAGRAM_MAX_LEN, max_datagram_size())`（§3 的 MTU 预算与 QUIC 实测值取 min）。
    #[error("1008 BAD_REQUEST (datagram too large): {context}")]
    DatagramTooLarge => datagram_too_large, datagram_too_large_owned, BadRequest, true;
    /// 调用方缓冲装不下收到的数据报 —— **不得静默截断**，该数据报被丢弃并计数。
    #[error("1008 BAD_REQUEST (datagram buffer too small): {context}")]
    DatagramBufferTooSmall => datagram_buffer_too_small, datagram_buffer_too_small_owned, BadRequest, true;
    /// 控制帧载荷超过 `CONTROL_MAX_PAYLOAD`（§4：单帧上限 64 KiB）。
    #[error("1008 BAD_REQUEST (control payload too large): {context}")]
    ControlPayloadTooLarge => control_payload_too_large, control_payload_too_large_owned, BadRequest, false;
    /// 控制流帧边界不可信（截断 / `payload_len` 与剩余字节不符 / 主版本字节不是 `0x02`）。
    #[error("1008 BAD_REQUEST (control frame malformed): {context}")]
    ControlMalformed => control_malformed, control_malformed_owned, BadRequest, false;
}

#[cfg(test)]
mod tests {
    // 测试代码不受实时路径的 unwrap / expect / panic 禁令约束（那三条针对运行时音频路径）
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// 全部变体（新增变体时这里会漏，上面的 `assert_eq!(all.len(), N)` 会立刻失败）。
    fn all() -> Vec<NetError> {
        vec![
            NetError::bind("x"),
            NetError::config("x"),
            NetError::handshake("x"),
            NetError::peer_identity("x"),
            NetError::transport("x"),
            NetError::datagram_unsupported("x"),
            NetError::datagram_too_large("x"),
            NetError::datagram_buffer_too_small("x"),
            NetError::control_payload_too_large("x"),
            NetError::control_malformed("x"),
        ]
    }

    /// `Display` 里硬编码了错误码前缀，这里把「消息」与「`code()`」钉在一起，防止将来改码忘了改消息。
    #[test]
    fn 显示前缀与错误码一致() {
        let cases = all();
        assert_eq!(cases.len(), 10, "新增了变体就要把它加进 all()");
        for err in cases {
            let prefix = format!("{} {}", err.code().as_u16(), err.code().name());
            let text = err.to_string();
            assert!(text.starts_with(&prefix), "{text} 未以 {prefix} 开头");
            assert!(text.ends_with("x"), "{text} 未携带上下文");
            assert_eq!(err.context(), "x");
        }
    }

    #[test]
    fn 码映射与统计口径固定() {
        assert_eq!(NetError::bind("x").code(), ErrorCode::BadRequest);
        assert_eq!(NetError::handshake("x").code(), ErrorCode::AuthFailed);
        assert_eq!(NetError::peer_identity("x").code(), ErrorCode::AuthFailed);
        assert_eq!(
            NetError::datagram_unsupported("x").code(),
            ErrorCode::CapUnsupported
        );

        // 只有音频数据报路径上的丢帧算统计类：会话语义不受影响
        assert!(NetError::datagram_too_large("x").is_statistical());
        assert!(NetError::datagram_buffer_too_small("x").is_statistical());
        // 控制帧是状态机的输入，丢一帧等于卡住 → 必须让上层看见
        assert!(!NetError::control_payload_too_large("x").is_statistical());
        assert!(!NetError::control_malformed("x").is_statistical());
        assert!(!NetError::transport("x").is_statistical());
        assert!(!NetError::bind("x").is_statistical());
    }

    #[test]
    fn 动态上下文构造器可用() {
        let err = NetError::datagram_too_large_owned(format!("{} > {}", 1163, 1162));
        assert_eq!(err.context(), "1163 > 1162");
        assert_eq!(err.code(), ErrorCode::BadRequest);
    }
}
