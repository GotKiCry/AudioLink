# AudioLink 交接文档（Handoff）

> **性质**：过程性文档。新会话接手本项目时先读本文，再按 `docs/05-roadmap.md` 推进。
> **最后更新**：2026-09-11 · 对应 commit `34194b6` · 远端 https://github.com/GotKiCry/AudioLink（PUBLIC）

---

## 1. 一句话现状

规划全部完成、仓库骨架可构建、CI 全绿（约 3 分钟），**代码尚未开始实现功能** —— 下一步是 M0-02：实现 `audiolink-proto` 协议编解码。

| 项 | 状态 |
|---|---|
| 远端仓库 | **PUBLIC** · main 分支 · 9 个 commit |
| CI（`.github/workflows/ci.yml`） | 4 job 全绿：`core`(fmt+clippy+test) / `android` / `desktop` / `version-consistency`，单轮 ~3.1 分钟 |
| 本地三条链 | ✅ 内核 `cargo test` · Android APK 12.3 MB（arm64-v8a + armeabi-v7a） · 桌面 exe 9.4 MB + NSIS 安装包 2.2 MB |
| M0 退出条件进度 | 第 1 项「CI 能构建内核」✅ 已达成；第 2 项「协议 golden vectors 全绿」⬜ 待做 |

---

## 2. 必读文档（按顺序）

| 文档 | 用途 |
|---|---|
| `docs/00-overview.md` | 一页纸总览 + 决策速查（谁定的、结论是什么） |
| `docs/01-requirements.md` | FR-01~41 / NFR-01~14 / 验收标准 / 决策追溯表 |
| `docs/03-protocol.md` | **ALP/2 协议规格（本次任务核心，含 §12 golden vectors）** |
| `docs/02-architecture.md` | 代码组织、依赖方向、线程模型与实时铁律 |
| `docs/05-roadmap.md` | M0–M5 里程碑、每阶段退出条件 |
| `docs/06-dev-environment.md` | 构建命令 + **实测踩坑清单（§4.1，务必看）** |
| `docs/04-tech-stack.md` | 版本矩阵 + 12 条 ADR（含回退路径） |

---

## 3. 不可变更的约束（已与用户确认，改动需先问）

| 约束 | 值 | 原因 |
|---|---|---|
| Android `targetSdk` | **锁 36** | 升 37 会强制 `ACCESS_LOCAL_NETWORK` 运行时权限，局域网收发直接 EPERM |
| Android `minSdk` | **26** | `PERFORMANCE_MODE_LOW_LATENCY` 的门槛 |
| 传输层 | **QUIC（quinn）** | 用户裁决；两条调研建议裸 UDP 但被否，理由与回退路径见 ADR-004 |
| 编解码 | **`opus-rs`（纯 Rust）** | 不装 CMake；音乐场景**关闭** in-band FEC，主防线是「冗余双发 + PLC」 |
| 内核复用 | **Rust 内核两端共用** | Android 经 JNI/UniFFI 调用，协议只实现一次 |
| 拓扑 | **完全网状**：任意节点互推（含 Android → Android） | ADR / 需求 §4.2 |
| 丢包对抗 | **冗余双发 + PLC 为主**，NACK 辅助 | 见 `03-protocol.md` §8.1 |
| 同步 | 四时间戳时钟同步 + 预约播放，组内 ±10 ms（设计目标 ≤5 ms） | §6/§7 |
| 延迟口径 | **P50 80–110 ms（Wi-Fi）/ 40–55 ms（有线），P95 ≤ 150 ms** | 不承诺单一数字；40 ms 非承诺值 |
| 不做 | 云音乐 / R1 灯效 / USB-adb / 远程 Web 页 / 跨公网 / iOS-Web 端 / Android 开机自启 | 需求 §2.2 |

---

## 4. 下一次开工：M0-02 实现 `audiolink-proto`

**目标**：按 `docs/03-protocol.md` 实现协议编解码，让 §12 的三组 golden vectors 在单元测试中全部通过。

**要动的文件**
```
core/crates/audiolink-types/src/lib.rs   # 常量（proto_version=0x0201、ptype、flags、OP 命令码、错误码）、
                                          # 枚举、AudioLinkError、StreamStats 结构
core/crates/audiolink-proto/src/lib.rs   # 帧编解码：
                                          #  • 音频数据报头（24B：version/ptype/flags/stream_id/seq/sample_index/epoch_id）
                                          #  • 控制帧（ver/type/flags/len + request_id + postcard 载荷）
                                          #  • 发现报文（UDP 广播 "AUDIOLINK" magic + ver + len + JSON）
                                          #  • mDNS TXT 字段编码/校验
core/crates/audiolink-proto/tests/       # golden vectors round-trip 测试（建议单独文件）
```

**验收（全部满足才算完成）**
1. `cargo test -p audiolink-proto` 全绿，且包含 §12 全部三组向量（含 `960 = 20ms×48000` 的 sample_index 校验）；
2. **拒绝用例**：截断包 / 超长包 / 未知 ptype / 保留位非 0 / length 越界 → 返回 `BadRequest`，**绝不 panic**；
3. 编码 → 解码 → 再编码，字节完全一致；
4. `cargo fmt --all --check` 干净；
5. `cargo clippy --workspace --exclude audiolink-desktop --all-targets --all-features -- -D warnings` 干净；
6. 若实现中发现规格有歧义：**先改 `docs/03-protocol.md`，再改代码**（文档是契约）。

**纪律**
- 实时路径禁用 `unwrap` / `expect` / `panic`（workspace lint 已设 `deny`）；确需处 `#[allow]` + 写明理由；
- golden vectors 是契约：**不允许为了让测试通过而修改向量**（要改必须先改文档并说明理由）；
- 新增 crate / 依赖版本以 `docs/04-tech-stack.md` 为准；
- 提交用约定式提交（`feat(proto): ...`），提交前跑 fmt + clippy + test。

---

## 5. 本机环境要点

| 项 | 说明 |
|---|---|
| 工作目录 | `C:\_Project\AudioLink` |
| Rust | 1.96.1（`rust-toolchain.toml` 锁定），MSVC host；Android targets 已装 |
| Android 构建 | **用 `pwsh tools/gradlew.ps1 assembleDebug`** —— 它会校验 JDK 17+ 并注入 `ANDROID_HOME`（本机 `JAVA_HOME` 是 JDK 11，直接用 `gradlew.bat` 会启动失败） |
| 交叉编译内核 | `pwsh android/scripts/build-rust.ps1 -Abi arm64-v8a,armeabi-v7a`（自动探测 SDK/NDK） |
| 桌面端 | `cd desktop; pnpm tauri dev` / `pnpm build` |
| **不要安装 CMake** | Opus 走纯 Rust `opus-rs`（ADR-003） |
| Gradle 隔离 | 建议 `GRADLE_USER_HOME=C:\_Project\AudioLink\.gradle-home`（全局 `~/.gradle/gradle.properties` 含公司私服凭据） |

---

## 6. 已踩过并已修的坑（别重复踩）

详见 `docs/06-dev-environment.md` §4.1，速查：

1. Cargo 的开发 profile 叫 **`dev`**（`debug` 是保留名）；
2. PowerShell 里 **`$profile` / `$home` 是只读自动变量**，不能当参数名；
3. **Gradle 9 移除了任务级 `exec {}`** → 注入 `ExecOperations`；
4. Tauri 必须有 `desktop/src-tauri/build.rs`（否则 `generate_context!` 报 `OUT_DIR not set`）；
5. Android 依赖**必须显式写版本号**（非 Compose BOM 管辖的）；
6. **`.gitignore` 不支持行尾注释**（会让规则失效）；
7. 仓库必须含 gradle wrapper 三件套；`gradlew` 需 LF + 执行位（Ubuntu CI）；
8. **不要把本机 JDK 路径写进 `android/gradle.properties`**（Ubuntu runner 上路径不存在 → 必挂）。

---

## 7. 待办与遗留

| 项 | 说明 | 优先级 |
|---|---|---|
| 文档脱敏 | `docs/06-dev-environment.md` 含 `C:\Users\liuzh\...` 等本机路径，仓库已公开 | 低（用户未决） |
| GitHub 看板 | 需先 `gh auth refresh -s project`（当前 token 无该 scope），之后可把 M0–M5 建成 Project 看板 | 低 |
| `release.yml` 未跑过 | 首次 tag 触发时才验证；其中 updater 签名产物已暂时关闭（`createUpdaterArtifacts:false`，M5 接入时打开） | 中（M5） |
| CI 可再优化 | `android` job 每次 `cargo install cargo-ndk` 约 2 分钟，可换预编译 action | 低 |
| QUIC 弱网实测 | ADR-004 的回退路径（裸 UDP）是否被触发，取决于 M2 实测 | 中（M2） |
