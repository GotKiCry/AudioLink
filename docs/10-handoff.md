# AudioLink 交接文档（Handoff）

> **性质**：过程性文档。新会话接手本项目时先读本文，再按 `docs/05-roadmap.md` 推进。
> **最后更新**：2026-09-14（M1 主线大会战后）· 远端 https://github.com/GotKiCry/AudioLink（PUBLIC）

---

## 1. 一句话现状

M0 全部完成。**M1 的代码面已全部落地并各自跑绿**（QUIC 通道 / 身份与 PIN 配对 / 引擎编排 / FFI 桥 /
Android 低延迟播放 / 桌面最小 UI / PC↔PC 端到端验收工具），
**唯一未闭环的是「Android 真机验收」—— 卡在硬件（本机 `adb devices` 为空、无可用 AVD），不是卡在代码。**

| 项 | 状态 |
|---|---|
| 远端仓库 | **PUBLIC** · main 分支 · 看板 https://github.com/users/GotKiCry/projects/2 |
| 本地三条链 | ✅ 内核 `cargo test --workspace`（**214 项**）· ✅ Android `assembleDebug` + 31 个 JVM 用例 · ✅ 桌面 `pnpm build` + Tauri `cargo test` |
| 质量门 | ✅ `cargo fmt --all --check` 干净 · ✅ `cargo clippy --workspace --exclude audiolink-desktop --all-targets --all-features -- -D warnings` 干净 |
| M1 代码面 | ✅ 全部落地（见下表） |
| M1 真机验收 | ⬜ **阻塞在硬件**：Android 低延迟模式 / 出声延迟 / 30 min 无断流 |
| PC↔PC 实测 | ✅ 真实 QUIC 链路：探针「帧封口→出声」**P50 40.2 ms / P95 40.6 ms**，零丢包零欠载；含模型段的合计约 **70 ms** |

| M1 交付物 | crate / 路径 | 证据 |
|---|---|---|
| QUIC 通道（控制流 #0 + 数据报 + 时钟估计） | `core/crates/audiolink-net` | 19 单测 + 5 真 QUIC 集成测试 |
| 自签证书 / 指纹 / 信任库 / PIN 配对 | `core/crates/audiolink-identity` | 22 单测 + 8 黑盒验收 |
| 引擎编排（状态机 / 收发管线 / 遥测） | `core/crates/audiolink-engine` | 60 测试 |
| FFI 桥（UniFFI + 跨端 golden vectors 夹具） | `core/crates/audiolink-ffi` | 18 测试 + aarch64 交叉检查 |
| Android 低延迟播放 | `android/app/src/main/kotlin/.../audio/` | APK 构建 + 31 JVM 用例 |
| 桌面最小 UI（真实引擎，无 mock） | `desktop/` | `pnpm build` + 10 Rust 测试（含 1 个起两个真 Engine 的接缝测试） |
| PC↔PC 端到端验收工具 | `core/crates/audiolink-tools/src/bin/link_loop.rs` | 见上表实测数字 |

**本机 PC↔PC 实测（`link-loop`，30 s，默认端点，20 ms 帧 / 160 kbps Opus）**：
真实 127.0.0.1 QUIC（TLS + 证书指纹互认 + §5 握手 + PIN 配对实跑）。
探针实测「帧封口 → sink 写出」P50 **40.2 ms** / P95 **40.6 ms** / P99 40.7 ms / max 44.1 ms；
加两段模型（采集半周期 10 ms + 组帧 20 ms）合计约 **70 ms**。码率 160.4 kbps（目标 160），
丢包 0.00%，欠载 0，迟到丢弃 0，抖动 P50/P95 = 0.11/0.43 ms。
复测：`cargo run -q -p audiolink-tools --bin link-loop -- run --seconds 30`。


---

## 2. 必读文档（按顺序）

| 文档 | 用途 |
|---|---|
| `docs/00-overview.md` | 一页纸总览 + 决策速查 |
| `docs/01-requirements.md` | FR-01~41 / NFR-01~14 / 验收标准 |
| `docs/03-protocol.md` | **ALP/2 协议规格（含 §12 golden vectors）** |
| `docs/02-architecture.md` | 代码组织、依赖方向、线程模型与实时铁律 |
| `docs/05-roadmap.md` | M0–M5 里程碑、每阶段退出条件（M1 的缓冲口径已按实测修正） |
| `docs/06-dev-environment.md` | 构建命令 + **坑清单（§4、§4.1、§4.2 务必看）** |
| `docs/04-tech-stack.md` | 版本矩阵 + ADR（含 ADR-003 的 PLC 实测注记） |
| `target/notes/wasapi-0.24-api.md`、`target/notes/opus-rs-0.1.33-api.md` | **源码级 API 侦察 + 实测数字**（一次性产物，不入库；下次要复现就重跑） |

---

## 3. 不可变更的约束（已与用户确认，改动需先问）

| 约束 | 值 | 原因 |
|---|---|---|
| Android `targetSdk` | **锁 36** | 升 37 会强制 `ACCESS_LOCAL_NETWORK` 运行时权限，局域网收发直接 EPERM |
| Android `minSdk` | **26** | `PERFORMANCE_MODE_LOW_LATENCY` 的门槛 |
| 传输层 | **QUIC（quinn）** | 用户裁决；两条调研建议裸 UDP 但被否，理由与回退路径见 ADR-004 |
| 编解码 | **`opus-rs`（纯 Rust）** | 不装 CMake；音乐场景**关闭** in-band FEC，主防线是「冗余双发 + **自建丢包掩盖**」 |
| 内核复用 | **Rust 内核两端共用** | Android 经 JNI/UniFFI 调用，协议只实现一次 |
| 拓扑 | **完全网状**：任意节点互推（含 Android → Android） | ADR / 需求 §4.2 |
| 丢包对抗 | **冗余双发 + 掩盖为主**，NACK 辅助 | `03-protocol.md` §8.1；**注意 CELT-only 无真 PLC**（ADR-003 注记） |
| 同步 | 四时间戳时钟同步 + 预约播放，组内 ±10 ms（设计目标 ≤5 ms） | §6/§7 |
| 延迟口径 | **P50 80–110 ms（Wi-Fi）/ 40–55 ms（有线），P95 ≤ 150 ms** | 不承诺单一数字；40 ms 非承诺值 |
| 采样率 | **全链路 48 kHz，禁止静默重采样** | 设备不是 48 kHz 时要引导用户改系统设置，不是让内核偷偷 SRC |
| 不做 | 云音乐 / R1 灯效 / USB-adb / 远程 Web 页 / 跨公网 / iOS-Web 端 / Android 开机自启 | 需求 §2.2 |

---

## 4. 下一次开工

### 4.1 第一优先：Android 真机验收（**唯一的 M1 阻塞项，卡在硬件**）

M1 的代码面已经齐了，缺的只是「插上手机跑一遍」。需要一台 **API 26+** 的 Android 真机（局域网可达 PC）。要跑：

| 项 | 标准 | 怎么看 |
|---|---|---|
| 低延迟模式 | `getPerformanceMode()` 返回 `PERFORMANCE_MODE_LOW_LATENCY` | `PlaybackReport.lowLatency` + UI 显示的原值（**不做粉饰**） |
| 出声 | 连接成功 ≤ 300 ms 内出声 | 人工计时 + 遥测首帧时间 |
| 延迟 | P50 ≤ 110 ms、P95 ≤ 150 ms | `link-loop` 的探针口径换成跨机（见 §4.2）；M1 现只验了 PC↔PC |
| 零重采样 | 采集/内核/播放采样率均 48000，运行时无 SRC | 遥测显示 + 音频路径日志断言（引擎侧已在**线程启动时**硬断言） |
| 稳定性 | 30 min 连续无断流 | 记录 `underruns` / `plc_count` |

接线状态：FFI 绑定（UniFFI Kotlin）已生成，`AudioLinkService.feedPcm()` 是内核 PCM 的接缝。
`engineStart(config, playout, capture)` 的生命周期**跟随 Service 生命周期**（服务启 → 引擎启；
`capture = null` → `canReceive=true / canSend=false`），理由是 Android 在 M1 是接收端，必须在被连接前就监听，
而「启动服务」这个动作本身已经由用户显式点出来。

### 4.2 第二优先：跨机端到端测量手段

`link-loop` 现在只量得了 PC↔PC：它的探针（`MeasurementTap`）按 `seq` 配对两侧 `Instant`，
**而跨机的单调时钟不可相减**。真机验收前需要一个跨机方案，两条路：

1. **推荐**：接 §6 的时钟同步 —— `audiolink-net::ClockEstimator` 已实现并单测通过（200 样本窗口、
   RTT 最小 8 个取中位数、回归出 `drift_ppm`），但**引擎侧一根线都没接**（`clock_offset_us` / `drift_ppm`
   现在恒为 0）。接上之后 `e2e = 本地出声时刻 − (对端采集时刻 + offset)` 就能跨机算。
2. 退路：人工双机录音 + 波形对齐（属 M3 的 `sync-measure`）。

### 4.3 已知待办（按优先级）

| 项 | 说明 | 优先级 |
|---|---|---|
| 真机验收 | §4.1 | **P0** |
| 跨机测量 | §4.2（建议顺便把 §6 时钟同步接进引擎，M3 反正要做） | **P0**（M1 验收依赖） |
| WASAPI 真实音频链 | 桌面端的采集/播放工厂按真实签名写好了，但**没启动过应用**（会真采集真放音）。需要一次带声卡的实跑 | P1 |
| `EngineEvent::Telemetry` 加 `peer: NodeId` | 现在对端遥测与本机采样共用同一事件，**多对端会串**（M3 必须修） | P1（M3） |
| PCM 回调装箱开销 | UniFFI 0.29 无 `FloatArray` 映射 → `List<Float>` 每帧约 30 KB 装箱。真机 GC 抖动**未验证**；逃生通道（UDL `bytes` + `ByteBuffer.asFloatBuffer()`）已写进 `audiolink-ffi/src/audio_bridge.rs` 顶部 | P1（真机定） |
| 抖动缓冲 | M1 只做了「起步攒 2 帧 + 时钟驱动提交」（`PRIME_FRAMES`）。自适应深度（15–60 ms）属 M2 | M2 |
| 时钟同步接线 | 见 §4.2 第 1 条 | M3 |
| `docs/06-dev-environment.md` 脱敏 | 含 `C:\Users\liuzh\...` 等本机路径，仓库已公开 | 低（用户未决） |

**纪律（不变）**
- 实时路径禁用 `unwrap` / `expect` / `panic`（workspace lint 已 `deny`）；错误一律显式返回 + 计数，不允许静默停止；
- 新增跨线程数据结构先看 `audiolink-audio::ring` 与 `latency` 的口径，别再造一套；
- 提交前跑 `cargo fmt --all` + `cargo clippy --workspace --exclude audiolink-desktop --all-targets --all-features -- -D warnings` + `cargo test --workspace --exclude audiolink-desktop`；
- 提交用约定式提交（`feat(net): ...`）；改文档与改代码同一次提交里带上（文档是契约）；
- **改 `docs/11-m1-contract.md` 之前先想清楚**：它是本轮 5 条并行流的唯一对齐依据，形状改动要同步所有消费方。

---

## 5. 本机环境要点

| 项 | 说明 |
|---|---|
| 工作目录 | `C:\_Project\AudioLink` |
| Rust | 1.96.1（`rust-toolchain.toml` 锁定），MSVC host；Android targets 已装 |
| Android 构建 | **`pwsh tools/gradlew.ps1 -JavaHome 'C:\Users\liuzh\scoop\apps\corretto17-jdk\current' assembleDebug`**。⚠️ `-JavaHome` 是**第一个位置参数**：`pwsh tools/gradlew.ps1 assembleDebug` 会把 `assembleDebug` 当成 JDK 路径。本机 `JAVA_HOME` 指向 **JDK 11**，而 Gradle 9 要 17+ |
| Android 单测 | 同上加 `testDebugUnitTest`（31 个 JVM 用例）；另有 `lintDebug` |
| 交叉编译内核 | `pwsh android/scripts/build-rust.ps1 -Abi arm64-v8a,armeabi-v7a`（走 cargo-ndk，会注入 CC）；快速验证用 `cargo check -p audiolink-ffi --target aarch64-linux-android` |
| 桌面端 | `cd desktop; pnpm build` / `pnpm tauri dev`；Rust 侧 `cargo test -p audiolink-desktop` |
| **不要安装 CMake** | Opus 走纯 Rust `opus-rs`（ADR-003）；rustls 必须 **ring** 后端（默认 aws-lc-rs 要 CMake/NASM） |
| Gradle 隔离 | `GRADLE_USER_HOME=C:\_Project\AudioLink\.gradle-home`（全局 properties 含公司私服凭据） |
| 自环测量 | `cargo run -q -p audiolink-tools --bin self-loop -- run --seconds 20`（**会放音，先调小音量**） |
| **PC↔PC 端到端验收** | `cargo run -q -p audiolink-tools --bin link-loop -- run --seconds 30`（两个真实 Engine + 真实 QUIC，**不出声**，可放心跑） |

---

## 6. 已踩过并已修的坑（别重复踩）

详见 `docs/06-dev-environment.md` §4 / §4.1 / §4.2，速查：

1. Cargo 的开发 profile 叫 **`dev`**（`debug` 是保留名）；
2. PowerShell 里 **`$profile` / `$home` 是只读自动变量**，不能当参数名；
3. **Gradle 9 移除了任务级 `exec {}`** → 注入 `ExecOperations`；
4. Tauri 必须有 `desktop/src-tauri/build.rs`；
5. Android 依赖**必须显式写版本号**；
6. **`.gitignore` 不支持行尾注释**；
7. 仓库必须含 gradle wrapper 三件套；`gradlew` 需 LF + 执行位（Ubuntu CI）；
8. **不要把本机 JDK 路径写进 `android/gradle.properties`**；
9. **`wasapi` 0.24 是 API 重写**（无 `make_device_enumerator!` / `set_buffer_duration` / 四参数 `initialize_client`），且**所有 COM 对象 `!Send`** → 谁用谁建；
10. **共享模式缓冲下限 22 ms**、引擎周期 10 ms —— 别再把「10–20 ms 缓冲」当可实现目标；
11. **`autoconvert=true` 会静默重采样**，本项目一律关；
12. **空闲端点零数据**：采集超时不是错误，要计数不要报错；
13. **`opus-rs` 48k 只接受 240/480/960 帧样本**（2.5 ms 会 panic）；`decode` 返回**每声道**样本数；
14. **CELT-only 没有真 PLC**（第 2 个丢失帧起硬静音）→ M2 必须自建掩盖，别指望 `packet_loss_perc`。

### 本轮（M1 主线）新踩的坑

15. **`cargo check --target aarch64-linux-android` 的裸命令会挂在 `ring` 的 C 编译上**：
    `cc` 1.4.5 的 `autodetect_android_compiler` **只从 PATH 找 `<triple>-clang(.cmd)`，不读 `ANDROID_NDK_HOME`/`NDK_HOME`**（实测设了也没用）。
    把 NDK 的 `toolchains/llvm/prebuilt/windows-x86_64/bin` 加进 PATH 即通过。
    `cargo check -p audiolink-net` 裸跑同样失败 → 是**既有环境问题**，不是某个 crate 引入的。
    走 `android/scripts/build-rust.ps1`（cargo-ndk）不受影响，因为它会显式注入 CC。
16. **rustdoc 里写路径通配要小心 `/*`**：`audiolink-ffi/src/error.rs` 的文档注释里出现过 `` `bindings/kotlin/**.kt` ``，
    它被原样带进 UniFFI 生成的 Kotlin KDoc —— 而 **Kotlin 的块注释可以嵌套**，
    一个 `/*` 就把生成物从第 1779 行整段吞到文件尾（`Unclosed comment` + 800 多个未解析符号）。
    教训：**源文件注释里的字符会改变另一门语言的代码语义**；生成物的护栏要断言「不含裸 `/*` 序列」。
17. **非 ASCII 标识符里不能夹关键字**：`fn 目标路径非法时返回错误而不是 panic() {` 会被解析成
    「函数名 + 参数列表 = `panic()`」→ `error: missing parameters for function definition`。
    Rust 允许中文标识符，但**不许含关键字**。测试名统一用 ASCII snake_case 最省事。
18. **UNIFFI 生成物的字段名会和 `kotlin.Exception` 撞**：`FfiError::Failure { message }` 与
    `Exception.message` 冲突（生成的 `override val message` getter 又引用同名字段）。
    错误结构里那个字段改名为 `detail` 之类即可。
19. **播放线程的节奏必须由时钟驱动，不能靠 `recv_timeout` 兜底**：合成/空 sink 的 `write()`
    **不会自己计时**，收到就返回。若在超时分支立刻补静音再循环，线程会空转并按帧率上限刷欠载
    （欠载率直接失去意义）。正确做法是「睡到下一个提交时刻 → 再决定这一拍写数据还是写静音」，
    这也正是 §7「播放指针由 epoch 驱动而非到达时间驱动」的最小实现。
20. **`VecDeque` 没有 `swap_remove`**（那是 `Vec` 的）；`VecDeque::remove` 是 O(n) 但保持顺序 ——
    在「队列满了丢最旧」的场景里，保序比 O(1) 更要紧。

---

## 7. 待办与遗留

| 项 | 说明 | 优先级 |
|---|---|---|
| M1 主线 | QUIC 通道 / Android 播放 / 最小 UI / PIN 配对 / 真机验收（§4） | **P0** |
| 跨端一致性夹具 | §12 golden vectors 在 Rust 与 FFI 双跑（需 ffi 有导出路径） | P3（M1） |
| `alp2-dump` 支持 pcap | 当前只吃 hex 文本；等真有抓包需求再加 | P3 |
| 自环测量增强 | 目前只测本机；真机链路需要一个「回环标记」端到端测量手段（网络抖动会掩盖标记） | P2（M1 验收时想清楚） |
| `docs/06-dev-environment.md` 脱敏 | 含 `C:\Users\liuzh\...` 等本机路径，仓库已公开 | 低（用户未决） |
| `release.yml` 未跑过 | 首次 tag 触发时才验证；`createUpdaterArtifacts:false`（M5 打开） | 中（M5） |
| CI 可再优化 | `android` job 每次 `cargo install cargo-ndk` 约 2 分钟；`core` job 因 tools 引入 quinn/rustls 涨到 ~4 分钟 | 低 |
| QUIC 弱网实测 | ADR-004 的回退路径（裸 UDP）是否触发，取决于 M2 实测 | 中（M2） |
