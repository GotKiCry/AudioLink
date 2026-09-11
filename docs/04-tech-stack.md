# AudioLink 技术选型（ADR 汇总）

> 每条决策含：结论 / 理由 / 备选 / 代价。版本号核验日期 **2026-09-11**（以调研报告为准）。
> 版本号规则：本文件是**唯一版本来源**，CI 与各清单必须与本表一致。

---

## 1. 版本矩阵（冻结线）

| 组件 | 版本 | 备注 |
|---|---|---|
| Rust | **1.96.1**（MSVC host） | `rust-toolchain.toml` 锁定；Android 目标已装 `aarch64-linux-android` |
| edition | 2024 | 新代码库不背旧 edition |
| Tauri | **2.11.5**（CLI 2.11.4，api 2.11.1） | 托盘用内置 `tray-icon` feature |
| Node / 包管理 | Node **24.13.0** / pnpm **11.22.0** | 本机已装 |
| React / TS / Vite / Tailwind | React **19.3** / TS 最新 / Vite **8.3** / Tailwind **4.3** | 配 shadcn/ui |
| Android AGP | **9.4.0** | 要求 Gradle ≥ 9.6、JDK 17、BuildTools ≥ 36.0.0 |
| Gradle | **9.7.1**（wrapper 锁定） | 首次构建需联网拉取 |
| Kotlin | **2.4.20** | AGP 9 内置 Kotlin 支持（不再单独应用 Kotlin 插件） |
| Compose BOM | **2026.09.00**（ui 1.12.1 / material3 1.4.0） | 强制 `compileSdk 37` |
| compileSdk / targetSdk / minSdk | **37 / 36 / 26** | **targetSdk 必须停在 36**（见 ADR-009）；**minSdk 26** = 低延迟播放模式门槛 |
| JDK | **17**（构建） | 本机须把 `JAVA_HOME` 从 JDK 11 改到 Corretto 17 |
| NDK | **28.2.13676358** | 本机已装 |
| 双 ABI | `arm64-v8a` + `armeabi-v7a` | 由 `cargo-ndk` 产出 |

---

## ADR-001 · 内核语言与跨端复用

**结论**：Rust 内核（`core/` workspace），桌面端 in-process 直接调用，Android 端经 **JNI（UniFFI 生成绑定）** 调用。

**理由**
- 协议/编解码/抖动缓冲/时钟同步/混音只实现一次 → 两端行为一致（这是 N×N 网状 + ±10ms 同步的硬前提）。
- Rust 无 GC，适合实时音频；交叉编译到 Android 成熟（`cargo-ndk`）。
- 备选"两端各写一遍"会把协议实现分裂成两份，是旧版 bug 复现的温床。

**代价**：Android 构建链多一步（Rust 交叉编译 + `jniLibs` 装配）；JNI 边界需严格设计（见 ADR-011）。

---

## ADR-002 · 桌面外壳与 UI

**结论**：**Tauri 2**（Rust 外壳 + React 19 + TS + Vite + Tailwind + shadcn/ui）。

**理由**
- 内核已是 Rust → Tauri 外壳可 `in-process` 调用，零 FFI 成本，托盘/自启/更新插件齐全。
- 包体小（最小应用 NSIS ≈ 2.5–3 MB），本机 WebView2 已装（152.x），无额外运行时下载。
- 备选 WPF（用户明确不满"老旧 WPF"）、Avalonia（.NET 侧优势在本项目用不上）、Flutter Desktop（音频采集生态最弱）。

**代价**：WebView2 依赖（Win10 1809+ 通常自带，缺失时引导安装）；UI 与内核之间通过 `command/event` 通信，需注意事件节流（遥测 500 ms 批量推送，避免高频 IPC）。

**互补决策**：窗口关闭 = 隐藏到托盘；托盘为常驻主入口（FR-30）。

---

## ADR-003 · 音频编解码实现

**结论**：**`opus-rs` 0.1.33（纯 Rust，libopus 1.6 移植，BSD-3-Clause）**，桌面与 Android 同一 crate。

**理由**
- **零 C 依赖**：本机没有 cmake/NASM/perl，`opus` 0.4.0（`opusic-sys`）会直接构建失败；引入 cmake 会污染跨端构建链（Android 侧还要配 NDK 工具链）。
- 官方 bench：48 kHz / 20 ms / 立体声比 C libopus **快约 12%**。
- 同一 crate 可交叉编译到 `aarch64-linux-android` / `armv7-linux-androideabi`，两端编解码行为一致。

**备选（仅当性能不足时）**
- `rusty-opus` 0.9.1（CELT 路径 1.5–1.6×）：性能换成熟度，切换成本低（封装在 `audiolink-audio` 内部 trait）。
- `opus`（C 绑定）：**不采用**（需 cmake；已废弃的 `audiopus`/`magnum-opus` 也不采用）。

**参数基线**：48 kHz / 立体声 / 20 ms 帧 / 160 kbps VBR / complexity 7 / **`RESTRICTED_LOWDELAY` 应用模式** / `packet_loss_perc` 显式设置；10 ms 帧为可选档。

**关于解码端 FEC 的取舍（已知差异，不影响本项目）**：`opus-rs` 的解码路径**不提供带内 FEC 重建**（只有 `opus` 0.4.0 的 `decode(input, out, fec)` 与 `rusty-opus` 提供）。本项目在音乐场景**默认关闭带内 FEC**、主防线是"冗余双发 + PLC"（见 `03-protocol.md` §8.1），因此该差异不影响功能；若未来需要语音 FEC 重建，可在 `audiolink-audio` 内部的编解码 trait 后切换到 `opus` 0.4.0（需装 CMake）或 `rusty-opus`（纯 Rust）。

**代价**：纯 Rust 实现的边界情况（极端码率、DTX）需自测覆盖；已列入 M1 验收。

---

## ADR-004 · 传输层（**已裁决：维持 QUIC**）

**结论**：**QUIC（`quinn`）** —— 音频走 QUIC 数据报（RFC 9221），控制走可靠双向流 #0。

> 裁决记录（用户最终确认）：两份独立调研（Rust 栈 / Android 栈）均提议改用"裸 UDP + 轻量 RTP 头"，理由是局域网无 NAT、QUIC 属额外开销。**用户裁决维持 QUIC**，依据是下表——控制面可靠流、TLS1.3、连接迁移三件事都不必自研。

**理由**
- 我们协议需要**可靠有序的控制通道**（配对、协商、同步组指令、音量）→ QUIC 流开箱即得；裸 UDP 需自研重传/排序/拥塞控制，等于重造半个 QUIC。
- **TLS 1.3 内置**，直接满足"配对 + 全链路加密"（NFR-11）；裸 UDP 需另配 DTLS 或自研 AEAD 握手。
- 无队头阻塞（音频走数据报）、连接迁移（换 Wi-Fi 不断流）、BBR 拥塞控制成熟。

**调研提出的替代方案（裸 UDP + 轻量 RTP 头）**
- 优点：头开销更低（12 B vs QUIC 数据报约 20–30 B）、实现更薄、无需 TCP 系拥塞控制。
- 缺点：上述三条能力都要自建；加密需 DTLS/自研；Android 侧仍复用 Rust 内核（即"简单"的收益被内核复用抵掉大半）。
- 量化：头开销差约 10 B/包 × 50 pps ≈ **500 B/s**（占默认码率 0.3%），不足以成为决策依据。

**决策建议**：**保留 QUIC**，理由是"控制流 + 加密 + 连接迁移"三件事都不必自研；同时采纳调研的抗丢包策略（见下）。

**抗丢包策略（无论最终选哪个传输层都适用；已按 Opus 官方语义修正）**
| 手段 | 策略 |
|---|---|
| **NACK 重传 + PLC** | **主力**：局域网 RTT < 1 ms，重传窗口 1 s、重试 ≤ 5 次、间隔 10 ms，解码器 PLC 兜底 |
| Opus **in-band FEC** | **音乐场景默认关闭**：`opus_defines.h` 明确该机制 *only applicable to the LPC layer*，开启会迫使编码器走 SILK，**损伤音乐**并抬高码率；仅语音源可开 |
| 双发冗余 | 丢包 > 3% 且重传来不及（RTT 高）时启用（用带宽换连续可听） |
| `OPUS_SET_PACKET_LOSS_PERC` | **必须显式设置**（按实测丢包率反馈），默认 0 会让抗丢包机制形同虚设 |
| Opus DRED | 可选增强（深度冗余，代价是延迟与算力） |
| 自适应码率 | 见 `03-protocol.md` §8 |

**风险**：quinn 在弱网/高频丢包下的行为需 M2 实测；若 QUIC 在目标 WiFi 环境下表现不佳，**回退路径**是保留同一协议头、替换传输后端为裸 UDP（`audiolink-net` 内部抽象使切换成本约 1 周）。

---

## ADR-005 · 平台音频 API

| 平台 | 采集 | 播放 |
|---|---|---|
| Windows | **`wasapi` 0.24.0** —— `initialize_client(渲染端点, Direction::Capture, Shared)` 自动加 `AUDCLNT_STREAMFLAGS_LOOPBACK`；事件驱动 `StreamMode::EventsShared`（Win10 1703+ 正式支持事件驱动 loopback） | WASAPI render（事件驱动 + 显式缓冲） |
| Android | Kotlin `AudioRecord`（麦克风，FR-07）+ `MediaProjection`/`AudioPlaybackCapture`（内录，API 29+，FR-06） | Kotlin `AudioTrack`：`PERFORMANCE_MODE_LOW_LATENCY` + `WRITE_NON_BLOCKING`（**API 26+，本项目最低版本即由此决定**） |

**Android 侧解码不走 MediaCodec（重要，源码级核实）**：`c2.android.opus.decoder`（`OMX.google.opus.decoder` 自 Android 12 起只是它的别名）存在四个硬缺陷 ——
① **完全不支持 PLC 与带内 FEC**（`decode_fec` 恒为 0）；
② **一个损坏包会让 codec 实例永久报废**（进入 error 后每次 process 直接返回 `C2_BAD_VALUE`）；
③ 强制 3 个 CSD、输出硬锁 48 kHz/int16；
④ `setLowLatency()` 对 Opus 是**空操作**。
→ 解码统一走共享 Rust 内核的 `opus-rs`（ADR-001 + ADR-003）：既不依赖系统组件，也与桌面端行为一致，且规避上述全部缺陷。

**为何不用 cpal**：cpal 0.18.2 支持 loopback 但为**隐式行为**（把输出设备当输入用），无 min-period 控制、无 `IAudioClock`、无进程级 loopback；`wasapi` crate 具备这些能力，且**独占模式 + loopback 会明确报错**（避免静默失败）。

**额外能力（后续可加）**：`wasapi::new_application_loopback_client(pid, include_tree)`（Win10 2004+）可**只采集某个进程的声音**——为未来"按应用分流"留口。

**本机环境警示**：本机存在 AudioRelay / 网易虚拟音频设备，且有 3 个同名"扬声器"活动端点 → **设备选择 UI 必须显示唯一实例 ID 与默认设备标记**，并在"默认输出是虚拟声卡"时给出警告（否则会采到空流）。

---

## ADR-006 · 发现与配对

**结论**：`mdns-sd` 0.21.3（主）+ UDP 广播兜底 + 手工 IP 兜底；身份 = 自签证书指纹（TOFU），配对用 6 位 PIN。

**理由**
- mDNS 是跨平台标准（Android `NsdManager` 原生支持），但部分 OEM ROM 与"AP 客户端隔离"场景不可用 → 必须有多层兜底。
- 自动发现即连接是旧版最大安全漏洞（同网段任何人可推流）→ 必须配对。
- PIN 的作用是防止"同网段静默配对"，而非传输机密（机密由 TLS 保证）。

**契约**：服务类型 `_audiolink._udp.local.`，TXT 字段见 `03-protocol.md` §9。

---

## ADR-007 · 前端技术栈

**结论**：React 19 + TypeScript + Vite + Tailwind 4 + shadcn/ui；状态用 TanStack Query 风格的轻量 store（Tauri 事件为数据源）。

**理由**：Tauri 生态与组件库最成熟，shadcn/ui 可直接产出"现代桌面工具"观感；体积差异（几十 KB）相对 8 MB 级 exe 可忽略。

**约束**：UI 不得直接做网络/音频工作；所有能力经 Tauri `command` 落到内核。

---

## ADR-008 · 遥测与日志

**结论**：内核产出结构化指标（`StreamStats`），UI 侧 500 ms 批量刷新；日志分级 + 滚动 + 可导出（脱敏）。

**理由**：FR-32 与全部 NFR 验收都依赖同一数据源，"遥测即功能"。

**约束**：日志**不得**包含音频内容、配对 PIN、私钥；导出前做键名白名单过滤。

---

## ADR-009 · Android 构建矩阵与权限

**结论**：`compileSdk 37` / `targetSdk 36` / `minSdk 24` / 双 ABI / R8 开启 / 密钥由 CI Secret 注入。

**理由（硬约束）**
- Compose 1.12+ 要求 `compileSdk 37` + AGP 9；
- **`targetSdk` 37 会强制 `ACCESS_LOCAL_NETWORK` 运行时权限**（NEARBY_DEVICES 组），未授权时局域网收发全部失败（UDP EPERM）→ **停在 36**；
- Android 17 后台音频加固（targetSdk ≥ 37 需 FGS 具备 while-in-use 能力，否则音频写入被静默吞掉）→ 同样规避，但 FGS 声明按未来标准预先合规；
- `minSdk 24`：覆盖旧设备（用户的闲置手机场景），但**低延迟播放需 API 26+**、**内录需 API 29+** → UI 需能力分级提示。

**代价**：升级 targetSdk 需实现权限门禁与降级引导（已登记技术债）。

---

## ADR-010 · 平台抽象与 JNI 边界

**结论**：内核定义 `CaptureSource` / `PlayoutSink` trait；Android 侧由 Kotlin 实现并通过 **JNI 环形缓冲**与内核交换 PCM；内核**不持有** Android 音频线程。

**理由**：`AudioTrack` / `MediaProjection` 有明确的线程与生命周期亲和性，跨 JNI 托管播放线程是经典崩溃来源（旧版 `mAudioTrack` 跨线程无同步即为此类）。

**边界规则**
- PCM 传递用**预分配的直接 ByteBuffer 环形缓冲**（零拷贝、无每次 JNI 分配）；
- 内核 → Kotlin：给出 `(pcm 视图, 目标播放时刻, 期望速率)`；Kotlin 负责按目标时刻提交；
- Kotlin → 内核：`(pcm 视图, 采集时刻)`；
- 所有 JNI 调用必须可重入安全、无阻塞（JNI 侧不做 IO）。

---

## ADR-011 · 配置与密钥存储

| 平台 | 私钥 | 信任库 | 配置 |
|---|---|---|---|
| Windows | 文件 + ACL（后续可加 DPAPI） | `%APPDATA%\AudioLink\trust.json` | `%APPDATA%\AudioLink\config.json` |
| Android | **AndroidKeyStore**（硬件支持时用 TEE） | 应用私有目录 JSON | DataStore |

**规则**：任何密钥/口令**不入库**（旧版 `sign.jks` + 明文口令是必须避免的反面教材）；CI 用 GitHub Secret 注入发布签名。

---

## ADR-012 · Android 前台服务与通知

**结论**：`foregroundServiceType="mediaPlayback|connectedDevice"`（两者均**无时长上限**）+ 采用 media3 `MediaSessionService`；服务启动即进入前台（`startForegroundService`）；**Android 端不做开机自启**。

**理由**
- Android 15+ **禁止**由 `BOOT_COMPLETED` 启动 mediaPlayback 前台服务 → "开机自动恢复接收"在系统层面已不可行（且用户明确不需要）；
- media3 `MediaSessionService` 的媒体通知**可豁免 `POST_NOTIFICATIONS` 被拒**的情况 —— 用户拒绝通知权限时仍能看到播放通知；
- Android 16 起 **FGS 内的后台作业同样受运行时配额限制** → 接收循环**绝不能**挂在 `WorkManager` / `JobScheduler` 上，必须常驻前台服务；
- `WIFI_MODE_FULL_HIGH_PERF` 自 API 34 起弃用并被降级为"亮屏 + 前台才生效"→ **不要指望 WifiLock 对抗熄屏省电**，可靠做法是引导用户加入省电白名单。

**代价**：常驻通知无法避免（这正是需求要求"通知栏可直接停止"的原因）。

---

## 2. 依赖清单（起点，锁版本）

### Rust（`core/` + `desktop/src-tauri/`）

| crate | 版本 | 用途 |
|---|---|---|
| `quinn` | 0.11.x | QUIC（含 `send_datagram`） |
| `rustls` / `rcgen` | 最新稳定 | TLS + 自签证书生成 |
| `tokio` | 1.x（full） | 异步运行时 |
| `opus-rs`（或 `rusty-opus`） | 0.1.33 / 0.9.1 | Opus 编解码 |
| `wasapi` | 0.24.0 | Windows 采集/播放 |
| `mdns-sd` | 0.21.3 | 服务发现 |
| `serde` / `postcard` | 最新 | 控制面序列化 |
| `serde_json` | 1.x | 发现报文的 JSON 载荷（`03-protocol.md` §9.2）；**仅发现层使用**，音频/控制面一律 postcard |
| `crossbeam` / `ringbuf` | 最新 | 无锁环形缓冲 |
| `tracing` / `tracing-subscriber` | 最新 | 结构化日志 |
| `uniffi` | 最新 | FFI 绑定生成 |
| `rubato`（或自研） | 最新 | 采样率转换（若质量/延迟不达标可换自研多相滤波器） |
| `anyhow` / `thiserror` | 最新 | 错误处理（实时路径禁用 panic） |

### Tauri 插件

`single-instance 2.4.4`、`autostart 2.5.1`、`store 2.4.4`、`log 2.9.1`、`window-state 2.4.1`、`opener 2.5.5`、`notification 2.4.0`、`updater`（自动更新）。

### Android

`androidx.compose:*`（BOM 2026.09.00）、`material3 1.4.0`、`androidx.datastore`、`androidx.lifecycle`、`kotlinx-coroutines`、`kotlinx-serialization`；**不引入** okhttp/ExoPlayer/androidasync（旧版重型依赖全部不需要）。

---

## 3. 回退路径（风险预案）

| 决策 | 触发条件 | 回退动作 |
|---|---|---|
| QUIC（ADR-004） | 弱网实测劣于预期 / quinn 在目标环境异常 | 保留协议头，替换传输后端为裸 UDP（约 1 周） |
| `opus-rs`（ADR-003） | 性能或质量不达标 | 换 `rusty-opus`（同 trait，约 1 天） |
| `wasapi` crate（ADR-005） | 驱动兼容问题 | 回退 `cpal` loopback（隐式行为，功能受限） |
| mDNS 发现（ADR-006） | 目标网络普遍隔离 | 默认走 UDP 广播 + 手工 IP（已是兜底路径） |
| Tauri（ADR-002） | WebView2 缺失率过高 | 打包含 bootstrapper（默认策略已覆盖） |
