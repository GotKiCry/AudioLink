# M5 · 桌面外壳：常驻托盘 + 开机自启 + 优雅退出

> 日期 **2026-09-16**。`desktop/src-tauri/src/lib.rs` 里挂着三条 `TODO(M5)`，这一轮做掉两条，
> 外加把「开机自启」从 M5 大项里落地 —— 它们本来就属于同一批：**这个应用该怎么"常驻"**。

---

## 1. 三件事与它们的理由

| 改动 | 为什么 |
|---|---|
| **关窗 ≠ 退出**：`CloseRequested` 里 `prevent_close()` + `hide()` | AudioLink 是后台常驻的音频工具：关掉窗口不代表用户想停掉正在播放的音频 |
| **托盘菜单**：显示主窗口 / 退出；左键唤出窗口、右键出菜单 | 窗口藏起来之后，托盘是用户唯一能**叫回窗口**和**真正退出**的地方 |
| **退出前优雅收尾**：`quit_app` 先 `engine.shutdown()` 再 `exit(0)` | 这条最容易一直没人管，而它的代价落在**对端**身上 —— 对端要多等一次 QUIC 空闲超时才能把会话判掉 |
| **开机自启开关**（`autostart_enabled` / `set_autostart`） | FR-31 的一部分；插件早就注册了，但一直没有入口 |

### 1.1 两个刻意的细节

**退出路径带 3 秒上限**：`shutdown_engine` 内部用 `tokio::time::timeout` 包住 `engine.shutdown()`。
退出流程不能因为对端没响应就卡住用户 —— 超时就超时，照样退出，但会在 stderr 留一行。

**自启状态从系统读，不本地记**：界面上那个勾是 `autolaunch().is_enabled()` 的结果。
用户在系统设置里改过之后，这里显示的必须仍然是**真实状态**，而不是应用自己以为的样子。

### 1.2 顺手清掉一处重复定义

`tauri.conf.json` 里原本有 `app.trayIcon`（图标）配置，而这一轮托盘改由 Rust 侧建（要挂菜单与事件）。
两处都用 `id = "main"` → 同一个托盘被定义两遍。已**删掉配置里那份**，图标改取 `default_window_icon()`：
一处定义，不留"到底哪个生效"这种问题。

## 2. 实测

| 核对项 | 结果 |
|---|---|
| `cargo check -p audiolink-desktop` | 通过（含 `generate_context!` 对配置的编译期校验） |
| `cargo test -p audiolink-desktop` | **23 项通过** |
| `pnpm build`（tsc 严格 + vite） | 通过 |
| `pwsh tools/tauri-build.ps1`（release 打包） | 成功：`AudioLink_0.1.0_x64-setup.exe` **3 793.5 KB** + `.sig` |
| 安装包内容（`7z l` 直读） | `audiolink-desktop.exe`（15 181 824 B）+ **根级** `THIRD-PARTY-NOTICES.md` |

## 3. 没验的（诚实清单）

- **托盘图标在真机上的显示、左键唤出、右键菜单**：都需要 GUI 会话，无法在构建环境里自动验证；
- **关窗隐藏后进程仍存活**：同上；
- **退出时对端真的收到 `BYE`**：引擎侧的优雅关闭本来就有端到端用例（`core/crates/audiolink-engine/tests/engine_shutdown.rs`），
  本轮接的是「托盘 / 窗口」这条**调用路径** —— 那条路径本身只有真机点一次才能确认；
- **自启在系统里的实际效果**（注册表项 / 启动项）：需要在真机上看一眼系统设置。
