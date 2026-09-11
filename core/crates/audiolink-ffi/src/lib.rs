//! audiolink-ffi —— UniFFI 导出（供 Kotlin 调用）
//!
//! 规格：docs/02-architecture.md、docs/03-protocol.md

#![deny(unsafe_code)] // 必须使用 unsafe 的 crate（如 FFI 绑定）在文件顶部显式 #[allow] 并注明理由
#![deny(clippy::unwrap_used, clippy::expect_used)] // 实时路径禁止 panic；确需处用 #[allow] 并注明理由

// TODO(M0): 按 docs 里的模块划分填充实现
