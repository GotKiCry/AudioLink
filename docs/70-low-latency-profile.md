# 低延迟档（10 ms Opus 帧 + 20 ms 输出目标）

日期：2026-09-21。承接 [docs/68](68-audioshare-latency-comparison.md)（AudioShare 对比分析）与 [docs/69](69-playout-latency-recovery.md)（播放积压回收），本轮把「低延迟」做成一个**用户可开的档位**，默认关。

## 档位语义

| | 标准档（默认） | 低延迟档 |
|---|---|---|
| Opus 帧长 | 20 ms（`CodecConfig::m1_default()`） | **10 ms**（`CodecConfig::m1_low_delay_tight()`） |
| Android 播放水位默认值 | 1440 帧（30 ms） | **960 帧（20 ms 目标，实际约 10–20 ms）** |
| 其余链路 | 不变（RESTRICTED_LOWDELAY、QUIC 数据报、自适应抖动保护垫毫秒制：起步 20 ms = 1 帧） | 不变（同一套毫秒保护垫按帧长换算：起步 20 ms = 2 帧，墙钟余量不减半） |

原方案的理论收益：组帧 −10 ms、播放目标 −20 ms，端到端 P50 从 80–110 ms 压到约 55–80 ms；2026-09-23 将输出目标加到 20 ms 后，此数字不再适用，真机延迟仍待实测。

代价：包率翻倍（50 → 100 pps）、每包协议开销占比上升、调度抖动余量变薄（弱网欠载风险上移）。

## 与 AudioShare 手段的对照（为什么档位只压这三处）

AudioShare（提交 dac02fd）的低延迟 = 全链路零积压：采集回调直发裸 PCM、TCP_NODELAY、接收 socket 缓冲压缩到 `getMinBufferSize()` 制造背压、`AudioTrack.write()` 后立刻 `flush()`、收发两侧都是「忙则丢帧」。

本项目逐条对应后，机制层已全部覆盖或超越（`PERFORMANCE_MODE_LOW_LATENCY`、QUIC 数据报天然无 TCP 积压、水位门控是 `flush()` 的稳定等价物、WRITE_NON_BLOCKING + 环缓冲丢最旧）。真正剩下的差距只有三处**保守默认值**，即上表三行；flush 每帧清空的打法刻意不学 —— 它在本项目的多路混音 / 预约播放场景有害（docs/69 的预算回收已是更稳的等价物）。

## 实现落点（三端）

- **内核**：`EngineConfig::with_codec()` builder（core/crates/audiolink-engine/src/runtime.rs）；FFI `EngineStartConfig.low_latency`（`#[uniffi(default = false)]`），`engine_start` 按档位选 `CodecConfig`。Kotlin 生成物 `audiolink_ffi.kt` 手工逐字段同步（kotlin_guard 护栏钉住一致性；UniFFI 按值序列化 record，函数 checksum 不变）。
- **desktop**：settings.json `lowLatency` 键（settings.rs 三件套，读坏回默认）；`engine_config()` 现读设置显式选档；设置面板开关（中英双语）。WASAPI 播放缓冲**不联动**：共享模式系统下限 ~22 ms，压请求值换不来实际收益（engine_bridge.rs 有决策注释）。
- **Android**：SharedPreferences `low_latency`（LowLatencyPreference，与 PowerWhitelistProbe 同款模式）；`engineStart` 读**会话镜像**（单一数据源）；播放水位由 `LowLatencyDefaults.queueTargetFor/Switch` 联动（纯 JVM 可测，7 条单测）；设置页 `LowLatencyDeck` 开关卡。

## 三条硬约束（写进了代码注释与 UI 文案）

1. ~~两端必须同档位~~ → **已被帧长联动取代（2026-09-22，见下节）**。开关语义变为「本端作为**发送端**时的帧长」；接收端开流期从 `OPEN_STREAM.codec_prefs` 读发送端帧长并自动跟随解码，两端档位不同也能正常工作。混档被当 PLC 掩盖的静默坑已根治。
2. **引擎档位下次启动生效**。切开关只落盘 + 更新会话镜像，不重启引擎/前台服务（与 set_auto_broadcast 先例一致：不为一个开关掐断用户正在听的音频）。播放水位例外：切档**即时**重置（每拍下发，不碰 AudioTrack，遵守线程亲和纪律）。
3. **手动水位滑杆永远优先**。档位只决定「默认值 / 切档那一刻的重置值」；DiagnosticsDeck 的手动滑杆是排障工具，若被档位每拍覆盖会出现「拖了没用」的幽灵 bug（LowLatencyDefaults.kt 的类注释锁死了这条）。帧长联动的水位跟随同样遵守：协商值**变化时**重置一次，之后手动调节依旧优先。

## 帧长联动（2026-09-22 落地）

**机制**：流的帧长由发送端档位决定。接收端在 `OpenStream` 分支取 `codec_prefs` 首个 `Opus` 项的 `frame_ms`（合法值 ∈ {10,20,40,60}，非法/缺失回退本地档），构造「有效 codec」建整套接收链路（`StreamRxState`：解码器/掩盖器/重排窗/抖动起步/播放线程），`OPEN_STREAM_ACK.codec_chosen` 回报实际生效帧长。发送端所有音频包（含冗余副本与 NACK 重传）按帧长置位 `FRAME_10MS`（§3 flags bit 2，此前已预留但未接线），接收端逐包校验、不符只计数告警不断流。

**时序取舍**：先到音频包按本地档解、协商后 `retarget` 整体重建 —— 控制流与数据报不保证先后，「推迟到协商后建」会稳定丢掉开流头几帧；「先解后重建」在同档（绝大多数）时头几帧完全正确，异档时最坏几帧 PLC（runtime.rs 有完整注释）。

**两端外壳**：

- 协商结果经 `Engine::negotiated_frame_ms(peer)` / FFI `PeerView.negotiated_frame_ms` 暴露；desktop PeerCard 质量区与 Android DiagnosticsDeck 均展示「协商帧长」。
- **Android 播放水位跟随生效帧长**：协商确立/变化时重置默认水位（10 ms→960 帧，20 ms→1440 帧）；断流不重置、同值幂等不重复重置、非法值不猜（真值表钉在 LowLatencyDefaultsTest）。desktop WASAPI 水位不联动（共享模式系统下限 ~22 ms，照旧）。
- 两端设置页文案已改口径：「本端作发送端时的帧长；接收端自动跟随，两端不必再手动同档」。

**验证**：engine 集成新增 `frame_length_negotiation.rs` 5 条（异档双向对齐 + 同档对照 + 非法帧长回退，异档下 `plc_count==0` 证明混档指纹消失）；engine lib 4 条（flags 置位三路径 / 校验计数 / prefs 选取 / retarget 重建）；FFI 1 条（协商帧长到达对端 PeerView）；kotlin_guard 新增护栏 5（钉住 record 转换器逐字段写全）。desktop 54 + 前端 184、Android 257 单测全绿。

**遗留边界**：引擎级 `PcmMixer` 不支持同时混合不同帧长；现在会明确拒绝第二条异帧长流，不再静默输出半帧。Android 多对端时水位跟随首个给出协商值的 peer（语义未定义，单对端常态下无影响）。

## 接收抖动保护垫毫秒化与播放节拍精度（2026-09-22 修复）

10 ms 档此前名存实亡的两个根因与修复：

1. **接收侧抖动保护垫是帧数制**（起步 1 帧 / 中等 2 帧 / 上限 6 帧 / 全重排阈值 3 帧 / 重排容量 3 帧），10 ms 帧下墙钟余量全部砍半。现全部改为毫秒制（20/40/120/60/60 ms），运行时按帧长向上取整换算：20 ms 帧 = 1/2/6/3/3（标准档行为零变化），10 ms 帧 = 2/4/12/6/6。NACK「等重传」门同步从「≥ 2 帧」改为墙钟 ≥ 40 ms。
2. **播放节拍 `std::thread::sleep` 误差**：Windows 默认定时器粒度 15.6 ms，超过一整个 10 ms 帧。现播放线程以 `timeBeginPeriod(1)` 提粒度（guard 的 Drop 对称还原），并改为「睡到死线前 1 ms + 自旋校准」的混合等待（跨平台生效，Android 播放线程同一段代码）。

测试：jitter.rs 单测钉住两档换算表与 10 ms 行为（起步 2 帧 / 中等 4 帧 / 上限 12 帧 / 全重排 6 帧 / 容量 6 帧）。

## 验证

- 新增端到端测试 `engine/tests/engine/low_latency_profile.rs`（2 条：10ms 档 960 样本/帧 + plc_count==0 + 遥测 frame_ms==10；20ms 对照证明断言真在分辨档位）。
- 合并后全量：fmt ✅；audio 64 / ffi 28+3 / engine lib 190 / engine 集成 54 ✅（3 条既有 flaky 单独重跑即绿，均不含 codec 引用）；desktop lib 43 + 前端 180 ✅；Android compileDebugKotlin + 250 单测 ✅。
- 2026-09-23 已用静音手机与 PC 合成正弦源验证 10 ms 帧 + 960 帧目标的供帧连续性；真实系统音频采集、可听质量与声学端到端延迟仍未验证。

## 2026-09-23 欠载排查

- Android `CapturePump` 将“20 ms”误作 20 个采样帧，实际每次仅读 0.417 ms；`AudioRecordSupport` 的 80 ms 缓冲计算也把 20 ms 当成 20 个采样帧。现在每次拉取 480 帧（10 ms），申请容量按 960 帧 × 4（80 ms）计算；标准档也使用 10 ms 的采集块，由内核组帧器拼成 20 ms。
- `QueueWatermark.shouldWrite` 的门槛为 `target − chunk`。低延迟档原先 `target = chunk = 480`，所以设备队列约耗尽才开始补写，1 ms 轮询加线程调度抖动会使 AudioTrack 欠载。改成 `target = 960`，门槛 480，留一整块 10 ms 的输出余量；比标准档仍浅 10 ms。
- **电脑发送、手机接收的根因**：同一个 Android 引擎先收 10 ms 流、再收 20 ms 流时，接收线程虽按新帧长解码，但引擎级 `PcmMixer` 仍复用上一条流的 480 采样帧配置。每次送入 960 采样帧，只取出前 480 帧；手机播放环于是每秒收到约 0.5 秒音频、补约 0.5 秒静音。真机修复前的 12.24 秒窗口中，音频与静音各增长 288960 帧，供给欠载增长 602 次，系统欠载仍为 0。现在混音器在空闲且帧长变化时重建；有活跃源时拒绝异帧长并发，避免静默截断。
- 修复版重新编译两套 Android ABI 并安装到 PHK110，媒体音量全程为 0。按 10→20→10 ms 顺序用 PC 合成正弦源推流；20 ms 的 12.29 秒窗口音频增长 577920 帧，静音填充与供给欠载均为 +0；反向切回 10 ms 的 12.26 秒窗口同样音频增长 577920 帧、静音和供给欠载 +0，系统欠载均为 0。内核仍有少量迟到/欠载事件，且合成源不代表真实 WASAPI 采集或听感；不能据此声称端到端延迟达标。
