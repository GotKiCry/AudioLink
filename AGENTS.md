# AGENTS.md — AudioLink(仓库根)

局域网实时音频分发:Windows ⇄ Android 互推音频,QUIC + Opus,组内同步 ±10 ms。
本文件是**导航图**:先按这里定位,细节去指向的文件读,不要全仓库乱翻。

## 布局地图

| 路径 | 是什么 |
|---|---|
| `Cargo.toml` | Rust workspace 根(成员 = `core/crates/*` + `desktop/src-tauri`),版本单一来源 |
| `core/crates/audiolink-types` | 协议常量/枚举/错误码(零依赖) |
| `core/crates/audiolink-proto` | ALP/2 编解码(L1 严格解码)+ golden vectors |
| `core/crates/audiolink-audio` | 采集/播放、重采样、Opus、抖动缓冲、混音 |
| `core/crates/audiolink-net` | quinn QUIC、FEC/NACK、时钟同步 |
| `core/crates/audiolink-discovery` | mDNS + UDP 广播兜底 |
| `core/crates/audiolink-identity` | 自签证书、节点身份 |
| `core/crates/audiolink-engine` | 编排层:引擎/会话/同步组/遥测/事件总线 |
| `core/crates/audiolink-ffi` | UniFFI 导出供 Android(`bindgen` feature = 绑定生成器) |
| `core/crates/audiolink-tools` | 开发 bins:`alp2-dump`、`latency-probe`、`self-loop`、`soak-runner` |
| `desktop/` | Tauri 2 + React 桌面端 → 见 `desktop/AGENTS.md` |
| `android/` | Compose + UniFFI 移动端 → 见 `android/AGENTS.md` |
| `tools/` | PowerShell 运维脚本(gradlew 包装、版本校验、打包、许可审计) |
| `docs/` | 契约文档体系,`00-overview.md` 是文档地图 |

依赖方向单向:`types → proto → audio/net/discovery/identity → engine → ffi → {desktop, android}`;`audio`/`net` 互不依赖。

## 动手前

1. 按序读契约:`docs/00-overview.md` → `docs/01-requirements.md`(FR 编号即契约)→ `docs/02-architecture.md` → `docs/03-protocol.md`;协作约定见 `CONTRIBUTING.md`。
2. 跨端契约(协议/视图形状/FFI 面)改动必须三端齐动 + 同步 golden vectors。

## 门禁(仓库根,提交前全绿)

```powershell
cargo fmt --all --check        # 非可选
cargo clippy --workspace --exclude audiolink-desktop --all-targets --all-features -- -D warnings
cargo nextest run --workspace --exclude audiolink-desktop --lib --tests
cargo clippy -p audiolink-desktop --all-targets --all-features -- -D warnings
cargo test -p audiolink-desktop
cd desktop; pnpm build
```

别名(`.cargo/config.toml`):`cargo xt` = nextest,`cargo lint` = clippy。发布前加跑 `pwsh tools/check-version.ps1`。

## 红线(违反即返工)

- 不引需要 CMake/NASM/Perl 的依赖;rustls 强制 ring 后端(不开默认 features、不开 `quinn/rustls-aws-lc-rs`)。
- workspace lints 已 deny `unwrap_used`/`expect_used`/`panic`/`unsafe_code`;必需时局部 `allow` 并注明理由。
- Android `targetSdk` 锁 36(ADR-009),`minSdk` 26;ABI 仅 arm64-v8a + armeabi-v7a。
- 行尾一律 LF(仅 `*.bat/*.cmd/*.ps1` 用 CRLF);提交信息用 Conventional Commits。

## 环境速记(Windows)

- Gradle 一律走 `pwsh tools/gradlew.ps1 <task>`(自动处理 JDK/ANDROID_HOME);JDK 路径禁写进 `android/gradle.properties`。
- 设 `GRADLE_USER_HOME=C:\_Project\AudioLink\.gradle-home` 隔离全局凭证。
- cargo 产物在根 `target/`;Android .so 输出到 `android/app/src/main/jniLibs/`(`target-android/` 是遗留)。
- CI 只有 `.github/workflows/release.yml`;README/docs 中其他工作流描述已过时。
