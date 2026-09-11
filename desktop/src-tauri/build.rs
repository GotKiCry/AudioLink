// Tauri 构建脚本：生成 context（读取 tauri.conf.json、内嵌图标/资源、生成权限 schema）
//
// 缺这个文件会直接编译失败：
//   error: OUT_DIR env var is not set, do you have a build script?
//   --> src/lib.rs: tauri::generate_context!()
//
// 这是 Tauri 2 的硬要求（tauri-build 必须作为 build-dependency 并在此调用）。
fn main() {
    tauri_build::build()
}
