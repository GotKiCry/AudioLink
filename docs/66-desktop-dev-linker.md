# 桌面开发启动的 MSVC 链接错误

2026-09-21，执行 `cd desktop; npm run tauri:dev` 可复现：前端 Vite 正常启动，Rust 构建 `audiolink_desktop_lib.dll` 时出现 `LNK2001` / `LNK2019`，最终 `LNK1120: 50`。缺失符号集中在 `anon.*.llvm.*` 及 Rust 生成的析构、排序函数。

上一轮用进程内 `CARGO_INCREMENTAL=0` 跑桌面测试能够通过，但没有把这一设置带入日常开发命令，因此用户启动仍然失败。

修复放在 workspace 根 `Cargo.toml`：

```toml
[profile.dev.package.audiolink-desktop]
incremental = false
```

桌面库及可执行程序在开发构建中不再复用出错的增量对象，继承 dev 的 test profile 同样适用。核心 crate 的增量构建、依赖优化等级及 release 构建配置沿用原配置。代价是修改桌面 Rust 源码后需要重编整个桌面 crate；未变化的依赖仍可复用。

验证使用原始 `npm run tauri:dev`，不设置 `CARGO_INCREMENTAL`、不清空整个 `target`：

- 重编时核对桌面 `rustc` 命令行已无 `-C incremental`。
- 开发构建 58.56 s 完成，实际运行 `audiolink-desktop.exe`；AudioLink 窗口句柄非零且 `Responding=True`。
- `http://localhost:1420/` 返回 HTTP 200。
- `cargo fmt --all --check`、桌面 `clippy --all-targets --all-features -- -D warnings` 通过；直接执行 `cargo test -p audiolink-desktop`：49 项通过、1 项忽略，无需临时环境变量，日志为 `target/evidence/desktop-profile-tests.log`。
- 验证完毕后结束本轮启动的开发进程，释放 1420 端口，用户可以直接重新执行启动命令。

本机证据日志：`target/evidence/desktop-dev-before.log`、`target/evidence/desktop-dev-after.log`。
