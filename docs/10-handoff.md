# AudioLink 交接文档（Handoff）

> **性质**：过程性文档。新会话接手本项目时先读本文，再按 `docs/05-roadmap.md` 推进。
> **最后更新**：2026-09-14 · 对应 commit `b3c7bd5` 之后的 M1-P0 工作 · 远端 https://github.com/GotKiCry/AudioLink（PUBLIC）

---

## 1. 一句话现状

M0 全部完成，**M1 的三项 P0（WASAPI 采集 / Opus 编码 / 桌面自环+延迟分解）已完成并本机实测跑通**；
下一步是 M1 的关键路径主线：**QUIC 通道 + Android 播放 + 最小 UI**，把它们接成「PC → 手机」的真链路。

| 项 | 状态 |
|---|---|
| 远端仓库 | **PUBLIC** · main 分支 · 看板 https://github.com/users/GotKiCry/projects/2 |
| CI（`.github/workflows/ci.yml`） | 4 job：`core`(fmt+clippy+test) / `android` / `desktop` / `version-consistency` |
| 本地三条链 | ✅ 内核 `cargo test`（84 项）· Android 交叉 `cargo check --target aarch64-linux-android` ✅ · 桌面 exe + NSIS ✅ |
| M0 退出条件 | ✅ 全部达成（`cargo test` 全绿、协议 golden vectors 全绿、CI 能构建内核） |
| M1 进度 | ✅ 采集 / ✅ Opus 编解码 / ✅ 自环+延迟分解 ／ ⬜ QUIC 通道 · Android 播放 · 最小 UI · PIN 配对 · 真机验收 |
| 看板 | 26 个条目，M0 全 Done；M1 三项 P0 已置 Done（由 `tools/sync-board.ps1` 幂等同步） |

**本机自环实测（默认 HDMI 端点，20 ms 帧 / 160 kbps Opus）**：
采集(半周期模型) 5.0 ms + 组帧 20.0 ms + 编码 2.6 ms + 解码 0.08 ms + 播放 12.0 ms = **39.7 ms**；
标记法实测设备往返 **6.78 ms**（同口径模型 7.00 ms，差 0.22 ms）；999 帧零欠载零丢弃。

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

## 4. 下一次开工：M1 主线（QUIC 通道 → Android 播放 → 接成真链路）

**目标**：把已经各自跑通的两端接起来 —— PC 采集/编码 → QUIC 数据报 → Android 解码/播放。

**要动的文件**

```text
core/crates/audiolink-net/src/lib.rs        # quinn 端点（监听 + 发起）、控制流 #0、音频数据报
                                            # 发送侧必须 min(1200, max_datagram_size())（实测 1162 B）；
                                            # 数据报头用 audiolink-proto 的 AudioDatagram
core/crates/audiolink-engine/src/lib.rs     # 把 audio 与 net 拼起来：会话状态机、编码线程、
                                            # 遥测聚合（StreamStats，1 Hz）
android/app/src/main/kotlin/...             # AudioTrack 低延迟播放（PERFORMANCE_MODE_LOW_LATENCY +
                                            # WRITE_NON_BLOCKING + getPerformanceMode() 断言）、
                                            # JNI 环缓冲、前台服务
core/crates/audiolink-ffi/src/lib.rs        # UniFFI 导出：启停、状态、遥测
desktop/src-tauri/src/lib.rs + desktop/src/ # 最小 UI：手工 IP 连接 + 开始/停止
```

**验收（真机；`docs/05-roadmap.md` M1）**

| 项 | 标准 |
|---|---|
| 出声 | 连接成功 ≤ 300 ms 内出声 |
| 延迟 | `e2e_latency_us` P50 ≤ 110 ms、P95 ≤ 150 ms（自环已占 39.7 ms，留给网络+Android 播放约 70 ms 预算） |
| **零重采样** | 采集/内核/播放采样率均 48000，运行时无 SRC 环节 |
| **低延迟模式** | Android `getPerformanceMode()` 返回 `PERFORMANCE_MODE_LOW_LATENCY` |
| 稳定性 | 30 min 连续无断流；记录 `underruns` / `plc_count` |

> 两项硬指标（零重采样、低延迟模式）**不达标不得进入 M2**（roadmap M1 备注）。

**纪律**
- 实时路径禁用 `unwrap` / `expect` / `panic`（workspace lint 已 `deny`）；错误一律显式返回 + 计数，不允许静默停止；
- 新增跨线程数据结构先看 `audiolink-audio::ring` 与 `latency` 的口径，别再造一套；
- 提交前跑 `cargo fmt --all` + `cargo clippy --workspace --exclude audiolink-desktop --all-targets --all-features -- -D warnings` + `cargo test --workspace --exclude audiolink-desktop`；
- 提交用约定式提交（`feat(net): ...`）；改文档与改代码同一次提交里带上（文档是契约）。

---

## 5. 本机环境要点

| 项 | 说明 |
|---|---|
| 工作目录 | `C:\_Project\AudioLink` |
| Rust | 1.96.1（`rust-toolchain.toml` 锁定），MSVC host；Android targets 已装 |
| Android 构建 | **用 `pwsh tools/gradlew.ps1 assembleDebug`**（校验 JDK 17+ 并注入 `ANDROID_HOME`） |
| 交叉编译内核 | `pwsh android/scripts/build-rust.ps1 -Abi arm64-v8a,armeabi-v7a`；快速验证用 `cargo check -p audiolink-ffi --target aarch64-linux-android` |
| 桌面端 | `cd desktop; pnpm tauri dev` / `pnpm build` |
| **不要安装 CMake** | Opus 走纯 Rust `opus-rs`（ADR-003） |
| Gradle 隔离 | `GRADLE_USER_HOME=C:\_Project\AudioLink\.gradle-home`（全局 properties 含公司私服凭据） |
| 自环测量 | `cargo run -q -p audiolink-tools --bin self-loop -- run --seconds 20`（**会放音，先调小音量**） |

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
