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
