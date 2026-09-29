# 欠载三分量（docs/73 P0）落地与本机实测：debug/release 对照 + 阳性对照

日期：2026-09-24。状态：**P0 已实施并实测；结论是不改生产逻辑**。
上游文档：`docs/73-playout-queue-full-lockup.md`（§2 三写入源、§3 被推翻的 v1、§4 P0 方案）。

## 1. 背景

docs/73 确认 `underruns` 有三个语义完全不同的写入点（A 播放节拍超时 / B 满队列拒收 / C 游标追赶），
而现场只有聚合值（峰值 48.8 次/s、水位顶格 160 ms、链路零丢包）。v1 凭聚合值反推机制被审核推翻，
因此 P0 纪律是：**先把三张嘴分开计数，再谈修复**。本文是 P0 的实施与实测记录。

## 2. 做法（零行为改动）

三个**进程内**计数挂在 `TelemetryAggregator`，先例 `depth_drops`（**不进 `StreamStats`** —— 那是
postcard 严格解码的冻结 wire 载荷，追加字段即打断旧版本节点解码）：

| 计数 | 写入位置 | 语义 |
|---|---|---|
| `schedule_slips`（A 线） | `runtime.rs` 播放循环 missed 路径（与 `record_underruns(missed)` 同锁同点） | 播放拍超时跳过的拍数 |
| `queue_rejects`（B 线） | `AudioReceiver::receive` 两个 emit 失败点（`ReceiveReport.queue_rejects`） | 待播队列满被 `try_push` 拒收的帧数；同帧仍记 `late_drops`（原口径不动） |
| `cursor_catchups`（C 线） | `take_due_frame` 追赶分支（`distance` 超前游标） | 游标逐拍追赶，每格一次；同拍仍按 `Missing` 记 `underruns` |

分解关系：`Δunderruns ≈ ΔA + ΔC + 空队列欠载（余量）`；B 不占 `underruns`（落在 `late_drops`）。
已知口径边界：C 里可能混有 ≤ `Δdepth_drops` 次降档/回收路径上的追赶（那一拍记 `depth_drops`
而非 `underruns`），对照 `depth_drops` 增量即可校正。

出口（三选一全部落地）：

1. **`Engine::underrun_breakdown(peer) -> Option<UnderrunBreakdown>`** —— 进程内 API（同 `depth_drops` 的形状）。
2. **1 Hz 变化门控 `tracing::info!`**（会话遥测 tick 内，仅当三分量任一增量 ≠ 0 时打印；
   健康链路全程静默）。desktop 由 `tauri-plugin-log` 接管 tracing，`npm run tauri:dev` 终端可见。
3. **soak-runner 报告**：进度行尾部追加 `A/B/C` 三列；摘要打印「欠载三分量」块（含占比与余量）；
   JSON 报告 `summary.underrun_breakdown`（first/last/delta/`queue_empty_residual`）与每条
   `coarse[].underrun_breakdown`（趋势）。

`underruns` 语义一字未改 —— 它是 soak-runner 的验收口径（`soak.rs` 的 `check_underruns`）。

## 3. 实测数字（本机，Windows，Realtek 48 kHz；原始件在 `target/evidence/p0-breakdown/`）

| 装置 | profile | 时长 | Δunderruns | A | B | C | 备注 |
|---|---|---|---|---|---|---|---|
| soak-runner（NullPlayout，真 QUIC 回环）10 ms 帧 | debug | 180 s | **0** | 0 | 0 | 0 | 水位 10–20 ms，判定 ok |
| 同上 | release | 180 s | **0** | 0 | 0 | 0 | 判定 ok |
| 引擎播放循环 + **真实 WASAPI sink**（新装置 `tests/p0_realsink_probe.rs`）10 ms 帧 | debug | 60 s | **0** | 0 | 0 | 0 | `write` 阻塞均值 **8.2 µs** / p95 11 µs / max 77 µs；99.93 拍/s |
| 同上 | release | 60 s | **0** | 0 | 0 | 0 | `write` 均值 7.1 µs；99.95 拍/s |
| self-loop（裸管线：采集→Opus→WASAPI render）10 ms 帧 | debug | 30 s | — | — | — | — | encode p50 **2569 µs**、decode 42 µs、sink 欠载 0 |
| 同上 | release | 30 s | — | — | — | — | encode p50 **2560 µs**、decode 37 µs、sink 欠载 0 |
| soak-runner + **16 核 CPU 饱和**（阳性对照）10 ms 帧 | debug | 60 s | **+2402** | **+1994（83.0%）** | 0 | 0 | 余量（空队列）408；`late_drops` 末次累计 2207；水位涨到 70 ms；判定 failed（预期） |

要点：

- **三分量管线端到端可用**：阳性对照里 A 线如期着火（饱和时播放线程被抢、拍超时跳拍），
  且 `ΔA / Δunderruns = 83%` 量级合理；单测钉住 B（emit 拒收）与 C（追赶分支）各自的写入点。
- **本机复现不出现场现象**：NullPlayout 与真 WASAPI sink、debug 与 release，全部 0 滑拍。
  事件驱动共享模式 WASAPI 的 `write` 在本机几乎不阻塞（均值 8 µs，20 ms 缓冲提供整拍余量）。
- **debug ≈ release**：dev profile 下 workspace 代码 opt-level 1、依赖（含 opus）opt-level 2
  （根 `Cargo.toml`），Opus 编码两 profile 同速（2569 vs 2560 µs）。docs/73 的
  「debug 构建太慢」假说在本机**不成立**。

## 4. 结论（阶段 3 的处置）

**不改生产逻辑。** 本地分量数据没有任何一条指向生产缺陷：干净链路三种配置两个 profile 全 0；
唯一非零数据来自人为 CPU 饱和（A 线按设计着火 —— 「被抢占后不追赶」是既有意图）。

两个对现场认知的修正（比数字本身重要）：

1. **现场读数是*对端*（Android）视角**。desktop 桥接对「发送方向」的会话展示的是**对端
   `STREAM_STATS` 反馈**（`engine_bridge.rs` `aggregate()`：「接收方向用本地数据，发送方向用
   对应设备的反馈」）。PC→Android 场景里水位/欠载是 **Android 播放侧**的账 —— docs/73 把
   「desktop 实例是 debug 构建」列为头号嫌疑，**搞错了进程**：desktop 的 build profile 与那个
   播放循环无关。Android 侧是 release APK（`out/`），其播放路径是 UniFFI → Kotlin → AudioTrack，
   与本轮测的 WASAPI 路径不同。
2. 现场现象独有的、本机不具备的变量：Android 播放端每拍成本/阻塞行为、真机调度环境。
   这些只能等真机复跑读三分量（见 §5）。

## 5. 未验清单（诚实记账）

- **真机未复跑**：`adb devices` 为空，PC→Android 现场三分量一次都没读过。下一轮：
  连上手机后重跑现场，desktop 终端的 1 Hz 门控日志（desktop 自身若是接收方）与/或
  经 FFI 暴露后的 Android 侧读数，直接给出 A/B/C 份额。
- **Android 侧读取手段未建**：引擎的 `tracing` 在 Android 进程没有 subscriber（已核实，
  `audiolink-ffi` 与 Kotlin 侧均无初始化）。要让真机（Android 为接收方）可读，需把
  `Engine::underrun_breakdown` 经 UniFFI 暴露 + Kotlin 侧打点 —— FFI 面三端齐动，本轮未做。
- **1 Hz 门控日志未实读验证**：代码编译通过、单测覆盖了计数本身，但工具链（soak-runner 等）
  不初始化 tracing subscriber，桌面实例本轮也未启动 —— 该日志行的实际输出尚未在任何
  带 subscriber 的进程里看到过。
- **B 线本机仍 0**：满队列拒收在本机构造不出来（发送节奏钉死供给速率；滑拍的帧走
  `late_drops` 而不是撑满队列）—— B 的真伪只能等真机读数。
- **现场根因未定位**：为什么 Android 播放消费跟不上（→ 节拍超时/水位顶格）仍待真机分量数据。
- CPU 饱和对照里 `late_drops` +2207 的构成（`take_due_frame` 迟到分支 vs 重排窗丢弃）
  未进一步拆分 —— 不在本任务范围。
- 水位/欠载治理的历史教训（docs/12 §11.12–§11.14，两版护栏代价 7×/103× 被真机否决）已读；
  本轮没有触碰任何回收/结算逻辑。

## 6. 门禁（本工作树，全绿）

```text
cargo fmt --all --check                                                     ✅
cargo clippy --workspace --exclude audiolink-desktop --all-targets --all-features -- -D warnings   ✅
cargo nextest run --workspace --exclude audiolink-desktop --lib --tests     ✅ 561 通过 / 3 跳过
cargo clippy -p audiolink-desktop --all-targets --all-features -- -D warnings                      ✅
cargo test -p audiolink-desktop                                            ✅ 50 通过 / 1 忽略
cd desktop; pnpm build                                                     ✅（tsc strict + i18n + vite）
```

（desktop 门禁日志存 `target/evidence/p0-breakdown/gate-*.log`。）

## 7. 复现命令

```powershell
# 三分量本机对照（各 180 s，10 ms 帧）
cargo run -p audiolink-tools --bin soak-runner -- run --seconds 180 --frame-ms 10 --report target/evidence/p0-breakdown/soak-debug-10ms.json --quiet
cargo run --release -p audiolink-tools --bin soak-runner -- run --seconds 180 --frame-ms 10 --report target/evidence/p0-breakdown/soak-release-10ms.json --quiet
# 真 WASAPI sink 探针（debug / release 各 60 s；Windows only，默认渲染端点，输出已静音）
cargo test -p audiolink-engine --test p0_realsink_probe -- --ignored --nocapture
cargo test --release -p audiolink-engine --test p0_realsink_probe -- --ignored --nocapture
```
