# M1 真机验收报告（PC → Android）

> 设备：MI 8 Lite（platina）· adb cd46884 · Android 10 / API 29 · arm64-v8a · Wi-Fi 172.16.2.54
> PC：Windows · 192.168.3.200（以太网）· Rust 1.96.1 · 工具：`tools/device-link`（本轮新增）· 2026-09-15
> 性质：**验收报告**。所有数字标注 [实测] / [模型] / [未测]；模型值不混进实测值；未测不写 0。

---

## 0. 结论摘要

**PC → Android 全链路首次在真机上打通**：真实 QUIC（mTLS）→ §5 握手 → PIN 配对（人类节奏）→ OPEN_STREAM →
Opus 20 ms/160 kbps → 手机 AudioTrack 低延迟输出，60 s 连续推流零丢包、零迟到丢弃。

| 验收项 | 结果 | 证据 |
|---|---|---|
| 内核在真机可用 | ✅ 自检 `PASS (5/5)`（§12 向量 + 冻结向量源逐字节比对） | `device-bringup/ui-02-selftest.xml` |
| 低延迟模式生效 | ✅ `getPerformanceMode() = LOW_LATENCY` | `device-bringup/ui-05-scroll.xml` · `06-audio_flinger.txt` |
| 零重采样 | ✅ 采集/内核/播放全 48000 Hz / 2ch；flinger 处理格式 `PCM_FLOAT`、主输出 `PRIMARY\|FAST` | `acceptance/flinger-run3.txt` |
| 出声 | ✅ 推流中手机 AudioTrack 活跃（`FrmRdy 3844/3844`、`Underruns 0`），且对端 1 Hz 遥测报「播放环水位 80 ms」——即 PCM 已进到待播队列 | `acceptance/flinger-run3.txt`（推流中）· `run3.log` 逐秒行 |
| PIN 配对（人类节奏） | ✅ 提交 PIN 后配对通过、双方写入信任库（落盘证据可证 **≥25 s** 延迟；终端另打印 27 s） | `acceptance/run5-pin-delay.log`（+ `pin-delay-test.ps1`） |
| 断流/丢包 | ✅ 四轮 60/60/90/75 s 连续推流全程保持 streaming、`late_drops 0`、`plc 0`；丢包末值 0.00%（**run5 有 1 个 1 s 窗口出现 2.00%**，见 §2.2 注） | run1c/run3/run4/run5 的 JSON |
| **e2e P50 ≤ 110 ms** | ❌ **当前不满足**：实测下限 ≥ 115 ms（不含设备侧输出缓冲）；见 §3 | `acceptance/run3.log` |
| 30 min 无断流 | ✅ **1800 s 全程 `streaming`、无断连**（但水位顶到 320 ms 上限、迟到丢弃 53、欠载 7→69） | §5.1 · `acceptance/soak30.log` |

**一句话**：链路与设备侧两项硬指标（低延迟模式、零重采样）达标；**延迟账本当前超预算**，
超出的部分已定位到两个可拧的旋钮（设备输出缓冲 80–101 ms、接收侧播放环水位 80 ms），处置见 §5。

---

## 1. 环境与拓扑（解释数字时必须一起读）

| 项 | 值 |
|---|---|
| 设备 | MI 8 Lite · Android 10 / API 29 · arm64-v8a · 包名 `com.gotkicry.audiolink.debug` |
| 手机网络 | Wi-Fi `vimedia-5G` 172.16.2.54/23 · RSSI -56 · Link 81 Mbps（Tx 81 / Rx 57）—— 机器口径采样于 15:47（`acceptance/network-facts.txt`），**晚于 run3（15:29）**，只作拓扑说明，不回证 run3 时刻的无线状态 |
| PC 网络 | 以太网 192.168.3.200/24 |
| **路径形态** | **跨子网三层转发**：PC → 192.168.3.1 → 192.168.0.2 → 192.168.20.2 → 手机（**不是同 AP 二层**）· 机器输出见 `acceptance/network-facts.txt`（`ping` 5/5 通、min 4 / avg 30 / max 90 ms、`tracert` 首跳 192.168.3.254） |
| 传输 | QUIC/UDP 58290，**强制 mTLS**（服务端索取客户端证书，见 `audiolink-net::tls`） |
| 设备音频 | 主输出 `PRIMARY\|FAST`，HAL 突发 192 帧（4 ms），普通周期 960 帧（20 ms），FastMixer 活跃 |

### 1.1 网络形态对测量的影响（本轮最重要的环境结论）

同一条链路，**探测节奏不同，RTT 差一个数量级**：

| 探测节奏 | §6 探针 RTT P50 | 质量 | 说明 |
|---|---|---|---|
| 1 Hz（`--count 60 --interval-ms 1000`） | **114.9 ms** | Poor | 低频探测的**无线唤醒代价** |
| 10 Hz（`--count 50 --interval-ms 100`） | **10.6 ms** | Fair | 路径本身很快（min 4.5 ms） |
| 推流中（音频 50 包/s 持续） | **11.7 ms** | Fair | 与 10 Hz 同量级 |

**推论**：设备侧无线电省电策略会把「每秒一个探针」拖到 100 ms 量级；
**端到端延迟预算的前提是链路持续有报文**（本项目音频 50 包/s 天然满足）。
裸用 1 Hz 的 RTT 去估网络段会**高估 10 倍**——报告与 M2 的抖动缓冲深度都必须按推流中的口径取值。

---

## 2. 验收矩阵（逐项）

### 2.1 设备侧（`adb` 直读，独立证据）

| 指标 | 实测值 | 出处 |
|---|---|---|
| `getPerformanceMode()` | `LOW_LATENCY` ✅ | `ui-05-scroll.xml` |
| 采样率 / 声道 | 48000 Hz / 2 ✅ | 同上 + flinger |
| AudioTrack 缓冲（请求 → 实际） | 960 帧 → **3844 帧 = 80.08 ms** ⚠️ | `ui-06-scroll.xml` · flinger `FrmCnt 3844` |
| flinger 对该 track 报的 Latency | **101.00 ms** ⚠️ | `acceptance/flinger-run3.txt`（推流中） |
| 播放环容量 | 2880 帧（60 ms） | `ui-run3.xml` |
| 播放环水位 / 环溢出 / 读空 | ⚠️ **证据缺口**：唯一落盘的 dump（`ui-run3.xml`）是**推流结束后**采的（水位 0 / 2880、溢出 5 896 800、读空 59 292）；Lead 在推流中另读过一次（水位 1920 / 2880、溢出 4 332 000、读空 58 788），但**当时没有落盘**。
|  | → 按本项目纪律不写进验收数字；**由 task-8 以「推流中 dump + 区间增量」重新采证**（见 §2.1b） |

### 2.1b 设备侧播放缓冲 A/B（task-8，真机同一条连接内切换）

被测对象：**设备侧输出缓冲容量**（AudioTrack `FrmCnt`）与**播放队列目标水位**（`FrmRdy`）。
全部在同一条 device-link 连接内切换，因此不受环境漂移影响；`Latency ≈ FrmRdy/48k + 21 ms` 全部自洽。

| 配置 | `FrmCnt`（容量） | `FrmRdy`（水位） | flinger Latency | 系统欠载 | 供给欠载 Δ | 静音填充 Δ | 保住 FAST |
|---|---|---|---|---|---|---|---|
| **满灌（默认，= 改前）** | 3844 | 3844（80.1 ms） | **101.00 ms** | 0 | 0 / 35 s | 0 | ✔ |
| 路② 队列目标 20 ms | 3844 | 672（14.0 ms） | **35.00 ms** | 0 | +1251（窗口 48 s 墙钟 / 36.3 s 音频） | +600 480（占该窗口 ≈34% 静音） | ✔ |
| 路② 队列目标 **30 ms** | 3844 | 1440（30.0 ms） | **51.00 ms** | 0 | **0 / 74 s** | **0 / 74 s** | ✔ |
| 路② 队列目标 40 ms | 3844 | 1632（34.0 ms） | 55.00 ms | 0 | — | +1 505 280 / 113 s（只算本档窗口；先前记的 +2 105 760 是**跨档累计**，已结案） | ✔ |
| 路① 容量收缩到 20 ms | **960r** | 960（20.0 ms） | **41.00 ms** | 0 | — | — | ✔ |

- 路①（`setBufferSizeInFrames`）**生效且没丢 FAST**（`FrmCnt` 3844→960r，FastMixer 持续 `MIX_WRITE`）—— 原先担心的「收缩会掉出快速链路」没有发生；
- 路② 的 **30 ms 档是唯一「既降 50 ms 又零代价」的档**（20 ms 档用 ≈34% 静音换延迟，不可取；数字与窗口由独立复核者从原始 UI 计数复算）；
- **但这 50–66 ms 只是把延迟从 AudioTrack 容量搬到了内核播放队列，`device-link` 账本里的 e2e 下限没变** ——
  因为 `buffer_level_us`（对端上报的播放环水位）**不会被档位切换压下去**（三轮 P50 分别 120 / 140 / 180 ms，P95 到 220 ms），由上游推送速率决定。见 §4 #9。
- 装机默认：**保持满灌不变**（`DEFAULT_QUEUE_TARGET_FRAMES = QUEUE_TARGET_UNLIMITED`），档位留成 UI chip；启用 30 ms 只需改这一行常量。

> 出处：`device-bringup/README-task8.md`（含 3 轮带完整账本的 device-link 输出 + 4 份被脚本中断的记录）、各档 UI dump 与 flinger 片段（满灌 `flinger-A.txt`、20 ms `flinger-B.txt`、30 ms `flinger-q30.txt`、40 ms `flinger-q40.txt`、路① `flinger-D-shrink20.txt`）。
> 复核（`verification/README.md` §12）逐份核对了 `FrmCnt/FrmRdy/Latency`、`Underruns=0`、`flags 0x6 (PRIMARY|FAST)`、FastMixer `MIX_WRITE` 递增与自洽式 `Latency ≈ FrmRdy/48k + 21 ms`。

### 2.2 链路侧（`device-link` 账本；表内列三轮推流，run5 见 §0 与 §2.2 注）

| 指标 | run1c（synth 60 s） | run3（**真实 WASAPI** 60 s） | run4（synth 90 s） |
|---|---|---|---|
| 配对方式 | PIN（首次） | AUTH（信任库命中） | PIN（新身份） |
| §6 offset 收敛 | 1.0107 s | 1.0108 s | 1.009 s（三份 JSON 的 `clock.first_converged_at_s`） |
| §6 探针计数（sent/recv/replies/unmatched） | — | 105/105/105/0 | — |
| 会话内 §6 逐探针 RTT P50 / P95 | — | **11.73 / 81.11 ms** | — |
| 独立测量连接（mTLS）RTT P50 / P95 | — | **9.36 / 106.25 ms**（30/30） | — |
| 网络单向 P50 [模型]（**账本取 §6 逐探针 RTT P50 ÷ 2**） | 9.37 ms（18.749÷2） | **5.86 ms（11.728÷2）** | 13.02 ms |
| 逐探针 `t2−t1−offset` P50 / P95 [模型]（独立测量连接，交叉印证用） | — | 4.68 / 53.13 ms | — |
| QUIC 平滑 RTT（1 Hz 采样）P50 / P95 | — | **13.33 / 39.50 ms**（⚠️ 仅工具运行期汇总串，**逐秒样本未导出**，不可独立复算） | — |
| 对端播放环水位 P50 [实测] | 100 ms | **80 ms** | 120 ms |
| 对端欠载 / 迟到 / PLC（**末值**口径） | 6 / 0 / 0 | **3 / 0 / 0** | **57 / 0 / 0**（首值 3 → 90 s 内新增 54；`run4-pin-regression.json` 首/末字段） |
| 丢包率 / 码率 | 0.00% / 160.4 kbps | 0.00% / 160.4 kbps | 0.00% / 160.4 kbps |
| **e2e 下限（模型合计）** | ≥ 139 ms | **≥ 115 ms** | ≥ 163 ms |

> ⚠️ **两处端点口径披露**（独立复核者提出，补记以免误读）：
> ① run3 的 `--capture wasapi` 用的是 **`--device id:c85b743e`（Realtek 扬声器）**，**不是本机默认渲染端点**；工具为此会打一条「当前不是默认渲染端点」的提醒 —— 即：本轮的「真实 WASAPI」是**指定端点**上的真实系统输出，不是默认端点上录到的声音。
> ② 另有一次 run2 试图用**默认端点**（NVIDIA HDMI，无输出）做真实采集，全程 0 帧封口 → 工具按设计 **exit 5** 并打印「PC 采集疑似静音」。**这次失败没有进账本表**（它不是一次成立的观测），在此披露以免「四轮全成功」被误读成「默认端点也可用」。
>
> ⚠️ 「e2e 下限」= 采集半周期 10 ms + 组帧 20 ms + 网络单向 + 对端播放环水位；
> **不含**设备侧 AudioTrack 输出缓冲、PC 封口→发出、对端解码 —— 这三项没有上界，所以只给下限，不给区间。

### 2.3 未测项（诚实清单，补齐前不给承诺区间）

| 项 | 为什么未测 | 现状 |
|---|---|---|
| 对端 AudioTrack 输出缓冲 | 只能在设备侧读 | **本轮已补**：3844 帧 = 80.08 ms；flinger track Latency 101 ms |
| 对端底层是否低延迟 | 同上 | **本轮已补**：`LOW_LATENCY` + 主输出 `FAST` + FastMixer 活跃 |
| PC 封口 → 数据报发出 | 引擎不导出该埋点 | 同口径替代：编码 P50 0.06 ms（`device-tool` E1）；M2 加埋点 |
| 对端解码耗时 | 同上 | 同口径替代：本机 decode P50 57 µs（`docs/09-benchmark-notes.md`） |

---

## 3. 延迟账本（把每一段摊开）

以 run3（真实 WASAPI 采集 · 60 s · 推流中）为例，[实测]/[模型] 分开：

```text
采集半周期        [模型]  10.0 ms   （帧长 20 ms 的一半；设备周期实测 10 ms）
组帧              [结构]  20.0 ms   （20 ms 帧长本身，不可压缩）
封口→发送         [未测]   —        （引擎未导出埋点）
网络单向          [模型]   5.86 ms  （= §6 探针 RTT P50 11.73 ms ÷ 2。另一路**独立测量连接**的逐探针 t2−t1−offset P50 = 4.68 ms 可交叉印证；两路都含对称路径假设）
对端收包→播放环   [参考]   解码 0.06 ms（**本机**同配置实测；对端真值未导出，engine 无埋点）
对端播放环水位    [实测]  80.0 ms   （对端 1 Hz STREAM_STATS buffer_level_us P50）
对端 AudioTrack   [实测]  80.1 ms   （设备侧实际缓冲 3844 帧；flinger 报 track Latency 101 ms）
─────────────────────────────────────────────
合计（P50 口径）  ≈ 195 ms（下限 115 ms + 设备输出 80 ms）
```

**M1 目标 vs 现状**：目标 P50 ≤ 110 ms / P95 ≤ 150 ms；**当前超预算**。
预算被吃在两处，且都是**可拧的旋钮**（不是协议或物理限制）：
1. 对端播放环水位 80 ms —— 接收侧待播队列深度。**口径要说准**：`PRIME_FRAMES=2` 只是「攒够 2 帧才开播」的 40 ms 门槛，
   队列容量是 `PLAYBACK_QUEUE_FRAMES=16`（320 ms），实测水位在 40→160 ms 之间随运行增长（满灌 + 消费/生产节奏差）；
2. 对端 AudioTrack 实际缓冲 80 ms（请求 960 帧、实际生效 3844 帧；3844 = 80.08 ms。**容量与队列水位是两件事**）。

---

## 4. 本轮挖出的缺陷（真机才暴露得出来）

| # | 现象 | 根因 | 处置 |
|---|---|---|---|
| 1 | 手机从不显示配对 PIN，§5 配对在 Android 侧走不通 | FFI 早已导出 `displayedPin()`，Kotlin 从未调用 | ✅ task-7 修复并真机验证（56sp 大字卡片） |
| 2 | 对端断开后 PIN 卡仍挂着旧值 | FFI `apply_event()` 只在 `PairCompleted{ok:true}` 清缓存，`PeerDisconnected` / 失败落进 `_ => {}` | 🟡 UI 侧标注「已失效」；根治在 ffi 写域（待办） |
| 3 | **人工提交 PIN 必然失败**（`1002 unknown peer`） | 握手死线 10 s **硬编码**，而 §5 的 PIN 有效期 60 s | ✅ task-10 修复（配对等待不吃死线，默认 10 s / 75 s；含负向对照）；真机延迟提交（落盘可证 ≥25 s）配对通过 |
| 4 | 低延迟模式生效但输出缓冲仍是 80 ms | 请求 960 帧被框架顶到实际 3844 帧（`getMinBufferSize` 侧约束）；缓冲**容量**与队列**水位**是两件事 | 🔧 task-8 做 A/B（容量收缩 / 注水深度控制） |
| 5 | 播放环接近满且溢出/读空计数很大 | 收侧固定深度 + 满灌，节奏抖动被 80 ms AudioTrack 吸收 | 🔧 task-8 一并量化为区间增量 |
| 6 | M0 的 `latency-probe` 打不了生产端点 | 生产端强制 mTLS，而该工具客户端不带证书 | ✅ task-2 补自签客户端证书，20/20 样本可用 |
| 7 | `Ptype::ClockReply` 文档注释写「恰 24 B」 | 笔误（应为 28 B） | ✅ 已修（`audiolink-types`） |
| 8 | 绝对 offset 跨连接按墙钟 1:1 漂 | `now_monotonic_us()` 在 Android 是**进程相对**基准（`LazyLock<Instant>`） | 📝 语义已澄清：**offset 只在同一次连接内有效**；跨连接/重启后不得复用 |
| 9 | **接收侧延迟总量降不下来**：设备侧省下的 50–66 ms 只是在内核队列里排队 | **内核→Kotlin PCM 推送 ≈ 2× 实时**（task-8 区间增量实测：推流 78 s 环溢出 +3 791 040 帧 = **48 603 帧/s = 1.01× 实时率**，同期**读空 +0**）→ 环始终满、消费侧永不缺数据。
　注：**「1.01×」是溢出速率、「≈2×」是推送速率**，两者不矛盾 —— 推送 ≈ 溢出 48 603 + 消费 48 000 ≈ **96 600 帧/s ≈ 2.01× 实时**（也可读作 50.6 次/s 丢弃 + 50 次/s 播放） | 🔧 **task-13 立项**（ffi/engine 写域，M2 抖动缓冲的前置）；修好前改默认档只会让延迟在管线间搬家 |
| 10 | `buffer_level_us` 文档写「1 Hz 下通常 0–1 帧」，真机**稳定 7 帧（140–180 ms）** | 该字段口径 = `PlayoutHandle::depth_frames() × 帧长`（内核侧已解码未取走的帧数），文档与实现不一致 | 📝 一并纳入 task-13 核对（文档或实现改一个） |
| 12 | soak 中「已封口 4096 帧」看起来像**发送端停发** | `device-link` device 模式的该显示值 = `MeasurementTap::sealed_orphans`，而 tap 容量固定 **4096**（`device_link.rs:732`），满了丢最旧 ⇒ **计数饱和**，不是停发（旁证：同期链路 loss 0.00%、对端队列仍在变化） | 📝 工具显示缺陷：device 模式应换成无上限的朴素计数（不影响任何验收数字） |
| 11 | 推流速率与消费速率不完全匹配 | **实测**：run3 对端水位 20 → 80 ms / 60 s（斜率 **+1 ms/s**）；**速率差**：PC 侧 60 s 封口 3030 帧 = 50.5 fps vs 对端消费 50 fps（差 0.5 帧/s）。⚠️ 两者量级不一致（按速率差应为 +10 ms/s，60 s 会涨 600 ms 并撑爆 320 ms 的队列容量，但实测 `late_drops=0`）—— **机制未查清** | 📝 属 M2「速率匹配 / 漂移补偿」；M2 必须先解释这个不一致，再定缓冲策略 |

---

## 5. 结论与下一步

**达标**：链路可用性、设备低延迟模式、零重采样、PIN 配对（人类节奏）、四轮 60/60/90/75 s 连续推流无断流（`late_drops 0`、`plc 0`、丢包末值 0.00%）。
**未达标**：e2e 延迟（P50 目标 110 ms；实测下限 115 ms + 设备输出 80 ms ≈ 195 ms）。

下一步（按收益排序）：
1. **task-13（唯一入口）**：定位并修掉「内核→Kotlin PCM 推送 ≈2× 实时」（§4 #9）—— 设备侧那 50–66 ms 之所以降不下来，就是因为延迟被搬进了内核队列，**不修上游，任何设备档位都只是搬家**；
2. ~~30 min soak~~ ✅ **已跑**（§5.1：无断连，但水位/迟到/欠载需修）—— 其结论并入第 1 条的必要性论证；
3. **兑现设备侧杠杆**：上游修好后把 `DEFAULT_QUEUE_TARGET_FRAMES` 切到 30 ms（实测 −50 ms、零欠载代价，§2.1b）；
4. **速率匹配 / 漂移补偿**（M2）：PC 封口 vs 手机消费的速率失配（§4 #11，实测斜率口径见注）；
5. ffi 写域：`apply_event()` 补 `PeerDisconnected` / 失败分支的 PIN 清空（缺陷 #2 根治）；顺带把设备侧 `queuedFrames` 补进对端遥测（现在设备侧省下的那 50 ms 在账本上看不见）。

### 5.1 30 min soak（真机 · 最终固件 · synth 采集）

命令：`pwsh target/evidence/acceptance/soak-30min.ps1 -Seconds 1800 -Dir target/device-link-3 -Tag soak30`（exit 0）

| 指标 | 结果 | 判定 |
|---|---|---|
| 会话存活 | **1800 s 全程 `streaming`，无断连、无重连** | ✅ **30 min 连续无断流达标** |
| 丢包 | 末值 **0.00%**（但个别 1 s 窗口最大 **6.25%**） | ⚠️ 有瞬时丢包，非全程干净 |
| 码率 | 末值 160.4 kbps / 均值 160.3 kbps（目标 160） | ✅ |
| 迟到丢弃 `late_drops` | 0 → **53**（t=438 s / 7.3 min 到 43，尾段再 43→53） | ❌ 出现过丢弃 |
| 供给欠载 `underruns` | 7 → **69**（平均 +2.07/min，非匀速：前 6 min +48、中段 10 min 平台、尾段 +14） | ⚠️ 未收敛 |
| 播放环水位 | P50 **200 ms** / P95 300 ms / P99 **320 ms（= 队列容量上限）** | ❌ 水位顶到上限并在此振荡 |
| e2e 下限（模型合计） | **≥ 235 ms**（网络单向 5.08 + 水位 200 + 10 + 20） | ❌ 远超声明的 P50 ≤ 110 ms |
| PLC | 0 | ✅ |

**读法**：链路层是稳的（30 min 不断、丢包主要来自瞬时窗口），**坏在接收管线**：
水位从 160 ms 一路顶到 320 ms 容量上限并在该区间振荡，迟到丢弃与供给欠载在前几分钟各涨了一截。
这与 §4 #9（内核→Kotlin 推送 ≈2× 实时）和 #11（速率失配）**同源** —— 上游持续多推，接收侧只能用「丢最旧 / 迟到就丢」消化，
于是延迟被钉在队列上限附近。**结论：M1 的「30 min 无断流」达成，但「延迟不漂、不丢帧」未达成，且根因已定位到上游推送速率。**

> 证据：`acceptance/soak30.log`（1800 行逐秒）、`acceptance/soak30.json`（账本）、`acceptance/soak30-counters.txt`（**事后**设备侧读数：因采集脚本末尾被中断，只留下流停止后的快照），
> `acceptance/soak30-console.log`。另外三条 soak 新事实（独立复核者从 JSON 提出）：`drift_reliable=true` / **`drift_ppm=2`**（M2 漂移补偿的现成基线）、探针 **1834 发 / 1816 回 / 18 未回（≈1%）**、`rtt_samples=200`（§6 分位只覆盖最近 200 样本）。
> ⚠️ `counters.txt` 里的「供给欠载 8478」是**设备侧累计**、与对端 69（同一 30 min 窗口）不是同一口径，不要混读。
> 注：逐秒行里的「已封口 4096 帧」是**工具计数饱和**（tap 容量 4096），不是发送端停发 —— 见 §4 #12。

---

## 6. 复跑命令（全部可复现）

```powershell
# 1) 真机 bring-up：装包 → 启动 → 点「启动并开始接收」→ 自检 PASS → 监听 58290
pwsh android/scripts/build-rust.ps1
pwsh tools/gradlew.ps1 -JavaHome 'C:\Users\liuzh\scoop\apps\corretto17-jdk\current' assembleDebug testDebugUnitTest
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n com.gotkicry.audiolink.debug/com.gotkicry.audiolink.MainActivity

# 2) 真机推流验收（synth 对照 / 真实 WASAPI 采集）
target/x86_64-pc-windows-msvc/debug/device-link.exe run --peer 172.16.2.54 --seconds 60 --capture synth --pin-file target/evidence/acceptance/pin.txt
# 真实系统声音：先在渲染端点播放内容，再指定端点采集
cargo run -q -p audiolink-tools --bin self-loop -- run --seconds 140 --capture synth --device id:c85b743e --sink-device id:c85b743e --gain 0.3 --marker-ms 0
target/x86_64-pc-windows-msvc/debug/device-link.exe run --peer 172.16.2.54 --seconds 60 --capture wasapi --device id:c85b743e --cross-probe 30

# 3) 网络段交叉验证（冻结工具，已补客户端证书）
cargo run -q -p audiolink-tools --bin latency-probe -- probe 172.16.2.54:58290 --count 50 --interval-ms 100
```

> 设备侧读数：`adb shell dumpsys media.audio_flinger`、`adb shell uiautomator dump` + `adb pull`（UI 原值），
> 证据全部落在 `target/evidence/acceptance/` 与 `target/evidence/device-bringup/`。

---

## 7. 独立复核与本报告的修订（task-6）

本报告由 Lead 产出后交给**未参与实测**的独立验证者复核（`target/evidence/verification/README.md`；41 个溯源项：可信 29 / 存疑 8 / **不成立 4**）。
下面 4 项被判「不成立」，已全部修正（保留修订记录，不隐藏）：

| # | 原写法 | 核实后的真实证据 | 修订 |
|---|---|---|---|
| 1 | run4「对端欠载 3」 | `run4-pin-regression.json` 末值 **57**（首值 3，90 s 内新增 54）—— 原表把**首值**当成了该次读数 | 改为 57，并注明首/末口径 |
| 2 | 「播放环水位（推流中）1920/2880」 | 唯一落盘 dump 是**推流结束后**的（水位 0/2880）；1920/2880 无落盘出处 | 撤下，标注证据缺口，交 task-8/soak 重采 |
| 3 | 「环溢出 4 332 000 / 读空 58 788」 | 落盘 dump 实为 5 896 800 / 59 292（更晚时刻）；前者无出处 | 同上：改为「推流中 dump + 区间增量」 |
| 4 | §3「网络单向 [模型] 4.68 ms」 | 账本实际采用 **5.86 ms**（= §6 逐探针 RTT P50 11728 ÷ 2，`device_link.rs:1903-1913` 第 1 顺位）；4.68 是另一路的交叉印证值 | 已改为 5.86 并说明两路关系 |

另有 8 项「存疑」按建议就地修正：§6 收敛时间写实为 1.009–1.011 s；网络单向一行不再混装两种口径；QUIC RTT 标注「样本未导出、不可独立复算」；
run5 丢包补披露「1 个窗口 2.00%」；§3 解码耗时标注为本机参考；`PRIME_FRAMES` 与队列容量口径分开写；§1 网络事实补机器输出（`network-facts.txt`）。

**复核发现的证据缺口（未修，交后续任务）**：run4 只有 JSON、无逐秒 log；「27 s」只在脚本终端输出（落盘可证「≥25 s」）；手机侧「推流中」的 UI dump 缺失。

复核明确**未覆盖**：真机操作、Android `testDebugUnitTest`、30 min soak、task-8 的 A/B（复核窗口内尚不存在或手机被占用）。
复核者自跑门禁：`fmt` / `clippy` / `cargo test --workspace --exclude audiolink-desktop` 全部 exit 0（`verification/regression-gates.txt`）。

> 结论：**「链路已打通」与「延迟超预算」两个主结论不受修订影响**；被修掉的是数字口径与出处。

### 7.1 第二轮与第三轮复核（均已修正）

**第二轮（6 条）**：四轮时长笔误（60/90/75/90 → **60/60/90/75**）；§1 网络事实改用机器口径（RSSI −56 / Link 81 Mbps，采样 15:47，**不回证 run3 时刻**）；§0「出声」行撤下无出处的 `ui-run3.xml` 引用，改用 flinger（推流中）+ 逐秒遥测；「4×961」撤下（961 无出处）；表头「三轮」注明口径；§2.1 表格行结构与 §6 空行。

**第三轮（7 条）**：
- **§5 自相矛盾（P0）**：原文写「task-8 是唯一的达标路径」，与 §2.1b「只是把延迟搬了个位置」直接冲突 → §5 重写为 **task-13（推送 ≈2× 实时）是唯一入口**，设备档位在其后兑现；顺带修掉编号跳号（1,2,3,5 → 1..5）；
- A/B 代价列的**窗口时长**按原始 UI 计数复算更正（20 ms 档 48 s 墙钟 / 36.3 s 音频；30 ms 档 74 s）；「≈18% 静音」更正为 **≈34%**（600 480 ÷ 1 744 320）；
- 40 ms 档的「+2 105 760 异常」**结案**：那是**跨档累计**（含 20 ms 档的 600 480），本档窗口净额 1 505 280 / 113 s；
- `buffer_level_us` 更正为「档位切换**压不下去**」（三轮 P50 = 120 / 140 / 180 ms、P95 220 ms），不再写「恒定 140–180」；
- 出处注更正为「3 轮带完整账本 + 4 份被中断」，flinger 出处逐档点名；
- #9 补上「1.01×（溢出）vs ≈2×（推送）」的桥接推导；**#11 的 +1 ms/s 与 0.5 帧/s 不一致，如实标注为「机制未查清」**，不再制造自洽的假象。

验证者另自跑 Android 单测：首次 `UP-TO-DATE`（未真执行）→ 加 `--rerun` 强制重跑得 **XML=8 / tests=69 / failures=0**（`verification/android-unit-tests-rerun.txt`）。

> 四轮复核共 **61** 个核对项（41 + 6 + 7 + 7）；**每一条都在本文件留下修订痕迹**。主结论四轮均未被推翻。

---

## 8. 真机联调补充（2026-09-15 深夜，用户实机试听阶段）

### 8.1 网络变了：手机换到与 PC 同子网

手机从 `172.16.2.54/23`（跨三层路由）换到 **`192.168.3.59/24`**（与 PC `192.168.3.200` 同 AP）。
同一套代码在该路径上：**§6 逐探针 RTT P50 6.65 ms / P95 19.69 ms**，e2e 下限 **93 ms** —— 比 §2.2 的跨三层口径好一倍以上。
（§2.2/§3 的历史数字仍按当时路径有效，不要与新数字混读。）

### 8.2 听感缺陷：接收侧队列持续上涨 → 周期性丢帧（**新增 P0 条目**）

对照实验：用桌面端身份（AUTH 直连、无需 PIN）推 **40 s 纯 440 Hz**，手机端上报：

| 时段 | 水位 | 欠载 | 迟到 | 丢包 |
|---|---|---|---|---|
| 前 15 s | 20–60 ms | 0 | 0 | 0.00% |
| 40 s 末值 | **260 ms** | **20** | **6** | 0.00% |

队列以 **≈0.5%/s**（≈5500 ppm）稳定上涨，**远超晶振漂移（±50 ppm）**，是**节奏 bug** 而非硬件差异；
涨到内核队列上限（`PLAYBACK_QUEUE_FRAMES` = 16 帧 = 320 ms）后开始「迟到就丢」→ 听感即为**周期性卡顿**。
与 §4 #9（内核→Kotlin 推送 ≈2× 实时）同族、机制不同（那是推送速率、这是收发速率）。

> ⚠️ **口径修正**：先前文档建议「满灌」以保证连续 —— 对这条缺陷**方向是反的**：满灌会把 0.5% 的富余攒成批量丢弃（更断续），
> 而 **20/30 ms 档把小富余摊成均匀丢帧**（约每 2 秒一帧，听感更接近连续）。待按此复测后更新 §5。

### 8.3 桌面 UI：缺采集端点选择器

桌面壳只能采「默认输出端点」，没有 CLI 那样的 `--device`。用户切换 PC 输出设备后，
App 侧可能仍绑在启动时的端点上 → 表现为「连上了但没声音 / 声音断续」。已立项。

### 8.4 Android：PIN 卡有时不显示

PC 侧已收到 `PAIR_REQUIRED`（工具打印「进入 PIN 配对分支」），但手机屏幕没有 PIN 卡 → 无法完成配对。
怀疑 FFI 事件泵的 broadcast 丢事件（`engine_bridge.rs` 注释自述「背压丢事件」）。已立项。
另：`uiautomator dump` 因界面每 500 ms 刷新**拿不到 idle**（`could not get idle state`），自动化读屏要改用 `screencap` + 人工/视觉读取。

### 8.5 发行构建：release 路径首次打通（含两个真缺口）

| 项 | 结果 |
|---|---|
| `android/app/proguard-rules.pro` | **原本不存在**（release 引用了它）→ 已补 JNA + UniFFI 的 keep 规则（否则 R8 删掉反射目标，装上也一启动就崩） |
| 签名 | 生成 release keystore（RSA 4096 / 30 年）→ `app-release.apk` **7.98 MB**，`apksigner` 校验 V2 签名通过 |
| `.gitignore` | 🔴 **`keystore.properties` 原本没被忽略**（只有 `*.jks`）→ PUBLIC 仓库存在口令入库风险 → 已修（顺带踩到本项目自己记录的坑：`.gitignore` 不支持行尾注释） |
| MIUI 安装 | `adb install` release 包被拒 **Failure [-99]**（debug 包可装）→ 需开发者选项打开「USB 安装」；属环境项 |


