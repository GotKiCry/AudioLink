//! audiolink-ffi —— UniFFI 导出（供 Kotlin / 未来 Swift 调用）
//!
//! 规格：`docs/02-architecture.md`（§1 第 3 条线程纪律、§11 错误处理）、`docs/11-m1-contract.md` §8。
//!
//! # 本 crate 的位置
//!
//! ```text
//! ... ──► engine ──► **ffi** ──► { android(Kotlin) , desktop/src-tauri }
//! ```
//!
//! 它是**唯一的跨语言边界**：把 `audiolink-engine` 的 Rust API 翻成 UniFFI 类型，
//! 并把两端的错误收敛成契约 §1 的 `{code, message, context}` 形状。
//!
//! # 为什么这里有 `unsafe`
//!
//! 这是全工作区**唯一**允许 `unsafe` 的 crate（契约 §8），理由是编译器与 UniFFI 的硬要求，
//! 而不是本 crate 手写了 `unsafe` 代码：
//!
//! 1. `uniffi::setup_scaffolding!()` 与各 `#[uniffi::export]` 宏会生成
//!    `#[unsafe(no_mangle)] pub extern "C" fn ...` —— Rust 2024 起，`no_mangle` / `export_name` /
//!    `link_section` 这些属性被归入 `unsafe` 范畴，必须写成 `#[unsafe(...)]`；
//! 2. 工作区在 `[workspace.lints.rust]` 里把 `unsafe_code` 设为 `deny`（根 `Cargo.toml` 第 57 行），
//!    cargo 以 `-D unsafe-code` 传给 rustc。**实测**：去掉本行的 `allow` 后编译直接失败
//!    （`error: declaration of a no_mangle function ... requested on the command line with -D unsafe-code`），
//!    加上后通过 —— 即 crate 级 `allow` 能且必须覆盖工作区级 `deny`。
//!
//! 因此这里的 `#![allow(unsafe_code)]` 只覆盖「导出符号需要 `no_mangle`」这一件事；
//! 本 crate 内**没有任何手写的 `unsafe` 块**。
#![allow(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

uniffi::setup_scaffolding!();

pub mod audio_bridge;
pub mod engine_bridge;
pub mod error;
pub mod golden;
// 只在测试里编译：它扫源码与**入库的生成物**，是护栏而不是运行时功能（踩坑记录见其模块文档）。
#[cfg(test)]
mod kotlin_guard;

pub use audio_bridge::{PcmFeed, PcmPull};
pub use engine_bridge::{
    EngineStartConfig, LocalStatus, PeerView, TelemetryView, connect, displayed_pin, engine_start,
    engine_stop, local_status, peers, start_send, stop_send, submit_pin, telemetry,
};
pub use error::FfiError;
pub use golden::protocol_self_test;
