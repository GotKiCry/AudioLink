package com.gotkicry.audiolink.ui.i18n

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import java.util.Locale

/**
 * UI 文案（中 / 英）。**界面语言跟随系统** —— 中文系统说中文，其它一律英文。
 *
 * 分层立场：这一层只管**外壳自己写的字**。服务层（`service/SenderStateMapper`、
 * `service/PowerWhitelistMapper` 等）给出的提示与错误文案仍是中文常量，且被 JVM 单测逐字钉住 ——
 * 本次不动它们（改文案会动到测试，等于改口径）。所以英文界面下，那几行来自内核/服务的提示
 * 会保持中文：这是**已知的残留不一致**，不是漏写。
 *
 * 文案纪律（沿用 docs/08-ui-spec.md §4 的既定要求）：短句、动词开头、说人话；
 * 内核真值（水位、欠载、PLC）只出现在诊断区。
 */
@Immutable
data class UiStrings(

    // ── 顶部应用栏 ──────────────────────────────────────────────────
    val themeAction: String,
    val themeSystem: String,
    val themeLight: String,
    val themeDark: String,
    val licensesAction: String,

    // ── 接收台 ─────────────────────────────────────────────────────
    /** 丝印分区标签（大写化后显示）。 */
    val deckReceiver: String,
    val stateReceiving: String,
    val stateNotReceiving: String,
    val stateServiceStopped: String,
    val stateDegraded: String,
    val stateFailed: String,
    val receiverSource: String,
    val receiverSourceNone: String,
    val receiverLatency: String,
    val receiverBitrate: String,
    val receiverLoss: String,
    val readoutUnknown: String,
    val volume: String,
    val actionStartReceiving: String,
    val actionStopReceiving: String,
    val receiverHint: String,
    val receiverDegradedHint: String,
    val serviceRunning: String,
    val serviceStopped: String,

    // ── 配对 PIN ───────────────────────────────────────────────────
    val pairingTitle: String,
    val pairingTitleStale: String,
    val pairingNote: String,
    val pairingStaleNote: String,

    // ── 发送台 ─────────────────────────────────────────────────────
    val deckSender: String,
    val senderCaptureLabel: String,
    val captureOff: String,
    val captureMic: String,
    val captureLoopback: String,
    val captureHint: String,
    val targetAddress: String,
    val targetAddressPlaceholder: String,
    val actionConnect: String,
    val actionConnecting: String,
    val pinLabel: String,
    val actionSubmitPin: String,
    val actionStartSend: String,
    val actionStopSend: String,
    val senderHint: String,

    // ── 设备通道 ───────────────────────────────────────────────────
    val deckDevices: String,
    val devicesCountFormat: String,
    val devicesEmpty: String,
    val labelFingerprint: String,
    val labelAddress: String,
    val labelState: String,
    val labelTrust: String,
    val trustYes: String,
    val trustNo: String,
    /** 发送会话标签（由 UI 组装：sessionLabel 的中文常量在 service 层，语言跟随系统时不能直接用）。 */
    val senderSessionDisconnected: String,
    val senderSessionConnected: String,
    val senderSessionSending: String,
    /** 对端会话状态标签（同上：内核字面量 → 当前语言）。 */
    val peerStateIdle: String,
    val peerStateHandshaking: String,
    val peerStateStreaming: String,
    val peerStateDegraded: String,
    val peerStateReconnecting: String,
    val peerStateFailed: String,

    // ── 诊断 ───────────────────────────────────────────────────────
    val deckDiagnostics: String,
    val diagnosticsSummary: String,
    val semanticsExpanded: String,
    val semanticsCollapsed: String,

    val diagEngine: String,
    val diagEngineRunning: String,
    val diagEngineStopped: String,
    val diagLocalName: String,
    val diagLocalId: String,
    val diagListenAddr: String,
    val diagCaps: String,
    val diagCapsBoth: String,
    val diagCapsReceiveOnly: String,

    val diagLowLatency: String,
    val diagLowLatencyOn: String,
    val diagLowLatencyOff: String,
    val diagPerformanceMode: String,
    val diagSampleRate: String,
    val diagChannels: String,
    val diagBufferRequested: String,
    val diagBufferActual: String,
    val diagFramesUnit: String,

    val diagCounters: String,
    val diagUnderrunsSystem: String,
    val diagUnderrunsSource: String,
    val diagFramesWritten: String,
    val diagSilenceFrames: String,
    val diagFramesDropped: String,
    val diagWriteErrors: String,
    val diagWriteErrorsDetail: String,

    val diagRing: String,
    val diagRingLevel: String,
    val diagRingOverflowTotal: String,
    val diagRingOverflowWindow: String,
    val diagRingUnderrunTotal: String,
    val diagRingUnderrunWindow: String,
    val diagRingWindowSeconds: String,
    val diagRingReset: String,
    val diagRingNote: String,

    val diagQueue: String,
    val diagQueueCapacity: String,
    val diagQueueLevel: String,
    val diagQueueTarget: String,
    val diagQueueShrink: String,
    val diagQueueShrinkNone: String,
    val diagQueueTargetLabel: String,
    val diagQueueShrinkLabel: String,
    val leverFull: String,
    val leverRestore: String,
    val diagQueueNote: String,

    val diagSelfTest: String,
    val diagSelfTestIdle: String,
    val diagSelfTestPassed: String,
    val diagSelfTestRejected: String,
    val diagSelfTestUnavailable: String,
    val diagSelfTestCode: String,
    val diagSelfTestShortName: String,
    val diagSelfTestDetail: String,
    val actionRunSelfTest: String,
    val actionSelfTesting: String,

    val diagExplanation: String,

    // ── 错误 ───────────────────────────────────────────────────────
    val errorTitle: String,
    val errorRecoveryPlayback: String,
    val errorRecoveryGeneric: String,
    val errorKernelDetail: String,
    val errorPairingReadFailed: String,

    // ── 许可页 ─────────────────────────────────────────────────────
    val licensesTitle: String,
    val actionBack: String,
    val licensesBundledFormat: String,
    val licensesMissing: String,
)

/** 中文（默认；原文案逐字沿用，只补新结构需要的句子）。 */
private val Zh = UiStrings(
    themeAction = "主题",
    themeSystem = "跟随系统",
    themeLight = "浅色",
    themeDark = "深色",
    licensesAction = "许可",

    deckReceiver = "接收台",
    stateReceiving = "接收中",
    stateNotReceiving = "未在接收",
    stateServiceStopped = "未在接收（服务已停止）",
    stateDegraded = "接收中（质量下降）",
    stateFailed = "接收异常",
    receiverSource = "来源",
    receiverSourceNone = "还没有电脑连过来",
    receiverLatency = "延迟",
    receiverBitrate = "码率",
    receiverLoss = "丢包",
    readoutUnknown = "—",
    volume = "音量",
    actionStartReceiving = "开始接收",
    actionStopReceiving = "停止接收",
    receiverHint = "接收由电脑端发起：电脑连过来并推流后本机会自动开始播放。",
    receiverDegradedHint = "链路质量下降，内核正在自动调整；声音可能短暂断续。",
    serviceRunning = "服务运行中",
    serviceStopped = "服务已停止",

    pairingTitle = "配对请求：在电脑端输入这 6 位数字",
    pairingTitleStale = "配对请求（已失效）",
    pairingNote = "PIN 由内核生成且限时有效（§5：60 s 过期、失败多次短时锁定）。" +
        "配对成功后本卡自动消失；对端主动断开则 PIN 作废，需重新连接。",
    pairingStaleNote = "当前没有等待配对的对端 —— 这个 PIN 来自上一次连接，内核侧的 PinGate 已经随连接销毁，" +
        "照着输必然失败。请让对方重新连接，本卡会自动换成新的 PIN。",

    deckSender = "发送台",
    senderCaptureLabel = "发送源",
    captureOff = "关闭",
    captureMic = "麦克风",
    captureLoopback = "系统内录",
    captureHint = "系统内录每次会话都要重新授权（系统行为，绕不过）；麦克风要录音权限。" +
        "被声明为不可捕获的应用与 DRM 内容录不到。",
    targetAddress = "电脑地址",
    targetAddressPlaceholder = "192.168.1.5 或 192.168.1.5:58290",
    actionConnect = "连接电脑",
    actionConnecting = "连接中…",
    pinLabel = "电脑上显示的 6 位数字",
    actionSubmitPin = "提交配对码",
    actionStartSend = "开始推流",
    actionStopSend = "停止推流",
    senderHint = "停止推流只停声音，不断开连接（信任与配对保留）。采集源要排在连接之前选：切换" +
        "「关闭 ⇄ 非关闭」会重启引擎，把已建立的连接掐断。",

    deckDevices = "设备通道",
    devicesCountFormat = "已连接 %d 台",
    devicesEmpty = "还没有设备连接。电脑端发起连接后这里会出现。",
    labelFingerprint = "短指纹",
    labelAddress = "地址",
    labelState = "状态",
    labelTrust = "信任",
    trustYes = "已信任（信任库已落盘）",
    trustNo = "未信任（还需 PIN 配对）",
    senderSessionDisconnected = "未连接",
    senderSessionConnected = "已连接 · 未发送",
    senderSessionSending = "发送中",
    peerStateIdle = "空闲",
    peerStateHandshaking = "握手中",
    peerStateStreaming = "已连接",
    peerStateDegraded = "质量下降",
    peerStateReconnecting = "重连中",
    peerStateFailed = "失败",

    deckDiagnostics = "诊断",
    diagnosticsSummary = "内核真值默认折叠，需要时再展开",
    semanticsExpanded = "已展开",
    semanticsCollapsed = "已折叠",

    diagEngine = "内核引擎（接收端）",
    diagEngineRunning = "运行中（已监听 QUIC 端口）",
    diagEngineStopped = "未启动",
    diagLocalName = "本机名称",
    diagLocalId = "指纹短码",
    diagListenAddr = "监听地址",
    diagCaps = "能力",
    diagCapsBoth = "可接收 / 可发送",
    diagCapsReceiveOnly = "可接收 / 不可发送（M1 未实现）",

    diagLowLatency = "低延迟",
    diagLowLatencyOn = "已生效（PERFORMANCE_MODE_LOW_LATENCY）",
    diagLowLatencyOff = "未生效",
    diagPerformanceMode = "getPerformanceMode()",
    diagSampleRate = "采样率",
    diagChannels = "声道数",
    diagBufferRequested = "输出缓冲（请求）",
    diagBufferActual = "输出缓冲（实际生效）",
    diagFramesUnit = "帧",

    diagCounters = "统计",
    diagUnderrunsSystem = "欠载（系统口径）",
    diagUnderrunsSource = "欠载（供给口径）",
    diagFramesWritten = "已写入音频帧",
    diagSilenceFrames = "静音填充帧",
    diagFramesDropped = "丢弃帧",
    diagWriteErrors = "写入错误次数",
    diagWriteErrorsDetail = "%d（最近错误码 %d）",

    diagRing = "播放环（内核 PCM 落地缓冲）",
    diagRingLevel = "水位",
    diagRingOverflowTotal = "溢出丢弃帧（累计）",
    diagRingOverflowWindow = "溢出丢弃帧（区间）",
    diagRingUnderrunTotal = "读空次数（累计）",
    diagRingUnderrunWindow = "读空次数（区间）",
    diagRingWindowSeconds = "区间时长",
    diagRingReset = "区间清零",
    diagRingNote = "区间增量 = 自上次清零以来的增量（前后各读一次相减）—— 用它区分「历史上丢过一大段」" +
        "与「现在正在持续丢」；累计值不受清零影响（口径不许为了好看而改）。",

    diagQueue = "输出队列水位（task-8 延迟杠杆）",
    diagQueueCapacity = "设备容量",
    diagQueueLevel = "设备队列水位",
    diagQueueTarget = "队列目标",
    diagQueueShrink = "容量收缩结果",
    diagQueueShrinkNone = "未执行",
    diagQueueTargetLabel = "路② 队列目标（控注水深度，下一拍生效）",
    diagQueueShrinkLabel = "路① 容量收缩（play() 后 setBufferSizeInFrames；丢了 FAST 就点「恢复容量」回滚）",
    leverFull = "满灌",
    leverRestore = "恢复容量",
    diagQueueNote = "容量是「最多能压多少」，水位是「现在压了多少」—— 低延迟模式生效 ≠ 缓冲小：" +
        "真机上容量 3844 帧（80.1 ms）而队列被灌满，flinger 才会报 Latency=101 ms。",

    diagSelfTest = "内核自检（协议 golden vectors）",
    diagSelfTestIdle = "尚未自检。自检验的是协议编解码层，不依赖引擎与连接 —— 服务未启动也能跑。",
    diagSelfTestPassed = "通过",
    diagSelfTestRejected = "内核拒绝",
    diagSelfTestUnavailable = "自检未能运行",
    diagSelfTestCode = "错误码",
    diagSelfTestShortName = "短名",
    diagSelfTestDetail = "细节",
    actionRunSelfTest = "运行自检",
    actionSelfTesting = "自检中…",

    diagExplanation = "引擎随服务启停。接收由电脑侧发起连接；对端还没连上或还没推流时，供给欠载会持续增长、" +
        "输出是静音，这是预期行为。发送方向见上方「发送台」。低延迟是否生效与缓冲帧数不受此影响，可直接读。",

    errorTitle = "出了点问题",
    errorRecoveryPlayback = "重新点一次「开始接收」。若还是不行，展开上面的「诊断」看内核原文。",
    errorRecoveryGeneric = "再试一次；仍然失败时展开「诊断」，用内核原文定位。",
    errorKernelDetail = "内核原文（诊断用）",
    errorPairingReadFailed = "配对状态读取失败",

    licensesTitle = "开源许可",
    actionBack = "返回",
    licensesBundledFormat = "随包分发 · %d KB",
    licensesMissing = "未找到声明文件",
)

/** English. Same structure, same semantics — 不是逐字直译，而是同一件事的英文说法。 */
private val En = UiStrings(
    themeAction = "Theme",
    themeSystem = "Follow system",
    themeLight = "Light",
    themeDark = "Dark",
    licensesAction = "Licenses",

    deckReceiver = "Receiver",
    stateReceiving = "Receiving",
    stateNotReceiving = "Not receiving",
    stateServiceStopped = "Not receiving (service stopped)",
    stateDegraded = "Receiving (degraded)",
    stateFailed = "Receiver error",
    receiverSource = "Source",
    receiverSourceNone = "No PC connected yet",
    receiverLatency = "Latency",
    receiverBitrate = "Bitrate",
    receiverLoss = "Loss",
    readoutUnknown = "—",
    volume = "Volume",
    actionStartReceiving = "Start receiving",
    actionStopReceiving = "Stop receiving",
    receiverHint = "The PC starts it: once a PC connects and streams, playback begins automatically.",
    receiverDegradedHint = "Link quality dropped; the core is adjusting. Audio may stutter briefly.",
    serviceRunning = "Service running",
    serviceStopped = "Service stopped",

    pairingTitle = "Pairing request — type these 6 digits on the PC",
    pairingTitleStale = "Pairing request (expired)",
    pairingNote = "The PIN is generated by the core and time-limited (60 s, then short lockout after repeated failures). " +
        "This card disappears once pairing succeeds; if the peer disconnects the PIN is void and a new one is needed.",
    pairingStaleNote = "No peer is waiting to pair — this PIN belongs to the previous connection and its PinGate is already " +
        "destroyed, so typing it can only fail. Ask the peer to reconnect; a fresh PIN will appear here.",

    deckSender = "Sender",
    senderCaptureLabel = "Capture source",
    captureOff = "Off",
    captureMic = "Microphone",
    captureLoopback = "System audio",
    captureHint = "System audio needs a fresh system consent each session (unavoidable); the microphone needs the record " +
        "permission. Apps marked non-capturable and DRM content cannot be recorded.",
    targetAddress = "PC address",
    targetAddressPlaceholder = "192.168.1.5 or 192.168.1.5:58290",
    actionConnect = "Connect to PC",
    actionConnecting = "Connecting…",
    pinLabel = "6-digit code shown on the PC",
    actionSubmitPin = "Submit code",
    actionStartSend = "Start streaming",
    actionStopSend = "Stop streaming",
    senderHint = "Stopping a stream keeps the connection (trust and pairing stay). Pick the capture source before " +
        "connecting: switching Off ⇄ On restarts the engine and drops an established connection.",

    deckDevices = "Devices",
    devicesCountFormat = "%d connected",
    devicesEmpty = "No device connected yet. It appears here once a PC connects.",
    labelFingerprint = "Fingerprint",
    labelAddress = "Address",
    labelState = "State",
    labelTrust = "Trust",
    trustYes = "Trusted (stored on disk)",
    trustNo = "Not trusted (PIN pairing required)",
    senderSessionDisconnected = "Not connected",
    senderSessionConnected = "Connected · idle",
    senderSessionSending = "Streaming",
    peerStateIdle = "Idle",
    peerStateHandshaking = "Handshaking",
    peerStateStreaming = "Streaming",
    peerStateDegraded = "Degraded",
    peerStateReconnecting = "Reconnecting",
    peerStateFailed = "Failed",

    deckDiagnostics = "Diagnostics",
    diagnosticsSummary = "Kernel truth, collapsed by default",
    semanticsExpanded = "expanded",
    semanticsCollapsed = "collapsed",

    diagEngine = "Kernel engine (receiver)",
    diagEngineRunning = "Running (QUIC port listening)",
    diagEngineStopped = "Not started",
    diagLocalName = "Local name",
    diagLocalId = "Fingerprint",
    diagListenAddr = "Listen address",
    diagCaps = "Capabilities",
    diagCapsBoth = "receive + send",
    diagCapsReceiveOnly = "receive only (send not implemented in M1)",

    diagLowLatency = "Low latency",
    diagLowLatencyOn = "active (PERFORMANCE_MODE_LOW_LATENCY)",
    diagLowLatencyOff = "not active",
    diagPerformanceMode = "getPerformanceMode()",
    diagSampleRate = "Sample rate",
    diagChannels = "Channels",
    diagBufferRequested = "Output buffer (requested)",
    diagBufferActual = "Output buffer (effective)",
    diagFramesUnit = "frames",

    diagCounters = "Counters",
    diagUnderrunsSystem = "Underruns (track)",
    diagUnderrunsSource = "Underruns (source)",
    diagFramesWritten = "Frames written",
    diagSilenceFrames = "Silence frames",
    diagFramesDropped = "Frames dropped",
    diagWriteErrors = "Write errors",
    diagWriteErrorsDetail = "%d (last error code %d)",

    diagRing = "Playout ring (kernel PCM buffer)",
    diagRingLevel = "Level",
    diagRingOverflowTotal = "Overflow drops (total)",
    diagRingOverflowWindow = "Overflow drops (window)",
    diagRingUnderrunTotal = "Empty reads (total)",
    diagRingUnderrunWindow = "Empty reads (window)",
    diagRingWindowSeconds = "Window length",
    diagRingReset = "Reset window",
    diagRingNote = "Window delta = change since the last reset (two reads subtracted). It separates a big chunk dropped " +
        "in the past from drops happening right now. Totals are never rewritten by a reset.",

    diagQueue = "Output queue level (latency lever)",
    diagQueueCapacity = "Device capacity",
    diagQueueLevel = "Queue level",
    diagQueueTarget = "Queue target",
    diagQueueShrink = "Shrink result",
    diagQueueShrinkNone = "not attempted",
    diagQueueTargetLabel = "Lever 2 — queue target (controls fill depth, next tick)",
    diagQueueShrinkLabel = "Lever 1 — capacity shrink (setBufferSizeInFrames after play(); tap Restore to roll back)",
    leverFull = "Full",
    leverRestore = "Restore",
    diagQueueNote = "Capacity is how much can be squeezed; level is how much is held right now. Low-latency mode being " +
        "active does not mean the buffer is small: on real devices capacity was 3844 frames (80.1 ms) with a full " +
        "queue, which is when flinger reports Latency=101 ms.",

    diagSelfTest = "Kernel self-test (protocol golden vectors)",
    diagSelfTestIdle = "Not run yet. It exercises the protocol codec only — no engine or connection needed, so it works " +
        "even with the service stopped.",
    diagSelfTestPassed = "passed",
    diagSelfTestRejected = "Rejected by the kernel",
    diagSelfTestUnavailable = "Self-test could not run",
    diagSelfTestCode = "Error code",
    diagSelfTestShortName = "Short name",
    diagSelfTestDetail = "Detail",
    actionRunSelfTest = "Run self-test",
    actionSelfTesting = "Running…",

    diagExplanation = "The engine follows the service. The receiver side is initiated by the PC; until a peer connects and " +
        "streams, source underruns keep growing and the output stays silent — that is expected. Sending is above, in " +
        "Sender. Whether low latency is active and the buffer size are unaffected and readable as they are.",

    errorTitle = "Something went wrong",
    errorRecoveryPlayback = "Tap Start receiving again. If it still fails, expand Diagnostics above for the kernel text.",
    errorRecoveryGeneric = "Try again; if it keeps failing, expand Diagnostics and read the kernel text.",
    errorKernelDetail = "Kernel message (diagnostics)",
    errorPairingReadFailed = "Could not read pairing state",

    licensesTitle = "Open source licenses",
    actionBack = "Back",
    licensesBundledFormat = "Bundled with the app · %d KB",
    licensesMissing = "Notices file not found",
)

object AudioLinkStrings {
    val zh: UiStrings = Zh
    val en: UiStrings = En

    /** 系统语言 → 文案。中文（含 zh-CN / zh-TW，按语言前缀判断）说中文，其余说英文。 */
    fun forLanguage(language: String): UiStrings =
        if (language.lowercase(Locale.ROOT).startsWith("zh")) zh else en
}

/** 当前语言下的文案表；由 [com.gotkicry.audiolink.ui.screens.AudioLinkRoot] 提供。 */
val LocalStrings = staticCompositionLocalOf { AudioLinkStrings.zh }
