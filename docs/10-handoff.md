# AudioLink 交接文档（Handoff）

> **性质**：过程性文档。新会话接手本项目时先读本文，再按 `docs/05-roadmap.md` 推进。
> **最后更新**：2026-09-16（接收队列增长已修复，M2 自建 PCM 丢包掩盖已落地）· 远端 https://github.com/GotKiCry/AudioLink（PUBLIC）

---

## 1. 一句话现状

M0 全部完成。M1 的代码面已全部落地；**2026-09-15 真机（MI 8 Lite / Android 10 / API 29）接上后，M1 真机验收首次跑通**：
PC → Android 全链路（真实 QUIC/mTLS → §5 PIN 配对 → Opus → AudioTrack 低延迟输出）已通，**设备侧两项硬指标达标**
（`getPerformanceMode()=LOW_LATENCY`、全链路 48 kHz 零重采样），四轮 60/60/90/75 s 推流零断流。
**e2e 延迟未达标**（目标 P50 ≤ 110 ms；此前实测下限 ≥115 ms + 设备输出 80–101 ms），另有用户试听断续与队列增长问题（`docs/12` §8.2）。
**2026-09-16：Issue #1 的 PCM 长度错误已修复**，真实 Engine/QUIC 收发测试覆盖 10/20 ms；用户已确认新 APK 音频正常传输和播放，Issue #1 已关闭；历史延迟数字仍属修复前结果，定量延迟和长跑验收需另测。

| 项 | 状态 |
|---|---|
| 远端仓库 | **PUBLIC** · main 分支 · 看板 https://github.com/users/GotKiCry/projects/2 |
| 本地三条链 | ✅ 内核 workspace（engine 95 + audio 50 + …）· ✅ Android `assembleDebug` + **73** JVM 用例 · ✅ 桌面 `pnpm build` + Tauri `cargo test` |
| 质量门 | ✅ `cargo fmt --all --check` 干净 · ✅ workspace clippy `-D warnings` 干净（独立复核者自跑 exit 0） |
| M1 真机验收 | ✅ **已跑通**（2026-09-15，真机 MI 8 Lite）：自检 PASS(5/5) / 低延迟模式 ✅ / 零重采样 ✅ / PIN 配对 ✅ / 四轮零断流 ✅ |
| M1 延迟指标 | ⏳ **未完成**：PHK110 修复后待播队列 P50/P95/max = 40/40/60 ms，不再增长；已有段模型下限约 73 ms，但对端解码与 AudioTrack 输出仍未纳入上限 |
| **验收报告** | `docs/12-m1-device-acceptance.md`（含账本、缺陷表、独立复核的 4 处不成立与修订留痕） |
| PC↔PC 实测 | ✅ link-loop 探针「帧封口→出声」P50 40.2 / P95 40.6 ms，零丢包零欠载 |

| M1 交付物 | crate / 路径 | 证据 |
|---|---|---|
| QUIC 通道（控制流 #0 + 数据报 + **§6 时钟同步接线**） | `core/crates/audiolink-net` + `audiolink-engine/src/clock.rs` | 20 单测 + 6 真 QUIC 集成；真机 60/60 探针样本、收敛 1.01 s |
| 自签证书 / 指纹 / 信任库 / PIN 配对 | `core/crates/audiolink-identity` | 22 单测 + 8 黑盒验收 + 真机配对（人工节奏延迟 ≥25 s 提交） |
| 引擎编排（状态机 / 收发管线 / 遥测 / 对端遥测 / 配对死线） | `core/crates/audiolink-engine` | 95 单测 + 10 集成测试 |
| FFI 桥（UniFFI + 跨端 golden vectors 夹具） | `core/crates/audiolink-ffi` | 24 单测 + 2 真 QUIC/FFI 配对与重启回归 + 双 ABI 构建 |
| Android 低延迟播放 + **配对 PIN UI** + **队列水位档位** | `android/app/src/main/kotlin/.../` | APK 构建 + **73** JVM 用例 + 真机验证 |
| 桌面最小 UI（真实引擎，无 mock） | `desktop/` | `pnpm build` + 12 Rust 测试（含接缝）+ 4 端点手工 WASAPI 测试 |
| **PC→真机验收工具**（连接/PIN/推流/跨机账本） | `core/crates/audiolink-tools/src/bin/device_link.rs` | 真机四轮实测 + 本机自检 e1–e8 证据 |
| PC↔PC 端到端验收工具 | `core/crates/audiolink-tools/src/bin/link_loop.rs` | 见上表实测数字 |

**本轮真机实测（`device-link`，真机 MI 8 Lite，20 ms 帧 / 160 kbps Opus）**：
- 网络：跨三层路由（PC 192.168.3.200 ↔ 手机 172.16.2.54/23）。**1 Hz 探测 RTT ~115 ms（无线唤醒代价），推流中 §6 逐探针 RTT P50 11.73 / P95 81.11 ms** —— 只能用推流中口径；
- 链路：四轮 60/60/90/75 s，`late_drops 0`、`plc 0`、丢包末值 0.00%、码率 160.4 kbps；
- 设备：`LOW_LATENCY`、48000 Hz/2ch、flinger `PRIMARY|FAST` + FastMixer 活跃；AudioTrack 请求 960 帧 → **实际 3844 帧（80.08 ms）**，flinger track Latency 101 ms；
- 延迟账本（真实 WASAPI 采集）：采集半周期 10[模型] + 组帧 20[结构] + 网络单向 5.86[模型] + 对端播放环水位 80[实测] = **下限 ≥115 ms**；加设备输出 80–101 ms ≈ **195 ms**。

复测：`target/x86_64-pc-windows-msvc/debug/device-link.exe run --peer 172.16.2.54 --seconds 60 --capture synth`（脚本见 `target/evidence/acceptance/`）。


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
| `docs/17-m2-pcm-concealment.md` | M2 自建 PCM 丢包掩盖的算法、接收接线、回归与真机证据 |
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


### 4.0 最新开发结果与下一项

- Issue #1 已按用户要求关闭，PCM 修复任务 Done；用户确认新 APK 音频正常传输和播放。
- 桌面采集端点选择器已完成：真实枚举/刷新、指定完整 ID、失效与格式提示、实际端点展示；修正开流失败回传。4 个真实 WASAPI 端点验证及 Tauri 窗口检查通过，详见 `docs/13-desktop-capture-selection.md`。
- 上述两项已提交并推送至 `main`：`8262bf5`。
- Android PIN 缺失/残留已改为 Engine 同步快照，清除丢广播缓存；同时修复旧连接退出误删重连会话。3 项内核回归 + 1 项真 QUIC/FFI 配对回归通过，旧 FFI 对照在断开清 PIN 处失败。详见 `docs/14-pairing-state.md`；PHK110 真机已验证显示/错误/成功/断开/到期/锁定，PIN 看板转 Done（`docs/16`）。
- 同端口重启任务已完成：Engine 等全部任务与音频线程退出，net 等真实套接字释放，FFI 启停串行且取消后不遗留半成品。新增 5 项回归，覆盖 15 次五状态重启、并发、取消、IO poller 和未完成 QUIC 握手；既有 PIN 回归恢复同端口。详见 `docs/15-engine-restart.md`。
- PHK110（Android 16 / API 36）真机复现并修复 Android 服务销毁后旧协程回写“运行中”：进程级启停队列 + 实例代次 + 主线程状态发布，4 项新增 JVM 回归及手机普通/快速/推流中/配对中重启通过。新 APK 已安装后回拉核对，详见 `docs/16-android-service-lifecycle.md`。
- 护栏与工具补齐（三项）：① **定长载荷自洽护栏** —— 长度常量改成「字段偏移 + 字段宽度」算式，编解码偏移同源，新增 §3 载荷表独立副本测试 9 项（三组人为破坏均被当场抓红）；② `docs/` 本机绝对路径全部换成占位符（`docs/06` §1 有对照表）；③ `alp2-dump` 支持 pcap / pcapng（按 magic 自动分派、默认按 58290 端口过滤、9 项合成夹具回归 + 端到端实测）。内核 fmt / clippy / 全量测试通过；详见 `docs/20-guards-and-tooling.md`。提交 `6419333` / `402636f` / `2a01879` 已推送 `main`，CI run `35069992337` 四个 job（core / android / desktop / version-consistency）全绿。
- **M2 NACK 重传已落地**：接收侧洞检测（重试 ≤ 5 次 / 间隔 10 ms / 窗口 1 s，最旧的洞先要，1 000 帧跳变按时间轴重建，同时跟踪 ≤ 64 个洞）+ 发送侧 1 s 重传缓冲（同序号只留一份、重发不进发送侧分母）+ 重排窗「有界等重传」窗口（只有存在洞、且播放队列深度 ≥ 2 帧时才等 `min(2×RTT+5 ms, 30 ms)`）。真实双 Engine + QUIC + 有损 UDP 中继（每 10 个丢连续 2 个）实测：**接收侧 `plc_count = 0`**，把 RTT 门控关掉的对照为 16（= 丢包数）—— 15 项单测 + 1 项端到端回归；已知取舍：每次重传仍伴随一次播放欠载（≈20 ms 静音），与抖动缓冲协同优化留待 M2 后续。提交 `2eaff74` 已推送 `main`，CI run `35071772598` 四个 job 全绿。详见 `docs/21-m2-nack-retransmit.md`。
- **M2 soak-runner 已就绪**：新增 `audiolink-tools --bin soak-runner`（真实双 Engine + QUIC 回环，合成源 + NullPlayout，不出声），每 1 s 采接收侧遥测，欠载 / PCM 掩盖 / 丢包 / 迟到 / NACK / 码率偏离 / 挂住任一越界都留**异常快照**（时刻 + 种类 + 当时完整遥测），结束写 JSON 报告（summary + violations + 每分钟粗采样），**退出码即判定**（0 无异常 / 1 有异常 / 2 用法错误）。实测：20 s 冒烟 exit 0（码率稳定 320.8 kbps、全部计数为 0、队列水位 20 ms、RTT 1.8 ms）；把 `--expected-bps` 故意写错 → 6 条 `bitrate_out_of_range` 快照、exit 1。8 项单测；真正的 8 h 长跑尚未执行。提交 `be52681` 已推送 `main`，CI run `35072898528` 四个 job 全绿。详见 `docs/22-m2-soak-runner.md`。
- **M2 自适应码率已落地（§8 的码率档）**：新增 `engine::adaptive`（纯状态机：丢包 > 1% 持续 3 s → −20%，下限 96 kbps；连续 10 s 无丢包 → +10%，上限 320 kbps，每 5 s 最多一级；RTT 超 30 ms 不恢复）与发送侧接线 —— 1 Hz 用**对端** `STREAM_STATS` 判定，结果写进采集线程的 `AtomicI32`（每帧前 load，变化即 `encoder.set_bitrate`，不重建编码器），变更从 `EngineEvent::CodecAdapted` 可观测。实测：端到端从 96 kbps 起步 → 两次恢复事件（96→105.6→116.16 kbps），接收侧链路码率 192.8→233.2 kbps（双发两倍）；破坏性对照（不写新码率）→ 线上不变、测试变红。8 项单测 + 1 项端到端。**未做**：单声道降级（要动 `format::CHANNELS`）与帧长 10 ms 切换（跨端契约），已在 `LinkSeverity` 记账。**另有一个必须记住的发现**：回环上「降级」不可复现 —— 40% 丢包被双发 + NACK 全部修复，接收侧丢包率恒为 0（修好了就不该降码率），降级的真实验收要靠弱网注入。提交 `f66a551` 已推送 `main`，CI run `35074883157` 四个 job 全绿。详见 `docs/23-m2-adaptive-bitrate.md`。
- **M2 弱网测试台与弱网验收已落地**：新增 `audiolink-tools::netem`（确定性注入内核：丢包 / 延迟 / 抖动 / 令牌桶限速，7 项单测）与 `--bin netem-sim`（透明 UDP 中继工具）；`soak-runner` 加 `--netem-*`（长跑与注入一条命令）与 `--tolerant`（弱网档判据：只钉会话不断 + 掩盖比例 ≤ 1%）。**M2 弱网口径实测（60 s，2% 丢包 / 15±15 ms / 5 Mbps）**：全程 streaming、接收侧丢包 0（被双发 + NACK 完全修复）、PCM 掩盖 4 帧（0.13%）、欠载 10、迟到 12、NACK 0、RTT ≈ 41 ms → **判定 ok（exit 0）**；注入自账：观察 5676 / 转发 5571 / 丢 105（1.85%）。顺带修掉一个真实缺陷：`SoakThresholds` 的 `max_underruns/max_plc/max_late_drops/max_nack` 此前定义了却没被判定逻辑使用。结论：弱网下**不该**降码率（链路被修复 = 链路没坏），触发降级需要更高丢包档（记为后续）。提交 `556e504` 已推送 `main`，CI run `35077260006` 四个 job 全绿。详见 `docs/24-m2-netem-sim.md`。
- **M2 遥测面板（曲线 + CSV 日志导出）已落地**：`TelemetryRow` 是**独立的持久化契约**（10 列 camelCase，刻意不复用界面字段名 —— 界面改名不该污染用户已导出的文件），`render_telemetry_csv` 是纯函数（表头 + 一行一采样点、换行收尾），`write_telemetry_csv` 落在 `<用户目录>\AudioLink\telemetry-<毫秒>.csv`（不弹文件对话框 → 无头/CI 也能跑），新增 `export_telemetry` 命令把路径回给 UI 显示。前端 `useAudioLink` 维护 500 ms 一个采样点、上限 600 点（5 分钟）的环形历史，`TelemetryPanel` 手绘 4 条 SVG 折线（端到端延迟 / 缓冲水位 / 码率 / 丢包率；纵轴按本窗口最大值归一化，`non-scaling-stroke` + `preserveAspectRatio="none"` 保证线宽不随容器变形）并给出「导出 CSV（N 点）」按钮。前后端各有一份镜像测试（Rust `TelemetryRow::from_view` + TS `telemetryRowOf`），字段漏了/改名了编译期就红。质量门：内核 fmt / clippy `-D warnings` / 全部测试，桌面 clippy / 15 项测试，`cd desktop; pnpm build`（tsc 严格模式 + vite）全绿。**未做**：真实窗口里的曲线观感、以及「点一下导出真的写到我机器上」需要人看着（本机无头），与 `docs/13` 桌面 UI 同一口径。提交 `87cb059` 已推送 `main`，CI run `35079292910` 四个 job 全绿。详见 `docs/25-m2-telemetry-panel.md`。
- **CI 时间的两条待办被实测处理（一次证伪，另有一处真回收 107 s）**：先量再改。core job 4.0 min = test 139 s + rust-cache 恢复 53 s + clippy 25 s，而 test 里真正的测试运行只有 39.2 s（其余是 9 个本地 crate 编译 + 每个 target 链接一个测试二进制）；android job 3.2 min = assembleDebug 130 s + ndk build 31 s + `cargo install cargo-ndk` **1 s**。两条待办的前提都不成立：① quinn/rustls/tokio 被 rust-cache 以 full match 完整复原（日志里 test 步骤只有 9 个本地 crate 在 `Compiling`），「tools 引入 quinn 后 core 涨到 4m10s」是引入当天的**一次性冷编译**，feature 门控没有可回收的时间，且 core 的 clippy 必须保留 `--all-features` 的 lint 覆盖；② cargo-ndk 安装实测 1 s（rust-cache 默认缓存 `~/.cargo/bin`）。真正能回收的是 android 的 Gradle：缓存 `~/.gradle` 后 **android job 192 → 85 s、assembleDebug 130 → 16 s（↓107 s）**。core 侧做了两项零风险削减（5 个无测试 bin 标 `test = false`；core 的 test 用 `--lib --tests` 跳过 9 个全 0 项 doctest），测试零损失（355 项不变、41 → 27 个 suite），但收益小于 run 间波动（139 → 138 → 167 s），**core 无可测回收，不粉饰**。提交 `6ae748b` / `b7e737f`，CI run `35080728329` / `35081356931` 四 job 全绿。详见 `docs/26-ci-key-path.md`。
- **M3 的「量」这一半就绪：双机录音对齐工具 sync-measure 已落地**（路线图 M3 交付物 5）。新增 `audiolink-tools::syncmeasure`（纯逻辑）+ `--bin sync-measure`：自研 WAV 解析（PCM 16/24/32 与 float 32/64、EXTENSIBLE、奇数块填充、data 写一半按整帧截断）、脉冲粗检测（快攻击慢释放包络 + 阈值穿越插值）、亚采样精细对齐（脉冲邻域归一化互相关 + 抛物线插值）、统计与判定（均值 / P50 / P95(|·|) / 极值 + 漂移 ppm 线性回归）、JSON 报告，退出码 0 达标 / 1 超标 / 2 用法 / 3 输入。**测具先被量一遍**：`--self-test` 合成已知真值（4.7 ms 偏差 + 120 ppm 漂移）逐对比对，实测单对最大误差 **0.010 ms**、漂移 **1.0 ppm**。本轮踩到并处理一个真坑：**单一频率长正弦的互相关有周期歧义**（lag 差一个周期时同样高，偏差因此散开约 1 ms）—— 夹具改用宽带噪声突发，且搜索范围内出现几乎等高的另一峰时该对标 `ambiguous` 并写进报告与 notes；真机验收请用宽带 click 而不是单一频率长正弦。测具不撒谎：静音 / 纯直流 / 检测不到脉冲 / 脉冲不足 / 波形对不上（NCC < 0.5）一律明确报错，负向用例（脉冲串 vs 伪随机噪声）必须报错。端到端：`--self-test --write target/evidence/sync` 写出两段真 WAV（各 192000 帧）再走完整命令行，6 对 NCC ≈ 1.0、逐对偏差 4.708–5.000 ms（真值 4.7 + 0.06·i）、判定 within、退出码 0。16 项单测 + 4 项 CLI 测试。**真机双机同期录音验收（±10 ms P95）尚未执行** —— 工具已就绪，等有第二台设备。提交 `6e2fddd` 已推送 main，CI run `35083496842` 四个 job 全绿。详见 `docs/27-m3-sync-measure.md`。
- **M3 交付物 3（预约播放：接收端按 epoch 排播）已完成**：新增 audiolink-engine::epoch（纯逻辑：EpochSchedule + local_target_us + PlayoutAction 三分支，8 项单测）；播放线程接线 —— 有排播且未到目标时补静音等待且**不推进播放游标**（那帧还要在目标时刻播），超过目标 20 ms 的帧丢弃并计入 late_drops，首次排播发 EngineEvent::PlayoutScheduled 时间线；没设置排播时完全走 M1/M2 的本地游标，老行为一字未改。换算基准取本流首个数据报的 (seq, sample_index)，之后按 48 kHz 帧长推算。控制面新增 Engine::schedule_playout(peer, Option<EpochSchedule>)。公式与符号是本轮最容易翻车的地方：offset = 对端 − 本机，local(epoch) = epoch_local_us − offset，符号反了整组偏两倍偏移量 —— 单测把正负两侧都钉住了。3 项播放面测试 + 引擎 133 项单测全绿。**未做**：§7 的 buffer_ms / dac_latency_ms 补偿项（要设备 DAC 延迟估计）、GROUP_EPOCH 载荷与 dispatch 接线（已另立待办）、真机双机起播对齐验证。提交 `20218cb` 已推送 main，CI run `35085530753` 四个 job 全绿。详见 `docs/28-m3-scheduled-playout.md`。
- **M3 的协议通道接上了：GROUP_EPOCH（0x43）载荷与接线完成**。新增 GroupEpochPayload{epoch_id, epoch_local_us, lead_ms}；dispatch 层把 0x43 从「本里程碑不实现」计数里移出并补上 ControlRequest::GroupEpoch（覆盖度测试与 L1 往返测试同步更新）；接收侧收到后用本机时钟偏移换算并写入排播（与 Engine::schedule_playout 同一条路径）；发送侧新增 Engine::announce_group_epoch(peer, epoch_id, lead_ms)。端到端（真双 Engine + 真 QUIC + 真 PIN + NullPlayout）：0x43 送达、epoch_id 原样到达、target_local_us 是换算后的本机 µs、等待量落在「帧的流内时刻 + 提前量」量级。**本轮澄清了一个容易想错的地方**：target = local(epoch) + sample_index/48000 + lead，接收端等的不是 lead 而是「流内时刻 + lead」（e2e 断言最初就是这么写错的）；推论是 epoch 必须与当前推流位置对齐，否则目标落在过去、帧会被 age 判定全部丢掉 —— 那是既定语义，不是缺陷，但配 epoch 的人必须知道。未做：GROUP_CREATE/JOIN/LEAVE 组管理三帧、真机双机同时出声验证、§7 的 buffer/dac 补偿项。引擎与桌面全量测试、fmt、clippy 全绿。详见 `docs/29-m3-group-epoch.md`。
- **M3 的组管理链路打通：GROUP_CREATE / JOIN / LEAVE（0x40–0x42）与成员账本**。新增三个载荷（成员身份用 NodeId 指纹）+ dispatch 编解码映射与样本（覆盖度测试同步更新，未实现计数只剩 SET_VOLUME_LOCK / TELEMETRY_PUSH）；引擎侧新增组账本（group_id → epoch / lead_ms / 成员有序集）与 API：create_group(成员, lead_ms) -> group_id、join_group、leave_group、groups() 快照，以及 EngineEvent::GroupUpdated；接收侧收到 GROUP_CREATE 就用本机时钟偏移换算并排播（与 0x43 同一条路径）。**动态加入语义**：新成员除 JOIN 外还补一条 GROUP_EPOCH —— 它需要组基准才能排播，老成员不受影响（§7 的 FR-22）。端到端（真双 Engine + 真 QUIC + 真 PIN）：建组后账本恰好一个组、成员表就是被点名的对端、lead_ms 原样保留、成员排播用的 epoch 与建组帧一致、退光后组条目被清掉。**未做**：桌面 / Android 的勾选成组面板与成员同步质量展示（引擎侧事件已就绪）、真机三台同时出声（±10 ms P95）、多会话（≥8 台）、§7 的 buffer/dac 补偿项。引擎与桌面全量测试、fmt、clippy 全绿。详见 `docs/30-m3-group-management.md`。
- **桌面端同步组面板（M3 交付物 4 的 UI 部分）**：引擎侧账本与四帧就绪后，把「同步组」接成界面 —— 新增 4 个 Tauri 命令（list_groups / create_group / join_group / leave_group）、桥接层视图转换（成员短码与 PeerView 同口径；**epoch_id 字符串化**，因为 u64 在 JS 里只有 53 位安全整数，直接传数字会悄悄丢精度）、前端类型 + IPC + hook 动作、以及 GroupPanel（勾选设备 + 提前量 + 建组 + 组卡片含逐成员退出与一键加入），挂在遥测面板上方。验证：Rust 视图转换单测、tsc 严格模式 + vite build、桌面 clippy/测试与内核全量门禁全绿。**未做**：成员同步质量展示、事件驱动自动刷新、真机三台同时出声、多会话（≥8 台）。详见 `docs/31-m3-group-panel.md`。
- **成员同步质量展示（§6.5 分级随成员表出网）**：协议 §7 明写「某成员 Poor → UI 必须明示『该设备同步质量差』」，而质量此前没出网。GroupSnapshot 的成员从裸指纹改为 GroupMember{id, quality, offset_us}，groups() 填入每个会话自己的时钟分级（**没有估计时按 Poor** —— 没有估计等于不可同步；并先放组表锁再查会话表，避免两把锁同时持有）；桌面侧视图加 quality 字符串与 offsetUs，面板按分级上色（poor 红字「同步质量差」、fair 琥珀、good 绿）+ 悬停显示偏移。验证：引擎端到端（成员表 + 质量三值之一）、桌面视图转换单测、tsc 严格 + vite build、内核与桌面全量门禁全绿。**未做**：事件驱动刷新、质量变化的显式提示、真机三台、多会话（引擎里最后一块结构工作：共享采集 → N 会话编码发送）。详见 `docs/32-m3-member-quality.md`。
- **M3 多会话：共享采集枢纽 + start_send_many**（交付物 1 的结构部分）。核心判断写清：**共享采集不是优化而是正确性**问题 —— 各会话各自采集时读取起点与丢样彼此错开几十毫秒，接收端按同一个 epoch 播放时这点错位会原封不动加到组内偏差上。实现：CaptureHub（一路采集 → 切片 → 统一分配 seq/sample_index → 广播；容量 8 帧、慢会话丢旧帧不拖住采集）+ 每会话编码线程（订阅共享帧、用本会话 codec/码率编码，所以各会话仍可各用码率）+ 引用计数生命周期 + Engine::start_send_many + 同引擎必须同帧长的显式约束 + 封口时刻只在枢纽记一次。验证：HubClock 单测（编号与回绕）+ tests/multi_session.rs（1 发 3 收、真 QUIC + 真 PIN，三台接收侧链路码率 ≈ 321–328 kbps）+ 引擎全量与桌面/内核 clippy、fmt 全绿。**那条「接收端 peers() 查不到发起端」的疑点已在下一轮复核并更正**：是测试把断言写在 shutdown 之后造成的假象 —— 链路上对端表一直是 1 条会话，关闭后才清 0。现已改成正式断言（链路上可见 + 关闭后清空），docs/33 §4 保留了这段更正。**未做**：≥8 台真机压测、每会话音量（SET_GAIN 载荷已定义但未接线）、真机三台同时出声。详见 `docs/33-m3-multi-session.md`。
- **每会话独立音量（SET_GAIN / SET_MUTE 接线）**：补上 M3 交付物 1 的另一半。新增 audiolink-engine::gain（纯逻辑：GainState 逐帧逼近 + f32 严格转千分点；NaN/inf/负数/超限一律拒绝，**不静默夹到合法值假装成功**）；播放侧每帧写前应用增益、每帧最多走一个步长（硬切就是爆音）；增益状态挂在播放句柄上 —— **每会话一份**，多会话各调各的而采样时间轴仍共享；SET_GAIN/SET_MUTE 从「明确不做」变成真的生效（含 stream_id 校验，静音用 50 ms 渐变）；新增 Engine::set_peer_gain(peer, gain, ramp_ms)，非法值在发送端就拒绝。验证：8 项纯逻辑单测 + 端到端 tests/gain_control.rs（真 QUIC + 真 PIN，记录写出的 PCM：基线 RMS **0.1772 → 静音 0.0000 → 恢复 0.1773**；越界 3.0 在发送端 Err）+ 引擎 142 单测与全部集成、桌面全量、两边 clippy/fmt 全绿。**未做**：UI 音量控件、真机听感、音量回读、真机三台同时出声。详见 `docs/34-m3-per-session-gain.md`。
- **复核并更正上一轮的疑点：接收端「看不见发起端」是测试时序造成的假象**。上一轮我把断言写在 shutdown 之后，看到对端表 0 条就记成了疑点；本轮按时序采样（0.5/2/3.5/5 s）看得很清楚 —— 链路上一直是 **1 条会话、状态 Streaming/Degraded、遥测有值**，只有关闭后才清 0（正常清理）。处置：multi_session 增加正式断言（链路上接收端恰好看到发起端一条会话 + id/状态正确）与反向断言（关闭后对端表必须清空），docs/33 §4 与交接同步更正。教训写进 docs/35：**断言的位置本身就是结论的一部分**（shutdown 后查表、渐变期测稳态、priming 前量队列是同一类错误）。顺带把 SET_GAIN 通到桌面外壳（EngineBridge::set_peer_gain + 命令 + api/hook 动作，渐变 200 ms；**UI 滑块未做**，因为对端卡片的三元 JSX 要包 fragment 才能并列渲染，不留半成品）。详见 `docs/35-receiver-visibility-correction.md`。
- **桌面端把「音量」与「同步组刷新」都接成界面了**：① 对端卡片在推流中显示音量滑块（0–200%，200 ms 渐变），经 onGain → api.setPeerGain → 桥接 resolve_peer → Engine::set_peer_gain 下发，非法值由引擎边界拒绝并人话化；实现上**刻意没动**原来那个「开始/停止/等待配对」的三元结构 —— 滑块作为独立的 streaming 条件块渲染在操作区之上，不为了让三元并列放两个孩子去包 fragment（上轮就是在这里停的手）。② 同步组面板改成**事件驱动刷新**：桥接新增 audiolink://groups，EngineEvent::GroupUpdated 分支 emit 一条轻量信号，前端 subscribeEvents 的 onGroupUpdated 收到就 refreshGroups() 拉最新（明细仍以 list_groups 为准）；不轮询的理由是建组/加入/退出都是低频人工动作。验证：桌面 clippy（-D warnings）/测试、tsc 严格 + vite build、内核全量门禁全绿。详见 `docs/36-desktop-volume-and-group-refresh.md`。
- **M4 交付物 3（混音器）完成：多路汇聚到一路输出**。新增 audiolink-audio::mixer（纯逻辑 11 项单测）：多路求和 + 软限幅（拐点 0.7 以上指数逼近 ±1，永不削顶）+ 每路增益/静音 + 缓冲满丢最旧 + FR-12 的 8 路上限，并把「参与路数 / 被限幅样本数 / 限幅前峰值」变成可观测数字。引擎接线用 **owner 模式**：同一台接收端只打开一次播放设备 —— 第一个会话是 owner，它的播放线程照旧跑 M1/M2 全套调度（抖动深度、欠载补静音、迟到丢弃、§7 排播），只把写出去的内容换成混音器输出；其余会话只把帧混进来、不碰设备。**理由**：M1/M2 的播放纪律是三轮调出来的，混音只该换「写什么」而不是重写「什么时候写」。每路增益走 §4.1 的 SET_GAIN（各调各的），会话停止时把自己的源摘掉（否则重连几次撞 8 路上限），超限报 STREAM_LIMIT。端到端 tests/mixer_convergence.rs：两个发送端 → 一个接收端（真 QUIC + 真 PIN + 记录 PCM），两路混音 RMS **0.2501 → 静音 A 路 0.1769**，比值 **0.707 = 1/√2**（不相关信号去掉一路的功率半减）—— 这是「两路都真的进了输出」的定量证据。**未做**：Android 内录/麦克风（交付物 1，需真机）、能力协商与降级（交付物 4）、**多路时钟对齐**（交付物 5：不同源按各自 epoch 换算后再混，目前按到达顺序）、真机无爆音与双向验收、8 路压测。详见 `docs/37-m4-mixer.md`。
- **M4 混音对齐：把能观测的做完，把做不到的立项**。mixer 的 MixStats 增加 missing_sources，PcmMixer 累计 partial_frames/total_frames 并给出 MixSnapshot；引擎新增 Engine::mixer_stats()。**掉队帧是对齐程度最直接的读数**：两路都持续供帧时应长期为 0。实测 tests/mixer_alignment.rs（两个发送端 → 一个接收端，真 QUIC + 真 PIN）：**源 2 路 · 帧 298 · 掉队帧 1（0.3%）· 限幅样本 0** —— 证明稳态下两路都在稳定供帧、混音不靠补静音撑场面。**同时把缺口讲清楚（并立项）**：协议 §7 的 epoch 是「发送端指定、接收端排播」，所以同一发送端 → 多台接收端能对齐；但「2 部手机 → 1 台 PC」这种多源拓扑里，**两个发送端之间没有共同时间基准**，采样级对齐做不到。要补需要：① 接收端作为基准来源广播 epoch（协议方向要扩展）；② 发送端「预约发送」（把 epoch 用在自己的发送时间轴上）。这两件已作为独立待办立项，没有塞进文档角落。详见 `docs/38-m4-mixer-alignment.md`。
- 接收待播队列增长、PCM 丢包掩盖、自适应抖动/有界重排和默认冗余双发均已落地。副本延迟一帧并标记 `FEC_REDUNDANT`，接收侧按序号去重且 jitter 只取首个有效副本；60 ms 最高档欠载现在也能按现档重新补水。PC 双 Engine 20 s 稳态码率 320.8 kbps、零欠载/迟到；PHK110 90 s 码率末值/均值 320.8 kbps，队列 P50/P95/max 均 60 ms，链路丢包/PLC 0，欠载/迟到 24/22 且第 75--90 s 无新增，AudioTrack underrun 0、FastMixer writeErrors 0。下一步推进 NACK、码率自适应、真实弱网与 8 h soak；详见 `docs/18-m2-adaptive-jitter.md`、`docs/19-m2-redundant-send.md`。

- **M2 的 8 h 弱网长跑真的跑完了：28800 s，判定器自己被抓出三个缺陷**。
  `soak-runner run --seconds 28800 --netem-loss-pct 2 --netem-delay-ms 15 --netem-jitter-ms 15 --netem-bandwidth-kbps 5000 --tolerant`
  实跑 480 min：采样 28798 次、粗采样 481 桶、异常 7055 条 —— 但**全部是同一类** `bitrate_out_of_range`，
  且集中在 2692..8293 s，后面 5.7 h 再没有一条现场。不是链路上在抖，是**判定器有三个缺陷**：
  ① `--tolerant` 弱网档放宽了欠载/迟到/NACK/瞬时丢包/掩盖，**漏关码率判据**（弱网下瞬时码率随自适应与重传在
  205~427 kbps 摆动，目标 320 kbps，于是弱网验收永远红）；② 字段文档写着「0 = 不判」，判定式 `deviation > tolerance`
  在 0 时却退化成「全判」；③ 快照总量配额（200 条）被刷得最凶的那一类独占，**后半程一条证据都不剩**。
  修法：档位集中到 `SoakThresholds::weak_network(planned_secs, frame_ms)`；码率以 `bitrate_tolerance_pct_x100 = 0`
  显式关掉并补回归测试；快照配额**按类**给（默认每类 25 条），并把 `violations_by_kind` / `last_violation_at_secs` /
  `last_violation_kind` 写进报告。判据与报告单测 13 项（+5），clippy 零警告。同参数 90 s 对照：
  `--tolerant` → 0 异常 / exit 0；去掉 `--tolerant` → 18 条异常 / exit 1 —— 放宽只发生在宽容档。
  **8 h 结论如实记账**：掩盖 950 帧 ≈ 0.066%（弱网档上限 1%）、注入自账 51527 丢包 = 2.01%、
  末次码率 349 kbps / 水位 120 ms / RTT 41.8 ms / 漂移 −4 ppm。这次**没有**给出「通过」：修完判据需要重跑一次，
  且长跑期间这台机器同时在编译与测试（CPU 争用使欠载/迟到计数偏保守）。详见 `docs/22-m2-soak-runner.md` §9 与 `docs/24-m2-netem-sim.md` §3。
- **长跑产物有了离线判定通路**：新增 `tools/soak-report.ps1` —— 读 soak-runner 的 JSON 报告，输出违规按类聚合
  （次数 + 首次/末次秒数 + 样例）、终值遥测与人话判定；退出码 0/1/2 与 soak-runner 同口径，`-Json` 供 CI / 看板回填。
  用四个夹具自测：`smoke.json` → exit 0、`negative.json` → 6 条 `bitrate_out_of_range` / exit 1、缺失路径与非 JSON → exit 2；
  8 h 与 90 s 报告均已复算（旧 schema 报告仍可解析）。提交 `fc05568`。详见 `docs/22-m2-soak-runner.md` §8。

- **M5 分 ABI 打包：`assembleRelease` 从「一个 7.84 MB 的合包」变成「两个按 ABI 的包」**。路线图 M5 交付物 3 写的是
  「APK×2 ABI」，此前产出的却是 universal 单包 —— 每台设备都下载一份用不到的本地库。改为 `splits.abi`
  （`isEnable` + `include(arm64-v8a, armeabi-v7a)` + `isUniversalApk = false`）：`app-arm64-v8a-release.apk` **5.21 MB**（−33.5%）、
  `app-armeabi-v7a-release.apk` **3.84 MB**（−51.0%）；两个包都用 `apksigner verify --print-certs` 验过 **V2 签名**，
  并用 zip 目录逐条核对「每个包只含自己的 ABI」；debug 变体同样分 ABI（16.54 / 15.17 MB，原 19.17 MB）。
  **踩到一个真缺陷**：ABI 列表先被写在 `ndk.abiFilters` 与 `splits.abi.include` 两处，AGP 直接以
  `Conflicting configuration ... cannot be present when splits abi filters are set` 拒绝 —— 现在 ABI 只在
  `splits.abi.include` 声明一处。顺带给 `build-rust.ps1` 补护栏：清理不属于本次 `-Abi` 的旧 ABI 产物
  （残留的 `.so` 会被挡在包外，却让人误以为那个 ABI 还在支持）。
  **支持集合保持 FR-40 的 arm64-v8a + armeabi-v7a 不变**：我先按常识假设成 x86_64，读到 FR-40 后全部回退 ——
  需求文档是契约，既不该凭「现代设备都是 arm64」收窄，也不该凭「模拟器方便」扩张。
  **仍未验**：两个包装到真机跑一遍（真机阻塞项）。详见 `docs/42-m5-release-pipeline.md` §9。

- **M5 桌面端中英双语落地：139 条界面文案变成「一张表 + 一个函数」**。此前文案全散在 JSX 里写死中文（`App.tsx` 顶部还留着
  「不做：双语」的 M1 范围注释）。新增 `desktop/src/i18n.ts`：`zh` 表 139 条（`as const` 定义键集合）+ `en` 表声明为
  `Record<MessageKey, string>`（英文漏一条 tsc 就红）、模块级 `t(key, vars)` 与 `useLocale()`（`useSyncExternalStore` 订阅，
  语言一变 App 重渲染，`t` 是渲染期求值所以整棵树跟着换）；语言偏好走 `settings.json` 的 `locale` 键（新命令 `locale` /
  `set_locale`，`set_locale` 只接受 `zh-CN` / `en-US`，切换顺序是**先立即生效、再落盘**）。18 个文件；可机械化的部分
  （字符串字面量 / JSX 属性 / 单独成行的文本节点）用脚本按「中文原文 → 键」一次替换 —— **字典即映射表**，
  不另维护对照表；13 处模板串与混合节点手工改。**残留中文 UI 文案 0**。**两个值得单独记的坑**：
  ① `PEER_STATE_LABEL` 与 `VERDICT_TEXT` 原本是**模块级常量** —— 常量只在模块加载那一刻求值，语言切换后整个界面都变了、
  只有这几个字不动，且不报错不崩溃，最容易留在发布版里；两个都改成函数并写明理由。② `TelemetryPanel` 里 `const t = telemetry;`
  把翻译函数**遮蔽**了，`t("tm.peers")` 被解析成「调用一个遥测对象」，改名 `tele` 才过。**新增护栏**
  `desktop/scripts/check-i18n.mjs`（已接进 `pnpm build`）：键集合 + 占位符集合 + 空文案三项 —— tsc 管不了占位符，
  英文漏写 `{name}` 会在界面上静默少一个值；破坏性验证过（去掉 `en` 的 `{name}` → exit 1，恢复 → exit 0）。
  验证：`pnpm build` 通过（36 modules）、残留扫描 0、`cargo clippy -p audiolink-desktop -D warnings` 零警告、desktop 测试通过。
  **未验**：没有真实 GUI 会话实际切换语言看观感（中英文案长度差异导致的换行/按钮宽度未看）；Android 侧仍是单语言
  （`values-en/` 未做 —— 路线图 M5 的「中英双语」写在桌面交付物里，不擅自扩张）。详见 `docs/45-m5-bilingual.md`。**补记（流程漏项）**：这次 CI 第一轮挂在 `core-light` 的 `cargo fmt --all --check` —— 本地只跑了 clippy 与 test、没跑 fmt 就提交了；已格式化并重跑。**门禁清单里 fmt 不是可选项**。

- **M5 绿色版（便携包）落地：同一个 exe 的另一种形态**。路线图 M5 交付物 3 要的是「安装包 + 绿色版 + APK×2 ABI」，
  前两轮补了后两项，这一轮补绿色版：新增 `tools/tauri-portable.ps1` —— 把 exe + `THIRD-PARTY-NOTICES.md` +
  `README-portable.txt` 打成一个 zip（**5,373.7 KB**，exe 14,826 KB 压缩后），并输出 SHA256 校验和文件。
  **一个真实的正确性点**：Tauri 的 `resource_dir()` 在便携形态下**就是 exe 所在目录**，所以第三方声明必须与 exe
  同级 —— 不摆的话「关于 / 第三方声明」面板显示「未找到声明文件」，而那是合规项不是装饰；安装版由
  `bundle.resources` 处理，绿色版得自己摆。配置仍写用户目录（`%APPDATA%\com.gotkicry.audiolink`），README 里
  写明「设置不跟着 zip 走」，免得用户拷到另一台机器以为丢了。**验证**：zip 内容逐条核对；`-Verify` 开关
  解压到临时目录并**真启动一次**（进程存活 8 s，不是「应该能启动」）；CI 侧 `release.yml` 的 desktop job
  加了打包步骤，产物随 artifact 进 Release（`files: dist/**/*` 自动带上）。**未验**：`window-state.json` 的
  时间戳在这次启动里没变 —— 因为 `-Verify` 用 `Stop-Process -Force` 收尾，而窗口状态是正常退出时才写，
  且 AudioLink 是「关窗不退出」（托盘常驻），无法在不做 GUI 交互的前提下优雅退出；「配置写用户目录」因此按**代码事实**
  陈述（`app_config_dir()`，与安装形态无关）。绿色版也没做代码签名（与安装包同一条待办）。详见 `docs/42-m5-release-pipeline.md` §10。

- **M5 用户文档落地：用户手册（中/英）+ 故障排查（中/英）+ `CONTRIBUTING.md`**。路线图 M5 交付物 4 此前**完全没做**
  （仓库里连 `CONTRIBUTING.md` 都没有）。新增 `docs/manual/user-guide.{zh-CN,en-US}.md`（两份同构，13 节：装哪个形态、
  三步配对、推流与接收、Windows 采集、设置、同步组与多源对齐、遥测读数、配置与数据位置、卸载、许可与第三方声明、
  以及**「还没做的」**）与 `docs/manual/troubleshooting.{zh-CN,en-US}.md`（每条都来自真实踩坑：跨网段互 ping 不通、
  AP 客户端隔离、播放器独占模式采不到、MIUI `adb install -99`、装错 ABI、关窗不退出是设计、SmartScreen 未签名、
  绿色版缺 WebView2、自动更新的前置条件），以及仓库根 `CONTRIBUTING.md`（先读顺序、跨端契约纪律、
  **门禁清单含 `cargo fmt --all --check`**、提交信息约定、注释写「为什么」、界面文案走 i18n、纯逻辑优先、诚实记账、
  护栏要自证、看板同步、许可审计）。**顺手修掉一个对外缺陷**：`README.md` 第一屏还写着「状态：规划完成，等待开发
  （`v0.1.0` 尚未实现）」—— 而安装包 / 绿色版 / APK 都已产出、CI 全绿；已改为真实状态并加「用户文档」索引表。
  手册只写**已实现**的能力：Android 内录与麦克风、Android 自启与省电引导、代码签名都明确列在「还没做的」里，不粉饰。
  **未验**：手册里的操作步骤没有逐条在真机上走一遍（需要真机，与 M1/M2 真机验收同一条阻塞）。
  详见 `docs/manual/`、`CONTRIBUTING.md`。

- **M4 能力协商（内核侧）落地：把「连得上但没声音」提前到握手期拒绝**。M4 交付物 4（能力协商与降级）此前完全没做。
  新增 `audiolink_types::Capabilities` 位图（`OPUS` **必需**，`PCM16`/`CAPTURE`/`PLAYOUT`/`SYSTEM_LOOPBACK`/
  `MICROPHONE`/`MIXER`/`GROUP_EPOCH` 可选）+ `intersect` / `missing_required` / `describe` / `unknown_bits`；
  `HELLO` 加 `caps`、`HELLO_ACK` 加 `caps` + `agreed_caps`；握手期算交集，**缺必需位当场拒绝** ——
  沿用版本不匹配的既有范式（`accepted=false` + 人话 reason + 事件里的错误码），用既有的 `1005 CAP_UNSUPPORTED`，
  **没有新增错误码**。拒绝原因把双方各自有什么、缺哪一项写在一行里，直接给用户与排障人看。**两条设计判断**：
  ① 只有不可替代的能力才标必需（现在只有 Opus）—— 标多了会把「能用但功能少」误判成「连不上」，比不做协商更糟；
  ② 可选位缺失**不拒绝**，交给调用方降级。另加**双向复核**：发起方自己再算一遍交集并与对端报来的 `agreed_caps` 比对，
  两侧算法一致时是恒等检查，一旦将来只改了一侧就立刻失败。`Capabilities::CURRENT` 只声明内核真做到的：
  内录与麦克风要平台侧接上才算，**现在不声明** —— 宁可少声明，也不要声明一件做不到的事。实测：位图单测 5 项
  （新文件 `core/crates/audiolink-types/tests/capabilities.rs`）+ 握手单测 2 项（缺必需位 → 拒绝且点名「Opus 编码」；
  能力不同但兼容 → 两端协商出同一交集），引擎 146 项单测全过、clippy 零警告、fmt 通过。
  **测试抓出一个小问题**：`describe` 对「全是未知位」的位图返回「无」，界面会显示成「对端没有能力」——
  而真相是「对端比本端新，声明了本端看不懂的位」；已改为输出「未知能力」。**未做**：引擎层查询 API 与事件
  （`Handshake::agreed_caps()` 有了，但桌面端还看不到协商结果 —— 已立看板条目）、界面降级置灰、平台能力注入
  （`with_capabilities` 就是留的口子，测试已在用）。详见 `docs/46-m4-capability-negotiation.md`。

- **M4 能力协商接进界面：协商结果不再只躺在引擎里**。上一轮把协商做进握手（`HELLO`/`HELLO_ACK` 位图 +
  `CAP_UNSUPPORTED` 拒绝），但结果只有 `Handshake::agreed_caps()` 能看见 —— 界面看不到，用户仍然不知道
  「为什么同步组对这一台用不了」。这一轮接通 `HandshakeEvent::Established`（带 `local_caps`/`peer_caps`/`agreed_caps`，
  自包含）→ `PeerSession.capabilities` → `PeerStatus.capabilities` → 桥接层 `PeerView.capabilities`
  （`PeerCapabilitiesView`：`local`/`peer`/`agreed`/`missingOnPeer[]`，**位图在 Rust 侧就翻成人话** ——
  把 `1 << 3` 丢给前端等于每个前端各实现一遍解释）→ 对端卡片一行「可用：…」，本端有而对端没有的能力非空时补一句
  「对端不支持：…」。`missingOnPeer = local & !peer`，是「置灰」那一半的直接输入。
  **契约形状测试立刻抓到了变化**：`PeerView` 多了字段 → `peer_and_local_json_shape_match_contract` 变红
  （字段名漂移会让前端静默拿到 `undefined`，这正是它的用途）；已扩成「未协商（null）」与「已协商（含嵌套与数组）」两个形状。
  验证：引擎 170 项、desktop 27 项、`pnpm build`（tsc 严格 + i18n 护栏）全过，fmt/clippy 零警告。
  **仍未做**：真正禁用依赖该能力的入口（现在只**说明**缺什么）；平台能力注入（`with_capabilities` 口子已开）。
  详见 `docs/46-m4-capability-negotiation.md` §8。

- **M4 能力协商的「降级」落地：把「不能用」变成「选不了」**。前两轮只做到「说明」（卡片上写清对端缺什么），
  这一轮把它变成行为：建组勾选列表里，对端交集不含 `GROUP_EPOCH` 时复选框**直接禁用**。
  **一条必须坚持的边界**：界面判断不能去解析中文文案 —— 拿 `agreed` 那串中文 `includes("同步组预约播放")`
  会「改一次文案断一次判断」，而且断得静默（界面照常显示，只是不再置灰）。所以位图现在出**两条**：
  `agreed`（人话，给人看）与 `agreedKeys`（键，给判断用）；`Capabilities::key()` 与 `name()` 并列，
  并有测试钉住「每个已知位都有独立且非 unknown 的键」「键不重复」。**另一个刻意的区分**：`capabilities === null`
  （还没协商完）也禁用，但说的是「等待能力协商…」而不是「不支持」—— **「等一下」与「别等了」是两件事**，
  混成一句会让用户在握手那几百毫秒里以为设备不兼容。验证：types + engine 171 项、desktop 27 项、
  `pnpm build`（tsc 严格 + i18n 护栏）全过，fmt / clippy 零警告。**仍未做**：无损档 PCM16 与混音路数在界面上
  还没有控件，等有了按同一模式处理；平台能力注入仍缺（口子已开）。详见 `docs/46-m4-capability-negotiation.md` §9。

- **M4 平台能力注入落地：把「内核能做什么」与「这台机器能做什么」分开**。此前 `Capabilities::CURRENT`
  被写死在握手构造函数里，于是「所有设备都声称自己一样」—— 能力协商只剩形式。现在 `EngineConfig.capabilities`
  （默认仍是 `CURRENT`）由**平台侧**声明并带进握手：桌面端（Tauri）声明 `CURRENT | SYSTEM_LOOPBACK`
  （WASAPI loopback 采集已实现），Android（FFI）保持 `CURRENT` 不声明内录 —— **未实现的就不声明**，
  并留了注释说明接上之后怎么打开那一位。**端到端证据**（不是「字段赋值对不对」）：新增
  `core/crates/audiolink-engine/tests/engine/capability_negotiation.rs`，起两个真实 Engine（真 QUIC + PIN 配对），
  一端声明内录、另一端不声明，断言**对端** `peers()` 里读到的那份能力：`peer` 位含内录、`local` 位不含、
  `agreed` 交集不含 —— 正是界面按 `missingKeys` 置灰用的输入。验证：types + engine **172 项**测试全过
  （新增 1 项端到端）、desktop 27 项、fmt / clippy 零警告。**仍未做**：Android 内录本身（`AudioPlaybackCapture`）、
  无损档 PCM16 与混音路数在界面上还没有控件。详见 `docs/46-m4-capability-negotiation.md` §8.3.5。

- **M1 端到端延迟进 CI：从「人工跑一次看数字」变成「每次都被测」**。M1 的验收线（P50 ≤ 110 ms / P95 ≤ 150 ms）
  此前只有人工测量 —— `tools/link-loop` 跑一次、人眼看数字；问题不在测量本身，而在于**人工测量不会在回归时报警**：
  某次改动让延迟翻倍，要等下次有人想起来跑一遍才会发现。新增 `core/crates/audiolink-engine/tests/engine/latency_budget.rs`：
  两个真实 Engine（真 QUIC + PIN 配对）+ 合成源 + `NullPlayout`，两端装同一个 `MeasurementTap`（发送侧记「帧封口」、
  接收侧记「sink 写出」，按 seq 配对）—— 量法与 `link-loop` 完全同一套，不另写一份。**实测基线（本机，且 8 h 长跑同时在跑）**：
  P50 = 40 615 µs、P95 = 41 007 µs、P99 = 41 132 µs，约等于「两帧」（20 ms 编码 + 20 ms 抖动缓冲），
  与项目历史记录（40.28/40.73 ms）一致。阈值取实测的 2–3.7 倍（P50 ≤ 80 ms、P95 ≤ 150 ms）——
  **按「实测 × 倍数」定而不是按验收线定**：宁可漏报一次轻微退化，也不要让 CI 因共享 runner 抖动随机变红，
  因为一条会随机变红的护栏很快会被学会忽略。**边界写清了**：回环没有 Wi-Fi 抖动与扬声器缓冲，
  这个数字**不能**用来宣称 M1 验收通过，它只回答「延迟有没有突然变坏」。验证：engine 167 项全过（含本项）、fmt / clippy 零警告。
  详见 `docs/47-m1-latency-guard.md`。

- **M5 更新清单与产物同源校验：真的验了一次签，并当场抓到「清单过期」**。自动更新链路是「读 `latest.json` → 下载 →
  **验签** → 安装」，前两步肉眼可查、第三步只有算一遍才知道。新增 `tools/check-update.mjs`：读清单与 `tauri.conf.json`
  的公钥，对本地安装包做**真正的 Ed25519 验签**（minisign prehashed 模式，签名对象是 `BLAKE2b-512(file)`）。
  **为什么用 Node 而不是 PowerShell**：Ed25519 与 BLAKE2b-512 都是 Node 内置，而 PowerShell 的 .NET 没有 BLAKE2b、
  也不保证有 Ed25519 —— 自己实现一遍密码学不是这里该做的事。**第一次运行就报错**：清单里的签名（420 字符 base64）
  与 `.sig` 文件（560 字符）不是同一份，时间戳显示清单比产物**旧了 9 小时**（清单是当初生成的，之后重新构建过安装包
  而没人重新生成清单）—— 客户端拿这份清单更新会**验签必然失败**，用户看到的是一句「签名不匹配」，真正原因藏在别处。
  已用 `tools/tauri-latest-json.ps1` 重新生成并**验签通过**。验证（含破坏性）：同源 → ✓ 通过 [prehashed (BLAKE2b-512)]；
  **改一个字节 → ✗ 失败**（证明真在验密码学）；用过期副本 → ✗ 失败（能区分串不串）。已接进 `release.yml`：
  生成清单后跑一次，**清单过期会让发布流程当场失败**。**仍未做**：真正的「装上去」（两版本 + Release 托管 + 干净机器）。
  详见 `docs/42-m5-release-pipeline.md` §11。

- **M3 的「动态加入」补上自动化验证：运行中第 3 台加入，前两台不中断**。M3 验收表四条（并发 / 同步 / 动态 / 时钟）里，
  并发由 `multi_session` 覆盖、同步要真机同期录音、时钟由 `clock_sync` 覆盖 —— **只有「动态」没有任何自动化证据**，
  而它最容易出问题：新会话加入要新建采集订阅、编码器、QUIC 会话，任何一处争用都可能让**已在跑的会话**停顿。
  新增 `core/crates/audiolink-engine/tests/engine/dynamic_join.rs`：两台先开流跑 4 s，第三台在运行中加入
  （连接 → PIN → 开流），再跑 4 s 量三台各自的**样本增量**。判据刻意用「接收侧 sink 的样本增量」而不是
  「状态还是 Streaming」—— 状态是采样值，断流几百毫秒可能采不到；样本数不增长才是直接证据。
  实测：前两台各新增 **387 840**、第三台 382 080 个样本，理论满额 = 4 s × 48 kHz × 2 声道 = 384 000 ——
  **第三台的加入没让前两台掉一个样本**。**顺带修正面板**：M3 父项此前标 Todo，但它的五条交付物
  （多会话 / 时钟同步 / 预约播放 / 同步组 / sync-measure）全都有 Done 条目，剩下的只是真机验收与 ≥8 台压测 →
  已改为 In Progress。验证：engine 168 项全过、fmt / clippy 零警告。详见 `docs/33-m3-multi-session.md` §6。

- **M3「组内同步 ≤ ±10 ms」补上回环量法，并顺手发现一个待观察的行为**。验收方法原本写的是「双机同期录音 + 波形对齐」，
  那要两台设备；但这条指标在同一进程里就能量：两个接收端共用同一个单调时钟，记下各自 sink 写出每一帧的时刻，
  「第 k 次写出」的时刻差就是播放偏差。新增 `group_sync.rs`（1 发送 + 2 接收 + 建组同一 epoch 排播 + `start_send_many`）。
  **量法出两个数字**：`absolute`（偏移 0，含起播差 —— 对应验收线）与 `aligned`（扣掉整数帧偏移后的抖动 —— 护栏断言用）。
  **连跑 4 次**：第 1 次 19.65/20.14 ms（两端起播**整整错开一帧**），后三次 0.02–0.51 / 0.56–0.80 ms（偏移 0 帧，
  几乎完全同步）。结论：常态远优于 ±10 ms，但观察到过起播错一帧 —— 样本不足以断定是缺陷还是偶然，
  所以**立了一条观察项**而不是写成断言（会偶发红的断言只会被忽略；但问题必须记下来）。
  验证：engine 169 项全过、fmt / clippy 零警告。详见 `docs/33-m3-multi-session.md` §7。

- **M3「组内起播偶发错一帧」查清并修掉了：两层原因，改完 4 次全 0.00 ms**。第一层：`EpochSchedule::action`
  直接比较「现在 ≥ target」而 target 是任意微秒时刻 → 两端判定点落在它前后 1 µs 就一头等一头播；
  修法是把 target 向上对齐到帧边界（含回归测试 `both_ends_decide_the_same_within_one_frame`）。
  **第二层才是根因**：判定发生在**每一拍**上，而播放线程的 `next_write` 初始化成 `Instant::now()` ——
  **线程启动那一刻**；两端启动时刻不同，拍点相对全局帧网格的相位就不同，一端落在目标前、一端落在后，
  **起播整整差一帧**。这也解释了观测到的结果只有「0 或 ~20 ms」两种而非连续抖动 —— 相位差是固定的。
  修法：起播时把拍点**锚到全局帧网格**（`now_monotonic_us() % frame_us` 的下一个整数倍），之后每拍保持同网格。
  **实测**：从「5 次里 2 次 19.7/20.2 ms」变成「连跑 4 次全部 P50 0.00 ms / P95 0.03–0.04 ms」（偏移 0 帧）——
  比 ±10 ms 验收线低约 5 个数量级。**护栏升级**：断言从 `aligned`（扣掉起播差）改为直接盯验收线 `absolute ≤ 10 ms`，
  另加扣帧后 ≤ 5 ms 的自检 —— 当初不敢这么断言正是因为这个偶发缺陷。验证：engine 170 项全过、fmt / clippy 零警告。
  详见 `docs/33-m3-multi-session.md` §7.5–7.6。

- **M3「时钟」验收补上回环证据：稳定后 offset 抖动 ≤ 2 ms**。验收表第 4 条写的是「稳定后 offset **抖动**」，
  而 `clock_sync` 此前只断言了**收敛**（2 s 内攒够 8 个样本、单次 |offset| ≤ 2 ms）—— 那是「一次快照准不准」，
  不是「稳定之后抖不抖」；缓慢漂移与周期性跳变都不会体现在一次快照上。新增
  `offset_jitter_stays_within_two_milliseconds_after_convergence`：配对后每 100 ms 采一次估计
  （探测间隔也是 100 ms），跑 12 s，丢掉前 2 s 收敛期，取稳定期 offset 的极差。**实测**：
  `稳定期 83 个样本 · offset 21..24 µs · 抖动 3 µs` —— 比 2 ms 验收线低约 600 倍；同期收敛测试
  offset = 73 µs、quality = Good、RTT P50 = 321 µs。**边界**：两引擎同进程共享单调时钟，真实偏移恒为 0，
  所以 3 µs 完全是「估计器 + 收发节奏」的误差 —— 它证明估计器本身稳，真机上的晶振漂移仍属真机验收。
  代价：这条跑 12 s，进 core-heavy 的 test 步骤。详见 `docs/33-m3-multi-session.md` §8。
- **M2 断网自愈边界实测 + QUIC 链路时间参数可注入**：`EngineConfig` 新增 `idle_timeout` / `keep_alive`
  （默认由硬编码的 10 s / 3 s 改为 30 s / 1 s）与 `with_link_timeouts()`，`Engine::start` 不再写死这两个值；
  新增 `tests/engine/network_outage.rs`（双向闸门 UDP 中继模拟拔网，闸门关上=两个方向都丢）。三点实测：
  拔网 2 s → 恢复 0.57～0.74 s；4 s → 0.90～1.13 s；**10 s → 4.24～4.84 s（连测四次，稳定）**。
  关键改进：idle_timeout 提到 30 s 之后，10 s 拔网从「会话终结」（两侧 `peers()` 里对端消失、8 s 观测窗毫无恢复）
  变成「能自愈」。**未达标**：恢复延迟由 QUIC 的 PTO 指数退避决定（静默越久，下一次探测排得越晚 ——
  2 s→0.6 s、4 s→0.9 s、10 s→4.5 s），仍越过 M2 验收的 3 s 预算；缺口与四个候选方案记在
  `docs/48-m2-outage-boundary.md` §5（首选应用层主动探活，其次接上 `session.rs` 里已存在但未接线的 FR-27 重连状态机）。
  另记一个判据陷阱：第一版把「拔网前的通道积压」读成 **1.1 µs 恢复**，修正为带时间戳的判据
  （恢复必须由晚于插回时刻的非静音样本证明 + 拔网期间 400 ms 后必须真的安静 + 恢复后 2 s 持续出声）。
  质量门：fmt / clippy -D warnings / 全套 engine 27 项测试全绿。提交 `fa2c466`（代码）与 `61601ea`（文档 + 看板），
  CI run `35151080904` 五个 job（core-light / core-heavy / android / desktop / version-consistency）全绿。
- **新发现的真实缺陷（未修，已记账）**：CI run `35151576499` 的 `core-heavy` 红在 `group_sync::two_receivers_play_the_same_frame_within_ten_milliseconds` ——
  绝对偏差 P50 **19.98 ms** / P95 **20.38 ms**（正好一帧），而扣掉起播差只有 0.01 / 0.02 ms：**排播准、起播差一整帧**。
  复现率：CI 4 次里 1 次、本机 12 轮里 2 轮、单跑 3 次 0 次（负载相关，约 20～40%），重跑即绿。
  根因：播放线程的起播锚点只把拍点对齐到**全局帧网格**（固定相位），但没有固定**落在哪一格** ——
  两端进入攒帧完成分支的时刻相差 Δ，若 Δ 之间恰好夹着一个网格边界，两端就指向相邻两个边界，起播差一帧（踩中概率 ≈ Δ / frame_us ≈ 20～40%，与实测吻合）。
  修法（未实施，见 `docs/49-m3-group-start-phase.md`）：① 首拍锚到 `EpochSchedule` 的**绝对目标时刻**（与各自准备时刻无关）；
  ② `frame_aligned_target_us` 的取整从「向上取整」改为「贴近最近边界」（两端目标只差 ≤2 ms 的时钟估计误差，snap 后必然同界）。
  看板已新增 In Progress 行跟踪；验证基线 = 连跑 20 轮统计失败轮数。
- **第三层根因已修（2026-09-17）**：`EpochSchedule::target_us` 不再做任何网格对齐、新增 `phase_us`
  （相位由该帧 target 算出，且必须传真实序号）、播放线程起拍锚点 `playout_start_anchor` 改用**待播帧 target 的相位**。
  理由：根因不是「取整方式」而是**参考系** —— 拍点锚全局网格、target 又取整到同一网格，两端时钟估计差 ε 就会落到相邻两格。
  实测：单次全套 `[group-sync]` 绝对偏差 **P50 0.00 ms / P95 0.02 ms（偏移 0 帧）**（修复前 19.98 / 20.38 ms）；
  `epoch.rs` 单测 14 项全绿（含「target 恒落在本端拍点上」「两端 ε 内必同拍」「早起容差吸收唤醒抖动」三条回归）。
  **但复现率没有收敛**：四次 20 轮采样分别是 50% / 30% / 20% / 25%（基线 17%），失败样本始终是同一条
  `group_sync` 断言、始终是「整段差一帧、扣帧后 0.00 ms」，看板因此**保持 In Progress**，不假装修好了。
  三次改动各消除一条真实机制（取整跨界 / 唤醒抖动二值化 / 升档 Hold 后移时间轴），但没有一条是主因。
  **本轮还做了一次关键对照**：把这条测试**单独**跑 30 次（去掉 27 项测试争抢）仍是 **6 / 30 失败（20%）**，
  形态与并行时完全一致 —— 并行竞争被排除，20% 是真实独立失败概率（此前几次「改善」很可能也是噪声）。
  **下一步第一嫌疑**：排播可能在**起拍之后**才生效（`start_send_many` 先跑、`GROUP_EPOCH` 后到），
  于是拍点相位与 target 不同相、部分帧白等一拍 → 整段差一帧；修法方向是 schedule 首次生效时重锚拍点。
  详见 `docs/49-m3-group-start-phase.md` §5.3。
- **第 57 轮诊断（把「差一帧」钉到起拍那一拍）**：给 `group_sync` 加了两个诊断量（两端写入次数、首次偏离的位置 k）。
  失败样本显示 **k = 0**（19.95 ms）且两端写入次数只差 1（298 vs 297）—— 偏移从**第一次写出**就存在，
  不是中途漂移也不是丢帧补拍，而是**起拍整体晚了一拍**。同轮试的第 5 个机制（`PlayoutSync.generation` 生效重锚）
  单跑 30 次 8/30、与基线 6/30 无差别，已回退。**五个机制假设至此全被数据否掉**，失败率始终 20~27%。
  下一步很具体：诊断里补两端的 `PlayoutSync.base`、起拍 `next_write` 与首帧 `target_us`、`wait_us` ——
  第一嫌疑变成「两端锚定时用的首帧不是同一帧」（`playout_start_anchor` 取队列最旧一帧，而真正的首帧可能还在路上）。
- **第 58 轮突破：排播根本没生效**。诊断做到「这一端有没有进过排播」后，失败样本显示 **排播事件 A=[] · B=[]** ——
  两端从头到尾走的是**本地游标**（队列攒够帧就播），起拍时刻由到达时序决定，所以偶尔差一帧、`aligned` 却恒 0.00 ms。
  根因是**产品缺陷**：`GROUP_EPOCH` 常常**早于开流**到达，而 `handle_control` 那个分支只在 `playback` 已存在时才
  `set_schedule`，否则**静默跳过**（测试流程恰好是 `create_group` → `start_send_many`，20% 就是这场时序竞争的概率）。
  **这也解释了前六个机制假设为何全都不动失败率 —— 它们改的是排播内部逻辑，而排播压根没跑起来。**
  影响：`[M3] 组内同步回环护栏`（Done）此前测的是两个本地游标播放器的相位差，不是 epoch 排播的同步质量，必须重测。
  下轮修复：给 `PeerSession` 加「待应用排播」槽位，`OpenStream` 建好播放句柄后立刻应用；`Engine::schedule_playout` 同样处理。
- **第 59 轮：修好了**。`PeerSession` 加 `pending_schedule` 槽位 + `apply_schedule` / `stash_schedule` /
  `take_stashed_schedule` / `clear_stashed_schedule` 四个小函数，把「有播放句柄就应用、没有就暂存」集中到一处；
  三处控制路径统一走它并在开流后补应用：`GroupCreate`（**这才是实际路径** —— `create_group` 只发 GROUP_CREATE，
  第 58 轮只改了 `GroupEpoch` 分支，所以那次修复后排播事件仍然是空的）、`GroupEpoch`、`SchedulePlayout`（不再回
  `cap_unsupported`），外加 `OpenStream` 建好句柄后立刻补应用。
  **实测：单跑 30 轮 0 失败（修复前 5~8/30）、排播事件 30/30 非空、绝对偏差 P50 0.00 ms / P95 ≤ 0.05 ms；
  两端拿到的 `target_local_us` 完全相同**。引擎集成 27 项 + 单测 150 项全绿。
  同批次三处改动（target 不再全局网格对齐 / 首拍锚 target / 2 ms 早起容差）到这一步才第一次被真正检验 ——
  此前「无效」是因为排播没跑起来。教训：连续多次改机制都不动指标时，先问「这段代码执行了吗」。
- **第 60 轮：补护栏**。`group_sync` 此前只**打印**排播事件、从不断言，于是上述缺陷能潜伏好几轮（本地游标模式下
  偏差只是「偶发差一帧」）。现在加了两条硬断言：两端 `PlayoutScheduled` 必须非空、且两端 `target_local_us` 之差
  ≤ 1 ms（实测恒为 0 µs）。有效性有对照：第 59 轮实测显示修复前是 **0 / 30 轮有排播事件** —— 同类回归会当场变红。
  连跑 10 轮全绿；排播生效后 M3 四条回环（并发 / 同步 / 动态加入 / 时钟稳定性）在同一轮全套里重跑通过，
  这也是它们第一次在**排播生效**条件下被检验。
- **第 61 轮：补「动态加入同步组」的回环并顺手抓到一个真缺陷**。`Engine::join_group` 此前没有任何自动化证据
  （`dynamic_join` 测的是加入**会话**）。新增 `tests/engine/group_join.rs`：A、B 先成组开流，
  **先让 C 加入组、再开流**（刻意覆盖「暂存 → 开流后补应用」）。第一版测试**通过**了但打印露馅：
  4 s 窗口里 A、B 各播 4 s 音频，C 只有 **0.32 s** —— 根因是 `join_group` 补发的 GROUP_EPOCH 复用了 `epoch_id`
  却把 `epoch_local_us` 取成**当前时刻**，而它是「样本序号 0 在发送端时钟上的时刻」，于是 C 的目标时刻被推到
  未来好几秒、大半时间在等待。修法：整份基准复用组里存的那份。实测 C 的非静音样本 30 720 → **374 400**
  （A、B 各 384 000），连跑 5 轮全绿，全套集成 28 + 单测 150 全绿。教训：断言写 `> 0` 太松 ——
  「几乎不出声」与「完全不出声」在它眼里一样，计数类判据要写量级。
- **第 62 轮：补「并行 ≥ 8 台接收端」**（`docs/33` §5 的未做表里写着「多会话：并行推 ≥ 8 台 —— M3 交付物 1 未开始」）。
  `multi_session.rs` 新增 `one_capture_feeds_eight_receivers`：1 发 + **8 收**、8 台同组、跑 5 s。判据三条都是直接证据：
  8 台的非静音样本数（不是「链路码率非零」）、8 台各自的 `PlayoutScheduled`、8 台播放量彼此相差不超过一成。
  **实测连跑 4 次逐台样本数一模一样**（各 470 400 = 9.8 s 音频）、8/8 都有排播事件；引擎集成 29 + 单测 150 全绿。
  这条把 M3 交付物 1 的**回环侧**钉住了，真机侧（8 台同时出声的听感与输出延迟差异）仍挂账。
- **第 63 轮：M4「共同时间基准」补证据 + 补观测出口**。看板 M4 行与 `docs/38` §2/§4 都写着
  「接收端广播 epoch」「发送端预约发送」未实现 —— 复核后发现**两件其实都实现了**（`Engine::broadcast_epoch` 走 `0x44`，
  发送端在 `ReceiverEpoch` 分支 `hub.apply_epoch`，采集线程每帧对齐编号），缺的是**接线到测试**与**可观测出口**。
  本轮加了 `Engine::capture_epoch_us()`（采集枢纽 `epoch_us` 原子量的出口 —— 此前它从建到现在没有任何读取者），
  并新增 `tests/engine/mixer_epoch.rs`：1 台 PC + 2 台手机、真 QUIC + 真 PIN。**广播前两端都是 `i64::MIN`（对照组），
  广播后两端拿到完全相同的值**（连跑 4 次每轮 A == B）；`broadcast_epoch` 返回 2。集成 30 + 单测 150 全绿。
  仍未做（诚实口径）：真机混音听感与输出延迟差异；以及采样级对齐的**精度**要在真机上量 ——
  本轮证据是「两端同一个原点」，不是「混音已对齐到 X ms」。
- **第 64 轮：多组共存边界**。组管理覆盖过「建组 / 组内同步 / 动态加入」，但**两个组同时存在**没测过 ——
  而它最容易写坏（组基准若按「发送端」而非按「组」存取，第二次 `create_group` 会冲掉第一个组）。
  新增 `tests/engine/group_multi.rs`：1 发 + 4 收、前两台一组后两台一组。**第一版判据自我更正**：
  原本断言「两组目标时刻必须不同」，实测只差 **27 µs**（两次建组相隔几十微秒）—— 立不住；
  「各有各的基准」的直接证据是 **`epoch_id`**。改判据后：组内两台 epoch_id 相同、组间不同、四台都出声
  （各 374 400 = 3.88 s），连跑 3 次全绿；集成 31 + 单测 150 全绿。
- **第 63–64 轮的看板同步受阻**：GitHub GraphQL 触发二级限流（`/rate_limit` 显示额度充足，但连续查询被拒），
  第 63 轮的同步动作本身成功了（脚本报「说明更新 1」），但**远端回读核对没做成**；第 64 轮的看板改动已写好、
  待限流恢复后执行 sync 并补核对。**下轮第一件事：核对 M4 行正文是否含本轮结论，并补第 64 轮的行。**
- **第 65 轮：`schedule_playout` 的 API 护栏 + 一处待查观察**。第 59 轮动过这条显式 API 的实现
  （「播放句柄未就绪」从报 `cap_unsupported` 改成暂存、开流后补应用），但**没有测试**；本轮在
  `group_epoch.rs` 补上 `schedule_playout_reports_the_timeline`（真双 Engine + 真 QUIC：配对 → 等 streaming
  → 开流 → 等首帧建立序号基准 → 设定基准 → 断言 `PlayoutScheduled` 的 epoch_id 原样报出；基准取未来 10 s，
  以免落在过去走 Drop 分支而不报时间线）。连跑 3 次全绿；集成 32 + 单测 150 全绿。
  **待查（实测行为，没写成契约）**：在「配对完成、但还没开流」的时刻调用这条 API 会拿到
  `BadRequest { context: "session task is gone" }`（`send_command` 只在 command 通道关闭时报这句），
  也就是那一刻**接受侧的会话任务已经不在了**。是「无流会话被提前回收」的缺陷，还是「会话任务只在流期间
  存在」的既定设计，需要单独查；查清之前这条测试只覆盖开流之后的语义。
- **看板同步仍受阻（第 63–65 轮）**：GitHub GraphQL 二级限流，连续三轮的远端写入/核对都没做成。
  第 64 轮（多组共存）与第 65 轮（schedule_playout 护栏）的行都已写进 `tools/sync-board.ps1` 且 `PARSE-OK`，
  **待限流恢复后执行一次 `pwsh tools/sync-board.ps1` 并回读核对**。下轮第一件事仍是这个。
- **第 66 轮：面板欠账还清 + 第 65 轮那个「待查」查清了（结论：非缺陷）**。
  ① 面板：限流窗口一恢复就补了 `sync-board.ps1`，把第 64（多组共存）、第 65（schedule_playout 护栏）两行
  写进远端，并删掉第 63 轮遗留的重复行；远端核对 **Done 100 / In Progress 7 / Todo 4**，M4 行正文确认含本轮结论。
  ② 待查项：现象是「配对完成、还没开流」时调用 `schedule_playout` 报 `BadRequest { session task is gone }`。
  用探针（连续 2.1 s、每 300 ms 采样两侧 `peers()`）看到会话是**长驻**的（两侧恒为 1），那一刻调用**返回 Ok**。
  真因是**测试时序**：接受侧的会话登记比 `submit_pin` 返回晚一拍，而这条 API 走的是本端会话表。
  既然路径确实可用，测试就改为覆盖它 —— 配对 → 等 streaming → 等接收侧登记 → **开流前**设定基准（断言 Ok）
  → 开流 → 断言 `PlayoutScheduled` 的 epoch_id 原样报出。连跑 3 次全绿、集成 32 + 单测 150 全绿；
  看板行标题已更正（去掉「+ 一处待查观察」），旧标题条目已删除。
- **第 67 轮：补「组内同步 × 链路丢包」这条交叉边界**。M3 的 ±10 ms 在干净回环上钉过、M2 的丢包不出洞
  也钉过，但两者**相交**那一格没人测：丢包时抖动缓冲 / PLC / NACK 重传都会动播放时刻，而组内同步要求的
  正是「两台在同一时刻播同一帧」。新增 `tests/engine/group_sync_under_loss.rs`：发送端 → **两个独立有损中继**
  → 两台接收端，各自注入（只丢客户端→服务端方向、≥512 B 的音频包，每 50 个成串丢 2 个 ≈ 4%）。
  **实测连跑 3 次**：注入丢弃 24 个包（前提断言：真的丢了）、两端排播事件各 1、
  **绝对偏差 P50 0.00 ms / P95 0.03~0.04 ms** —— 离 ±10 ms 验收线还有两个数量级。
  结论：4% 成串丢包不影响组内同步（被双发 + NACK 修复，播放时刻由 epoch 钉住）。
  未做：更狠的丢包档（30% 会触发降码率与 PLC，是另一条边界）、丢包与动态加入/多组共存的组合、真机弱网形状。
- **第 68 轮：把上一轮记为「未做」的重丢包档补上（结论：30% 也成立）**。注入改成「每 10 个成串丢 3 个」
  （约 30%），这一档会真正触发自适应降码率与 PLC。**实测连跑 3 次**：丢弃 264 个包、两端排播各 1、
  **绝对偏差 P50 0.00 ms / P95 0.04 ms**（与 4% 档同一量级，离 ±10 ms 两个数量级）；
  接收侧遥测 `(nack_count, plc_count, underruns) = (48, 15, 0)` —— 重传 48 次、PLC 15 次、**欠载 0**。
  这组数字把两件事分开了：双发 + NACK 补**内容**、PLC 兜没补上的内容，而**时刻**始终由 epoch 排播钉住、
  抖动缓冲吸收到达抖动，所以「丢包严重」影响的是音质，不是组内同步。看板行标题随之更新（4% → 4% 与 30%）。
- **第 69 轮：澄清一份「看起来很吓人」的历史报告，并用 90 s 对照坐实修复**。复查时看到
  `target/evidence/soak/soak-8h-netem.json` 里 `verdict: "failed"` + 200 条违规（全是 `bitrate_out_of_range`）、
  `dropped_violations: 6855`，差点当成当前代码的长跑失败。查代码后确认：`SoakThresholds::weak_network`
  把 `bitrate_tolerance_pct_x100` 设为 0（= 不判）、判定逻辑有 `> 0` 守卫、还有专门单测；
  而那份报告**正是 `soak.rs` 注释里描述的那一次修复前长跑**（`violations_total: 7055` 与 docs/22 §9 完全对上）。
  **同参数对照**：用当前二进制跑 90 s 宽容档 → `verdict=ok violations=0 dropped=0 samples=88`（vs 修复前 7055）。
  docs/22 新增 §10 写清「怎么一眼分清历史报告与当前判定」（违规类型 / 摘要字段 / 档位自述三条）。
  **真正的 8 h rerun（用当前代码）仍在跑，约 30 min 后出报告 —— 那才是 M2 稳定性行的回填依据。**
- **第 70 轮：丢包 × 动态加入的组合边界**（`docs/33` §10.2 记的未做项之一）。新增
  `group_sync_under_loss.rs::late_joiner_aligns_under_loss`：1 发 + 3 收，A、B 先成组开流并在丢包链路上跑 3 s，
  然后 C **在丢包进行中** `join_group` + 开流（三条链路各自注入约 4% 成串丢包）。**连跑 3 次全绿**：
  `注入丢弃 42 个包；三台排播 [1,1,1] · 非静音写出 [395,395,245]；A/B 绝对偏差 P50 0.00 ms · P95 0.04 ms`。
  结论：丢包不影响动态加入 —— 组基准走控制流（可靠通道），数据报丢包只影响音频内容，而后者已被
  双发 + NACK + PLC 兜住。集成 35 + 单测 150 全绿。剩余组合边界：丢包 × 多组共存。
  （本轮还在追加测试时踩了两次「锚点吃掉收尾括号 / 字段被上一版精简掉」的坑，靠 git checkout 回退 +
  重读锚点解决；教训：**追加测试前先读文件末尾**，别凭记忆写锚点。）
- **第 71 轮：组合边界清单收尾 —— 丢包 × 多组共存**。`docs/33` §10.2 清单的最后一格。新增
  `group_sync_under_loss.rs::two_groups_hold_under_loss`（1 发 + 4 收，A/B 一组、C/D 一组，
  **四条链路各自**注入约 4% 成串丢包）：连跑 3 次全绿 ——
  `注入丢弃 48 个包；四台排播 [1,1,1,1] · 非静音写出 [295,295,295,295]；组 1 P50 0.01/P95 0.05 ms · 组 2 P50 0.01/P95 0.10 ms`。
  两组各自的 P95 都离 10 ms 验收线两个数量级。**结论**：组基准「按组存」在丢包下依然成立 ——
  播放时刻由各自的 epoch 排播钉住、走控制流（可靠通道），与数据报丢包是两条路；丢包动的只有音质。
  顺手修掉一个 clippy 报错（`needless_range_loop`：`for index in 0..4` + `stamps[index]` →
  `stamps.iter().enumerate()`）。门禁：fmt ✓ / clippy `-D warnings` ✓ / engine 集成 **36 项**（新增 1）✓
  / lib **150 项** ✓。**又踩一次同一个坑**：`cargo fmt` 之后立刻 `edit` 撞
  `file changed since it was read` —— 跑过格式化就必须重读再改。详见 `docs/33` §10.5。
- **第 72 轮：拔网 4.5 s 的锅从「链路层」挪到「数据面」（先加读数、再谈根因）**。目标本是 M2 那条
  「拔网 10 s 后 ≤ 3 s 恢复」（现状 4.2~4.8 s）。上一轮的结论写在 `docs/48` §2.3：**PTO 指数退避，
  链路层不可能 3 s 内回来**。这一轮没直接改代码，而是先给闸门中继加读数，结果**把那个结论推翻了**：
  ```text
  [outage-diag] 到达中继的上行音频包 拔网前 9 → 1 s 后 28 → 拔网结束 36
                · 插回后回程首个包 196~326 ms · 上行音频数据报 4086~4273 ms · 声音恢复 4546~4820 ms
  [outage-diag] 插回后每 500 ms 的上行包数：[0, 0, 0, 0, 0, 0, 0, 0, 502, 27, 30, 25]
  ```
  三条读数各回答一个问题：① **回程首个包 196~326 ms** —— 回程包只有在对端收到上行包并回应后才可能出现，
  所以**链路 0.3 s 就通了**，「链路层回不来」当场被否；② **上行速率前 4 s 恒为 0**（连保活都没有），
  然后一格内涌出 **502 个**，再回落到 25~30 个/0.5 s（≈50~60 pps = 音频帧率）—— 502 ≈ 10 s × 50 pps，
  就是拔网期间积压的全部音频被一次放出；③ 声音恢复 4.5 s ≈ 上行恢复 4.1 s + 抖动缓冲填满 0.4 s。
  **真正缺口**不是路径不可用，而是应用与 QUIC 之间缺一条主动恢复路径：`report_peer_gone` 把会话直接标
  `Failed`，运行时没有重拨/重建（正是看板上 FR-27 重连那条）。**方法论**：§2.3 的推理不荒谬
  （延迟确实随断网时长增长，看着就像退避），但它只解释「为什么慢」，没验证「**谁**在慢」——
  一个「插回后 4 s 内上行 0 包」的读数就把链路层与数据面干净分开了。修复方向与判据已写进 `docs/48` §2.4 / §5，
  候选方案①（应用层 200 ms 探活）先做对照实验，不行再上②（FR-27 重连）。本轮门禁未跑（只改测试诊断代码 +
  文档，未动产品代码）；新读数留在 `network_outage.rs` 里，是有价值的常驻护栏。
  **过程中的一次自伤（值得记）**：新加的诊断读数最初是**串行等待**，插在「恢复」测量之前 ——
  于是把「恢复后 2 s 内的非静音回调数」那个窗口**等过期**了，`two_second_outage_recovery_is_recorded`
  与 `brief_outage_self_heals_within_budget` 两条测试**确定性变红**（隔离跑也红，所以不是并行 flake，
  而是真回归）。这与 `docs/48` §4 记过的是**同一个坑**（「窗口从 restore_at 起算，而恢复本身可能晚于它」），
  只是这次换成了「等待把窗口吃掉」。修法：诊断读数收 `Arc<AtomicU64>` 并用 `tokio::spawn` **并发**等，
  **判据先取、读数后收**。教训：**诊断代码也有副作用** —— 加读数时先问一句「它会不会改变被测对象的
  时间线」。修好后 outage 3/3 连跑两次、engine 36/36、lib 150/150、clippy 零告警（提交 fcb48da）。
  **看板同步仍被 GraphQL 二级限流挡住**：本轮 `pwsh tools/sync-board.ps1` 三次尝试全部失败 ——
  `gh project list --owner GotKiCry` 报 **`unknown owner type`**，而 `gh api graphql` 直连给出的真因是
  `API rate limit already exceeded for user ID 24805948`（错误类型 `RATE_LIMIT`/`graphql_rate_limit`）。
  **两条读数会互相矛盾**：REST 侧 `gh api rate_limit` 仍报 `graphql.remaining = 5000`，
  所以只看那一行会误判成「额度充足、是脚本或权限问题」；以 GraphQL 实况为准。
  Token scope 本身没问题（`gh auth status` 含 `project`）。脚本里的任务表（含第 70 与第 71 轮文本）
  已更新且幂等，窗口一恢复重跑即可补齐 —— 本轮不把限流当成同步成功，也不从脚本输出反推看板状态。
- **第 73–74 轮：用一次「排他实验」把拔网修复方案的候选砍到一条**。上一轮把 10 s 拔网 4.5 s 的根因
  从链路层挪到了数据面（`docs/48` §2.4：链路 0.3 s 就通、上行静默 4 s 后一次放出 502 个包）。本轮不再猜，
  直接做对照实验：给 `run_outage` 加一个可选探活钩子 —— 拔网期间由**应用层**每 200 ms 调
  `Engine::broadcast_epoch(200)` 投喂一个控制帧，**只动测试 helper、没碰产品代码**。背靠背结果：
  ```text
  基线（无投喂）：上行速率 [0,0,0,0,0,0,0,0,544,28,27] · 声音恢复 5081 ms
  有投喂（200 ms）：上行速率 [0,0,0,0,0,0,0,0,0,552,27] · 声音恢复 5461 ms（投喂 49 次 · 被接受 49 次）
  ```
  **49 次投喂全部「被接受」，静默期上行包数仍是 0** —— 应用把数据交给了 QUIC 发送队列，而传输层
  一个包都没往外发；静默 9 格与基线 8 格同量级，恢复还略晚。**结论：候选方案①（应用层探活）无效**，
  缺口是「传输层在长退避里挂死，连新投喂都不触发发送尝试」，只剩方案②：接上 FR-27 重连
  （`report_peer_gone` 之后主动重建连接，而不是标 `Failed` 就结束；`session.rs` 的
  `Reconnecting`/`ReconnectOk` 迁移表早已写好，缺的是运行时实现）。**这条实验的价值是低成本排掉一条路**：
  没改一行产品代码，却把「要不要在探活上继续投入」一次答清。**先做能排他的实验，再动架构。**
  门禁：fmt ✓ / clippy `-D warnings` 零告警 ✓ / engine 集成 **37 项**（新增对照实验 1 项）✓ / lib **150 项** ✓。
  过程中修了两处自查问题：参数位置上误用 `///`（Rust 不允许 doc comment 出现在参数位）→ 改普通注释；
  `run_outage` 参数到 9 个触发 `too_many_arguments` → 显式放行并写明理由（测试 helper 摊开参数比再包一层更好读）。

- **第 75 轮：用 AgentTeam 把 FR-27 重连做到 444 ms，又按纪律回退（并拿到更准的根因）**。新目标要求用 AgentTeam，
  于是立 3 条共享任务、派两位队友（`reconnect-test` 写验收测试、`reconnect-design` 写 `docs/50-m2-reconnect.md`），
  Lead 自己做实现。**实现三件**：① `Inner` 加 `reconnect_tx` + 重连监督任务（持 `Weak<Engine>` 避免自我引用环）；
  ② `reconnect_once` 摘旧会话 → 退避重拨（单次 1.5 s 超时、退避 200→1000 ms、预算 20 s）→ 成功后 `start_send`；
  重拨走完整的 `connect_inner`，它内部的 `is_trusted` 决定是否要 PIN —— **重连不绕过信任库**；
  ③ **静默看门狗（关键）**：拔网时 QUIC 不报错（`idle_timeout` 30 s 才判死），`report_peer_gone` 压根没被调用，
  所以要自己数「多久没听到对端」（`PeerSession.last_rx_us` + `ticker`，阈值 1 s）。**实测 4783 → 444 ms**，
  上行静默消失，验收测试的 3000 ms 断言转绿。**但引入回归**：接收侧 `peers()` 变成空表（`[Streaming] / []`），
  而声音仍在响（响的是旧连接自愈）。**两位队友独立给出更准的根因，比我自己的猜测对**：我当初用
  `stream_id.is_some()` 判「谁该拨号」，但**接收侧也会置 `stream_id`** → 变成**双向拨号**，接收侧去连一个
  **没在监听**的发送端（`Engine::start` 不自动监听），拨号失败后把**自己的表项**摘掉了。
  正确判据是 `Role::Initiator`。队友还抓到两个附带问题：裸 `tokio::spawn` 绕过了 `TaskTracker`、
  重连后 `start_send` 的错误被 `let _ =` 吞掉；以及**参数要按 3 s 反推**——1.5 s 单次尝试 + 1.0 s 退避
  最坏 ≈3.4 s 已经越线，应压到 ≤800 ms / ≤500 ms。**按质量纪律回退了产品代码**（主干不留已知回归），
  验收测试保留但标 `#[ignore]` 并写明理由（`--run-ignored` 可复现 444 ms）。**本轮价值**：把模糊的「接重连」
  拆成三件具体的事（静默看门狗 / 退避重拨 / 谁该拨号），前两件已被实测证明有效，第三件有了确切判据。
  门禁：fmt ✓ / clippy 零告警 ✓ / engine **37 passed 1 skipped** ✓。产出：队友的 `docs/50`（340 行、含行号锚点）、
  提交 `009bad4`。

### 4.1 定量验收待补：**PCM 长度修复后的真机链路**

Issue #1 已定位并修复：`OpusDecoder::decode_into()` 返回**交错样本数**，`receive_audio()` 又乘了声道数，
导致每包音频追加等长静音。接收端现按实际返回长度截取，解码缓冲收缩为单包容量。
`tests/pcm_delivery.rs` 通过两个真实 Engine、QUIC、PIN 配对和 Opus 验证 sink 实际收到的内容：
20 ms → 1920 个样本，10 ms → 960 个样本；两项测试在旧代码上均失败，修复后通过。
用户已确认音频正常传输和播放，PCM 修复看板已标 **Done**，Issue #1 已关闭；环溢出增量、队列斜率和延迟仍留在独立验收项。详见 `docs/12` §9。

以下为**修复前**的预算（`docs/12-m1-device-acceptance.md`）：

| 段 | 值 | 性质 |
|---|---|---|
| 采集半周期 + 组帧 | 10 + 20 ms | 模型/结构 |
| 网络单向（推流中） | 5.86 ms | 模型（§6 逐探针 RTT P50 ÷ 2） |
| 对端播放环水位 | 80 ms（且被上游推着涨） | 实测 |
| 对端 AudioTrack 输出 | 80–101 ms（请求 960 帧 → 实际 3844 帧） | 实测（设备侧） |

**合计下限 ≥115 ms，加设备输出 ≈195 ms —— 超 P50 ≤ 110 ms 目标。**

两条根因（都在 §12 §4 的缺陷表里）：
1. **内核→Kotlin PCM 推送 ≈ 2× 实时**（区间增量实测：推流 78 s 环溢出 +3 791 040 帧 = 48 603 帧/s，读空 +0）
   → 环始终满、消费侧永不缺数据；**设备侧省的 50–66 ms 只是搬进环里排队，总量不降**。
   代码根因与修复位于 `audiolink-engine/src/runtime.rs`，FFI 原样转交长度的行为无需更改。
2. 接收队列持续增长仍需独立复测；此前封口总数与观测窗口可能含起步阶段，不能直接当成稳态速率差。
   历史报告也已注明 50.5 fps 与 +1 ms/s 的量级不一致（`docs/12` §4 #11），暂不据此实施漂移补偿。

复测顺序：新 APK 默认档先建立基线 → 同连接切换 30 ms 档 A/B → 连续播放与端到端延迟验收。
此前 30 ms 档曾节省设备侧 50 ms，但在正确 PCM 供给下的欠载代价须重新测量，不能沿用旧结论。

### 4.2 已完成：30 min soak（结论已回填）

`pwsh target/evidence/acceptance/soak-30min.ps1 -Seconds 1800 -Dir target/device-link-3 -Tag soak30` → exit 0。
**结果：1800 s 全程 `streaming`、无断连（退出条件达成）；但水位顶到 320 ms 队列上限、迟到丢弃 0→53、供给欠载 7→69、e2e 下限 ≥235 ms。**
⇒ 与 §4.1 同源：链路稳、接收管线不稳。明细见 `docs/12` §5.1。

### 4.3 已知待办（按优先级）

| 项 | 说明 | 优先级 |
|---|---|---|
| PCM 长度修复后的真机复测 | 代码修复与 10/20 ms 回归已完成；待测溢出增量、队列斜率、听感和延迟 | **P0**（M1 延迟达标依赖） |
| ~~30 min soak~~ | ✅ 已完成（§4.2）：无断连，但水位/迟到/欠载暴露接收管线问题 | ✅ |
| ~~Android PIN 真机复测~~ | ✅ PHK110 的显示/重试/断开/到期/锁定均通过（`docs/16`） | ✅ |
| ~~同端口立即重启~~ | ✅ 已修复，真实网络/音频与取消回归通过（`docs/15`） | ✅ |
| 设备侧 `queuedFrames` 进遥测 | 现在账本看不到「对端 AudioTrack」那一段，设备侧省下的 50 ms 在报告里不可见 | P1 |
| `EngineEvent::Telemetry` 加 `peer: NodeId` | 对端遥测与本机采样共用同一事件，**多对端会串**（M3 必须修） | P1（M3） |
| PCM 回调装箱开销 | UniFFI 0.29 无 `FloatArray` 映射 → `List<Float>` 每帧约 30 KB 装箱；逃生通道写在 `audiolink-ffi/src/audio_bridge.rs` 顶部。真机 GC 抖动仍未专门测 | P1（M2） |
| ~~抖动缓冲第一阶段~~ | ✅ M2 已完成 20 / 40 / 60 ms 自适应深度、30 s 降档迟滞、一帧有界重排和有界重缓冲；完整仿真缓冲/IIR/漂移微调及弱网验收继续留在 M2（`docs/18`） | ✅ / M2 继续 |
| 跨机时钟的工程边界 | `now_monotonic_us()` 在 Android 是**进程相对**基准（`CLOCK_MONOTONIC`，深睡停走）⇒ offset **只在同一次连接内有效**；M3 若要挂起后续播，需改用 `CLOCK_BOOTTIME` 或显式检测阶跃后重收敛 | M3 |
| ~~文档本机路径脱敏~~ | ✅ **已完成**：`docs/` 下全部本机绝对路径换成占位符（`%LOCALAPPDATA%\Android\Sdk`、`<JDK 17 根目录>`、`<你的 keystore 路径>`、`<本机用户目录>`），占位符约定见 `docs/06` §1 | ✅ |

**纪律（持续执行）**

- 用户要求：每次完成任务都同步 GitHub Project，回填状态、提交链接、验证结果及剩余真机验收；结束前读取远端确认，不能只改本地任务表。
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
| Android 构建 | **`pwsh tools/gradlew.ps1 -JavaHome '<JDK 17 根目录>' assembleDebug`**。⚠️ `-JavaHome` 是**第一个位置参数**：`pwsh tools/gradlew.ps1 assembleDebug` 会把 `assembleDebug` 当成 JDK 路径。本机 `JAVA_HOME` 指向 **JDK 11**，而 Gradle 9 要 17+ |
| Android 单测 | 同上加 `testDebugUnitTest`（31 个 JVM 用例）；另有 `lintDebug` |
| 交叉编译内核（Android） | `pwsh android/scripts/build-rust.ps1`（**默认 release**，双 ABI，产出 3.7 / 2.5 MB）。要带 debuginfo 的内核才加 `-Debug`（94 / 75 MB，**别拿去发布**）。快速验证用 `cargo check -p audiolink-ffi --target aarch64-linux-android`（裸命令会挂在 ring 的 C 编译上，见 §6 坑 15） |
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
13. **`opus-rs` 48k 只接受 240/480/960 帧样本**（2.5 ms 会 panic）；底层 `decode` 返回**每声道**样本数，
    **本项目 `OpusDecoder::decode_into()` 已换算成交错样本数**，调用方不可再乘声道数（Issue #1）；
14. **CELT-only 没有真 PLC**（第 2 个丢失帧起硬静音）→ 已落地自建 PCM 掩盖，别指望 `packet_loss_perc`；`opus-rs` 无模式 getter 时也不能凭 application 猜原生 PLC 可用。

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

### 本轮（真机验收）新踩的坑

21. **生产端点是强制 mTLS**：`audiolink-net::tls::server_config()` 索取客户端证书（§5 要求接收侧也能拿到对端证书 DER 验签）。
    ⇒ 任何「裸 QUIC 客户端」都连不上：M0 的 `latency-probe` 客户端原本不带证书，被服务端回
    `error 116: peer sent no certificates`（**看起来像网络不通，其实是缺客户端证书**）。已给它补自签客户端证书。
22. **握手死线必须给配对让路**：死线 10 s 是绝对的、而 §5 的 PIN 有效期 60 s ⇒「人在手机上看 PIN 再敲进 PC」必然超时，
    表现为 `1002 NOT_PAIRED（unknown peer）`（会话已被回收，peers 表里没有该对端）。
    现在进入配对等待时把死线顺延到 75 s（PIN_TTL 60 + 15 余量），权威判据仍在 `PinGate`。
23. **内核每连接新建 `PinGate`**：PIN 不是「这台设备的 PIN」，而是**每连接一个**，连接一断即作废。
    ⇒ 自动化流程必须「连上后轮询读屏 → 同一条连接内提交」（`device-link --pin-file` 就是为此）。
24. **低延迟模式生效 ≠ 缓冲小**：真机 `getPerformanceMode()=LOW_LATENCY`，但请求 960 帧被框架顶到 **3844 帧（80 ms）**，
    flinger 对该 track 报 Latency 101 ms。合法解释：缓冲**容量**与队列**水位**是两件事；
    容量可 `setBufferSizeInFrames` 收缩（实测 3844→960r **不掉 FAST**，Latency 101→41 ms），
    水位可用「按 getPlaybackHeadPosition 估已消费帧、只在 queued < 目标时写」压（30 ms 档 = −50 ms 且零欠载代价）。
25. **低频探测会被无线唤醒代价带偏**：同一条链路，1 Hz 探针 RTT P50 **115 ms**，10 Hz 或推流中（50 包/s）只要 **10.6–11.7 ms**。
    ⇒ 用 1 Hz 的 RTT 估网络段会高估一个数量级；M2 的抖动缓冲深度必须按推流中口径取值。
26. **Windows 的 `Instant` 与 Android 的 `Instant` 都从各自进程/系统起点计时**：`now_monotonic_us()` 在 Android 是
    `LazyLock<Instant>` 的**进程首次调用**起点。⇒ 跨连接比 offset 没有意义（会随墙钟 1:1 漂），
    **offset 只在同一次连接内有效**；不要把它缓存下来跨会话用。
27. **设备侧读数必须当场落盘**：终端读到的「播放环水位 1920/2880、溢出 4 332 000」没有存文件，
    独立复核时无法证实，只能判「不成立」并被撤下。工具（`uiautomator dump` + `adb pull`）要顺手存证。

---

## 7. 待办与遗留

`12cf674` 的 CI 暴露测量探针并发漏配，已统一状态锁并强化既有回归（2000/2000，见 `docs/16`）。
手机待播队列增长也已修复：PHK110 180 s 从 P50/P95/max 240/300/320 ms 降至 40/40/60 ms，首尾均 40 ms；
自建 PCM 丢包掩盖已补齐 CELT 连续丢包硬静音，迟到丢弃增加到 143 次的问题继续转入 M2 自适应抖动缓冲，不能据此宣称弱网听感完成。

| 项 | 说明 | 优先级 |
|---|---|---|
| **PCM 长度修复后的真机复测** | 修复已落地、真实收发回归通过；用户已确认播放正常；定量复测待补（见 §4.1 / `docs/12` §9） | **P0** |
| ~~30 min soak~~ | ✅ 已完成（§4.2 / `docs/12` §5.1） | ✅ |
| ~~Android PIN 真机复测~~ | ✅ PHK110 已通过，含配对中服务停止/重启（`docs/16`） | ✅ |
| ~~同端口立即重启~~ | ✅ 已修复，原端口立即重绑、并发与取消语义已验证（`docs/15`） | ✅ |
| ~~接收待播队列持续增长~~ | ✅ 播放序号随时钟推进，短暂停顿不再永久累积延迟；PHK110 180 s 首尾 40 ms（`docs/16`） | ✅ |
| ~~CELT-only 自建丢包掩盖~~ | ✅ 重复上一帧 + 120 ms 淡出 + 2.5 ms 恢复交叉淡化，接收序号与遥测已接线（`docs/17`） | ✅ |
| 设备侧 queuedFrames 进遥测 | 否则账本看不见「对端 AudioTrack」那一段 | P1 |
| 跨端一致性夹具 | §12 golden vectors 在 Rust 与 FFI 双跑 —— **已落地**（`audiolink-ffi::protocolSelfTest()`） | ✅ 完成 |
| ~~`alp2-dump` 支持 pcap/pcapng~~ | ✅ **已完成**：按 magic 自动分派 + `--port` / `--all-ports`，认 Ethernet(含 VLAN)/IPv4/IPv6/Linux SLL；9 项合成夹具回归 + 端到端实测（`docs/20` §3） | ✅ |
| ~~文档本机路径脱敏~~ | ✅ **已完成**：`docs/` 下全部本机绝对路径换成占位符（`%LOCALAPPDATA%\Android\Sdk`、`<JDK 17 根目录>`、`<你的 keystore 路径>`、`<本机用户目录>`），占位符约定见 `docs/06` §1 | ✅ |
| `release.yml` 未跑过 | 首次 tag 触发时才验证；`createUpdaterArtifacts:false`（M5 打开） | 中（M5） |
| CI 时间 | ✅ 2026-09-16 实测处理完毕：两条旧前提均被证伪（依赖由 rust-cache 全覆盖、cargo-ndk 安装仅 1 s），真正回收的是 android 的 Gradle 缓存（job 192 → 85 s） | 低 |
| QUIC 弱网实测 | ADR-004 的回退路径（裸 UDP）是否触发，取决于 M2 实测 | 中（M2） |
- **CI 的 core job 拆成 core-light / core-heavy（本轮的进度项）**。core 长期是 CI 墙钟关键路径（236–256 s），而依赖树早已被 rust-cache 全覆盖 —— 剩下的只有本地 crate 编译与链接，于是按依赖深度拆开：core-light 只做 types/proto/identity（协议 golden vectors 的快反馈通道），core-heavy 用 `--workspace --exclude audiolink-desktop --exclude` 三个轻内核（用 --exclude 而非逐个 -p：新增 crate 会自动落进重 job，漏测比慢贵得多；本地用 cargo metadata 校验集合 9 = 3 + 6、交集为空）。**稳态实测（run 35098695833）：core-light 80 s，core-heavy 297 s，五个 job 全绿** —— 协议层拿到结论从 236–256 s 压到 80 s（−69%），代价是墙钟 +20%（重 job 多付一份 rust-cache 恢复与 checkout）。**首次运行的冷缓存成本必须一起记**：run 35097376811 墙钟 744 s（两个新 job 各自重建 quinn/rustls/tokio/cpal 依赖树，clippy 295 + test 354）—— 拆 CI job 的真实成本不是脚本行数，而是缓存被切碎后的第一次运行。剩余杠杆：core-heavy 的 test 229 s 里 27 个测试二进制的链接是主体（engine 一个 crate 就 13 个），已立新待办。详见 `docs/26-ci-key-path.md` §7-8。
- **CI 测试步骤改用 nextest：先修正一个错误前提，再落地优化**。上一轮我在看板上写了「core-heavy 的 229 s 里 27 个测试二进制链接是主体」—— 这句是错的。本轮从 CI 日志逐 suite 取 `finished in`：26 个 suite 运行时间之和只有 **75.8 s**（最长单个 16.25 s），所以 229 s 里约 **153 s 是编译**；本地单独量一个空的 engine 测试二进制（编译 + 链接）只要 **0.75 s** —— 链接从来不是杠杆，真正的浪费是 `cargo test` 一次只跑一个二进制（26 个互不依赖的二进制排队）。改用 `cargo nextest`：本地热缓存 heavy 集 **78.2 s → 21.3 s**（连跑三次 18.8/18.8/18.8 s，344 项全过零 flaky，且与 `cargo test --lib --tests` 的 26 suite / 344 passed 逐项一致，无漏测）；CI 上 core-heavy 的 test 步骤 **229 → 199 s** —— 差异有解释：CI 的 199 s 里约 153 s 是编译（4 核且 Windows 链接贵），能并行的只有运行那部分，而本地测的几乎全是运行时间。新增 `.config/nextest.toml`（单个测试 60 s 告警、fail-fast=false）。**首次运行的安装成本也记了账**：`cargo install cargo-nextest --locked` 在两个 job 里各花 404 / 391 s，首次墙钟 666 s —— 与上一轮拆分 job 切碎缓存是同一个教训：新工具的第一笔账是安装与缓存重建。详见 `docs/26-ci-key-path.md` §9。**收尾修正**：bullet 里那句「rust-cache 的 bin 缓存会留住 cargo-nextest」被四次运行证否（安装步骤始终 375~404 s），已改用官方预编译包（get.nexte.st，版本 pin + 装完立刻校验），安装步骤 **2.0 s**；最终稳态 core-light 83 s / core-heavy 267 s（run `35103776301` 全绿），协议层反馈 236–256 s → 83 s。
- **CI：引擎集成测试合并成一个 harness（先量再改）**。这条待办自己就写着「动手前必须先量」，这轮量完也做完了。**先量**：清掉引擎产物重编，15 个可执行文件（1 lib + 14 集成测试）共 **7.4 s**（平均 0.5 s/个）。**两个踩坑**：① **crate root 的 `mod` 解析基准是它自己所在的目录** —— `tests/engine.rs` 里的 `mod x;` 找的是 `tests/x.rs` 而不是 `tests/engine/x.rs`；文件还留在 `tests/` 时这条回退路径**悄悄生效**，结果独立二进制与 harness **同时**构建、同一批测试跑两遍（nextest 报 **182** 而不是 163）。② 正确布局是 **`tests/engine/main.rs` 作 root、14 个文件放在它旁边**（目录里的 `main.rs` 也是合法测试目标，模块基准就是那个目录）。**结果**：测试二进制 15 → **2**；本地重编 7.4 → 6.3 s；**CI 的 core-heavy test 步骤 187–200 s → 166 s**；测试总数 437 → **437（一个没少）**。单跑仍可以：`cargo nextest run -p audiolink-engine --test engine -E "test(group_epoch)"`。**还剩**：其他 crate 同样结构（proto 4 个、tools 4 个且依赖 engine + QUIC、每个链接更贵），同样手法还能再压 —— 但收益随数量下降，值不值要再量。详见 `docs/26-ci-key-path.md` §10。
- **M5 发布链路跑到底：草稿 Release 真的建出来了**。过程里又抓到**自己的一个设计缺陷**：release job 的条件是「tag 推送 || publish=true」，而建 Release 的 tag 取自 `GITHUB_REF_NAME` —— 手动 dispatch 时那是**分支名**（`main`），于是会建出一个叫 `main` 的 Release。修法是手动路径**必须显式给版本号**（新增 `version` 输入）+ job 里先解析 tag，解析不出来就 `::error::` 失败 —— 宁可这一步红，也不建名字错的 Release。**实测（run `35126756341`，dispatch + publish=true + version=0.1.1）**：`desktop installer` success 612 s、`android apk` success 112 s、**`publish release` success 12 s**；建出草稿 `v0.1.1`（**draft=true**），资产 `AudioLink_0.1.0_x64-setup.exe`（3803 KB）+ `app-debug.apk`（19 770.2 KB）；**对外不可见** —— `GET /releases/latest` 返回 **404**；**没有打 tag**（`git ls-remote --tags` 为空，草稿发布不打 tag 是 GitHub 行为）。验证完删掉测试草稿，仓库回到没有 Release 的状态。**如实观察**：资产里的 APK 是 `app-debug.apk`（没配 keystore secrets → 降级分支，符合设计），要出真 release APK 还得配 secrets。剩下一步是**真发布**（草稿 → 正式），人工动作。详见 `docs/42-m5-release-pipeline.md` §8。
- **M5 Android release 链路：本地把「签名 + R8」跑通**。原先的问题是：CI 里没有 keystore secrets，所以 release.yml 里那条 `assembleRelease` **永远是降级走的** —— 等于「配置写了但从没跑过」，与 release.yml 一开始的状态是同一类毛病。于是本地用**测试 keystore** 把它跑通（keytool 生成 → 写 keystore.properties → `gradlew :app:assembleRelease`）：**BUILD SUCCESSFUL in 1m 34s**。核对：体积 **release 7.84 MB vs debug 19.17 MB（降 59%，R8/minify 确实生效）**；**`apksigner verify --print-certs` 通过：V2 Signer，证书 `CN=AudioLink Test, OU=Dev, O=AudioLink…`**；`assetsTHIRD-PARTY-NOTICES.md` 在 release APK 里也在。**为什么 `7z l` 看不到 `META-INF/*.RSA` 是正常的**：现代 Android 用 APK Signature Scheme **v2**，签名在 zip 结构里而非 META-INF —— 以 `apksigner` 输出为准。两个不入库的本地文件已确认被忽略（`.gitignore` 含 `*.jks` 与 `android/keystore.properties`）。**仍未验**：CI 侧的真 release 路径（需配 secrets，人工，已立 `[环境] 配置 CI secrets` 一条）、装到真机上跑；测试 keystore 不能用于发布。详见 `docs/42-m5-release-pipeline.md` §7。
- **M5 启动时自动连接上次设备（FR-31 收尾）**。常驻 + 自启 + 自动重连三件凑齐才是「开机就能用」，这是第三件。① 记住设备：**只有 `connect` 成功才写** `last_peer`（失败也记的话，开机就会去连一个已知连不上的地址）；② 该不该连：`AutoConnectPolicy::target()` —— 开关打开**且**确实有记录才返回目标；③ 启动时：前端挂载后调一次 `try_auto_connect`；④ 失败：**静默**返回 `null` —— 用户什么都没点，不该在开机时弹「连不上」的错误横幅。**一条刻意的边界**：只认「上次那台设备」，**不做**「扫描一遍、连第一个看到的设备」那类事（那等于在用户没要求时连到别人的机器上）；空字符串也按「没有记录」处理（否则存储被写坏会变成「连本机任意端口」）。设置持久化用 `tauri-plugin-store` —— 插件早就注册了却一直没用。**实测**：`cargo test -p audiolink-desktop` **27 项**（新增 4 项）、`pnpm build` ✓、全量内核门禁（fmt/clippy/437 项）全绿。**没验**：真实启动流程的自动重连（需要「曾经连过、现在开机」+ GUI 会话）、与手工连接之间的并发（都调 `connect`，外壳有去重判据但没在真机压过）。详见 `docs/43-m5-desktop-shell.md` §4。
- **M5 桌面外壳：常驻托盘 + 开机自启 + 优雅退出**。做掉 `lib.rs` 里两条 `TODO(M5)` 并补上自启入口：① **关窗 ≠ 退出**（`CloseRequested` 里 `prevent_close` + `hide` —— 后台常驻的音频工具，关掉窗口不代表用户想停掉正在播放的音频）；② **托盘菜单**（显示主窗口 / 退出；左键唤出、右键出菜单）；③ **退出前优雅收尾**（`quit_app` 先 `engine.shutdown()` 再 `exit(0)` —— 代价落在**对端**身上：对方要多等一次 QUIC 空闲超时才判得掉会话；桥接层内部 **3 s 上限**，超时照样退出但留 stderr）；④ **开机自启开关**（插件早已注册却没有入口）。两个刻意的细节：退出路径带 3 秒上限；自启状态从**系统**读（`autolaunch().is_enabled()`），不本地记布尔。顺手清掉重复定义：`tauri.conf.json` 的 `app.trayIcon` 与 Rust 侧新建托盘同 id，已删配置那份，图标改取 `default_window_icon()`。**实测**：cargo check ✓、cargo test -p audiolink-desktop **23 项** ✓、pnpm build ✓、**release 打包成功**（installer 3 793.5 KB + 签名 .sig），`7z l` 直读安装包含 `audiolink-desktop.exe`（15 181 824 B）+ 根级声明。**没验**：托盘图标显示/左键唤出/右键菜单、关窗后进程存活、退出时对端真收到 `BYE`、自启在系统里的实际效果 —— 都要真机 GUI 会话。详见 `docs/43-m5-desktop-shell.md`。
- **M5 合规：Android 侧的声明投放**。把上轮记的缺口补上：① `license-audit.ps1 -Notices` 生成后**同步一份**到 `android/app/src/main/assets/`（只有一个生成源 —— 手工复制迟早会漂移）；② 读取逻辑 `NoticesLoader.fromText(...)` 做成**纯函数**（读 assets 需要 Context，但真正要钉住的是「找不到怎么办」这条只在打包漏资源时才走到的规则，三条单测全在纯 JVM 上跑）；③ `LicensesScreen`（Compose，可滚动），入口是主界面顶栏的「开源许可」；④ 找不到时与桌面端同规则：**说清怎么生成**，不显示空白页。**实测**：`gradlew :app:testDebugUnitTest :app:assembleDebug` → BUILD SUCCESSFUL in 55s，`NoticesLoaderTest` 3 项通过（连同 9 个既有测试类共 **76 项全绿**），`7z l app-debug.apk` 直读 → **`assets\THIRD-PARTY-NOTICES.md`（1 056 775 B，压缩后 179 045 B）**，APK 19.17 MB。**仍未验**：真机上点开「开源许可」的实际渲染（本项验的是「资源进了 APK + 逻辑单测通过」）；Android 许可审计仍未进 CI；两端提示语常量各自维护。详见 `docs/41-compliance-license-audit.md` §9。
- **M5 合规：Android（Gradle/Maven）依赖纳入审计**。反复出现的「Android 依赖未覆盖」补上了：采集依赖坐标用 `gradlew :app:dependencies` 的依赖树（**不引第三方 Gradle 许可插件** —— 那要往构建里再塞一个供应链依赖还要联网），许可从 Gradle 缓存里每个 POM 的 `<licenses>` 节点读（完全离线），规范化 + 判定在 `audiolink-tools::license`（POM 写的是自然语言名 `Apache License, Version 2.0`，要先变成 SPDX）。**实测：Android 114 个组件，allowed 113 / notice 1 / denied 0** —— 那条 notice 是 `com.google.guava:listenablefuture 1.0`（POM 没声明任何许可的空壳包），进 notice 而不是 allowed 正是「未知不等于安全」在起作用。两个细节：**规范化顺序有语义**（`lesser general public license` 必须排在 `general public license` 之前，否则 LGPL 会被误判成 GPL —— 一个可分发一个不可分发，有单测钉住）；**`(c)` 是版本约束不是依赖**。新增 `tools/android-licenses.ps1` + `license-audit --android`。**还差**：Android 应用内没有声明入口（已立 Todo）、Android 许可审计还没进 CI。详见 `docs/41-compliance-license-audit.md` §8。
- **M5 发布工作流：先审计，再相信**。`.github/workflows/release.yml` **从仓库骨架提交起就在，却从未真正跑过**（从没打过 tag）。读一遍发现四个问题：① 产物路径写成 `desktop/src-tauri/target/...`，而 `.cargo/config.toml` 把 target 固定成 `x86_64-pc-windows-msvc`，产物在仓库根 `target/` —— 收集步骤必然空手而归，且 `-ErrorAction SilentlyContinue` 会把这件事吞掉；② 缺"生成第三方声明"（安装包要带它）；③ 缺 `latest.json`；④ 缺密钥时不给退路。全部修正，并把构建逻辑移进 `tools/tauri-build.ps1`（Actions 的 `run` 块里做「条件 + 覆盖配置」要过 PowerShell→pnpm→tauri 三层传参，内联 JSON 的引号会被吞掉）。**首次真实运行（run `35119443752`，workflow_dispatch + publish=false）：desktop installer success 580 s、android apk success 94 s、publish release 正确 skipped**；`gh run download` + `7z l` 直读 CI 产物：安装包 3776.5 KB，内含 `audiolink-desktop.exe`（15 080 448 B）与**根级** `THIRD-PARTY-NOTICES.md`。**一个值得记住的教训**：我中途把 `run: |` 行删掉，GitHub 的反应是「workflow 名字退化成文件路径 + **每次 push 都触发** + `workflow_dispatch` 消失」，而**本地 YAML 解析器照样解析成功** —— 「本地能解析」不等于「GitHub 能接受」，发布工作流必须真的跑一次才算数。仍未验：`tag → 创建 Release`（对外动作）。详见 `docs/42-m5-release-pipeline.md` §6。
- **M5 自动更新：从「配置在、功能不可用」到「签得出来」**。① `pnpm tauri signer generate` 生成密钥对（私钥放 `~/.tauri/audiolink.key` = **仓库外**，公钥写进 `tauri.conf.json`）；② `createUpdaterArtifacts: true`；③ 带 `TAURI_SIGNING_PRIVATE_KEY` 打包 → **实际产出 `AudioLink_0.1.0_x64-setup.exe.sig`**（minisign 格式）；④ 新增 `tools/tauri-latest-json.ps1` 生成 updater 要的 `latest.json`（版本取自 tauri.conf.json 单一来源，URL 与 `updater.endpoints` 同一个地址）。**两个坑**：环境变量必须是 `TAURI_SIGNING_PRIVATE_KEY`（私钥**内容**），用 `_PATH` 会报「找到了公钥但没有私钥」；**`tauri build` 缺私钥时先产出安装包再报错** —— 很容易留下「看起来打好了、其实更新不可用」的包，所以清单脚本把「找不到 `.sig` 就抛错」钉死。**仍未验**：真实更新流程（需旧版本 + 新版本 + Release 托管三件同时在位，且篡改包必须被拒绝）、CI secrets 未配（需人工）、本地私钥是开发用的。详见 `docs/42-m5-release-pipeline.md` §5。
- **M5 发布链路：首次真实打包 + 把大项拆成可追踪的小项**。① `pnpm tauri:build --bundles nsis` 首次冷编译 5m14s → `AudioLink_0.1.0_x64-setup.exe`（3.69 MB）；`7z l` 直读安装包，确认 `audiolink-desktop.exe`（15 089 152 B）与 `THIRD-PARTY-NOTICES.md`（1 031.5 KB）都在。**第一次打包就抓出一个真缺陷**：`bundle.resources` 写成数组时 Tauri 把 `..` 转义成 `_up_`，包内路径成了 `_up_/_up_/docs/compliance/...`，**与运行时 `resource_dir()/THIRD-PARTY-NOTICES.md` 对不上** —— 装了包「关于」面板必然说"未找到声明"；改成 map 形式后包内变根级，一致（重新打包 + 7z 复验）。**光看配置发现不了它，只有真打包才暴露** —— 这就是"真实安装包验证"的意义。② 顺手把 `docs/42-m5-release-pipeline.md` 写成**现状盘点**，并把四个缺口立成看板子任务：updater 公钥仍是占位符（**配置在、功能不可用**）、发布流程（tag → Release）、Android release 流水线、代码签名证书（外部资产）。**仍未验**：安装后实读、安装包未签名、Android 侧无对应流程。详见 `docs/42-m5-release-pipeline.md`。
- **M5 合规第三块：把声明投放到用户看得见的地方**。三处改动：① 打包 `bundle.resources` 收录 `docs/compliance/THIRD-PARTY-NOTICES.md`（安装后落在资源目录）；② 命令 `third_party_notices` 依次找「资源目录」→「仓库生成路径（开发版）」，返回来源 / 字节数 / 全文；③ 「关于 / 第三方声明」面板（**懒加载** —— 点击才拉，它 1 MB）。**找不到时不说「文件不存在」**，而是回「在仓库根执行 `pwsh tools/license-audit.ps1 -Notices`」—— 空面板只会让人以为软件坏了，这条有单测钉住。**打包前置条件**：没跑过 `-Notices` 就 `tauri build` 会因找不到资源失败 —— 有意的：宁可打不出包，也不要打出一个没有声明的包。**还差**：真实安装包验证（只到「资源被收录 + 命令能读到 + 界面能显示」，出包后在干净机器上打开「关于」还没验）、Android 侧没有对应投放。详见 `docs/41-compliance-license-audit.md` §7。
- **M5 合规第二块：第三方组件声明（THIRD-PARTY-NOTICES）**。`pwsh tools/license-audit.ps1 -Notices` 生成 `docs/compliance/THIRD-PARTY-NOTICES.md`（**1 MB**，平时不生成，发布前才需要）：组件清单（按许可原文分组，**625 个第三方组件**）+ 许可全文（从每个包的 `manifest_path` 同目录读 `LICENSE*`/`COPYING*`/`NOTICE*`，按**内容去重** → **324 份**）+ 「⚠️无全文」显式标记。两个刻意的口径：① **全量含构建期依赖** —— 它是分发包实际内容集的超集，多列不构成合规问题、漏列才是；② **自有 crate 不进第三方声明**（`source` 为 null 的就是本仓库自己的，不是第三方；许可文件在仓库根 `LICENSE` + `NOTICE` 里）—— **第一版把它们列进去还标了「无全文」，是分类错误**，已修（635 → 625 个组件）。单测 4 项（同文本去重、双许可两份都收、无文件包仍列出、声明幂等且排序稳定）。**还差**：**投放位置**（声明躺在仓库里 ≠ 用户看得见，放进安装包 / 关于页是发布流程的一步）、Android Gradle 依赖仍未进清单。详见 `docs/41-compliance-license-audit.md` §6。
- **M5 合规第一块：依赖许可审计（可重复跑 + CI 护栏）**。分两半：采集在 `tools/license-audit.ps1`（调 `cargo metadata` 与 `pnpm licenses` —— 环境知识），判定与渲染在 `audiolink-tools::license`（Rust，13 项单测：OR/AND/WITH、旧式 `MIT/Apache-2.0`、括号、未知许可、报告幂等）。三条刻意的保守规则：**未知许可进 `notice` 不进 `denied`**（未知不等于安全，也不等于能用）、`AND`/`OR` 混用一律进 `notice`（不做 SPDX 优先级推断）、`WITH` 当原子。**实测：Rust 635 包 + 前端 88 包，denied 0 / notice 17**，报告落 `docs/compliance/license-report.md`（自动生成、幂等 —— 无时间戳无路径）。报告暴露的真实待办：**uniffi 全家 8 个包是 MPL-2.0**（文件级 copyleft：可静态链接进闭源产品，但被修改过的 MPL 文件必须公开；我们没改过它源码 → 当前合规，但这条要进发布检查单）、lightningcss 2 个包 MPL-2.0（只进构建）、CDLA-Permissive-2.0 与混用表达式进 notice。**没有 GPL/AGPL/SSPL/非商业许可** —— 「可以发布」的前提结论现在是可重复验证的。CI：desktop job 加一步（那里同时有 cargo 与 pnpm）。未覆盖照实写：Android Gradle 依赖、二进制内部第三方库、字体图标资源；署名文本的投放位置还没做。详见 `docs/41-compliance-license-audit.md`。
- **M4 桌面「广播共同基准」入口：把上轮记的缺口补上**。三层一条线：`Engine::broadcast_epoch(lead_ms)`（上轮已有）→ Tauri `broadcast_epoch(lead_ms) -> u32`（发出的会话数）→ 面板上的按钮 + 提前量输入（**默认 200 ms**）。**提前量默认给宽是有理由的**：发送端收到基准后会**等到那个时刻**才从编号 0 开始发，提前量太小则还没等到就已经过去了（那一段采集整段丢掉），太大只是多等一会 —— 两种错法的代价不对称，所以默认取宽的那边。**这个按钮只能由接收端按**，这不是 UI 约定而是事实：`broadcast_epoch` 的语义就是「我（混音方）指定共同原点」，发送端按它没有意义（它正是要被对齐的那一方）。成功后提示会话数；没有已连接对端时引擎拒绝，原因由错误层翻成人话。详见 `docs/40-m4-alignment-panel.md` §6。
- **M4 桌面「多源对齐」面板：把跨度读数变成人能看的**。链路是一条直线：每包一次 64 位原子写的 (编号, 到达毫秒) → `Engine::stream_axes()` → 纯函数 `alignment_view(axes, now_ms)` → Tauri command `alignment` → 前端 `AlignmentPanel`。判据与阈值：跨度 ≤ 960 样本（1 帧 / 20 ms）= `aligned`；> 960 = `drifting`；有效读数不足两路 = `unknown`（**不给数字** —— 一路无法比较，也不拿一路硬凑）。阈值取一帧是刻意的：混音器按帧对齐，一帧以内听不出来，而实测对齐结果是 144 样本（3 ms），还有 6 倍余量。**1 Hz 轮询而不是事件驱动**：读数每包都在变（50 包/秒 × 路数），事件驱动会变成每包一次 IPC；没有对端时定时器不启动。纯函数单测 5 项；`tsc --noEmit` 严格 + `vite build` 把关。已知缺口：真机场景未验、无历史趋势、没有「一键广播基准」按钮。详见 `docs/40-m4-alignment-panel.md`。
- **M4 观测入口：补上「直接读数」，它当场抓出一个真缺陷**。上一轮报「共同时间基准完成」其实不准确 —— **发送端从未收到 `RECEIVER_EPOCH`**：`0x44` 只加进了 `OpCode` 枚举，漏了 `OpCode::ALL` 与 `from_u8`，dispatch 把它当未知命令码忽略。上一轮那个绿灯测试只验证了 `broadcast_epoch` 返回 2（帧发出去了），没验证对端**处理**了它 —— **没有直接读数就没有验收**。本轮补上 `Engine::stream_axes()`（接收端每包一次 64 位原子写「编号 + 到达毫秒」，读者不会读到撕裂组合；实时路径不加锁不分配）与 `StreamAxis::index_at(now)`，判据 = 「用同一个 now 推算两路编号的跨度」。实测：**对齐前 72048 样本（1.5 s 错位）→ 对齐后 144 样本（3 ms，1 帧 = 960）**。修复过程中被三处既有护栏当场拦住（`OpCode::ALL` 数量断言、net 的 opcode 全扫、engine dispatch 样本表）—— 护栏都生效了，这正是它们存在的意义。详见 `docs/39-m4-common-time-base.md` §6。
- **M4 共同时间基准（接收端广播 epoch）：把上一轮写下的缺口补上了**。§7 的 `GROUP_EPOCH` 是「发送端指定、接收端排播」，只能对齐「同一发送端 → 多台接收端」；M4 的「2 部手机 → 1 台 PC 混音」里两个发送端彼此独立，编号原点各是启流瞬间，按编号混音取到的不是同一时刻的声音。本轮把方向反过来：新增 `RECEIVER_EPOCH`（`0x44`），接收端用 `Engine::broadcast_epoch(lead_ms)` 把所有发送端共用的原点广播出去，发送端按自己的 §6 偏移换算到本端轴（本端时刻 = 对端时刻 − offset_us），再让采集线程的 `HubClock` 把编号 0 点钉上去 —— **到点之前一帧都不发**（那些帧的编号在接收端没有可比性），到点后从 0 起算、`seq` 不动（它只管丢包检测）。基准用 `AtomicI64` 传给采集线程，实时路径不加锁不分配。实测（真实双 Engine + 真实 QUIC，`tests/receiver_epoch.rs`）：**广播 2/2 条会话 · 对齐后两路仍各占一路 · 供帧未被打断（148 → 298 帧 / 3 s）· 掉队帧 0.3%**；HubClock 单测 3 项。**缺口仍然诚实记录**：这测的是「广播到达 + 对齐不打断供帧」，真正判据（同一编号 = 同一时刻，误差 < 1 帧）还读不到（接收端没暴露每路 `sample_index` ↔ 到达时刻），已另立待办。详见 `docs/39-m4-common-time-base.md`。
- **本轮的操作失误（记下来，别重复）**：① 用 `read` + `write` 追加内容时，`read` 有默认 **2000 行**上限 —— 我拿它当全文用了，导致 `runtime.rs` 被 `write` 截断，只能 `git checkout HEAD --` 恢复后重放全部编辑；② `edit` 的锚点若在文件里出现多次（本仓库的会话循环代码重复出现），会插到**第一处**而非文件末尾，结果 `mod tests` 被插进了函数体里。教训：追加内容用「尾部读 + 长锚」或直接用专用工具，别拿部分读取当全文；锚点必须唯一。
