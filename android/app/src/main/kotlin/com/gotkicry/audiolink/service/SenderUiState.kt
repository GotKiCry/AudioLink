package com.gotkicry.audiolink.service

import com.gotkicry.audiolink.capture.CaptureSourceKind
import com.gotkicry.audiolink.capture.CaptureState

/**
 * 发送方向（本机 → 对端）的 UI 快照。
 *
 * 为什么单独一个模型而不是把字段摊进 [PlaybackUiState]：与 [PairingUiState] 的分层立场一致 ——
 * 「连没连上、能不能发、发没发」是三个独立问题，混进播放面板后每个判断都要先跨过一堆播放字段。
 * 映射与门禁判定全部放在 [SenderStateMapper]（**纯逻辑，JVM 可测**），服务只负责调 FFI 与发布。
 *
 * @param targetAddr 用户输入的目标地址（`192.168.1.5` 或 `192.168.1.5:58290`；端口缺省见内核 `parse_addr`）。
 * @param connecting `connect()` 正在进行（按钮置灰用）。
 * @param peerIdShort 已建立的发送目标（`connect()` 返回的对端短指纹）；`null` = 还没有会话。
 * @param peerState 内核会话状态原始字面量（`idle`/`handshaking`/`streaming`/…）。
 * @param peerStateLabel 该状态的中文标签（复用 [PairingStateMapper.stateLabel]，未知状态原样透传）。
 * @param sending 本机是否已发出「开始推流」并成功建立流（见 [SenderStateMapper.isSending] 的口径说明）。
 * @param awaitingPin 内核要求本机**输入**对端屏幕上的 6 位 PIN（`connect()` 返回 `1002`）——**不是错误**。
 * @param note 提示（非错误）：例如「请输入电脑上显示的 6 位数字」「正在等待授权，授权后会自动开始发送」。
 * @param error 失败（人话）。与 [note] 分开：提示不是失败，混在一起会让用户以为出错了。
 */
data class SenderUiState(
    val targetAddr: String = "",
    val connecting: Boolean = false,
    val peerIdShort: String? = null,
    val peerState: String = "",
    val peerStateLabel: String = "",
    val sending: Boolean = false,
    val awaitingPin: Boolean = false,
    val note: String? = null,
    val error: String? = null,
) {
    /** 是否已有会话（能往里开始推流的前提）。 */
    val hasSession: Boolean get() = peerIdShort != null
}

/** 发送动作的门禁结果 —— 禁用原因本身就是文案来源，不再另写一套判断。 */
internal enum class SendGate {
    Allowed,
    EngineDown,
    Connecting,
    AddressEmpty,
    AddressInvalid,
    NoSession,
    AwaitingPin,
    AlreadySending,
    NotSending,
    /** 发送源 = 关闭（用户没选内录/麦克风）：先选源，否则推过去只有连接没有声音。 */
    CaptureOff,
    /** 采集正在停止：等它停完再判断（短暂的过渡态）。 */
    CaptureStopping,
    /** 刚连接返回的对端与当前会话表对不上（单对端语义的兜底，见 [SenderStateMapper.peerMatches]）。 */
    PeerMismatch,
}

/**
 * 发送接线的**纯逻辑**：地址校验、动作门禁、文案、以及两个跨状态联动判定。
 *
 * 这里的每一条都来自内核/FFI 的既有语义（不新造状态机）：
 *
 * 1. **`connect()` 的 `1002 NOT_PAIRED` 不是失败**：FFI 文档明写「QUIC 握手与会话已经在，UI 提示用户输入 PIN
 *    后调用 `submit_pin` 即可继续同一条连接」（`core/crates/audiolink-ffi/src/engine_bridge.rs` 的 `connect`）。
 * 2. **`startSend()` / `stopSend()` / `submitPin()` 都无 peer 参数**：内核 `current_peer()` 取「优先 streaming、
 *    否则第一个；一个都没有则 `1002`」—— 契约 §8 的单对端 UI 语义（Android M1 一次只连一台 PC）。
 * 3. **`stopSend()` 只停流、保持连接**：内核文档原文「关闭与对端的流（**保留连接与信任**）」
 *    （`audiolink-engine` 的 `Engine::stop_send`）—— 所以「停止发送」不等于「断开」，文案必须分开写。
 * 4. **「会话已建 + 采集尚未供数」是一个可见但不致命的中间态**：采集回调返回空时，FFI 侧只是
 *    `read_timeouts += 1`（`core/crates/audiolink-ffi/src/audio_bridge.rs` 的 `KotlinCaptureSource::read`，
 *    契约原文「返回空 = 暂无数据，不要忙等返回 0」），内核**不报错、不断流**；对端此刻的表现是
 *    「连着但没声音」。这一条是刻意接受的：授权回执马上会到，不该让用户为了几百毫秒反复点按。
 */
internal object SenderStateMapper {

    /** 协议 §2.3 的 `1002 NOT_PAIRED`。**不是连接失败**：会话与命令通道仍在，输入 PIN 后同一条连接继续。 */
    const val NOT_PAIRED_CODE: Int = 1002

    /** 端口缺省提示用（内核 `parse_addr` 缺省走 QUIC 默认端口）。 */
    const val DEFAULT_PORT_HINT: Int = 58_290

    /**
     * 目标地址的**格式**校验（只判本地能判的部分；地址是否可达由 `connect()` 说了算）。
     *
     * 口径与内核 `parse_addr` 对齐：接受 `ip` 或 `ip:port`，只有 `ip` 时用默认 QUIC 端口。
     * 这里判格式是为了**在按下连接之前**就能给出人话原因（避免一次必然失败的网络调用）。
     */
    fun addressGate(raw: String): SendGate {
        val text = raw.trim()
        if (text.isEmpty()) return SendGate.AddressEmpty
        val (host, portText) = if (text.count { it == ':' } == 1) {
            text.substringBefore(':') to text.substringAfter(':')
        } else {
            text to null
        }
        if (host.isEmpty() || host.any { it.isWhitespace() }) return SendGate.AddressInvalid
        if (portText != null) {
            val port = portText.toIntOrNull()
            if (port == null || port !in 1..65_535) return SendGate.AddressInvalid
        }
        return SendGate.Allowed
    }

    /**
     * 能否发起连接。
     *
     * `targetAddr` 必须是**输入框里的当前值**，不是 `sender.targetAddr`。
     * 为什么（2026-09-18 真机踩到）：`sender.targetAddr` 是**服务侧**的回填副本，只在 `connect()`
     * 被触发之后才有值。用它判门禁会形成**死锁** —— 地址还没进服务 → 按钮禁用 → 连接触发不了 →
     * 服务侧永远是空 → 按钮永远点不动，而界面还挂着「先填电脑的地址」。
     * 门禁判的是「用户现在能不能按」，输入框就是唯一的事实来源。
     */
    fun canConnect(sender: SenderUiState, engineRunning: Boolean, targetAddr: String): SendGate {
        if (!engineRunning) return SendGate.EngineDown
        if (sender.connecting) return SendGate.Connecting
        return addressGate(targetAddr)
    }

    /**
     * 能否开始推流 —— 本任务的判定核心。
     *
     * 四层顺序（先引擎、再会话、再流、最后采集），任一层不过就给出**那一条**原因：
     * 用户要看到的是「下一步该做什么」，不是一串条件。
     */
    fun canStartSend(
        sender: SenderUiState,
        engineRunning: Boolean,
        captureSelection: CaptureSourceKind?,
        captureState: CaptureState,
    ): SendGate {
        if (!engineRunning) return SendGate.EngineDown
        if (sender.connecting) return SendGate.Connecting
        if (!sender.hasSession) return SendGate.NoSession
        if (sender.awaitingPin) return SendGate.AwaitingPin
        if (sender.sending) return SendGate.AlreadySending
        if (captureSelection == null) return SendGate.CaptureOff
        // 采集已经在收尾：等它停完再判断（否则会出现「推流起来了、采集刚好被停」的空窗）。
        if (captureState == CaptureState.Stopping) return SendGate.CaptureStopping
        return SendGate.Allowed
    }

    /** 能否停止推流。 */
    fun canStopSend(sender: SenderUiState, engineRunning: Boolean): SendGate {
        if (!engineRunning) return SendGate.EngineDown
        if (!sender.sending) return SendGate.NotSending
        return SendGate.Allowed
    }

    /** 禁用原因 → 人话（[SendGate.Allowed] 没有禁用原因，返回 `null`）。 */
    fun gateNote(gate: SendGate): String? = when (gate) {
        SendGate.Allowed -> null
        SendGate.EngineDown -> "先启动服务（引擎还没起来）"
        SendGate.Connecting -> "正在连接，请稍候"
        SendGate.AddressEmpty -> "先填电脑的地址（例如 192.168.1.5）"
        SendGate.AddressInvalid -> "地址格式不对：应是 192.168.1.5 或 192.168.1.5:$DEFAULT_PORT_HINT"
        SendGate.NoSession -> "先连接一台电脑，再开始发送"
        SendGate.AwaitingPin -> "请输入电脑上显示的 6 位数字"
        SendGate.AlreadySending -> "已经在发送了"
        SendGate.NotSending -> "当前没有在发送"
        SendGate.CaptureOff -> "先选择发送源（系统内录 / 麦克风）"
        SendGate.CaptureStopping -> "采集正在停止，稍后再试"
        SendGate.PeerMismatch -> "当前会话不是刚才连接的那台设备，请重新连接"
    }

    /**
     * 允许推流、但采集还没到 `Running` 时的**补充提示**。
     *
     * 必须写明「授权后会**自动**开始发送」：否则用户会以为点错了、反复点按（这一条是明确要求）。
     */
    fun captureNote(captureSelection: CaptureSourceKind?, captureState: CaptureState): String? {
        if (captureSelection == null) return null
        return when (captureState) {
            CaptureState.Running -> null
            CaptureState.AwaitingPermission, CaptureState.Starting, CaptureState.Idle ->
                "正在等待授权，授权后会自动开始发送"
            CaptureState.Stopping -> null // 该状态本身会被 [canStartSend] 拦成禁用，不需要补充提示。
            CaptureState.Failed -> "采集当前出错；恢复后会自动开始发送（对端会先看到连接、暂时没有声音）"
        }
    }

    /** `1002` 走提示（notice）而不是错误：它不是失败，是「该你输码了」。 */
    fun connectFailureIsNotice(code: Int): Boolean = code == NOT_PAIRED_CODE

    /** 连接失败的文案。`1002` 是引导输入 PIN；其它是失败 + 可否重试。 */
    fun noteForConnectFailure(code: Int, detail: String?): String {
        val suffix = detail?.trim().orEmpty()
        return when {
            connectFailureIsNotice(code) -> "请输入电脑上显示的 6 位数字（60 秒内有效）"
            code > 0 -> "连接失败（$code）${if (suffix.isEmpty()) "" else "：$suffix"}；可以再试一次"
            else -> "连接失败${if (suffix.isEmpty()) "" else "：$suffix"}；可以再试一次"
        }
    }

    /** 开始/停止推流失败的文案。 */
    fun noteForSendFailure(code: Int, detail: String?): String {
        val suffix = detail?.trim().orEmpty()
        val reason = if (code > 0) "$code${if (suffix.isEmpty()) "" else "：$suffix"}" else suffix
        return if (reason.isEmpty()) "发送操作失败；请检查连接后重试" else "发送操作失败（$reason）"
    }

    /**
     * 推流中把采集源切到「关闭」时的联动：**必须先停流**，否则对端会一直等一个永远不会到的流。
     *
     * 注意这里只回答「要不要停」；「停完是否断连」由内核语义决定 —— `stopSend()` 只停流、
     * **保留连接与信任**（见类注释第 3 条），所以文案是「已停止发送」而不是「已断开」。
     */
    fun shouldStopSendOnCaptureChange(sending: Boolean, next: CaptureSourceKind?): Boolean =
        sending && next == null

    /**
     * 会话门禁：`startSend()` **不带 peer 参数**，内核按 `current_peer()` 选对端 —— 所以本机必须
     * 确认「当前会话就是刚连接的那一台」。
     *
     * 两种例外要说清（否则会把正常流程挡掉，或者静默发错）：
     * * **列表为空** → 放行。刚 `connect()` 完的一小段里 `peers()` 可能还没拉到，
     *   这段窗口里几乎只可能是刚连上的那一台；硬拦会让「连上就发」变成偶发报错。
     * * **列表有内容但对不上** → [SendGate.PeerMismatch] **报错**。这是单对端语义最危险的地方：
     *   本机若同时还有一条入站会话（别人连我），`current_peer()` 可能选中它 ——
     *   宁可让用户看到一句人话，也不能把声音发到错误的设备上。
     */
    fun sessionGate(expectedIdShort: String?, peers: List<PeerUi>): SendGate = when {
        peers.isEmpty() -> SendGate.Allowed
        peerMatches(expectedIdShort, peers) -> SendGate.Allowed
        else -> SendGate.PeerMismatch
    }

    /**
     * `connect()` 返回的对端是否仍是会话表里那一条。
     *
     * 为什么需要：`startSend()` / `stopSend()` / `submitPin()` **都不带 peer 参数**，内核按
     * `current_peer()`（优先 streaming、否则第一个）选对端 —— 若本机同时还有一条**入站**会话
     * （别人连我），就可能发到错误的设备上。宁可让用户看到一句人话，也不能静默发错。
     */
    fun peerMatches(expectedIdShort: String?, peers: List<PeerUi>): Boolean {
        val expected = expectedIdShort?.trim().orEmpty()
        if (expected.isEmpty()) return false
        return peers.any { it.idShort.equals(expected, ignoreCase = true) }
    }

    /**
     * 「正在发送」的显示口径：**本机意图**（最后一次 `startSend` 成功且还没 `stopSend`）× 会话仍在。
     *
     * 为什么不用遥测反推：内核没有「本机是否在推流」的直接查询（`peers()` 只给会话状态），
     * 而发帧计数在「连着但没数据」的中间态（见类注释第 4 条）同样是 0 —— 用它会把这个中间态
     * 显示成「没在发送」，与用户刚点的动作不符。会话一旦消失（断开/重连）这里自动归零。
     */
    fun isSending(localStarted: Boolean, sender: SenderUiState): Boolean =
        localStarted && sender.hasSession

    /** 会话一行字：把「已连接 / 发送中」分开说清（`stopSend` 之后是「已停止发送」而不是「已断开」）。 */
    fun sessionLabel(sender: SenderUiState): String {
        if (!sender.hasSession) return "未连接"
        val label = sender.peerStateLabel.ifEmpty { sender.peerState.ifEmpty { "已连接" } }
        return if (sender.sending) "$label · 发送中" else "$label · 未发送"
    }
}
