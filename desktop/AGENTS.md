# AGENTS.md — desktop(Tauri 桌面端)

Tauri 2 桌面外壳(仅 Windows x64)。只做**编排 + UI 适配**,音频/网络在 `audiolink-engine`。仓库级约定见根 `AGENTS.md`。

## 布局地图

| 路径 | 是什么 |
|---|---|
| `src-tauri/src/lib.rs` | 只声明 command、注册 handler、装配插件;**不写业务逻辑** |
| `src-tauri/src/engine_bridge.rs` | 外壳 ⇄ 引擎唯一接缝:事件翻译/状态映射/错误人话化 |
| `src-tauri/src/view.rs` | 前端视图形状,契约冻结于 `docs/11-m1-contract.md` §6 |
| `src-tauri/src/error.rs` | 前端可读错误 `{code, message, context}` |
| `src-tauri/src/{capture,discovery,settings,update}.rs` | WASAPI 枚举 / 发现 / 设置 / 更新 |
| `src-tauri/tests/engine_seam.rs` | 接缝验证:两个真实 Engine 走 127.0.0.1 真 QUIC |
| `src-tauri/tauri.conf.json` | 窗口/CSP/bundle(仅 nsis)/updater 配置 |
| `src-tauri/capabilities/default.json` | 最小权限,刻意不给前端 `updater:*` |
| `src/lib/ipc.ts` | 前端**唯一**碰 `invoke`/`listen` 处;事件 `audiolink://peer\|telemetry\|groups` |
| `src/lib/useAudioLink.ts` | 唯一状态 hook(不用 zustand/jotai,文件头有理由) |
| `src/i18n.ts` | 中英文案唯一来源;缺键 = 编译错误 |
| `src/components/` | 组件与同目录 `X.test.tsx` |
| `index.html` | 头部注释冻结设计语言(「播出调音台」,以此为准,勿看 `docs/design/fluent-2.md`) |

契约铁律:UI 只走 command/event;视图/命令签名改动需三端齐动。`stream_id` 等 snake_case 字段是契约原文,勿「修正」。

## 命令

```bash
# desktop/ 目录
pnpm install              # CI 用 --frozen-lockfile
pnpm tauri dev            # vite 1420 + Rust 外壳
pnpm tauri build          # NSIS;等价 pwsh ../tools/tauri-build.ps1(有 TAURI_SIGNING_PRIVATE_KEY 时带更新签名)
pnpm build                # 门禁:tsc strict + i18n 一致性 + vite
pnpm test                 # Vitest(jsdom)

# 仓库根(产物落根 target/)
cargo build -p audiolink-desktop
cargo test -p audiolink-desktop
cargo clippy -p audiolink-desktop --all-targets --all-features -- -D warnings
```

发布链路(仓库根):`tools/tauri-portable.ps1 [-Verify]`、`tools/tauri-latest-json.ps1`、`tools/check-update.mjs`、`tools/updater-local-e2e.ps1`。

## 注意

- `[profile.dev.package.audiolink-desktop] incremental = false` 修 Windows LNK 问题,`tauri dev` 依赖它,勿删;`src-tauri/build.rs` 必须存在。
- 无 eslint/prettier:门禁 = tsc(strict + noUncheckedIndexedAccess)+ `i18n:check` + vitest。
- 文案一律走 `i18n.ts` 中英双给(`i18n-literal-guard.test.ts` 抓硬编码);契约要求 snake_case 的命令用 `#[tauri::command(rename_all = "snake_case")]`。
- 前端栈:React 19 + TS + Vite(target `chrome120`,端口 1420)+ Tailwind 4,pnpm;数据写 `%APPDATA%\com.gotkicry.audiolink`。
