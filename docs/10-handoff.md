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
- 接收待播队列增长、PCM 丢包掩盖、自适应抖动/有界重排和默认冗余双发均已落地。副本延迟一帧并标记 `FEC_REDUNDANT`，接收侧按序号去重且 jitter 只取首个有效副本；60 ms 最高档欠载现在也能按现档重新补水。PC 双 Engine 20 s 稳态码率 320.8 kbps、零欠载/迟到；PHK110 90 s 码率末值/均值 320.8 kbps，队列 P50/P95/max 均 60 ms，链路丢包/PLC 0，欠载/迟到 24/22 且第 75--90 s 无新增，AudioTrack underrun 0、FastMixer writeErrors 0。下一步推进 NACK、码率自适应、真实弱网与 8 h soak；详见 `docs/18-m2-adaptive-jitter.md`、`docs/19-m2-redundant-send.md`。

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
| CI 可再优化 | `android` job 每次 `cargo install cargo-ndk` 约 2 分钟；`core` job 因 tools 引入 quinn/rustls 涨到 ~4 分钟 | 低 |
| QUIC 弱网实测 | ADR-004 的回退路径（裸 UDP）是否触发，取决于 M2 实测 | 中（M2） |
