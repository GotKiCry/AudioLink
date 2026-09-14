//! UniFFI 绑定生成器入口（**不进 Android 产物**，由 `bindgen` feature 门控）。
//!
//! 用法（在仓库根执行）：
//!
//! ```powershell
//! cargo build -p audiolink-ffi                     # 先产出宿主平台的 cdylib
//! cargo run -p audiolink-ffi --features bindgen --bin uniffi-bindgen -- `
//!   generate --library target/x86_64-pc-windows-msvc/debug/audiolink_ffi.dll `
//!   --language kotlin --out-dir core/crates/audiolink-ffi/bindings/kotlin `
//!   --config core/crates/audiolink-ffi/uniffi.toml
//! ```
//!
//! 走 `--library` 模式：接口元数据由 `uniffi::setup_scaffolding!()` + 各 `#[uniffi::export]`
//! 嵌在 cdylib 里，所以不需要 UDL 文件，也就不需要第二套脚手架生成链路。

fn main() {
    uniffi::uniffi_bindgen_main()
}
