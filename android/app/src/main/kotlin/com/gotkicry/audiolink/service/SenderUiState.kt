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
 * @param sessionDropped 上一次已知的会话已经从内核会话表里消失（断开 / 被对端关掉）。
 *   为什么不在这里写一句中文：这条提示出现在首屏，语言跟随系统时英文界面会被它拖回中文 ——
 *   所以 service 只给**事实标记**，怎么说由界面决定（`UiStrings.senderSessionDropped`）。
 * @param peerTrusted 对端是否已在信任库中（内核 `peers()` 的 trusted）。未授信的会话**也会**出现在
 *   列表里（手机 PIN 流程中的 handshaking 会话就是 trusted=false），所以「接入即推」要显式看它。
 * @param sessionInbound 当前会话是**对方连进来的**，而不是本机 `connect()` 出去的。
 *   自动推流只服务主机方向 —— 本机主动连出去时是在当接收端，不该擅自把声音推给别人。
 * @param manualStopFor 用户**手动**停过推流的那台设备（短指纹）。规则：自动不能打脸用户。
 * @param autoStartedFor 已经**自动**发起过一次推流的设备。规则：同一台只自动发起一次 —— 失败也不重试。
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
    /** 见类注释：断开是**事实**，文案由界面出。 */
    val sessionDropped: Boolean = false,
    val peerTrusted: Boolean = false,
    val sessionInbound: Boolean = false,
    val manualStopFor: String? = null,
    val autoStartedFor: String? = null,
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
     *
     * **为什么不再看引擎在不在跑**（2026-09 真机评审的 P0）：点「连接主机」现在会自动把服务拉起来
     * 并等它就绪（见 `AudioLinkService.connectSender`），"引擎没起来"不再是用户的问题 ——
     * 曾经它把按钮变灰、把用户赶去找另一个叫「开始接收」的按钮，而那两个其实是同一件事。
     * 用户能按、按了会成，这就是全部判据。
     */
    fun canConnect(sender: SenderUiState, targetAddr: String): SendGate {
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
     * 会话表刷新后 [SenderUiState] 该怎么变；`null` = 保持不变。
     *
     * 这段逻辑原先埋在 Service 的 `syncSenderWithPeers()` 里。搬出来的理由很具体：它的早退条件
     * **直接决定主机能不能被接入**，而 Service 在 JVM 单测里跑不起来 —— "主机明明有人连进来、
     * 推流按钮却一直灰着"这种缺陷，只能在真机上点一遍才看得见，代价太高。
     *
     * 四条口径：
     *
     * 1. **还没登记过会话时，会话表里唯一的那条就是当前会话** —— 无论它是本机 `connect()` 出去的，
     *    还是**对方连进来的**。主机只等接入，接入不该被"上一次请求"的残留挡住。
     * 2. `awaitingPin` 是这里**唯一**的让路条件：那是**本机出站流程的中间态**（`connect()` 返回
     *    `1002`：握手会话已在、配对未完成）。此刻登记会把界面上的「等待输码」悄悄变成「已连接」，
     *    用户会以为配好了。配对成功后 `awaitingPin` 归零，下一拍自然登记 —— 是让路，不是丢弃
     *    （设备通道与 PIN 卡始终在如实讲同一件事）。
     * 3. **出站失败残留（`error`）不是让路条件**：它讲的是另一次尝试（"连接 192.168.1.9 失败"），
     *    与此刻这条入站会话无关。把它和 `awaitingPin` 混在同一个早退里，就会让主机被接入之后
     *    推流按钮仍然灰着、旁边还写着「先连接一台主机」。登记时顺手清掉这条残留 ——
     *    屏幕不能一边说「已连接」一边报上一次的错。
     * 4. **空会话表 ≠ 断开**：刚 `connect()` 完的一小段里 `peers()` 可能还是空的，那会儿清零
     *    会让用户看到一条假断开；只有"表里确实有会话、但我们要的那条不在"才算断开。
     *
     * 历史（2026-09-18 真机定位）：旧版这里 `?: return` 直接放行 ⇒ 需要 PIN 时 `connect()` 返回的
     * 是 `1002` 而不是成功，`peerIdShort` 一直是 null ⇒ 会话**永远登记不进来**，界面停在
     * 「未连接」，而 [sessionGate] 又按 [SendGate.PeerMismatch] 把「开始推流」禁用 ——
     * 而链路明明是通的。
     */
    fun sessionAfterPeers(sender: SenderUiState, peers: List<PeerUi>): SenderUiState? {
        val expected = sender.peerIdShort
        if (expected == null) {
            // 让路给本机自己的 PIN 流程（口径 2）。这里**只看 awaitingPin**，不看 error。
            if (sender.awaitingPin) return null
            val only = peers.singleOrNull() ?: return null
            return sender.copy(
                peerIdShort = only.idShort,
                peerState = only.state,
                peerStateLabel = only.stateLabel,
                peerTrusted = only.trusted,
                // 走到这一支就说明会话是**从会话表里采纳来的**：没有本机 connect 的返回，
                // 那就只能是对方连进来的 —— 「接入即推」只认这一种会话（见 shouldAutoStartSend）。
                sessionInbound = true,
                note = null,
                error = null,
                // 会话回来了，断开提示必须退场 —— 否则屏幕会同时挂着「已连接」与「连接已断开」。
                sessionDropped = false,
            )
        }
        if (peers.isEmpty()) return null
        val peer = peers.firstOrNull { it.idShort.equals(expected, ignoreCase = true) }
        return if (peer == null) {
            sender.copy(
                peerIdShort = null,
                peerState = "",
                peerStateLabel = "",
                peerTrusted = false,
                sessionInbound = false,
                sending = false,
                awaitingPin = false,
                note = null,
                sessionDropped = true,
            )
        } else {
            // 注意 manualStopFor / autoStartedFor **不清**：它们记的是"这台设备"，不是"这条会话"，
            // 断开重连后短指纹不变，所以"停过一次就不再自动"的承诺要跨会话成立。
            sender.copy(
                peerState = peer.state,
                peerStateLabel = peer.stateLabel,
                peerTrusted = peer.trusted,
                sessionDropped = false,
            )
        }
    }

    /** 内核会话状态字面量：会话已建立、没有流在跑 —— 这正是「可以开始推」的那一刻。 */
    const val PEER_STATE_IDLE: String = "idle"

    /**
     * 要不要**自动**开始推流（「接入即推」）。纯判定，不改任何状态。
     *
     * 这是全套逻辑里最容易打脸用户的一条自动行为，所以它被六道条件围着，一道都不可省：
     *
     * 1. **只服务主机方向**（[SenderUiState.sessionInbound]）：对方连进来才自动。本机主动 `connect()`
     *    出去时正在当接收端 —— 不该擅自把声音推给别人。
     * 2. **只认 `idle`**（[PEER_STATE_IDLE]）：`failed` **不自动重试**（否则就是错误风暴）；
     *    `handshaking`（PIN 还没输完）等它自己变成 idle 再说。
     * 3. **必须是已授信会话**（[SenderUiState.peerTrusted]）：未授信的会话也会出现在列表里，
     *    不能因为"它在表里"就开始推。
     * 4. **用户手动停过的设备永不自动重启**（[SenderUiState.manualStopFor]）：这是"关掉自动"的
     *    正规出口 —— 比一个总开关更精准（用户停掉哪台，就是那台不再自动）。
     * 5. **同一台设备只自动发起一次**（[SenderUiState.autoStartedFor]）：既防事件抖动造成的重复
     *    调用，也顺带保证了"失败不重试"。
     * 6. 采集源必须已经选好，其余门禁沿用 [canStartSend]（引擎在跑、不在 awaitingPin、
     *    没在推流、采集不在停止中）。**没选发送源就不自动**：那样推过去只有连接没有声音，
     *    等于白让对端等一场。
     *
     * 将来若要加「自动推流」总开关，落点就是这里的第一行（加一个布尔字段，为 false 时直接返回）——
     * 本轮按需求先默认开、不加开关，所以没有这个字段。
     */
    fun shouldAutoStartSend(
        sender: SenderUiState,
        engineRunning: Boolean,
        captureSelection: CaptureSourceKind?,
        captureState: CaptureState,
    ): Boolean {
        if (!sender.sessionInbound) return false
        if (sender.peerState != PEER_STATE_IDLE) return false
        if (!sender.peerTrusted) return false
        val peer = sender.peerIdShort ?: return false
        if (sender.manualStopFor == peer) return false
        if (sender.autoStartedFor == peer) return false
        if (captureSelection == null) return false
        return canStartSend(sender, engineRunning, captureSelection, captureState) == SendGate.Allowed
    }

    /**
     * 自动推流**发起之前**先记账（同一台设备只自动一次）。
     *
     * 为什么必须"先记账、再发起"：`startSender()` 是异步的，从发起到 `sending = true` 之间隔着
     * 好几拍状态刷新，不先记账的话每一拍都会再触发一次 —— 自动就变成了刷屏。
     */
    fun afterAutoStartArmed(sender: SenderUiState): SenderUiState =
        sender.copy(autoStartedFor = sender.peerIdShort)

    /**
     * 用户手动停流之后记账：这台设备本轮不再自动开始。
     *
     * 记的是**设备**（短指纹）而不是"这一次会话"：用户表达的是"别再给我自动了"，
     * 而设备重连后短指纹不变，所以停一次就一直有效。
     */
    fun afterManualStop(sender: SenderUiState): SenderUiState =
        sender.copy(manualStopFor = sender.peerIdShort)

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
