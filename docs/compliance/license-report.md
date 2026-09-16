# 依赖许可审计报告（自动生成）

> 由 `tools/license-audit.ps1` 生成，**请勿手改**：改策略去改 `core/crates/audiolink-tools/src/license.rs` 的白/黑名单。
> 判定语义：`OR` 任一可用即可、`AND` 全部必须可用、`WITH` 当原子；
> 未声明的许可以及 `AND`/`OR` 混用的表达式一律进 `notice`（交给人看，不自动放行也不自动拒绝）。

| 来源 | 包数 | allowed | notice | denied |
|---|---|---|---|---|
| Rust（cargo metadata） | 635 | 620 | 15 | 0 |
| 前端（pnpm licenses） | 88 | 86 | 2 | 0 |

**Rust：无 `denied` 依赖。**

### Rust · notice（15 个，需要人看一眼）

| 包 | 版本 | 许可 |
|---|---|---|
| cssparser | 0.36.0 | MPL-2.0 |
| cssparser-macros | 0.6.1 | MPL-2.0 |
| dtoa-short | 0.3.5 | MPL-2.0 |
| option-ext | 0.2.0 | MPL-2.0 |
| selectors | 0.36.1 | MPL-2.0 |
| unicode-ident | 1.0.24 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| uniffi | 0.29.5 | MPL-2.0 |
| uniffi_bindgen | 0.29.5 | MPL-2.0 |
| uniffi_core | 0.29.5 | MPL-2.0 |
| uniffi_internal_macros | 0.29.5 | MPL-2.0 |
| uniffi_macros | 0.29.5 | MPL-2.0 |
| uniffi_meta | 0.29.5 | MPL-2.0 |
| uniffi_pipeline | 0.29.5 | MPL-2.0 |
| uniffi_udl | 0.29.5 | MPL-2.0 |
| webpki-root-certs | 1.0.9 | CDLA-Permissive-2.0 |

**前端：无 `denied` 依赖。**

### 前端 · notice（2 个，需要人看一眼）

| 包 | 版本 | 许可 |
|---|---|---|
| lightningcss | 1.32.0, 1.33.0 | MPL-2.0 |
| lightningcss-win32-x64-msvc | 1.32.0, 1.33.0 | MPL-2.0 |

## 覆盖面（诚实清单）

- **已覆盖**：Rust workspace 的全部依赖（`cargo metadata`）、桌面前端依赖（`pnpm licenses`）；
  Android（Gradle/Maven）依赖见**文末专节**（该节存在与否取决于是否采集过）。
- **未覆盖**：随包分发的二进制（.exe / .apk 内的第三方库）、字体与图标资源、以及 Android 侧的**投放位置**（声明入口）。
- 本报告回答的是「许可是否允许这样分发」；署名/免责文本的**实际投放位置**是另一件事。
## Android（Gradle/Maven）依赖

> 采集：解析 `gradlew :app:dependencies` 的依赖树 + 读 Gradle 缓存里 POM 的 `<licenses>`；
> POM 写的是自然语言许可名，规范化成 SPDX 与判定都在本 crate 里完成（可离线单测）。

| 指标 | 值 |
|---|---|
| 组件 | 114 |
| allowed | 113 |
| notice | 1 |
| denied | 0 |

| 包 | 版本 | 许可 | 判定 |
|---|---|---|---|
| com.google.guava:listenablefuture | 1.0 | (未声明) | notice |

