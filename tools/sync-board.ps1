<#
.SYNOPSIS
    把 AudioLink 的里程碑 / 任务同步到 GitHub Projects 看板（幂等：重复运行只补齐差异）。

.DESCRIPTION
    内容来源：`docs/05-roadmap.md`（里程碑与退出条件）、`docs/10-handoff.md`（当前进度）。
    **文档仍是唯一事实来源**，本脚本里的任务表只是它的机械映射 —— 改任务请先改文档，再同步这里。

    前置条件（一次性，交互式）：
        gh auth refresh -s project
    token 缺 `project` scope 时脚本会直接给出这条提示并退出。

.EXAMPLE
    pwsh tools/sync-board.ps1 -DryRun      # 只打印将要发生的变更
    pwsh tools/sync-board.ps1              # 真正同步（创建看板 / 字段 / 条目，并回填状态）
#>
[CmdletBinding()]
param(
    [string]$Owner = 'GotKiCry',
    [string]$Repo = 'GotKiCry/AudioLink',
    [string]$ProjectTitle = 'AudioLink 任务看板',
    [switch]$DryRun,
    [switch]$NoLink
)

$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# 任务表（docs/05-roadmap.md 的机械映射：M=里程碑，P=优先级，S=状态）
# 优先级含义：P0 = 当前关键路径，P1 = 本里程碑内，P2 = 近期技术债/护栏，P3 = 有余力再做
# ---------------------------------------------------------------------------
$Tasks = @(
    # ---- M0 地基（全部已完成）----
    @{ T = '[M0] CI 能构建内核（core / android / desktop / version-consistency）'; M = 'M0'; P = 'P1'; S = 'Done'; B = '.github/workflows/ci.yml：四 job 并行，单轮约 3–4 分钟' }
    @{ T = '[M0] audiolink-types：协议常量、枚举、错误码、遥测结构'; M = 'M0'; P = 'P1'; S = 'Done'; B = 'docs/03-protocol.md §3/§4.1/§10/§11；serde 为可选 feature（默认零依赖）' }
    @{ T = '[M0] audiolink-proto：ALP/2 编解码 + §12 三组 golden vectors'; M = 'M0'; P = 'P1'; S = 'Done'; B = '数据报 / 控制帧 / 发现报文；L1 严格解码层，非法帧一律 1008 且绝不 panic' }
    @{ T = '[M0] 规格修订：锁定 L1/L2 分层、载荷表、MTU 实测与拒绝矩阵'; M = 'M0'; P = 'P1'; S = 'Done'; B = '§1.1 分层、§3 定长载荷 + QUIC 1162 B 实测、§4 长度约束、§9.2 字段格式、§12 拒绝矩阵' }
    @{ T = '[M0] tools/alp2-dump：协议解码（hex → 人类可读）'; M = 'M0'; P = 'P1'; S = 'Done'; B = '失败时打印 L1 拒绝上下文；结构预筛避免噪声' }
    @{ T = '[M0] tools/latency-probe：QUIC 数据报 RTT / 时钟偏移测量'; M = 'M0'; P = 'P1'; S = 'Done'; B = 'listen/probe；输出 RTT 分位、抖动代理、§6 best8 偏移与极差、§6.5 质量分级' }

    # ---- M1 单链路（关键路径）----
    # 注：标题里的「10–20 ms 缓冲」是当初的规划口径；实测共享模式缓冲下限是 22 ms（1056 帧），
    #     能压到 10 ms 的是引擎周期 —— 为保持看板条目稳定（标题是幂等键），标题不改，口径修正记在 body 与文档里。
    @{ T = '[M1] WASAPI loopback 事件驱动采集（48 kHz / f32 / 2ch，10–20 ms 缓冲）'; M = 'M1'; P = 'P0'; S = 'Done'; B = 'audiolink-audio::wasapi（LoopbackCapture/RenderSink）。实测：缓冲下限 1056 帧=22 ms、引擎周期 10 ms（每周期 480 帧）、空闲端点零数据、COM 对象 !Send 需线程内构造；自环实测零欠载' }
    @{ T = '[M1] Opus 编码（opus-rs，20 ms / 160 kbps / VBR / 48 kHz 锁定）'; M = 'M1'; P = 'P0'; S = 'Done'; B = 'ADR-003：纯 Rust，不装 CMake；实测 encode P50 75 μs / decode 57 μs、160.4 kbps、热路径零分配；已知缺陷：CELT-only 无真 PLC 且 packet_loss_perc 为空设置（见 ADR-003 注记）' }
    @{ T = '[M1] 桌面自环链路 + 四段延迟分解（采集/编码/解码/播放）'; M = 'M1'; P = 'P0'; S = 'Done'; B = 'tools/self-loop：五段分解 + 19 kHz 标记往返实测（同口径模型 7.00 ms vs 实测 6.78 ms）；本机实测合计 39.7 ms，999 帧零欠载零丢弃' }
    @{ T = '[M1] QUIC 通道（quinn：控制流 #0 + 音频数据报）'; M = 'M1'; P = 'P1'; S = 'Done'; B = 'audiolink-net：端点/连接/控制流 #0/时钟估计。实测握手时 max_datagram_size=1162 B，但 DPLPMTUD 会话开始后抬到 1288 B → 发送侧按 min(1200, mds) 封顶（§3 的 1200 B 预算**真的是有效约束**）。未知 OpCode 走 Ok(op=None) 交 L2 忽略计数（§1.1）；peer_cert_der() 供 §5 验签；双向 TLS 让接收侧也能算 peer_id()。19 单测 + 5 真 QUIC 集成测试全绿' }
    @{ T = '[M1] Android 播放（AudioTrack 低延迟模式 + JNI 环缓冲 + 欠载检测）'; M = 'M1'; P = 'P1'; S = 'Done'; B = 'LowLatencyPlayer（PERFORMANCE_MODE_LOW_LATENCY + WRITE_NON_BLOCKING + USAGE_MEDIA + 48k/2ch/FLOAT，播放线程内 build/write/release，§1 第 3 条未反转）+ 纯逻辑播放环 + 欠载/溢出计数；assembleDebug 与 testDebugUnitTest 通过（31 个 JVM 用例）。⚠️ **真机指标全部未验证**（本机 adb 无设备、无可用 AVD）：getPerformanceMode() 实测值、出声延迟、30 min 无断流。顺带修掉一个 minSdk 26 上调用 API 29+ 三参 startForeground 的既有崩溃缺陷' }
    @{ T = '[M1] 最小 UI（桌面手工 IP 连接；Android 服务启停）'; M = 'M1'; P = 'P1'; S = 'Done'; B = 'FR-29 / FR-35 的最小形态。桌面：手工 IP → 连接 → 对端卡片（名字/短指纹/状态/是否受信）→ 开始·停止推流 → 遥测数字面板 → PIN 输入框；外壳已**接真实引擎**（无 mock），含一个起两个真 Engine 的接缝测试（真 QUIC + 真配对）。Android：服务启停 + 状态卡片（低延迟是否生效 / 采样率 / 缓冲帧数 / 欠载）。⚠️ **UI 交互与视觉未验证**（无头环境点不了，应用从未启动过）' }
    @{ T = '[M1] PIN 配对最小可用（白名单落盘）'; M = 'M1'; P = 'P1'; S = 'Done'; B = 'audiolink-identity：自签证书（SAN 固定 "audiolink"，展示名不进证书 → 改名不换身份）+ SHA-256 指纹 + ECDSA-P256 挑战应答 + 信任库原子落盘（损坏必须 Err，绝不静默重置）+ PinGate（6 位 / 60 s / 5 次锁 5 分钟 / 成功后不可重放）。30 测试全绿。已在 link-loop 里跑通真实 PIN 配对（node-b 显示 308011 → node-a 提交 → 双方落盘）' }
    @{ T = '[M1] 引擎编排：会话状态机 + 收发管线 + 遥测聚合'; M = 'M1'; P = 'P1'; S = 'Done'; B = 'audiolink-engine：§4 控制帧载荷（postcard）+ L2 分发层（未知/未实现/载荷非法三分支分别计数，一律不断流）+ 会话状态机（非法迁移显式报错，Failed 可重启）+ 采集线程（拥有 !Send 的 CaptureSource）→ 编码 → 数据报 + 数据报 → 解码 → 时钟驱动播放线程 + 1 Hz 遥测。60 测试全绿' }
    @{ T = '[M1] 验收：P50 ≤ 110 ms / P95 ≤ 150 ms、零重采样、低延迟模式生效'; M = 'M1'; P = 'P1'; S = 'In Progress'; B = '2026-09-16 更新：PHK110 / Android 16 新包已安装并回拉核对；LOW_LATENCY、48 kHz / 2ch、PCM_FLOAT 与 FAST 路径生效，PIN 与服务生命周期验收通过。待播队列增长已修复：180 s P50/P95/max 从 240/300/320 ms 降至 40/40/60 ms，首尾 40→40 ms；手机 PCM 环约 105 s 无新增溢出、读空、系统欠载或写错。当前已有段模型 e2e 下限约 73 ms，但对端解码、AudioTrack 输出及声学端到端 P50/P95 尚未完整测量；修复后 30 min 长跑也未执行，因此保持 In Progress。迟到丢弃 143 次转由 M2 自适应抖动缓冲/PLC 优化。详见 docs/16-android-service-lifecycle.md；历史 MI 8 Lite 验收见 docs/12-m1-device-acceptance.md。' }
    @{ T = '[工具] link-loop：PC↔PC 真 QUIC 端到端验收'; M = 'M1'; P = 'P1'; S = 'Done'; B = 'tools/link-loop：同进程起两个真实 Engine，走真实 QUIC（TLS + 证书指纹互认 + §5 握手 + PIN 配对），用共享 MeasurementTap 按 seq 配对量「帧封口→sink 写出」。与 self-loop 的分工：self-loop 量单进程音频链，link-loop 量含网络与会话的完整链路' }
    @{ T = '[M1] 真机验收：Android 低延迟/出声延迟/30 min 无断流'; M = 'M1'; P = 'P1'; S = 'In Progress'; B = '2026-09-16 PHK110 / API 36：最新双 ABI Release 已安装、V2 签名及回拉 SHA-256 一致；LOW_LATENCY、48 kHz / 2ch、PCM_FLOAT 与 FAST 生效，PIN、6 轮普通启停、3 轮四连点、推流中和配对中重启通过。待播队列修复后 180 s 全程 streaming，P50/P95/max 40/40/60 ms、首尾均 40 ms，链路丢包 0%；手机 PCM 环约 105 s 无新增溢出/读空/系统欠载/写错。仍缺修复后 30 min 长跑和声学出声延迟 P50/P95，保持 In Progress。详见 docs/16-android-service-lifecycle.md；历史 MI 8 Lite 结果见 docs/12-m1-device-acceptance.md。' }
    @{ T = '[M1] 桌面外壳接入真实 engine（替换 mock）'; M = 'M1'; P = 'P1'; S = 'Done'; B = 'MockEngine / run_mock_source 已整块删除，engine_bridge.rs 现为真实实现（Engine::start + spawn_accept_loop + subscribe → 三事件翻译 + 状态映射 + 遥测聚合）。**含一个起两个真 Engine 的运行时接缝测试**（真 QUIC + 真 PIN 配对，断言 connect 返回 1002 但对端已进 peers()、submit_pin 结果走事件回来、start_send 后接收端出现非零码率）。⚠️ 真实 WASAPI 采集/播放工厂按签名写好但**未启动过应用**（会真采集真放音）；UI 交互与视觉未验证' }
    @{ T = '[M1] Android 接入 FFI 绑定（JNA + 播放环适配 + 引擎随服务启停）'; M = 'M1'; P = 'P1'; S = 'Done'; B = '绑定源集**挂载而非复制**（../core/crates/audiolink-ffi/bindings/kotlin，保证绑定与 .so 永远同源）+ jna 5.19.1@aar；FfiPcmFeed 把 PcmFeed → PcmRingBuffer（环满时把 accepted<frames 上报给内核遥测）；engineStart/engineStop 跟随服务生命周期（listenPort=0 用内核默认端口，不在 Kotlin 侧硬编码）；catch(Throwable) 接住 JNA 的 UnsatisfiedLinkError 以免服务进程崩。验收：assembleDebug + testDebugUnitTest **38/38** + lintDebug 0 error，APK 22.7 MB。⚠️ JNA 运行期加载 .so、端到端 PCM 链路、GC 抖动均**未验证**（无真机）' }

    # ---- 本轮（M1 真机验收）新增与重写 ----
    @{ T = '[M1] §6 时钟同步接线进引擎（CLOCK_PROBE/REPLY + ClockEstimator + 对端遥测）'; M = 'M1'; P = 'P0'; S = 'Done'; B = 'audiolink-engine::clock：响应方立即回 CLOCK_REPLY（不依赖 §5 会话状态，latency-probe 这类裸测量客户端也能用）、发起方 100ms×50→1Hz 探测、估计写遥测；新增 clock_estimate()/peer_stats()/clock_probe_stats()（含逐探针 RTT 环 P50/P95）。真机实测 60/60 样本（旧内核 0 样本）、收敛 1.01 s、|offset| 38 µs（同进程真值 0）' }
    @{ T = '[M1] tools/device-link：PC 侧真机验收（连接 / PIN / 推流 / 跨机账本 / JSON）'; M = 'M1'; P = 'P0'; S = 'Done'; B = '真实 Engine + 真实 QUIC + §5 配对 + 可选真实 WASAPI loopback 采集；账本逐项标 [实测]/[模型]/[未测] 并给未测项清单；--pin-file 支持无头投喂（内核每连接新建 PinGate，必须同连接内提交）；--self-test 同进程双 Engine 自检；退出码 0/2/3/4/5 分门别类。真机实测四轮 60/60/90/75 s 推流零断流' }
    @{ T = '[M1] Android UI 显示配对 PIN + 对端列表'; M = 'M1'; P = 'P0'; S = 'Done'; B = '真机验收暴露的真缺口：FFI 早已导出 displayedPin()，但 Kotlin 从未调用 → 手机从不显示 PIN，§5 配对在 Android 侧根本走不通。已修（56sp 大字卡片 + 对端名/短指纹/受信/状态），真机验证；新增 17 个 JVM 用例' }
    @{ T = '[M1] 修复配对死线：握手 10 s < PIN 有效期 60 s（人工配对必然失败）'; M = 'M1'; P = 'P0'; S = 'Done'; B = '真机首跑报 1002 NOT_PAIRED（unknown peer）：握手死线 10 s 到点回收会话，而 §5 的 PIN 有效期 60 s。改为配对等待不吃死线（默认 10 s / 75 s 窗口，权威判据仍在 PinGate）；含负向对照（arm_pin_wait 改空操作 → 用例立刻变红）；真机延迟 ≥25 s 提交 PIN 配对通过' }
    @{ T = '[M1] Android 播放队列水位控制 + 设备侧缓冲 A/B'; M = 'M1'; P = 'P1'; S = 'Done'; B = '同连接内 A/B：满灌 101 ms / 队列目标 30 ms→51 ms（零欠载代价）/ 容量收缩 960r→41 ms，全部保住 FAST。结论：设备侧省的 50–66 ms 被上游推送速率吃掉（见下一条）。默认保持满灌，档位留 UI chip（改一行常量即可启用）' }
    @{ T = '[M1→M2] 内核→Kotlin PCM 推送 ≈ 2× 实时（环持续溢出 48.6k 帧/s）'; M = 'M1'; P = 'P0'; S = 'Done'; B = '2026-09-16：Issue #1 的代码修复已完成，用户已确认新 APK 音频正常传输和播放，Issue #1 已关闭。receive_audio 把封装已返回的交错样本数再次乘 2，导致等长静音尾部；现直接截取有效长度，解码缓冲收缩为单包。新增真实 Engine→QUIC→Opus→sink 回归：20 ms=1920 样本、10 ms=960 样本，旧代码均失败、修复后通过；fmt/clippy/内核完整测试通过。本机 60 s 推流欠载/迟到/PLC=0，队列末值40 ms，帧封口→sink P50/P95=40.28/40.73 ms；不能替代手机端到端验收。该修复标为 Done；长时间队列稳定性与端到端延迟仍由独立验收项追踪。buffer_level_us 已明确只含引擎待播队列，不含 Kotlin 环/AudioTrack。历史真机：78 s 环溢出 +3 791 040 帧、读空 +0。复测步骤见 docs/12-m1-device-acceptance.md §9。https://github.com/GotKiCry/AudioLink/issues/1' }
    @{ T = '[M1] 真机验收报告 + 独立复核（41 项三态判定）'; M = 'M1'; P = 'P0'; S = 'Done'; B = 'docs/12-m1-device-acceptance.md：设备侧两项硬指标达标（LOW_LATENCY / 48 kHz 零重采样）、四轮零断流；e2e 延迟未达标（下限 ≥115 ms + 设备输出 80–101 ms）；独立验证者复核 41 项 → 可信 29 / 存疑 8 / 不成立 4，4 项已修正并留痕（含 run4 欠载被写成更漂亮的值）' }

    # ---- 2026-09-15 深夜：发行构建 + 真机联调新发现 ----
    @{ T = '[M1] 发行构建：release APK 首次打通（签名 keystore + R8 规则 + gitignore 修口）'; M = 'M1'; P = 'P1'; S = 'Done'; B = '此前 release 路径从未跑过：proguard-rules.pro 根本不存在（R8 会把 JNA/UniFFI 反射目标删掉）。已补 JNA/UniFFI keep 规则 + 生成 release keystore（RSA4096/30 年）+ .gitignore 加 android/keystore.properties（实测原本没被忽略，PUBLIC 仓库存在口令入库风险）。产物 app-release.apk 7.98 MB 已签名（V2）。⚠️ MIUI 上 adb install 报 -99（需开发者选项「USB 安装」）' }
    @{ T = '[M1 听感缺陷] 接收侧待播队列持续增长，触顶后丢帧'; M = 'M1'; P = 'P0'; S = 'Done'; B = '2026-09-16 已修复并完成 PHK110 / API 36 真机复测。根因是欠载补静音或播放线程跨过时钟拍后没有推进音频序号，迟到旧帧仍在下一拍播放，短暂停顿会永久增加队列延迟。现以 expected_seq 作为播放时钟游标：静音和错过的拍也推进，迟到帧丢弃，未来帧保留，并支持 u32 回绕。真实 QUIC/Opus 回归注入 250 ms sink 停顿：旧代码恢复后 P50 271 ms，修复后连续 5 次 P95 20.6–40.7 ms。手机 180 s 队列从修复前 P50/P95/max 240/300/320 ms、首尾 40→300 ms，降至 40/40/60 ms、首尾 40→40 ms；链路丢包 0%，设备播放环约 105 s 内无新增溢出/读空/系统欠载/写错。欠载/迟到 0→143 是主动丢弃过期帧的明确账本，减少该计数转入 M2 自适应抖动缓冲与 PLC；M1 总延迟仍独立验收。Rust fmt/Clippy/全测试通过；详见 docs/16-android-service-lifecycle.md。' }
    @{ T = '[M1] 桌面 UI 缺采集端点选择器（只能吃默认输出端点）'; M = 'M1'; P = 'P1'; S = 'Done'; B = '2026-09-16：实现完成。新增真实 WASAPI 输出端点下拉、刷新、格式与不可用原因、虚拟设备提示、实际采集端点展示和启动日志；完整 ID 精确选择，缺失端点不回退默认。推流期间先停止再切换。Engine::start_send 现等待本地采集/编码初始化并回传失败，支持同连接重试、拒绝重复开流。完整内核/桌面测试与 Clippy 通过；本机 4 个真实端点选择 ID 与实际打开 ID 一致，缺失 ID 拒绝；Tauri 窗口验证选择与刷新，1100×720/880×560/深色截图通过。测试版 target/x86_64-pc-windows-msvc/debug/audiolink-desktop.exe；说明 docs/13-desktop-capture-selection.md。' }
    @{ T = '[M1] Android PIN 卡有时不显示（已进入配对分支但屏幕无 PIN 卡）'; M = 'M1'; P = 'P1'; S = 'Done'; B = '2026-09-16：完成。Engine 同步快照修复已通过自动回归；PHK110 / Android 16 / API 36 真机验证屏幕 PIN、错一次后保留、成功清除、未配对断开清除、同身份重连、60 s 到期、五次输错关闭连接，以及配对中停止/重启。UI XML、截图与真实 QUIC 客户端日志共同核对；新包安装后回拉比对 SHA-256 一致。Android 全部 73 项 JVM 测试通过，详见 docs/14-pairing-state.md、docs/16-android-service-lifecycle.md。当前 Release：android/app/build/phk110-validation/outputs/apk/release/app-release.apk。' }
    @{ T = '[M1] 引擎停止后立即重启：QUIC 端口尚未释放'; M = 'M1'; P = 'P1'; S = 'Done'; B = '2026-09-16：已修复。原 PIN 回归恢复同端口可稳定复现 Windows 10048，修复后通过。Engine 禁止新任务并等待接受/主动连接/会话及音频线程退出；net 等待真实 UDP 套接字及 IO poller 释放通知；FFI 启停串行，同配置幂等、不同配置 BUSY。等待锁时取消不执行，派发后取消等待仍完成操作。新增 5 项回归覆盖 15 次五状态同端口重启（空闲/等 PIN/已连接/接收/发送）、并发冷启动与重复停止、播放回调阻塞后取消停止再重启、未完成 QUIC 握手、socket/poller 持有与取消恢复。fmt、Clippy 与内核/桌面测试通过；双 ABI Debug/Release APK 构建并核对内核，Release V2 签名通过；未改动的 Kotlin 69 项 JVM 测试使用缓存通过。新 Release：android/app/build/restart-validation/outputs/apk/release/app-release.apk。详见 docs/15-engine-restart.md；本轮 ADB 无设备，手机界面复测仍留在真机验收项。' }
    @{ T = '[M1] Android 停止服务后旧协程回写运行中，无法再次启动'; M = 'M1'; P = 'P1'; S = 'Done'; B = '2026-09-16：已修复并完成 PHK110 / Android 16 真机验收。旧 Service 销毁后 stopEngine 协程会回写运行中，导致按钮无法再次启动；现用进程级请求队列保证旧停止先于新启动，按实例代次丢弃旧启动/PIN 查询结果，仅主线程发布 UI。新增 4 项生命周期回归，Android 73 项 JVM 测试、Debug/Release 构建、Release V2 签名及 Rust fmt/Clippy/内核测试通过。手机 6 轮普通启停、3 轮四连点、推流中与配对中重启通过；安装包回拉 SHA-256 与本地一致。产物 android/app/build/phk110-validation/outputs/apk/release/app-release.apk；详见 docs/16-android-service-lifecycle.md。' }

    @{ T = '[M1 工具] 延迟探针并发写入丢失时间戳配对'; M = 'M1'; P = 'P1'; S = 'Done'; B = '2026-09-16：已修复 MeasurementTap 并发竞态。CI 35056343048 的既有并发回归只配对 1992/2000；逐序号同步起跑强化后旧代码为 645/2000。现由同一状态锁原子完成查找、配对、留存、样本更新与清空，修复后配对 2000/2000、孤儿零、全部延迟 25 ms；测量模块 8 项测试、Rust fmt、全内核 Clippy 与测试通过。保持有界窗口及公开 API；Android 默认音频路径未启用此探针，接收队列增长仍独立跟踪。详见 docs/16-android-service-lifecycle.md 的 CI 后续记录。' }
    @{ T = '[环境] MIUI 上 adb install release APK 失败 -99（需「USB 安装」权限）'; M = 'M1'; P = 'P2'; S = 'Todo'; B = 'debug 包可装（debuggable），release 包在 MIUI 上被拒：Failure [-99]。需在开发者选项打开「USB 安装」（可能要求登录小米账号）。属环境/文档项，代码侧无问题（APK 签名校验通过、清单 minSdk26/targetSdk36 正常）' }
    # ---- M2–M5 ----
    @{ T = '[M2] 稳定性与质量：抖动缓冲 + 双发/PLC/NACK + 自适应码率 + 遥测面板'; M = 'M2'; P = 'P2'; S = 'In Progress'; B = 'docs/05-roadmap.md M2；验收含 8 h soak 与弱网（5 Mbps / 2% 丢包 / 30 ms 抖动）。2026-09-16：播放游标、PCM 丢包掩盖、自适应抖动/有界重排和默认冗余双发已完成。副本延迟一帧，接收按序号去重且质量指标只取首个有效副本；最高 60 ms 档欠载可同档补水。PC 双 Engine 与 PHK110 真实 QUIC 码率均稳定 320.8 kbps；手机 90 s 队列 P50/P95/max 60 ms，链路丢包/PLC 0，欠载/迟到 24/22 且第 75–90 s 无新增，AudioTrack underrun 0、FastMixer writeErrors 0。NACK 重传、soak-runner、自适应码率与弱网测试台均已落地（docs/21、docs/22、docs/23、docs/24）。M2 弱网口径已实测通过（60 s：2% 丢包 / 15±15 ms / 5 Mbps → exit 0、掩盖 0.13%、全程 streaming）。2026-09-16 遥测面板（5 分钟曲线 + CSV 日志导出）已完成（提交 87cb059 / CI 35079292910 / docs/25）。下一步只剩：**真正跑一次 8 h soak**（工具就绪）、真机弱网与更高丢包档（触发降级）；单声道降级与帧长 10 ms 切换已在 LinkSeverity 记账、留待专门任务。详见 docs/17-m2-pcm-concealment.md、docs/18-m2-adaptive-jitter.md、docs/19-m2-redundant-send.md。2026-09-16：8 h 弱网长跑（2% 丢包 / 15±15 ms / 5 Mbps，--tolerant 判据）已在后台启动，目标 28800 s，报告与注入自账落到 target/evidence/soak/soak-8h-netem.json（+ .netem.json）。长跑期间本机同时有开发编译与测试活动，结论出来时会把这一点一起记下来（不粉饰）。' }
    @{ T = '[M2] 自适应抖动缓冲（20–60 ms）+ 有界乱序重排'; M = 'M2'; P = 'P1'; S = 'Done'; B = '2026-09-16 完成。接收侧以 1 Hz 到达抖动 P95 和新增欠载驱动 20/40/60 ms 三档目标：升档立即，连续稳定 30 s 后逐档下降；播放线程按实际水位缺口执行最多 60 ms 的有界重缓冲，降档只丢真实多余深度，且原子发布不会覆盖并发欠载升档。仅 60 ms 档允许等待一帧修复乱序，待排序包上限 3；超时转 PCM 掩盖，覆盖重复、迟到、回绕和大跳变。后续双发真机回归补齐最高档欠载后的同档补水：队列重新出现数据才建立余量，完全断流时继续推进时间轴。真实 Opus、250 ms sink 停顿和 PHK110 90 s 回归通过；最终手机队列 P50/P95/max 60 ms，欠载/迟到 24/22 且第 75–90 s 无新增，AudioTrack underrun 0。真实 2%/30 ms 弱网及 8 h soak 留在 M2 父任务。详见 docs/18-m2-adaptive-jitter.md、docs/19-m2-redundant-send.md。' }
    @{ T = '[M2] 默认冗余双发（延迟一帧副本 + 接收去重）'; M = 'M2'; P = 'P1'; S = 'Done'; B = '2026-09-16 完成。发送当前主包后再发上一帧副本，同序号副本间隔至少一个 20 ms 包时距；副本保持 AUDIO 并标记 FEC_REDUNDANT。接收乱序窗按近期已交付和待排序序号静默去重，主/副本谁先到采用谁；丢包分母、PLC、迟到与 jitter 只按唯一序号，链路码率按两个数据报。真实 Opus 注入丢主包验证副本恢复且 0 loss/PLC/late；PC 双 Engine 20 s 码率 320.8 kbps、零欠载/迟到。PHK110 90 s 码率末值/均值 320.8 kbps，链路丢包/PLC 0，队列 P50/P95/max 60 ms，欠载/迟到 24/22 且第 75–90 s 无新增；APK SHA-256 4ef38c519454ba427ae69aa6c9341ea5665c5b97454da31e09dafb05f9e66140。下一步 NACK；真实弱网仍在 M2 父任务。详见 docs/19-m2-redundant-send.md。' }
    @{ T = '[M2] 自建丢包掩盖（CELT-only 无真 PLC：重复上一包 + 淡出 + 交叉淡化）'; M = 'M2'; P = 'P1'; S = 'Done'; B = '2026-09-16 完成。新增 PcmConcealer：重复上一帧、120 ms 线性淡出、恢复时 2.5 ms 交叉淡化；AudioReceiver 将 Opus、掩盖历史与序号统一推进，迟到包解码前丢弃，解码失败同样掩盖，长空洞最多补 120 ms 并重建解码器，避免塞满播放队列。opus-rs 无逐包模式 getter，所有档位保守跳过未经证明的原生 conceal；实测连续调用原生 conceal 会把 6 帧丢包后的恢复 RMS 压到约 0.002。真实 Opus 回归覆盖约 2% 确定性丢包（4/201，无序号洞且掩盖 RMS > 0.02）、120 ms 突发、u32 回绕、解码失败、迟到包和 999 帧空洞。PHK110 正常链路 60 s 队列 P50/P95/max 40/40/40 ms、链路丢包/PLC 0，APK SHA-256 b51180b4bb208ae273d4eb2bccb66e6728375fbafffa219c31dd13b2b05da2e7；真实网络 2%/30 ms 弱网仍属 M2 父任务。详见 docs/17-m2-pcm-concealment.md。' }
    @{ T = '[M2] NACK 重传（洞检测 + 1 s 重传窗口 + 有界等重传）'; M = 'M2'; P = 'P1'; S = 'Done'; B = '2026-09-16 完成。新增 audiolink-engine::runtime::nack：接收侧 MissingTracker 从序号洞里认出缺哪几帧（最旧的洞先请求、重试 ≤ 5 次、间隔 10 ms、窗口 1 s、跳变 ≥ 1000 帧按时间轴重建、同时跟踪 ≤ 64 个洞并计 overflow、序号回绕按半程判据），按 §3 的 16 项上限组 NACK；仅 QUIC 平滑 RTT < 30 ms 时发送，每次发一条记一次 nack_count。发送侧 RetransmitBuffer 保留最近 1 s 已发出的帧（同序号只留一份、容量 128 帧），收到 NACK 逐序号重发，序号与 sample_index 原样保留、且不计入发送侧 expected/received 分母（否则丢包率会被自己的重传擦干净）。关键改动：PacketReorderBuffer 新增 set_retransmit_grace —— 有洞且播放队列深度 ≥ 2 帧时才给有界等待 min(2×RTT+5 ms, 30 ms)，到点由 flush_expired 按原策略交付；无洞链路零额外延迟。验证：nack 单测 11 项 + jitter 等重传窗口 4 项 + 端到端 1 项（真实双 Engine + QUIC + 有损 UDP 中继，每 10 个大包丢连续 2 个 ≈ 20%）：接收侧 nack_count=16、plc_count=0（解码序列一个洞都没有）；破坏性对照 NACK_MAX_RTT_US=0 → nack_count=0、plc_count=16（= 丢包数）。已知取舍：每次重传仍伴随一次播放欠载（≈20 ms 静音，等重传期间播放队列只出不进），听感收益的最后一公里需与抖动缓冲联动，留 M2 后续。提交 2eaff74；CI run 35071772598（core / android / desktop / version-consistency）全绿。详见 docs/21-m2-nack-retransmit.md。' }
    @{ T = '[M2] 自适应码率（丢包驱动升降级 + 遥测时间线）'; M = 'M2'; P = 'P1'; S = 'Done'; B = '2026-09-16 完成。新增 audiolink-engine::adaptive（纯状态机，8 项单测）：丢包 > 1% 持续 3 s → 码率 −20%（下限 96 kbps，降完重新攒窗口，故持续丢包是每 3 s 一级）；连续 10 s 无丢包且 RTT ≤ 30 ms → +10%（上限 320 kbps），每 5 s 最多一级；初值夹到 96..=320 kbps。发送侧接线：1 Hz ticker 用**对端** STREAM_STATS 的 loss_pct_x100 / rtt_us 判定（丢包只有接收侧看得见），结果写进采集线程共享的 Arc<AtomicI32>，采集线程每帧前 load，变化即 encoder.set_bitrate（opus-rs 的 bitrate_bps 每帧读取，下一帧生效、不重建编码器）；变更通过新增 EngineEvent::CodecAdapted{from_bps,to_bps,reason} 形成验收时间线（桌面侧已同步处理该变体，目前只记日志）。端到端（真实双 Engine + QUIC，从码率下限起步）：事件 [(96000→105600), (105600→116160)]，接收侧链路码率 192.8→233.2 kbps（双发两倍）—— 决策真的落到线上。破坏性对照：去掉写新码率给采集线程 → 事件照发但线上码率不变（192800→192800），测试变红。未做（已记账、不假装）：立体声→单声道（要动 format::CHANNELS）、帧长 20→10 ms（OPEN_STREAM 已协商参数）、遥测面板曲线、真实弱网下的降级实测。重要发现：回环上「降级」不可复现 —— 40% 人为丢包被双发 + NACK 全部修复，接收侧丢包率恒为 0，而「修好了就不该降码率」是正确行为。提交 f66a551；CI run 35074883157（core / android / desktop / version-consistency）全绿。详见 docs/23-m2-adaptive-bitrate.md。' }
    @{ T = '[M2] 弱网测试台 netem-sim + 弱网长跑验收'; M = 'M2'; P = 'P1'; S = 'Done'; B = '2026-09-16 完成（路线图 M2 交付物 6）。新增 audiolink-tools::netem：确定性注入内核（xorshift64* 同种子可复现；按概率丢包、令牌桶限速、固定延迟 ± 抖动；只丢不改）7 项单测；NetemRelay 透明 UDP 中继（按来源地址区分客户端/上游，QUIC 端到端加密故只搬字节）。新增 --bin netem-sim（独立工具，打印监听地址供发起方连接，结束写 JSON 自账）。soak-runner 集成 --netem-*（两个 Engine 之间自动插中继，长跑与注入一条命令）与 --tolerant 弱网档（只钉会话不断 + 掩盖比例 ≤ 1%，不判欠载/迟到/NACK/瞬时丢包 —— 瞬时 loss 在 2% 注入下会跳到 2-4% 而窗口平均掩盖仅 0.13%，拿它当门槛只会假警报）。实测 M2 弱网口径 60 s（2% 丢包 / 15±15 ms / 5 Mbps）：全程 streaming、接收侧丢包 0、掩盖 4 帧（0.13%）、欠载 10、迟到 12、NACK 0、RTT≈41 ms → 判定 ok / exit 0；注入自账 观察 5676 / 转发 5571 / 丢 105（1.85%）。顺带修真实缺陷：SoakThresholds 的四个 max_* 字段此前定义了却未被判定逻辑使用（现在 check_delta 真按「累计值超过阈值的增量」判，默认 0 时行为不变）。结论：弱网下不该降码率（链路被修复=链路没坏），触发降级需更高丢包档，记为后续。提交 556e504；CI run 35077260006（core / android / desktop / version-consistency）全绿。详见 docs/24-m2-netem-sim.md。' }
    @{ T = '[M2] 遥测面板（5 分钟曲线 + CSV 日志导出）'; M = 'M2'; P = 'P1'; S = 'Done'; B = '2026-09-16 完成（路线图 M2 交付物「遥测面板：曲线 + P50/P95 + 日志导出」）。TelemetryRow 是独立的持久化契约（10 列 camelCase，刻意不复用界面字段名，避免界面改名污染用户已导出的文件）；render_telemetry_csv 纯函数（表头 + 一行一采样点、换行收尾、小数位固定）；write_telemetry_csv 落在 <用户目录>\AudioLink\telemetry-<毫秒>.csv（不弹文件对话框，无头/CI 可跑），export_telemetry 命令把路径回给 UI。前端 useAudioLink 维护 500 ms 一采样点、上限 600（5 分钟）的环形历史；TelemetryPanel 手绘 4 条 SVG 折线（端到端延迟 / 缓冲水位 / 码率 / 丢包率，纵轴按窗口最大值归一化，non-scaling-stroke 防变形）+「导出 CSV（N 点）」按钮。前后端各有一份镜像测试（Rust from_view / TS telemetryRowOf），字段漂移编译期即红。质量门：内核 fmt / clippy -D warnings / 全量测试、桌面 clippy / 15 项测试、pnpm build（tsc 严格 + vite）全绿。未做：真实窗口的曲线观感与点击导出落盘的人工复核（本机无头），同 docs/13 口径。提交 87cb059；CI run 35079292910（core / android / desktop / version-consistency）全绿。详见 docs/25-m2-telemetry-panel.md。' }
    @{ T = '[M3] 多设备与同步：时钟同步全流程 + 预约播放 + 同步组 + sync-measure'; M = 'M3'; P = 'P2'; S = 'Todo'; B = '组内 ±10 ms（P95）；§6 的 200 样本窗口 + 回归漂移估计在此落地。2026-09-16：交付物 5（双机录音对齐工具 tools/sync-measure）已完成 —— M3 的「量」这一半就绪（详见 docs/27-m3-sync-measure.md）。剩余交付物：多会话管理（并行推 ≥8 台）、临时同步组（勾选成组 / 动态加入退出 / 成员质量展示），以及真机双机同期录音验收（±10 ms P95）。2026-09-16：交付物 3（预约播放：接收端按 epoch 排播）已完成 —— 新增 audiolink-engine::epoch（§7 换算 + 等待/播放/丢弃三分支，8 项单测）、播放线程接线（未到目标补静音等待且不推进游标、过期 20 ms 丢弃并计 late_drops，只在显式设置排播时介入）、首个数据报的 seq → sample_index 换算基准、Engine::schedule_playout API 与 PlayoutScheduled 时间线；3 项播放面测试 + 引擎 133 项单测全绿。未接 GROUP_EPOCH 载荷与真机双机验证（见 docs/28）。2026-09-16 续：0x43 协议通道（docs/29）与组管理三帧 + 成员账本（docs/30）也已完成，2026-09-16 再续：多会话的共享采集枢纽与 start_send_many 已完成（docs/33），每会话音量（SET_GAIN 接线）与真机三台同时出声验收（±10 ms P95）是 M3 剩余项。' }
    @{ T = '[M3] 预约播放：接收端按 epoch 排播（§7 目标时刻 + 迟到丢弃）'; M = 'M3'; P = 'P2'; S = 'Done'; B = '2026-09-16 完成（路线图 M3 交付物 3）。新增 audiolink-engine::epoch（纯逻辑）：EpochSchedule + local_target_us（local(epoch) = epoch_local_us − offset，offset = 对端 − 本机）+ PlayoutAction 三分支（未到目标等待 / 到达 20 ms 门限内播放 / 超过则丢弃）；8 项单测把符号、lead、边界与 u32::MAX 样本序号都钉住。播放线程接线：有排播时未到目标就补静音等待且**不推进播放游标**（那帧还要在目标时刻播），超过目标 20 ms 的帧丢弃并计入 late_drops，首次排播发一条 EngineEvent::PlayoutScheduled 作为验收时间线；没设置排播时完全走 M1/M2 的本地游标，老行为一个字没改。换算基准取自本流首个数据报的 (seq, sample_index)，之后按 48 kHz 帧长推算。控制面新增 Engine::schedule_playout(peer, Option<EpochSchedule>) → SessionCommand::SchedulePlayout。3 项播放面测试 + 引擎 133 项单测全绿。未做：§7 的 buffer_ms/dac_latency_ms 补偿项、GROUP_EPOCH 载荷接线（目前靠上层显式调用）、真机双机起播对齐验证。详见 docs/28-m3-scheduled-playout.md。提交 20218cb；CI run 35085530753 四个 job 全绿。' }
    @{ T = '[M3] 组控制帧（GROUP_CREATE / JOIN / LEAVE / EPOCH）载荷与接线'; M = 'M3'; P = 'P2'; S = 'Done'; B = '2026-09-16 完成（四帧全部接线：0x43 见 docs/29，三帧见 docs/30）。新增 GroupCreatePayload{group_id, epoch_id, epoch_local_us, lead_ms, members}、GroupJoinPayload{group_id, member}、GroupLeavePayload{group_id, member}（postcard，成员身份用 NodeId 指纹）；dispatch 把三帧从「本里程碑不实现」计数里移出并补上编解码映射与样本（覆盖度测试同步更新，未实现计数只剩 SET_VOLUME_LOCK / TELEMETRY_PUSH）；引擎侧新增组账本（group_id → epoch/lead_ms/成员 BTreeSet）与 API：create_group(成员, lead_ms) -> group_id、join_group、leave_group、groups() -> 快照，以及 EngineEvent::GroupUpdated；接收侧收到 GROUP_CREATE 就用本机时钟偏移换算并排播（与 0x43 同一条路径）。动态加入语义：新成员除 JOIN 外还会补一条 GROUP_EPOCH（它需要组基准才能排播），老成员不受影响。端到端（真双 Engine + 真 QUIC + 真 PIN）：建组后账本恰好一个组、成员表就是被点名的对端、lead_ms 原样、成员排播用的 epoch 与建组帧一致、退光后组条目被清掉。未做：桌面/Android 的勾选成组面板与成员同步质量展示、真机三台同时出声（±10 ms P95）、多会话。详见 docs/30-m3-group-management.md。' }
    @{ T = '[M3] GROUP_EPOCH 载荷与接线（发送端指定组基准 → 接收端排播）'; M = 'M3'; P = 'P2'; S = 'Done'; B = '2026-09-16 完成。新增 GroupEpochPayload{epoch_id, epoch_local_us, lead_ms}（§4.1 的 0x43，postcard）；dispatch 层把 0x43 从「本里程碑不实现」计数里移出，补上 ControlRequest::GroupEpoch 与 op()/encode_body() 映射（覆盖度测试与 L1 往返测试同步更新，样本表补样本）；接收侧收到 GROUP_EPOCH 后用本机时钟偏移换算并写入排播（与 Engine::schedule_playout 同一条路径）；发送侧新增 Engine::announce_group_epoch(peer, epoch_id, lead_ms)，epoch_local_us 取本机单调时刻。端到端（真双 Engine + 真 QUIC + 真 PIN + NullPlayout）：0x43 送达、epoch_id 原样到达接收端、target_local_us 是换算后的本机 µs、等待量落在「帧的流内时刻 + 提前量」量级。本轮实测还澄清了一个容易想错的地方：target = local(epoch) + sample_index/48000 + lead，所以接收端等的不是 lead 而是「流内时刻 + lead」；由此推论 epoch 必须与当前推流位置对齐，否则目标落在过去、帧会被 age 判定全部丢掉（这是既定语义，不是缺陷）。未做：GROUP_CREATE/JOIN/LEAVE 组管理三帧、真机双机同时出声验证、§7 的 buffer/dac 补偿项。详见 docs/29-m3-group-epoch.md。' }
    @{ T = '[M3] 桌面端同步组面板（勾选成组 / 动态加入退出）'; M = 'M3'; P = 'P2'; S = 'Done'; B = '2026-09-16 完成。引擎侧组账本与四个组控制帧就绪后，把「同步组」接成界面：新增 4 个 Tauri 命令（list_groups / create_group(id_shorts, lead_ms) / join_group / leave_group）、桥接层视图转换（组成员短码与 PeerView 同口径，epoch_id 字符串化 —— u64 在 JS 里只有 53 位安全整数，直接传数字会悄悄丢精度）、视图形状 GroupView/GroupMemberView、前端类型 + 4 个 IPC + hook 状态与动作、以及 GroupPanel 组件（可加入对端勾选列表 + 提前量输入 + 建组按钮 + 组卡片：成员、epoch、逐成员退出、未入组成员一键加入），挂载在遥测面板上方。验证：Rust 视图转换单测（epoch 以 16 位 hex 出网、成员短码口径）、tsc --noEmit 严格模式 + vite build、桌面 clippy/测试与内核全量 fmt/clippy/测试全绿。未做：成员同步质量展示（需每成员时钟估计质量）、事件驱动自动刷新、真机三台同时出声（±10 ms P95）、多会话。详见 docs/31-m3-group-panel.md。' }
    @{ T = '[M3] 成员同步质量展示（§6.5 分级随成员表出网）'; M = 'M3'; P = 'P2'; S = 'Done'; B = '2026-09-16 完成。协议 §7 明确要求「某成员 Poor 质量 → UI 必须明示该设备同步质量差」，而质量此前压根没出网。本次把 GroupSnapshot 的成员从裸指纹改为 GroupMember{id, quality, offset_us}：groups() 为每个成员填 §6.5 的时钟分级（取自该会话自己的估计；还没有估计时按 Poor 处理 —— 没有估计等于不可同步，不能默认 Good），并先放掉组表锁再查会话表以避免「组表 ⇄ 对端表」两把锁同时持有。桌面侧 GroupMemberView 增加 quality（good/fair/poor 字符串）与 offsetUs，面板按分级上色（poor 红字「同步质量差」、fair 琥珀、good 绿），悬停显示偏移 µs 或「还没有时钟估计」。验证：引擎端到端断言成员表与质量分级（三个确定值之一）、桌面视图转换单测（质量字符串透传 + offset）、tsc 严格 + vite build、内核与桌面 clippy/测试与 fmt 全绿。未做：事件驱动刷新、质量变化的显式提示、真机三台同时出声、多会话。详见 docs/32-m3-member-quality.md。' }
    @{ T = '[M3] 多会话：共享采集枢纽 + start_send_many（一路采集喂 ≥3 台接收端）'; M = 'M3'; P = 'P2'; S = 'Done'; B = '2026-09-16 完成（路线图 M3 交付物 1 的结构部分）。核心判断：共享采集不是优化而是**正确性**问题 —— 各会话各自采集时读取起点与丢样彼此错开几十毫秒，接收端按同一个 epoch 播放时这点错位会原封不动加到组内偏差上（M3 验收是组内 ±10 ms）。实现：CaptureHub（一路 CaptureSource → 切片 → 统一分配 seq/sample_index → 广播，容量 8 帧、慢会话丢旧帧不拖住采集）+ 每会话编码线程 session_encoder_main（订阅共享帧、用本会话 codec/码率编码）+ 引用计数生命周期（第一个会话启流创建、最后一个停止时停采集、下一个会话重建）+ Engine::start_send_many（每台一份结果，一台失败不影响其它）+ 同引擎必须同帧长的显式约束 + 封口时刻在枢纽只记一次。编码留在会话侧是为了各会话能各用自己的码率（§8 自适应按对端丢包各自决策）。验证：HubClock 单测（编号与 u32 回绕）+ 端到端 tests/multi_session.rs（1 发 3 收，真 QUIC + 真 PIN；三台接收侧链路码率 ≈ 321–328 kbps 即三路都在收帧）+ 引擎全量与桌面/内核 clippy、fmt 全绿。**发现（已记 docs/33）**：接收端 peers() 里查不到发起端（accept 一侧会话表未登记），本轮改用接收侧链路码率当判据；这是既有行为待单独核。未做：≥8 台真机压测、每会话音量（SET_GAIN 未接线）、真机三台同时出声验收。详见 docs/33-m3-multi-session.md。' }
    @{ T = '[M3] 每会话独立音量（SET_GAIN / SET_MUTE 接线 + 播放侧渐变）'; M = 'M3'; P = 'P2'; S = 'Done'; B = '2026-09-16 完成（补上 M3 交付物 1 的「每会话独立音量」）。新增 audiolink-engine::gain（纯逻辑）：GainState 当前值 → 目标值按步长逐帧逼近，f32 增益严格转千分点整数（内部一律整数比较，浮点参数不可复现），NaN/inf/负数/超上限一律拒绝而**不静默夹到合法值假装成功**；播放侧每帧写前取一次增益并乘到样本上（每帧最多一个步长 —— 硬切增益就是爆音）；增益状态挂在播放句柄上，**每会话一份**（多会话各调各的，采样时间轴仍共享）；ControlRequest::SetGain/SetMute 从「明确不做」变成真的生效（含 stream_id 校验），SET_MUTE 用 50 ms 渐变；新增 Engine::set_peer_gain(peer, gain, ramp_ms)（发送端调接收端音量，非法值在发送端就拒绝、不发到线上）。验证：gain 纯逻辑 8 项单测（分步/零渐变/极小差值/往返/超限拒绝/严格转换/放大）+ 端到端 tests/gain_control.rs（真 QUIC + 真 PIN + 记录写出的 PCM：基线 RMS 0.1772 → 静音 0.0000 → 恢复 0.1773；越界 3.0 在发送端 Err）+ 引擎 142 单测与全部集成测试、桌面全量、内核与桌面 clippy、fmt 全绿。未做：UI 音量控件、真机听感验证、音量回读、真机三台同时出声。详见 docs/34-m3-per-session-gain.md。' }
    @{ T = '[M3] 接收端对端可见性核对（澄清误报 + 正式断言）'; M = 'M3'; P = 'P2'; S = 'Done'; B = '2026-09-16 完成。上一轮曾记下「接收端 peers() 查不到发起端」的疑点；本轮按时序采样（0.5s/2s/3.5s/5s）复核后确认那是**测试把断言写在 shutdown 之后**造成的假象 —— 链路上对端表一直是 1 条会话、状态在 Streaming/Degraded、遥测有值，只有关闭之后才清 0（正常清理）。处置：multi_session 增加正式断言（链路上接收端恰好看到发起端一条会话，id 与状态正确）+ 反向断言（关闭后对端表必须清空，否则 UI 会挂着不存在的设备）；docs/33 §4 与交接条目同步更正，不让错误结论留在仓库里。教训已写进 docs/35：断言的位置本身就是结论的一部分（shutdown 后查表、渐变期测稳态、priming 前量队列是同一类错误）。顺带把 SET_GAIN 通到桌面外壳（EngineBridge::set_peer_gain + set_peer_gain 命令 + api/hook 动作，渐变 200 ms）。未做：UI 滑块（对端卡片的三元 JSX 需要包 fragment 才能并列渲染，本轮未冒险改结构）。详见 docs/35-receiver-visibility-correction.md。' }
    @{ T = '[桌面] 音量滑块（set_peer_gain 已通到 IPC，UI 未接）'; M = 'M3'; P = 'P2'; S = 'Done'; B = '2026-09-16 完成。对端卡片在推流中显示音量滑块（0–200%，range 0/2 step 0.05，aria-label 与 title 都写明「拖动时按 200 ms 渐变生效」），经 onGain → api.setPeerGain(id_short, gain, 200) → 桥接 resolve_peer → Engine::set_peer_gain 下发；非法值由引擎边界拒绝并人话化，成功用 notice 报「音量已设为 N%」。实现上刻意**没有**动原来那个「开始/停止/等待配对」的三元结构：滑块作为独立的 streaming 条件块渲染在操作区之上，避免为了并列放两个孩子去包 fragment（上轮就是在这里停手的）。卡片顶部注释同步更新（原写着「不做音量滑块」）。验证：tsc --noEmit 严格 + vite build、桌面 clippy/测试、内核全量门禁全绿。' }
    @{ T = '[桌面] 同步组面板改为事件驱动刷新（GroupUpdated → audiolink://groups）'; M = 'M3'; P = 'P2'; S = 'Done'; B = '2026-09-16 完成。引擎侧本来就在建组/加入/退出时发 EngineEvent::GroupUpdated，但外壳只记日志、面板要用户手动点刷新。现在：桥接层新增 EVENT_GROUPS（audiolink://groups），GroupUpdated 分支除了日志还 emit 一条轻量信号（group_id/epoch_id/members 三元组）；前端 ipc 增加 EVENT_GROUPS 常量与 subscribeEvents 的 onGroupUpdated handler；hook 收到信号就 refreshGroups() 拉一次最新列表（明细仍以 list_groups 为准，信号只表达「变了」）。不做轮询的理由：建组/加入/退出都是低频人工动作，事件驱动既省 IPC 也不会漏。验证：桌面 clippy（-D warnings）/测试、tsc 严格 + vite build、内核全量全绿。' }
    @{ T = '[M4] 混音器（多路求和 + 软限幅 + 每路增益/静音）'; M = 'M4'; P = 'P2'; S = 'Done'; B = '2026-09-16 完成。新增 audiolink-audio::mixer（纯逻辑）：PcmMixer 多路求和、软限幅（拐点 0.7 以上指数逼近 ±1，连续单调永不削顶）、每路增益/静音、缓冲满丢最旧、FR-12 的 8 路上限；每次混音返回 MixStats（参与路数 / 被限幅样本数 / 限幅前峰值）并可累计，让「听起来有没有爆」有数字可看。引擎接线用 **owner 模式**：同一台接收端只打开一次播放设备 —— 第一个会话是 owner，它的播放线程照旧跑 M1/M2 的全套调度（抖动深度、欠载补静音、迟到丢弃、§7 排播），只是写出去的内容从「自己那一帧」变成混音器输出；其余会话继续跑自己的调度、只把帧混进去不碰设备。这么做的理由是 M1/M2 的播放纪律是三轮调出来的，混音只该换「写什么」而不是重写「什么时候写」。每路增益走 §4.1 的 SET_GAIN（落到本会话那条混音输入上，多会话各调各的）；会话停止时把自己的源摘掉（否则重连几次就撞上限）；超限报 STREAM_LIMIT。验证：mixer 11 项单测 + tests/mixer_convergence.rs（两个发送端 → 一个接收端，真 QUIC + 真 PIN + 记录 PCM：两路混音 RMS 0.2501 → 静音 A 路后 0.1769，比值 0.707 = 1/√2，即不相关信号去掉一路的功率半减，定量证明两路都真的进了输出）+ 引擎全量与桌面/内核 clippy、fmt、前端 build 全绿。未做：Android 内录/麦克风、能力协商与降级、多路时钟对齐（不同源按各自 epoch 换算后再混）、真机无爆音验收、8 路压测。详见 docs/37-m4-mixer.md。' }
    @{ T = '[M3] 双机录音对齐工具 sync-measure（脉冲 + 波形对齐，输出偏差与漂移）'; M = 'M3'; P = 'P2'; S = 'Done'; B = '2026-09-16 完成（路线图 M3 交付物 5）。新增 audiolink-tools::syncmeasure（纯逻辑）+ --bin sync-measure：自研 WAV 解析（PCM 16/24/32 与 float 32/64、EXTENSIBLE、奇数块填充、data 截断按整帧）、脉冲粗检测（快攻击慢释放包络 + 阈值穿越插值）、亚采样精细对齐（脉冲邻域归一化互相关 + 抛物线插值）、统计与判定（均值 / P50 / P95(|·|) / 极值 / 漂移 ppm 对「A 侧时刻 → 偏差」线性回归）、JSON 报告；退出码 0 达标 / 1 超标 / 2 用法 / 3 输入或检测失败。测具先被量一遍：--self-test 合成已知真值（4.7 ms 偏差 + 120 ppm 漂移）逐对比对，实测单对最大误差 0.010 ms、漂移 1.0 ppm。踩到并处理一个真坑：单一频率长正弦的互相关有周期歧义（偏差散开约 1 ms），夹具改用宽带噪声突发，且搜索范围内出现几乎等高的另一峰时该对标 ambiguous 并写进报告与 notes（不假装没看见）。测具不撒谎：静音 / 纯直流 / 检测不到脉冲 / 脉冲不足 / 波形对不上（NCC < 0.5）一律明确报错，负向用例（脉冲串 vs 伪随机噪声）必须报错。端到端：--self-test --write 写出两段真 WAV（各 192000 帧）再走完整命令行，6 对 NCC ≈ 1.0、逐对偏差 4.708–5.000 ms（真值 4.7 + 0.06·i）、判定 within、退出码 0。16 项单测 + 4 项 CLI 测试。真机双机同期录音验收尚未执行（工具已就绪）。提交 6e2fddd；CI run 35083496842 四个 job 全绿。详见 docs/27-m3-sync-measure.md。' }
    @{ T = '[M4] 网状与混音：Android 内录/麦克风 + PC 播放 + 混音器 + 能力协商'; M = 'M4'; P = 'P3'; S = 'Todo'; B = 'FR-06/07/08/12；内录受 allowAudioPlaybackCapture 限制需 UI 说明。2026-09-16：交付物 3（混音器：多路求和 + 软限幅 + 每路增益/静音）已完成，PC 侧汇聚机制就绪（docs/37）。剩余：① Android 内录（AudioPlaybackCapture）与麦克风路径（需真机 + allowAudioPlaybackCapture 实测）；② 能力协商与降级（CAP_UNSUPPORTED 的 UI 置灰与说明）；③ 多路时钟对齐（不同源的流按各自 epoch 换算到本地时钟后再混音）；④ 真机「2 部手机 → PC 无爆音」与「PC ↔ 手机双向」验收；⑤ 8 路压测（上限已实现，超限报 STREAM_LIMIT）。' }
    @{ T = '[M5] 产品化：托盘/自启/自动更新/双语/双 ABI 发布/合规清单'; M = 'M5'; P = 'P3'; S = 'Todo'; B = 'docs/05-roadmap.md M5' }

    # ---- 技术债与护栏 ----
    @{ T = '[护栏] 「定长载荷自洽」测试：LEN == 字段宽度之和'; M = 'M0'; P = 'P2'; S = 'Done'; B = '2026-09-16 完成。长度常量改为「最后一个字段偏移 + 字段宽度」算式（新增 U8/U16/U32/U64/I64_LEN 与 CLOCK_* 偏移常量），DATAGRAM_HEADER_LEN / CONTROL_HEADER_LEN 同源；Ptype::payload_len_rule()、ClockProbe/ClockReply::LEN、NACK_MAX_ITEMS 及编解码偏移全部引用同一批常量，定长拒绝消息由常量拼出（不再手抄）。新增 audiolink-proto/tests/payload_len_table.rs：§3 载荷表的独立副本（帧头 7 字段 / CLOCK_PROBE 2 / CLOCK_REPLY 4 / 控制帧头 4），断言常量 == 表值、实际编码长度 == 宽度之和、按表累加偏移读回哨兵字段、规则表与编码器一致（len±1 被拒、0 项 / max+1 项 / 非 4 倍数被拒、Opaque 吃到 MTU 上限）、ptype 表自洽；9 项用例。破坏性验证三组：LEN 漏一个 i64 → 3 用例红；规则表写死 24 → 3 用例红；解码侧 t2 偏移漂移 → 1 用例红。线上字节不变（帧头 24 / 控制头 8 / 载荷 12 与 28）。详见 docs/20-guards-and-tooling.md。提交 6419333；CI run 35069992337（core / android / desktop / version-consistency）全绿。' }
    @{ T = '[文档] docs/10-handoff.md 状态更新（M0 已完成）'; M = 'M0'; P = 'P2'; S = 'Done'; B = '已更新：M0 全绿 + M1 三项 P0 完成、实测基线、下一步为 M1 主线（QUIC/Android 播放/最小 UI）' }
    @{ T = '[安全] docs/06-dev-environment.md 本机路径脱敏'; M = 'M0'; P = 'P2'; S = 'Done'; B = '2026-09-16 完成。docs/ 下 7 个文件（06/10/11/12/14/15/16）的本机绝对路径全部换成占位符：%LOCALAPPDATA%\Android\Sdk、<JDK 17 根目录>、<Android SDK 根目录>、<你的 keystore 路径>、<本机用户目录>（含 local.properties 的 sdk.dir / storeFile 转义写法）。docs/06 §1 顶部新增占位符对照表：含义 + 怎么查本机实际值 + 命令示例替换方式（占位符连同单引号一起替换即可执行）。验证：全仓搜开发机用户名 0 命中，tools/ 示例原本已是 <你> 占位符。详见 docs/20-guards-and-tooling.md §2。提交 2a01879；CI run 35069992337 全绿。' }
    @{ T = '[护栏] 跨端一致性夹具：同一组 golden vectors 在 Rust 与 FFI 双跑'; M = 'M1'; P = 'P3'; S = 'Done'; B = 'audiolink-ffi::protocolSelfTest()：FFI 侧自带 §12 三组向量（解码→字段→再编码逐字节一致），并把 audiolink-proto 的 tests/golden_vectors.rs 原文 include_str! 进来**运行期逐字节比对**四份常量 —— 上游一改向量就红灯（该守卫当场抓到过一次解析注释的 bug）。18 测试绿' }
    @{ T = '[工具] alp2-dump 支持 pcap/pcapng 输入'; M = 'M1'; P = 'P3'; S = 'Done'; B = '2026-09-16 完成。输入按内容自动分派：4 字节 magic 命中 pcap/pcapng 走抓包分支，其余仍按 hex 文本（原行为不变）；新增 --pcap（强制抓包分支）、--port N（默认 58290，§1 的 QUIC 端口）、--all-ports。新增 audiolink-tools::pcap（不引第三方抓包库）：pcap 四种 magic（µs/ns × 大小端）、pcapng 的 SHB(byte-order magic 定端序)/IDB/EPB/SPB（块长 4 字节对齐且块尾副本必须等于块头）、链路层 Ethernet(含 802.1Q/802.1ad VLAN 标签链)/Linux SLL v1/raw IP/BSD loopback、IPv4(IHL)/IPv6(固定 40 B)、udp_len<8（GSO）按剩余字节；不做 IP 分片重组 / IPv6 扩展头链 / 隧道解封装（判为非目标包跳过）。剥不出 UDP 计「跳过」而非解码失败。回归：core/crates/audiolink-tools/tests/pcap_extract.rs 9 项（夹具现场按 RFC 逐字节合成）：pcap 与 pcapng 端到端剥出 §12 向量再交给 L1 解码、纳秒时间戳换算、双层 VLAN、IPv6 与 Linux SLL、ICMP/未知链路/扩展头跳过、udp_len=0 回退、坏输入全部 Err 无 panic、hex 文本不误判。端到端实测：pcap 与 pcapng 各 2 个目标报文解码成功、跳过 2 个非目标包、退出码 0；--all-ports 时 DNS 包计入失败（端口过滤要挡的噪声）；hex 路径回归通过。详见 docs/20-guards-and-tooling.md §3。提交 402636f；CI run 35069992337 全绿。' }
    @{ T = '[工具] self-loop 支持真机链路端到端测量'; M = 'M1'; P = 'P2'; S = 'Done'; B = '**已实现**：§6 时钟同步接进引擎后，跨机测量由新增的 tools/device-link 承担（PC 侧连真机、PIN 配对、推流、逐项标 [实测]/[模型]/[未测] 的延迟账本），latency-probe 也已能打生产端点（补了客户端证书）。历史背景：**曾被 tools/link-loop 部分取代**：link-loop 用两个真实 Engine 走真 QUIC，按 seq 配对探针量「帧封口→sink 写出」，PC↔PC 段已实测（P50 40.2 ms）。**跨机仍缺**：探针直接比较 Instant，跨机不可相减 —— 需要接 §6 时钟同步（ClockEstimator 已实现并单测，但引擎侧一根线没接，clock_offset_us 恒为 0）' }
    @{ T = '[CI] QUIC 依赖 feature 门控（回收 core job 的 ~2.5 min）'; M = 'M0'; P = 'P3'; S = 'Done'; B = '2026-09-16 实测证伪前提，已按实测处置。core job 4.0 min = test 139 s + rust-cache 恢复 53 s + clippy 25 s；rust-cache 日志 full match，test 步骤的 Compiling 只有 9 个 workspace 本地 crate，quinn/rustls/tokio 一行都没有，即依赖整棵树被 cache 完整复原、不在关键路径上。所以「tools 引入 quinn 后 core 由 1m36s 涨到 4m10s」是引入当天的一次性冷编译；feature 门控在这里没有可回收的时间，而 core 的 clippy 必须保留 --all-features 的 lint 覆盖（把 QUIC 藏进 feature 只会多出没被 lint 的组合）。test 的 139 s 里真正的测试运行只有 39.2 s，其余是 9 个 crate 编译加每个 target 链一个测试二进制。于是把刀口对准编译：5 个无 #[test] 的 tools bin 标 test = false（device-link 与 self-loop 里有测试、原样保留），core 的 test 步骤加 --lib --tests 跳过 9 个全 0 项 doctest；测试零损失（355 项不变，41 降到 27 个 suite）。诚实结论：这两项削减的收益小于 run 间波动（test 139 / 138 / 167 s），core 没有可测回收；进一步压只能拆并行 job，见 docs/26-ci-key-path.md §6。提交 6ae748b / b7e737f；CI run 35080728329 / 35081356931 四个 job 全绿。' }
    @{ T = '[工具] soak-runner（8 h 回环 + 指标采集 + 异常快照）'; M = 'M2'; P = 'P3'; S = 'Done'; B = '2026-09-16 完成。新增 audiolink-tools::soak（判定 + 报告，纯逻辑可单测）与 --bin soak-runner：同一进程起两个真实 Engine（node-a 发送 / node-b 接收），走 127.0.0.1 真实 QUIC 并实跑 §5 PIN 配对；合成源 + NullPlayout，不出声不采声卡，可无人值守长跑。每 1 s 采接收侧遥测，越界即留异常快照（时刻 + 种类 + 当时完整遥测）：not_streaming / underrun / playout_concealment / packet_loss / late_drop / nack_retransmit / bitrate_out_of_range / stalled（连续 5 次零码率）；快照上限 200 条（超出计 dropped_violations），另存每 60 s 一条粗采样（8 h 约 480 条）。默认 8 h（--seconds 28800），预热 3 s 不判，目标码率默认 320000（双发后期望）±30%。报告 JSON 手写字段（不把 serde feature 拉进内核依赖图）。退出码即判定：0 无异常 / 1 有异常 / 2 用法错误。实测：20 s 冒烟 exit 0，码率 312→321 kbps 收敛后稳定、丢包/欠载/掩盖/迟到/NACK 全 0、队列 20 ms、RTT 1.8 ms，报告 750 B；把 --expected-bps 写错 → 6 条 bitrate_out_of_range、exit 1（判定链路闭环）。8 项单测。真正的 8 h 长跑尚未执行。提交 be52681；CI run 35072898528（core / android / desktop / version-consistency）全绿。详见 docs/22-m2-soak-runner.md。' }
    @{ T = '[CI] Android job：预编译 cargo-ndk（省 ~2 min）'; M = 'M0'; P = 'P3'; S = 'Done'; B = '2026-09-16 实测证伪前提，并顺手把真正的 2 分钟拿回来了。cargo install cargo-ndk --locked 实测只花 1 s：Swatinem/rust-cache 默认缓存 ~/.cargo/bin，二进制早就在 cache 里，所以「每次 run 重编 cargo-ndk」不成立、无需预编译。android job 的真关键路径是 Gradle assembleDebug（130 s）。改为 actions/cache 缓存 ~/.gradle 的 caches 与 wrapper：首轮 miss（0.0 s），次轮命中（6 s）后 android job 192 降到 85 s、assembleDebug 130 降到 16 s，回收 107 s。提交 6ae748b；CI run 35081356931 四个 job 全绿。详见 docs/26-ci-key-path.md。' }
    @{ T = '[CI] core job 反馈时间：拆并行 job（快内核 + 重内核）'; M = 'M0'; P = 'P3'; S = 'Todo'; B = '实测依据（2026-09-16，docs/26-ci-key-path.md）：core 是 CI 的墙钟关键路径（236–256 s，其余 job 只有 85–112 s）。构成是 test 步骤 139–167 s（真正的测试运行只有 39 s，其余为 9 个本地 crate 编译加 27 个测试二进制链接，Windows 链接尤其贵）、rust-cache 恢复 45–57 s、clippy 22–25 s。依赖树已被 cache 全覆盖，再压只能动本地 crate 编译这一层：把 types/proto/identity 这类轻内核拆成独立快反馈 job，net/engine/ffi/tools 留重 job。墙钟仍由重 job 决定（约 3.5 min），收益在协议改动的反馈时间（1.5 min 内出结果）。' }
)

$STATUS_OPTIONS = @('Todo', 'In Progress', 'Done')
$MILESTONE_OPTIONS = @('M0', 'M1', 'M2', 'M3', 'M4', 'M5')
$PRIORITY_OPTIONS = @('P0', 'P1', 'P2', 'P3')

# ---------------------------------------------------------------------------
# 辅助
# ---------------------------------------------------------------------------

function Invoke-Gh {
    param([Parameter(Mandatory)][string[]]$Arguments, [switch]$AsJson)
    $output = & gh @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "gh $($Arguments -join ' ') 失败：`n$($output | Out-String)"
    }
    if ($AsJson) {
        return (($output | Out-String).Trim() | ConvertFrom-Json)
    }
    return $output
}

function Get-ProjectFields {
    param([Parameter(Mandatory)][int]$Number)
    return (Invoke-Gh @('project', 'field-list', "$Number", '--owner', $Owner, '--format', 'json') -AsJson).fields
}

function Ensure-SingleSelectField {
    param([Parameter(Mandatory)][int]$Number, [Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][string[]]$Options)
    $existing = Get-ProjectFields $Number | Where-Object { $_.name -eq $Name } | Select-Object -First 1
    if ($existing) {
        return $existing
    }
    if ($DryRun) {
        Write-Host "  [dry-run] 创建单选字段 '$Name'（选项：$($Options -join ', ')）"
        return $null
    }
    Write-Host "  + 创建单选字段 '$Name'（选项：$($Options -join ', ')）"
    Invoke-Gh @('project', 'field-create', "$Number", '--owner', $Owner, '--name', $Name, '--data-type', 'SINGLE_SELECT', '--single-select-options', ($Options -join ',')) | Out-Null
    return Get-ProjectFields $Number | Where-Object { $_.name -eq $Name } | Select-Object -First 1
}

function Item-Title {
    param($Item)
    if ($Item.title) { return "$($Item.title)" }
    if ($Item.content -and $Item.content.title) { return "$($Item.content.title)" }
    return ''
}

function Item-Body {
    param($Item)
    if ($Item.content -and $Item.content.body) { return "$($Item.content.body)" }
    return ''
}

function Item-IsDraft {
    param($Item)
    return ($Item.content -and "$($Item.content.type)" -eq 'DraftIssue')
}

# 草稿条目的正文要按 **content id**（`DI_...`）改，不是按项目条目 id（`PVTI_...`）——
# 实测 gh 会直接报「ID must be the ID of the draft issue content which is prefixed with DI_」。
function Item-DraftContentId {
    param($Item)
    $id = "$($Item.content.id)"
    if ($id.StartsWith('DI_')) { return $id }
    return ''
}

function Set-SingleSelect {
    param(
        [Parameter(Mandatory)][string]$ItemId,
        [Parameter(Mandatory)][string]$ProjectId,
        [Parameter(Mandatory)]$Field,
        [Parameter(Mandatory)][string]$OptionName,
        [Parameter(Mandatory)][int]$Number
    )
    if (-not $Field) { return }
    $option = $Field.options | Where-Object { $_.name -eq $OptionName } | Select-Object -First 1
    if (-not $option) {
        Write-Warning "字段 '$($Field.name)' 没有选项 '$OptionName'，跳过"
        return
    }
    Invoke-Gh @('project', 'item-edit', '--id', $ItemId, '--project-id', $ProjectId, '--field-id', "$($Field.id)", '--single-select-option-id', "$($option.id)") | Out-Null
}

# ---------------------------------------------------------------------------
# 主流程
# ---------------------------------------------------------------------------

Write-Host "AudioLink 任务看板同步（owner=$Owner, project='$ProjectTitle'$(if ($DryRun) { ', DRY-RUN' }))"

try {
    $projectList = Invoke-Gh @('project', 'list', '--owner', $Owner, '--limit', '100', '--format', 'json') -AsJson
}
catch {
    Write-Error "无法访问 GitHub Projects —— token 缺 scope。请先运行：`n    gh auth refresh -s project`n（交互式，需在浏览器完成授权）`n原始错误：$($_.Exception.Message)"
    exit 1
}

$project = $projectList.projects | Where-Object { $_.title -eq $ProjectTitle } | Select-Object -First 1
if (-not $project) {
    if ($DryRun) {
        Write-Host "  [dry-run] 创建看板 '$ProjectTitle'（并关联仓库 $Repo）"
        Write-Host "  [dry-run] 创建单选项字段：Status($($STATUS_OPTIONS -join '/'))、里程碑($($MILESTONE_OPTIONS -join '/'))、优先级($($PRIORITY_OPTIONS -join '/'))"
        Write-Host "  [dry-run] 将写入 $($Tasks.Count) 个条目："
        foreach ($task in $Tasks) {
            Write-Host ("      - [{0}/{1}/{2}] {3}" -f $task.M, $task.P, $task.S, $task.T)
        }
        exit 0
    }
    Write-Host "  + 创建看板 '$ProjectTitle'"
    Invoke-Gh @('project', 'create', '--owner', $Owner, '--title', $ProjectTitle) | Out-Null
    $projectList = Invoke-Gh @('project', 'list', '--owner', $Owner, '--limit', '100', '--format', 'json') -AsJson
    $project = $projectList.projects | Where-Object { $_.title -eq $ProjectTitle } | Select-Object -First 1
}
if (-not $project) {
    throw "看板创建后仍未找到（owner=$Owner, title=$ProjectTitle）"
}

$number = [int]$project.number
$projectId = "$($project.id)"
Write-Host "  看板：$($project.url)（number=$number）"

if (-not $NoLink -and -not $DryRun) {
    Invoke-Gh @('project', 'link', "$number", '--owner', $Owner, '--repo', $Repo) | Out-Null
    Write-Host "  已关联仓库 $Repo"
}

$statusField = Ensure-SingleSelectField -Number $number -Name 'Status' -Options $STATUS_OPTIONS
$milestoneField = Ensure-SingleSelectField -Number $number -Name '里程碑' -Options $MILESTONE_OPTIONS
$priorityField = Ensure-SingleSelectField -Number $number -Name '优先级' -Options $PRIORITY_OPTIONS

# 即使 dry-run 也要真的读一次现有条目 —— `project item-list` 是只读命令。
#
# 不读的后果（2026-09-14 实测暴露）：dry-run 里 `$existing` 恒为空，
# 于是「已存在 / 新建」的预览**全都是「新建」**。而本脚本唯一的危险动作恰恰是
# **造重复条目**（标题是幂等键，靠 `$existing` 匹配）—— 一个分不清重复的预览等于没有预览，
# 会让人误以为「一切都要新建」而去关掉 dry-run 直接跑。
$items = (Invoke-Gh @('project', 'item-list', "$number", '--owner', $Owner, '--limit', '200', '--format', 'json') -AsJson).items
$existing = @{}
foreach ($item in $items) {
    $title = Item-Title $item
    if ($title) { $existing[$title] = $item }
}
Write-Host "  现有条目 $($existing.Count) 个，任务表 $($Tasks.Count) 个"

$created = 0
$updated = 0
$bodies = 0
$unchanged = 0

foreach ($task in $Tasks) {
    if ($DryRun) {
        $mark = if ($existing.ContainsKey($task.T)) { '已存在' } else { '新建' }
        Write-Host ("  [dry-run] {0,-4} [{1}/{2}/{3}] {4}" -f $mark, $task.M, $task.P, $task.S, $task.T)
        continue
    }

    $item = $existing[$task.T]
    if (-not $item) {
        $createdItem = Invoke-Gh @('project', 'item-create', "$number", '--owner', $Owner, '--title', $task.T, '--body', $task.B, '--format', 'json') -AsJson
        $itemId = "$($createdItem.id)"
        Set-SingleSelect -ItemId $itemId -ProjectId $projectId -Field $statusField -OptionName $task.S -Number $number
        Set-SingleSelect -ItemId $itemId -ProjectId $projectId -Field $milestoneField -OptionName $task.M -Number $number
        Set-SingleSelect -ItemId $itemId -ProjectId $projectId -Field $priorityField -OptionName $task.P -Number $number
        $created++
        Write-Host ("  + [{0}/{1}/{2}] {3}" -f $task.M, $task.P, $task.S, $task.T)
        continue
    }

    $itemId = "$($item.id)"
    $statusChanged = "$($item.status)" -ne $task.S
    # 说明（body）也要跟着文档走：看板是文档的机械映射，正文里往往写着口径修正与实测结论
    $bodyChanged = (Item-IsDraft $item) -and ((Item-Body $item) -ne $task.B)
    if ($statusChanged) {
        Set-SingleSelect -ItemId $itemId -ProjectId $projectId -Field $statusField -OptionName $task.S -Number $number
        $updated++
        Write-Host ("  ~ 状态 {0} → {1}：{2}" -f $item.status, $task.S, $task.T)
    }
    if ($bodyChanged) {
        $contentId = Item-DraftContentId $item
        if ($contentId) {
            Invoke-Gh @('project', 'item-edit', '--id', $contentId, '--body', $task.B) | Out-Null
            $bodies++
            Write-Host ("  ~ 说明更新：{0}" -f $task.T)
        }
        else {
            Write-Warning "条目 '$($task.T)' 不是草稿条目，跳过说明更新"
        }
    }
    if (-not $statusChanged -and -not $bodyChanged) {
        $unchanged++
    }
}

Write-Host ""
if ($DryRun) {
    Write-Host "DRY-RUN 结束（没有做任何变更）。去掉 -DryRun 即真正同步。"
}
else {
    Write-Host "同步完成：新建 $created，状态更新 $updated，说明更新 $bodies，已是最新 $unchanged。"
    Write-Host "看板：$($project.url)"
}
