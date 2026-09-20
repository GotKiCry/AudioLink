# AudioLink

[![Release](https://img.shields.io/github/v/release/GotKiCry/AudioLink)](https://github.com/GotKiCry/AudioLink/releases/latest)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Android-informational.svg)](#)

> **局域网实时音频分发系统** —— 把任意节点的声音，低延迟、可同步、可混合地推给局域网内任意数量的其他节点。
> Windows ⇄ Android 双向，多设备并发，组内同步 ±10 ms，Opus 编码，全链路加密。

**状态：`v0.1.0` 已可用** —— Windows 安装包与绿色版、Android APK（arm64-v8a / armeabi-v7a）均已产出，
内核测试 437 项、CI 五个 job 全绿。**用户怎么用请看 [`docs/manual/`](docs/manual/)。**

---

## 与上游项目的关系

AudioLink 参考了 **[HeHang0/AudioShare](https://github.com/HeHang0/AudioShare)**（Apache-2.0）的产品形态与部分设计思路（声道分流、远端音量、局域网自动发现），**但为全新实现**：

| | AudioShare / AudioPipe（上游） | AudioLink |
|---|---|---|
| 拓扑 | Windows → Android 单向 | **任意节点互推**（含 Android → Android） |
| 编码 | 裸 PCM16（1.54 Mbps/设备） | **Opus 48k/20ms**（默认 160 kbps，可切 PCM 无损档） |
| 传输 | 裸 TCP + 短连接控制 | **QUIC**（数据报 + 可靠流） |
| 同步 | 事后 seek 拉平 | **时钟同步 + 预约播放**（组内 ±10 ms） |
| 稳定性 | 无抖动缓冲，忙则丢帧；异常即静音 | 抖动缓冲 + 丢包隐藏 + 断线自愈 + 遥测 |
| 安全 | 无鉴权、无加密、开放 HTTP 管理面 | **自签证书 + TOFU + PIN 配对**，无 HTTP 面 |
| 维护 | 2024-05 起停更，issue 无人处理 | 全新代码库，CI/测试/遥测齐备 |

上游代码为 Apache-2.0，本项目的许可与归属声明见 [`LICENSE`](LICENSE) 与 [`NOTICE`](NOTICE)。**不采用**上游协议、不包含云音乐接口调用、不包含斐讯 R1 私有集成与 adb 静默安装流程。

---

## 用户文档

| 文档 | 内容 |
|---|---|
| [用户手册 · 中文](docs/manual/user-guide.zh-CN.md) / [English](docs/manual/user-guide.en-US.md) | 装哪个版本、第一次怎么配对、推流与接收、同步组、遥测、配置位置、卸载 |
| [故障排查 · 中文](docs/manual/troubleshooting.zh-CN.md) / [English](docs/manual/troubleshooting.en-US.md) | 连不上、没声音、卡顿、MIUI 安装 -99、更新失败…… 每条都来自真实踩坑 |
| [贡献指南](CONTRIBUTING.md) | 环境、门禁、提交与文档约定 |

---

## 核心能力

| 能力 | 说明 |
|---|---|
| 双向网状 | 每个节点都能发也能收（**连接由接收端发起，主机只被连**）；PC ⇄ 手机、手机 ⇄ 手机 |
| 多设备并发 | 单发送端 ≥ 8 个接收端；接收端可同时接收 ≥ 4 路并混音 |
| 多房间同步 | 发送端勾选设备即成临时同步组，组内偏差 ≤ ±10 ms（P95） |
| 低延迟 | 端到端 P50 **80–110 ms**（Wi-Fi）/ **40–55 ms**（有线·理想），P95 ≤ 150 ms |
| 自适应抗弱网 | 抖动缓冲 + 默认冗余双发 + PCM 丢包掩盖 + NACK + 码率自适应（160→96 kbps） |
| 声道路由 | 立体声 / 仅左 / 仅右 / 左→A 设备 + 右→B 设备 |
| 手机内录 | Android 10+ 系统内录（AudioPlaybackCapture）作为发送源 |
| 可观测 | 内置遥测面板（RTT/抖动/丢包/码率/缓冲/端到端延迟）+ 日志导出 |
| 安全 | QUIC TLS1.3 + 自签证书指纹 + 6 位 PIN 配对白名单 |

**明确不做**：云音乐播放、音乐库、灯效、USB/adb 通道、远程 Web 管理页、跨公网穿透、iOS/Web 端。

---

## 技术栈

| 层 | 选型 |
|---|---|
| 跨平台内核 | **Rust**（`core/crates/*`）：协议 / Opus / 抖动缓冲 / 时钟同步 / 混音 / 发现 / 配对 |
| 桌面端 | **Tauri 2 + React 19 + TypeScript + Tailwind 4**（托盘 / 自启 / 自动更新） |
| Android 端 | **Kotlin + Jetpack Compose（Material 3）**，经 **JNI/UniFFI** 调用同一内核 |
| 音频 | Windows：`wasapi`（loopback 采集 + render 播放）；Android：`AudioPlaybackCapture` / `AudioRecord` / `AudioTrack` |
| 编解码 | `opus-rs`（纯 Rust，零 C 依赖） |
| 传输 | `quinn`（QUIC：音频走数据报，控制走可靠流） |
| 版本矩阵 | compileSdk 37 / **targetSdk 36** / **minSdk 26**（API 26 = 低延迟播放门槛）/ AGP 9.4 / Gradle 9.7.1 / JDK 17 |

> `targetSdk` **必须停在 36**：目标 SDK 37 会强制 `ACCESS_LOCAL_NETWORK` 运行时权限，未授权时局域网收发全部失败。详见 `docs/04-tech-stack.md` ADR-009。

---

## 目录结构

```
AudioLink/
├─ Cargo.toml                 # Rust workspace 根（成员：core/crates/* + desktop/src-tauri）
├─ rust-toolchain.toml        # 工具链锁定（可复现构建）
├─ core/crates/               # 跨平台内核（9 个 crate，依赖单向）
│  ├─ audiolink-types         #   常量 / 枚举 / 错误码 / 遥测结构
│  ├─ audiolink-proto         #   ALP/2 协议编解码 + golden vectors
│  ├─ audiolink-audio         #   采集播放抽象 / Opus / 抖动缓冲 / 混音
│  ├─ audiolink-net           #   QUIC 会话 / FEC-NACK / 时钟同步
│  ├─ audiolink-discovery     #   mDNS + UDP 广播发现
│  ├─ audiolink-identity      #   证书 / 指纹 / 信任库 / PIN 配对
│  ├─ audiolink-engine        #   编排：会话 / 同步组 / 遥测
│  ├─ audiolink-ffi           #   UniFFI 导出（供 Kotlin 调用）
│  └─ audiolink-tools         #   alp2-dump / latency-probe / soak-runner
├─ desktop/                   # Tauri 2 桌面端（src/ + src-tauri/）
├─ android/                   # Kotlin + Compose 应用（含 cargo-ndk 构建脚本）
├─ docs/                      # 规划文档（见下）
├─ tools/                     # 校验与测量脚本
└─ .github/workflows/         # CI 与发布流水线
```

---

## 文档索引

| 文档 | 内容 |
|---|---|
| [`docs/00-overview.md`](docs/00-overview.md) | 一页纸总览：定位、边界、关键决策速查 |
| [`docs/01-requirements.md`](docs/01-requirements.md) | 需求规格（FR-01…FR-40、NFR、验收标准、决策追溯表） |
| [`docs/02-architecture.md`](docs/02-architecture.md) | 总体架构：分层、线程模型、同步、混音、可观测性 |
| [`docs/03-protocol.md`](docs/03-protocol.md) | **ALP/2 协议规格**（字节级帧格式、命令表、时钟同步、测试向量） |
| [`docs/04-tech-stack.md`](docs/04-tech-stack.md) | 技术选型 ADR 与版本矩阵（含回退路径） |
| [`docs/05-roadmap.md`](docs/05-roadmap.md) | M0–M5 里程碑、验收标准、风险清单 |
| [`docs/06-dev-environment.md`](docs/06-dev-environment.md) | 开发环境、构建命令、踩坑清单、CI 设计 |
| [`docs/07-migration-notes.md`](docs/07-migration-notes.md) | 从 AudioPipe 迁移：继承什么、绝不再犯什么、合规红线 |
| [`docs/08-ui-spec.md`](docs/08-ui-spec.md) | 桌面端与 Android 端 UI 规格、文案与视觉验收 |
| [`docs/09-benchmark-notes.md`](docs/09-benchmark-notes.md) | 对标研究（Snapcast / Jamulus / AudioRelay 等）与设计约束 |

---

## 快速开始（开发环境）

```powershell
# 1) 环境准备（一次性，详见 docs/06-dev-environment.md）
[Environment]::SetEnvironmentVariable('JAVA_HOME','C:\Users\<你>\scoop\apps\corretto17-jdk\current','User')  # 必须 JDK 17
[Environment]::SetEnvironmentVariable('ANDROID_HOME',"$env:LOCALAPPDATA\Android\Sdk",'User')
rustup target add armv7-linux-androideabi
cargo install cargo-ndk --locked

# 2) 内核：格式 + 静态检查 + 测试（含协议 golden vectors）
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace --exclude audiolink-desktop

# 3) 桌面端
cd desktop; pnpm install; pnpm tauri dev

# 4) Android（先编 Rust 内核，再打 APK）
cd ..\android
pwsh .\scripts\build-rust.ps1 -Abi arm64-v8a,armeabi-v7a
pwsh ..\tools\gradlew.ps1 assembleDebug   # 包装脚本：自动校验 JDK 17+ 并注入 ANDROID_HOME
```

---

## 路线图（摘要）

| 里程碑 | 主题 | 退出条件 |
|---|---|---|
| M0 | 地基（协议 + CI + 工具） | `cargo test` 全绿，协议向量 round-trip 通过 |
| M1 | 单链路打通 | PC → 单台 Android，P50 ≤ 110 ms（Wi-Fi） |
| M2 | 稳定性与质量 | 弱网（5 Mbps / 2% 丢包 / 30 ms 抖动）可听；8 h soak 无故障 |
| M3 | 多设备与同步 | 3 台并发；组内 ±10 ms（双机录音验证） |
| M4 | 网状与混音 | 手机内录 → PC；手机 → 手机；4 路混音无爆音 |
| M5 | 产品化 | 安装包 + 绿色版 + 双 ABI APK；自动更新可用 |

---

## 已知限制

- **`targetSdk` 停在 36**：升级到 37 需先实现 `ACCESS_LOCAL_NETWORK` 权限门禁（技术债已登记）。
- **Android 8.0+（API 26）**：低延迟播放模式（`PERFORMANCE_MODE_LOW_LATENCY`）自 API 26 起可用，故最低版本与低延迟能力绑定。
- **系统内录需 Android 10+（API 29）**，且目标应用可声明禁止被捕获（DRM 内容不可捕获）。
- **蓝牙输出无法达成低延迟目标**：A2DP/SBC/AAC/LDAC 本体延迟 100–300 ms 属物理限制，UI 会明确提示；达标需内置扬声器 / 有线 / USB DAC / LE Audio（Android 13+）。
- **Android 端无开机自启**（系统已禁止由 `BOOT_COMPLETED` 启动媒体前台服务，且本项目不需要）。
- **便携版不参与自动更新**（文件占用限制），安装版支持。
- 局域网内依赖 mDNS 或广播发现；若 AP 开启客户端隔离，需手工添加 IP。

---

## 许可

[Apache License 2.0](LICENSE)。归属与第三方声明见 [`NOTICE`](NOTICE)。
"AudioShare"、"AudioPipe"、"picapico"、"Phicomm" 等名称与商标归各自权利人所有，本项目不使用。
