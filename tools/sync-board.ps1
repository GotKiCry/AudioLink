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
    @{ T = '[M2] 稳定性与质量：抖动缓冲 + 双发/PLC/NACK + 自适应码率 + 遥测面板'; M = 'M2'; P = 'P2'; S = 'In Progress'; B = 'docs/05-roadmap.md M2；验收含 8 h soak 与弱网（5 Mbps / 2% 丢包 / 30 ms 抖动）。2026-09-16：播放游标修复已阻止迟到帧累积延迟；自建 PCM 丢包掩盖也已完成，覆盖 120 ms 淡出、2.5 ms 恢复交叉淡化、迟到包预解码丢弃与长空洞限流。PHK110 正常链路 60 s 队列 P50/P95/max 均 40 ms、链路丢包/PLC 0，但欠载/迟到从 2 增至 49。下一步实现有界重排与自适应目标深度，再推进双发/NACK、码率自适应、真实弱网及 8 h soak；不能重新引入持续延迟。详见 docs/17-m2-pcm-concealment.md。' }
    @{ T = '[M2] 自建丢包掩盖（CELT-only 无真 PLC：重复上一包 + 淡出 + 交叉淡化）'; M = 'M2'; P = 'P1'; S = 'Done'; B = '2026-09-16 完成。新增 PcmConcealer：重复上一帧、120 ms 线性淡出、恢复时 2.5 ms 交叉淡化；AudioReceiver 将 Opus、掩盖历史与序号统一推进，迟到包解码前丢弃，解码失败同样掩盖，长空洞最多补 120 ms 并重建解码器，避免塞满播放队列。opus-rs 无逐包模式 getter，所有档位保守跳过未经证明的原生 conceal；实测连续调用原生 conceal 会把 6 帧丢包后的恢复 RMS 压到约 0.002。真实 Opus 回归覆盖约 2% 确定性丢包（4/201，无序号洞且掩盖 RMS > 0.02）、120 ms 突发、u32 回绕、解码失败、迟到包和 999 帧空洞。PHK110 正常链路 60 s 队列 P50/P95/max 40/40/40 ms、链路丢包/PLC 0，APK SHA-256 b51180b4bb208ae273d4eb2bccb66e6728375fbafffa219c31dd13b2b05da2e7；真实网络 2%/30 ms 弱网仍属 M2 父任务。详见 docs/17-m2-pcm-concealment.md。' }
    @{ T = '[M3] 多设备与同步：时钟同步全流程 + 预约播放 + 同步组 + sync-measure'; M = 'M3'; P = 'P2'; S = 'Todo'; B = '组内 ±10 ms（P95）；§6 的 200 样本窗口 + 回归漂移估计在此落地' }
    @{ T = '[M4] 网状与混音：Android 内录/麦克风 + PC 播放 + 混音器 + 能力协商'; M = 'M4'; P = 'P3'; S = 'Todo'; B = 'FR-06/07/08/12；内录受 allowAudioPlaybackCapture 限制需 UI 说明' }
    @{ T = '[M5] 产品化：托盘/自启/自动更新/双语/双 ABI 发布/合规清单'; M = 'M5'; P = 'P3'; S = 'Todo'; B = 'docs/05-roadmap.md M5' }

    # ---- 技术债与护栏 ----
    @{ T = '[护栏] 「定长载荷自洽」测试：LEN == 字段宽度之和'; M = 'M0'; P = 'P2'; S = 'Todo'; B = '针对 CLOCK_REPLY 那类「文档/常量算错」的机械护栏' }
    @{ T = '[文档] docs/10-handoff.md 状态更新（M0 已完成）'; M = 'M0'; P = 'P2'; S = 'Done'; B = '已更新：M0 全绿 + M1 三项 P0 完成、实测基线、下一步为 M1 主线（QUIC/Android 播放/最小 UI）' }
    @{ T = '[安全] docs/06-dev-environment.md 本机路径脱敏'; M = 'M0'; P = 'P2'; S = 'Todo'; B = '仓库已 PUBLIC，文档里仍有 C:\Users\liuzh\... 路径' }
    @{ T = '[护栏] 跨端一致性夹具：同一组 golden vectors 在 Rust 与 FFI 双跑'; M = 'M1'; P = 'P3'; S = 'Done'; B = 'audiolink-ffi::protocolSelfTest()：FFI 侧自带 §12 三组向量（解码→字段→再编码逐字节一致），并把 audiolink-proto 的 tests/golden_vectors.rs 原文 include_str! 进来**运行期逐字节比对**四份常量 —— 上游一改向量就红灯（该守卫当场抓到过一次解析注释的 bug）。18 测试绿' }
    @{ T = '[工具] alp2-dump 支持 pcap/pcapng 输入'; M = 'M1'; P = 'P3'; S = 'Todo'; B = '当前只吃 hex 文本；等真有抓包需求再加（需夹具验证）' }
    @{ T = '[工具] self-loop 支持真机链路端到端测量'; M = 'M1'; P = 'P2'; S = 'Done'; B = '**已实现**：§6 时钟同步接进引擎后，跨机测量由新增的 tools/device-link 承担（PC 侧连真机、PIN 配对、推流、逐项标 [实测]/[模型]/[未测] 的延迟账本），latency-probe 也已能打生产端点（补了客户端证书）。历史背景：**曾被 tools/link-loop 部分取代**：link-loop 用两个真实 Engine 走真 QUIC，按 seq 配对探针量「帧封口→sink 写出」，PC↔PC 段已实测（P50 40.2 ms）。**跨机仍缺**：探针直接比较 Instant，跨机不可相减 —— 需要接 §6 时钟同步（ClockEstimator 已实现并单测，但引擎侧一根线没接，clock_offset_us 恒为 0）' }
    @{ T = '[CI] QUIC 依赖 feature 门控（回收 core job 的 ~2.5 min）'; M = 'M0'; P = 'P3'; S = 'Todo'; B = 'tools 引入 quinn/rustls/tokio 后 core job 由 1m36s 涨到 4m10s' }
    @{ T = '[工具] soak-runner（8 h 回环 + 指标采集 + 异常快照）'; M = 'M2'; P = 'P3'; S = 'Todo'; B = 'M2 验收依赖，建议 M1 一结束就有最简版' }
    @{ T = '[CI] Android job：预编译 cargo-ndk（省 ~2 min）'; M = 'M0'; P = 'P3'; S = 'Todo'; B = 'docs/10-handoff.md §7 待办' }
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
