package com.gotkicry.audiolink.service

import com.gotkicry.audiolink.capture.CaptureSourceKind
import com.gotkicry.audiolink.capture.CaptureState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 发送接线（连接 + 推流）的**纯逻辑**测试。
 *
 * 断言一律用字面量（不引用被测实现里的常量/推导）—— 第 107 轮的教训：用实现常量写断言，
 * 阈值写错时测试永远绿。例如 `1002` 在这里是**字面量**，而 [SenderStateMapper] 里那份是另一处定义，
 * 错了会被下面「1009 不该被当成提示」那条抓住。
 */
class SenderStateMapperTest {

    private fun peer(id: String, state: String = "streaming") = PeerUi(
        idShort = id,
        name = "PC",
        addr = "192.168.1.5:58290",
        state = state,
        stateLabel = PairingStateMapper.stateLabel(state),
        trusted = true,
    )

    private fun sender(
        addr: String = "192.168.1.5",
        connecting: Boolean = false,
        peerIdShort: String? = null,
        sending: Boolean = false,
        awaitingPin: Boolean = false,
    ) = SenderUiState(
        targetAddr = addr,
        connecting = connecting,
        peerIdShort = peerIdShort,
        sending = sending,
        awaitingPin = awaitingPin,
    )

    // ---- 地址格式 ----

    @Test
    fun acceptsBareIpAndIpWithPort() {
        assertEquals(SendGate.Allowed, SenderStateMapper.addressGate("192.168.1.5"))
        assertEquals(SendGate.Allowed, SenderStateMapper.addressGate("192.168.1.5:58290"))
        assertEquals("两侧空白应被忽略", SendGate.Allowed, SenderStateMapper.addressGate("  192.168.1.5  "))
    }

    @Test
    fun rejectsEmptyAndMalformedAddress() {
        assertEquals(SendGate.AddressEmpty, SenderStateMapper.addressGate(""))
        assertEquals(SendGate.AddressEmpty, SenderStateMapper.addressGate("   "))
        assertEquals(SendGate.AddressInvalid, SenderStateMapper.addressGate("192.168.1.5:0"))
        assertEquals(SendGate.AddressInvalid, SenderStateMapper.addressGate("192.168.1.5:99999"))
        assertEquals(SendGate.AddressInvalid, SenderStateMapper.addressGate("192.168.1.5:abc"))
        assertEquals(SendGate.AddressInvalid, SenderStateMapper.addressGate("192.168.1.5 5"))
    }

    // ---- 连接门禁 ----

    /**
     * 真机评审 P0：服务没起来**不再**是连接的禁用原因。
     *
     * 点「连接主机」现在会自动把服务拉起来（`AudioLinkService.connectSender` 里的就绪等待），
     * 所以"引擎没起来"是应用自己的事，不是用户的事。曾经它把按钮变灰，把用户赶去找另一个
     * 叫「开始接收」的按钮 —— 而那两个按钮做的是同一件事。
     */
    @Test
    fun connectIsNeverBlockedByTheEngineBeingDown() {
        assertEquals(
            "填了合法地址就必须能按，哪怕服务一个都没启动",
            SendGate.Allowed,
            SenderStateMapper.canConnect(sender(), targetAddr = "192.168.1.5"),
        )
        assertFalse(
            "连接门禁里不该再出现 EngineDown 这个原因",
            SenderStateMapper.canConnect(sender(), targetAddr = "192.168.1.5") == SendGate.EngineDown,
        )
    }

    @Test
    fun connectBlockedWhileConnectingAndWhenAddressBad() {
        assertEquals(
            SendGate.Connecting,
            SenderStateMapper.canConnect(sender(connecting = true), targetAddr = "192.168.1.5"),
        )
        assertEquals(
            SendGate.AddressEmpty,
            SenderStateMapper.canConnect(sender(), targetAddr = ""),
        )
        assertEquals(
            SendGate.Allowed,
            SenderStateMapper.canConnect(sender(), targetAddr = "192.168.1.5"),
        )
    }

    /**
     * 真机回归（2026-09-18）：门禁曾经读 `sender.targetAddr`（**服务侧**那份副本），而它只在
     * `connect()` 被触发之后才由服务回填 —— 于是形成死锁：地址进了输入框但没进服务 → 按钮禁用 →
     * 连接触发不了 → 服务侧永远是空 → 按钮永远点不动。
     * 真机现象：用户填好地址后按钮点不动，界面还挂着「先填电脑的地址（例如 192.168.1.5）」。
     */
    @Test
    fun connectGateReadsTheTextFieldNotTheServiceCopy() {
        val fresh = sender(addr = "")
        // 注意 assertEquals 的三参形式是 (message, expected, actual) —— message 在**第一位**。
        assertEquals(
            "输入框里有合法地址就必须放行 —— 不能等服务的副本先知道",
            SendGate.Allowed,
            SenderStateMapper.canConnect(fresh, targetAddr = "192.168.3.200"),
        )
        assertEquals(
            SendGate.AddressEmpty,
            SenderStateMapper.canConnect(fresh, targetAddr = "   "),
        )
    }

    // ---- 推流门禁（本任务的核心判定）----

    @Test
    fun startBlockedWithoutEngineSessionOrCapture() {
        val connected = sender(peerIdShort = "aaaa0001")
        assertEquals(
            SendGate.EngineDown,
            SenderStateMapper.canStartSend(connected, engineRunning = false, captureSelection = CaptureSourceKind.Microphone, captureState = CaptureState.Running),
        )
        assertEquals(
            "还没连上电脑时不能开始发送",
            SendGate.NoSession,
            SenderStateMapper.canStartSend(sender(), engineRunning = true, captureSelection = CaptureSourceKind.Microphone, captureState = CaptureState.Running),
        )
        assertEquals(
            "发送源为「关闭」时不能开始发送（推过去只有连接没有声音）",
            SendGate.CaptureOff,
            SenderStateMapper.canStartSend(connected, engineRunning = true, captureSelection = null, captureState = CaptureState.Idle),
        )
    }

    @Test
    fun startBlockedWhileAwaitingPinOrAlreadySending() {
        val awaiting = sender(peerIdShort = "aaaa0001", awaitingPin = true)
        assertEquals(
            SendGate.AwaitingPin,
            SenderStateMapper.canStartSend(awaiting, engineRunning = true, captureSelection = CaptureSourceKind.Microphone, captureState = CaptureState.Running),
        )
        val sending = sender(peerIdShort = "aaaa0001", sending = true)
        assertEquals(
            SendGate.AlreadySending,
            SenderStateMapper.canStartSend(sending, engineRunning = true, captureSelection = CaptureSourceKind.Microphone, captureState = CaptureState.Running),
        )
    }

    @Test
    fun startBlockedWhileCaptureIsStopping() {
        assertEquals(
            SendGate.CaptureStopping,
            SenderStateMapper.canStartSend(
                sender(peerIdShort = "aaaa0001"),
                engineRunning = true,
                captureSelection = CaptureSourceKind.Microphone,
                captureState = CaptureState.Stopping,
            ),
        )
    }

    @Test
    fun startAllowedWhenCaptureIsRunning() {
        assertEquals(
            SendGate.Allowed,
            SenderStateMapper.canStartSend(
                sender(peerIdShort = "aaaa0001"),
                engineRunning = true,
                captureSelection = CaptureSourceKind.Microphone,
                captureState = CaptureState.Running,
            ),
        )
    }

    @Test
    fun startAllowedWhileWaitingForCaptureAuthorization() {
        // 用户意图明确（已选源）、授权回执马上到 —— 不该把他卡在这里反复点。
        listOf(CaptureState.AwaitingPermission, CaptureState.Starting, CaptureState.Idle).forEach { state ->
            assertEquals(
                "采集处于 $state 时应允许开始发送（配合提示）",
                SendGate.Allowed,
                SenderStateMapper.canStartSend(
                    sender(peerIdShort = "aaaa0001"),
                    engineRunning = true,
                    captureSelection = CaptureSourceKind.SystemLoopback,
                    captureState = state,
                ),
            )
        }
    }

    @Test
    fun startAllowedWhenCaptureFailedButSourceChosen() {
        // 采集出错（例如权限被拒）不禁用发送：用户已明确选源，禁用只会让他困惑；
        // 提示会说清「恢复后会自动开始发送」。对端此刻看到的是「连着但没声音」，属可接受中间态
        // （内核侧只表现为采集回调返回空 → read_timeouts 增长，不报错、不断流）。
        assertEquals(
            SendGate.Allowed,
            SenderStateMapper.canStartSend(
                sender(peerIdShort = "aaaa0001"),
                engineRunning = true,
                captureSelection = CaptureSourceKind.Microphone,
                captureState = CaptureState.Failed,
            ),
        )
    }

    // ---- 文案 ----

    @Test
    fun waitingForAuthorizationTellsUserSendingStartsAutomatically() {
        val note = SenderStateMapper.captureNote(CaptureSourceKind.SystemLoopback, CaptureState.AwaitingPermission)
        assertNotNull(note)
        assertTrue("必须说清会自动开始：$note", note!!.contains("自动开始发送"))
        assertTrue("必须点明在等授权：$note", note.contains("授权"))
        assertNull("采集正常运行时不需要补充提示", SenderStateMapper.captureNote(CaptureSourceKind.Microphone, CaptureState.Running))
        assertNull("发送源关闭时不提示", SenderStateMapper.captureNote(null, CaptureState.Idle))
        val failed = SenderStateMapper.captureNote(CaptureSourceKind.Microphone, CaptureState.Failed)
        assertNotNull(failed)
        assertTrue("采集失败要说明会恢复后自动开始：$failed", failed!!.contains("恢复后会自动开始发送"))
    }

    @Test
    fun connectNoticeDistinguishesPinFromFailure() {
        // 1002 是提示（该你输码），不是失败 —— 文案里不该出现「失败」二字。
        assertTrue(SenderStateMapper.connectFailureIsNotice(1002))
        assertFalse("别的错误码不能被当成提示", SenderStateMapper.connectFailureIsNotice(1009))
        assertFalse(SenderStateMapper.connectFailureIsNotice(0))

        val pinNotice = SenderStateMapper.noteForConnectFailure(1002, "peer requires pin pairing")
        assertTrue("要引导用户输码：$pinNotice", pinNotice.contains("6 位数字"))
        assertFalse("1002 不是失败：$pinNotice", pinNotice.contains("失败"))

        val realFailure = SenderStateMapper.noteForConnectFailure(1009, "busy")
        assertTrue("真失败要说明可重试：$realFailure", realFailure.contains("失败"))
        assertTrue("要带上可重试的语义：$realFailure", realFailure.contains("可以再试"))
    }

    @Test
    fun sendFailureIsWordedAsFailure() {
        val note = SenderStateMapper.noteForSendFailure(1002, "no peer session")
        assertTrue("推流失败就是失败：$note", note.contains("失败"))
        assertTrue("要带上错误码便于排查：$note", note.contains("1002"))
        assertTrue(
            "没有错误码时也要给人话：${SenderStateMapper.noteForSendFailure(0, "")}",
            SenderStateMapper.noteForSendFailure(0, "").isNotEmpty(),
        )
    }

    @Test
    fun gateNotesAreHumanReadable() {
        assertNull("允许时没有禁用原因", SenderStateMapper.gateNote(SendGate.Allowed))
        assertTrue(SenderStateMapper.gateNote(SendGate.CaptureOff)!!.contains("发送源"))
        assertTrue(SenderStateMapper.gateNote(SendGate.NoSession)!!.contains("连接"))
        assertTrue(SenderStateMapper.gateNote(SendGate.AwaitingPin)!!.contains("6 位数字"))
        assertTrue(SenderStateMapper.gateNote(SendGate.PeerMismatch)!!.contains("重新连接"))
    }

    // ---- 两个跨状态联动 ----

    @Test
    fun switchingCaptureOffStopsSendingFirst() {
        assertTrue(
            "推流中把源切到「关闭」必须先停流",
            SenderStateMapper.shouldStopSendOnCaptureChange(sending = true, next = null),
        )
        assertFalse(
            "换到另一个源不用停流（共用同一个出口）",
            SenderStateMapper.shouldStopSendOnCaptureChange(sending = true, next = CaptureSourceKind.Microphone),
        )
        assertFalse(SenderStateMapper.shouldStopSendOnCaptureChange(sending = false, next = null))
    }

    @Test
    fun sessionGateRejectsMismatchedPeer() {
        val peers = listOf(peer("bbbb0002"), peer("cccc0003"))
        assertEquals(
            "会话表里有别的对端、却没有我们连的那台 → 报错而不是静默发错",
            SendGate.PeerMismatch,
            SenderStateMapper.sessionGate("aaaa0001", peers),
        )
        assertEquals(
            "连的这台还在 → 放行",
            SendGate.Allowed,
            SenderStateMapper.sessionGate("bbbb0002", peers),
        )
        assertEquals(
            "还没拉到列表时放行（那段窗口里几乎只可能是刚连上的那台）",
            SendGate.Allowed,
            SenderStateMapper.sessionGate("aaaa0001", emptyList()),
        )
        assertEquals(
            "没记录过对端却有会话 → 不能猜",
            SendGate.PeerMismatch,
            SenderStateMapper.sessionGate(null, peers),
        )
    }

    // ---- 入站会话的登记（主机只等接入）----

    /**
     * 缺陷回归（task-10）：先有一次**失败的出站连接**（error 残留在状态里），随后**对方连进来**。
     *
     * 旧口径把 `error != null` 与 `awaitingPin` 塞进同一个早退条件 ⇒ 入站会话登记不上：
     * 主机明明已经被接入，推流按钮却一直灰着、旁边写着「先连接一台主机」。
     * 规则是**主机只等接入，接入不该被上一次请求的残留阻断**。
     */
    @Test
    fun inboundSessionIsAdoptedEvenAfterAFailedOutboundAttempt() {
        val afterFailedConnect = sender().copy(error = "连接失败（1001）；可以再试一次")
        val inbound = listOf(peer("aaaa0001", state = "streaming"))

        val adopted = SenderStateMapper.sessionAfterPeers(afterFailedConnect, inbound)
        assertNotNull("对方连进来就该被登记", adopted)
        assertEquals("aaaa0001", adopted!!.peerIdShort)
        assertNull("上一次出站失败的残留必须清掉：屏幕不能一边说已连接一边报上一次的错", adopted.error)
        assertTrue("登记之后必须真的算「有会话」", adopted.hasSession)

        // 门禁放行 —— 缺陷里"按钮一直灰着"就发生在这一处。
        assertEquals(
            SendGate.Allowed,
            SenderStateMapper.canStartSend(
                sender = adopted,
                engineRunning = true,
                captureSelection = CaptureSourceKind.Microphone,
                captureState = CaptureState.Running,
            ),
        )
    }

    /** pending 的 PIN 流程是**唯一**的让路条件：配对没完成就登记，会把「等待输码」说成「已连接」。 */
    @Test
    fun pendingPinKeepsTheSessionUnadopted() {
        assertNull(SenderStateMapper.sessionAfterPeers(sender(awaitingPin = true), listOf(peer("aaaa0001"))))
    }

    /** 会话表里有多个对端时不猜（单对端语义）：宁可让用户重新连接，也不能把声音发到错误的设备上。 */
    @Test
    fun adoptionRefusesToGuessWhenSeveralPeersArePresent() {
        assertNull(
            SenderStateMapper.sessionAfterPeers(
                sender(),
                listOf(peer("aaaa0001"), peer("bbbb0002")),
            ),
        )
    }

    /** 断开：登记过的对端不在表里了 → 清状态并留一个事实标记（文案由界面按语言出）。 */
    @Test
    fun adoptedSessionIsClearedWhenItLeavesTheTable() {
        val gone = SenderStateMapper.sessionAfterPeers(
            sender(peerIdShort = "aaaa0001").copy(sending = true),
            listOf(peer("bbbb0002")),
        )
        assertNotNull(gone)
        assertNull("会话没了，登记也就没了", gone!!.peerIdShort)
        assertFalse("会话没了就不该还在推流", gone.sending)
        assertTrue("断开是事实，界面据此出「与主机的连接已断开」", gone.sessionDropped)
    }

    /** 空表不是断开：刚 connect 完的一小段里 peers() 可能还是空的，那时清零会让用户看到假断开。 */
    @Test
    fun emptyPeerTableIsNotTreatedAsADisconnect() {
        assertNull(SenderStateMapper.sessionAfterPeers(sender(peerIdShort = "aaaa0001"), emptyList()))
        assertNull(SenderStateMapper.sessionAfterPeers(sender(), emptyList()))
    }

    /** 会话回来了，断开提示必须退场（否则「已连接」与「连接已断开」会同时挂在屏幕上）。 */
    @Test
    fun reconnectClearsTheDisconnectFlag() {
        val dropped = sender().copy(sessionDropped = true)
        val reconnected = SenderStateMapper.sessionAfterPeers(dropped, listOf(peer("aaaa0001")))
        assertNotNull(reconnected)
        assertFalse(reconnected!!.sessionDropped)
        assertEquals("aaaa0001", reconnected.peerIdShort)
    }

    /** 会话仍在表里时只刷新状态：不该把断开标记又点亮。 */
    @Test
    fun existingSessionOnlyRefreshesPeerState() {
        val alive = SenderStateMapper.sessionAfterPeers(
            sender(peerIdShort = "aaaa0001").copy(peerState = "handshaking", sessionDropped = true),
            listOf(peer("aaaa0001", state = "streaming")),
        )
        assertNotNull(alive)
        assertEquals("streaming", alive!!.peerState)
        assertFalse(alive.sessionDropped)
    }

    @Test
    fun sendingStopsWhenSessionDisappears() {
        val gone = sender(peerIdShort = null, sending = false)
        assertFalse(
            "会话没了就不该显示「发送中」",
            SenderStateMapper.isSending(localStarted = true, sender = gone),
        )
        assertTrue(
            "会话还在且本机点过开始 → 发送中",
            SenderStateMapper.isSending(localStarted = true, sender = sender(peerIdShort = "aaaa0001")),
        )
    }

    @Test
    fun sessionLabelSeparatesStoppedFromDisconnected() {
        assertEquals("未连接", SenderStateMapper.sessionLabel(sender()))
        val connected = sender(peerIdShort = "aaaa0001").copy(peerState = "streaming", peerStateLabel = "已连接")
        assertTrue(
            "停止发送后是「未发送」，不是「已断开」：${SenderStateMapper.sessionLabel(connected)}",
            SenderStateMapper.sessionLabel(connected).contains("未发送"),
        )
        val sending = connected.copy(sending = true)
        assertTrue(SenderStateMapper.sessionLabel(sending).contains("发送中"))
        assertFalse(SenderStateMapper.sessionLabel(sending).contains("断开"))
    }

    @Test
    fun stopSendIsBlockedWhenNothingIsSending() {
        assertEquals(
            SendGate.NotSending,
            SenderStateMapper.canStopSend(sender(peerIdShort = "aaaa0001"), engineRunning = true),
        )
        assertEquals(
            SendGate.Allowed,
            SenderStateMapper.canStopSend(sender(peerIdShort = "aaaa0001", sending = true), engineRunning = true),
        )
        assertEquals(
            SendGate.EngineDown,
            SenderStateMapper.canStopSend(sender(peerIdShort = "aaaa0001", sending = true), engineRunning = false),
        )
    }

    // ---- 「接入即推」（自动开始推流）----

    /** 主机方向的基线状态：对方连进来（入站）、已授信、会话空闲。 */
    private fun inboundIdle(
        manualStopFor: String? = null,
        autoStartedFor: String? = null,
        peerState: String = "idle",
        trusted: Boolean = true,
    ) = sender(peerIdShort = "aaaa0001").copy(
        sessionInbound = true,
        peerTrusted = trusted,
        peerState = peerState,
        manualStopFor = manualStopFor,
        autoStartedFor = autoStartedFor,
    )

    private fun autoStartWanted(
        state: SenderUiState,
        engineRunning: Boolean = true,
        capture: CaptureSourceKind? = CaptureSourceKind.Microphone,
        captureState: CaptureState = CaptureState.Running,
    ) = SenderStateMapper.shouldAutoStartSend(state, engineRunning, capture, captureState)

    /** 规则 1/2/3 的正面路径：入站 + 授信 + idle + 采集就绪 → 自动。 */
    @Test
    fun autoStartFiresForAnInboundIdleTrustedSession() {
        assertTrue("对方接入且一切就绪时就该自动开始", autoStartWanted(inboundIdle()))
    }

    /** 主机方向：本机**主动连出去**的会话绝不自动推流（那会儿本机是接收端，不该把声音推给别人）。 */
    @Test
    fun autoStartNeverFiresForOutboundSessions() {
        val outbound = sender(peerIdShort = "aaaa0001").copy(
            sessionInbound = false,
            peerTrusted = true,
            peerState = "idle",
        )
        assertFalse("出站会话不参与「接入即推」", autoStartWanted(outbound))
    }

    /** 刷新阶段不能把出站会话改判成入站 —— 否则自动推流会打到本机自己连的那条会话上。 */
    @Test
    fun refreshDoesNotTurnAnOutboundSessionIntoAnInboundOne() {
        val outbound = sender(peerIdShort = "aaaa0001").copy(sessionInbound = false)
        val refreshed = SenderStateMapper.sessionAfterPeers(outbound, listOf(peer("aaaa0001", state = "idle")))
        assertNotNull(refreshed)
        assertFalse(refreshed!!.sessionInbound)
        assertFalse("刷新之后依然不该自动", autoStartWanted(refreshed.copy(peerTrusted = true)))
    }

    /** 规则 2：只认 idle —— failed 不重试、handshaking 等它自己变。 */
    @Test
    fun autoStartOnlyAcceptsTheIdleState() {
        for (state in listOf("handshaking", "streaming", "degraded", "reconnecting", "failed")) {
            assertFalse("状态 $state 时不该自动开始", autoStartWanted(inboundIdle(peerState = state)))
        }
        assertTrue("idle 是唯一可推的时刻", autoStartWanted(inboundIdle(peerState = "idle")))
    }

    /** 规则 3 的前半：未授信的会话也会出现在列表里，不能因为"它在表里"就开始推。 */
    @Test
    fun autoStartSkipsUntrustedSessions() {
        assertFalse(autoStartWanted(inboundIdle(trusted = false)))
    }

    /** 规则 4：用户手动停过的设备永不自动重启 —— 而且只管那一台。 */
    @Test
    fun manualStopOptsThatDeviceAndOnlyThatDeviceOut() {
        assertFalse(
            "手动停过的设备不能再被自动拉起",
            autoStartWanted(inboundIdle(manualStopFor = "aaaa0001")),
        )
        assertTrue(
            "拒绝集合是按设备记的：换了设备照样自动",
            autoStartWanted(inboundIdle(manualStopFor = "bbbb0002")),
        )
    }

    /** 规则 3 的后半：同一台设备只自动发起一次（事件抖动不会造成重复调用）。 */
    @Test
    fun autoStartFiresOnlyOncePerDevice() {
        assertFalse(
            "已经自动发起过的设备不再重复发起（失败也不重试）",
            autoStartWanted(inboundIdle(autoStartedFor = "aaaa0001")),
        )
        assertTrue(
            "已经自动过的是别的设备，不影响这一台",
            autoStartWanted(inboundIdle(autoStartedFor = "bbbb0002")),
        )
    }

    /** 规则 6：没选发送源就不自动 —— 那样推过去只有连接没有声音，白让对端等。 */
    @Test
    fun autoStartNeedsACaptureSource() {
        assertFalse(autoStartWanted(inboundIdle(), capture = null))
    }

    /** 规则 6 后半：其余门禁一律沿用 canStartSend，不另造一套。 */
    @Test
    fun autoStartSharesTheManualGates() {
        assertFalse("引擎没起来不自动", autoStartWanted(inboundIdle(), engineRunning = false))
        assertFalse("已经在推流就不再自动", autoStartWanted(inboundIdle().copy(sending = true)))
        assertFalse("还在等 PIN 不自动", autoStartWanted(inboundIdle().copy(awaitingPin = true)))
        assertFalse(
            "采集正在停止时先等它停完",
            autoStartWanted(inboundIdle(), captureState = CaptureState.Stopping),
        )
        assertTrue(
            "等授权期间照样自动（授权一到就出声，与既有文案「授权后会自动开始推流」一致）",
            autoStartWanted(inboundIdle(), captureState = CaptureState.AwaitingPermission),
        )
    }

    /** 记账：自动发起**之前**先记设备，否则异步窗口里每一拍都会再触发一次。 */
    @Test
    fun autoStartArmedRemembersTheDevice() {
        val armed = SenderStateMapper.afterAutoStartArmed(inboundIdle())
        assertEquals("aaaa0001", armed.autoStartedFor)
        assertFalse("记过账之后就不再自动", autoStartWanted(armed))
    }

    /** 记账：手动停流记住的是**设备**，所以断开重连之后依然不再自动。 */
    @Test
    fun manualStopRemembersTheDeviceAcrossReconnects() {
        val stopped = SenderStateMapper.afterManualStop(inboundIdle(peerState = "streaming"))
        assertEquals("aaaa0001", stopped.manualStopFor)

        // 断开：会话消失，但"这台设备被停过"的记忆必须留着。
        val dropped = SenderStateMapper.sessionAfterPeers(stopped, listOf(peer("bbbb0002")))
        assertNotNull(dropped)
        assertEquals("aaaa0001", dropped!!.manualStopFor)

        // 重连（同一台设备重新连进来）：sessionInbound 重新为真，但拒绝集合仍然生效。
        val reconnected = SenderStateMapper.sessionAfterPeers(
            dropped.copy(peerIdShort = null, peerState = ""),
            listOf(peer("aaaa0001", state = "idle")),
        )
        assertNotNull(reconnected)
        assertTrue("重新进来依然是入站会话", reconnected!!.sessionInbound)
        assertFalse("但手动停过的事实不许被忘记", autoStartWanted(reconnected))
    }

    /** 采纳入站会话时会打上入站标记，断开时清除（自动推流的第一个条件就靠它）。 */
    @Test
    fun inboundSessionsAreMarkedAndTheFlagClearsOnDisconnect() {
        val adopted = SenderStateMapper.sessionAfterPeers(sender(), listOf(peer("aaaa0001", state = "idle")))
        assertNotNull(adopted)
        assertTrue("从会话表里采纳来的就是入站会话", adopted!!.sessionInbound)
        assertTrue("授信状态也要跟着记下来", adopted.peerTrusted)

        val gone = SenderStateMapper.sessionAfterPeers(adopted, listOf(peer("bbbb0002")))
        assertNotNull(gone)
        assertFalse("断开之后就不再是入站会话", gone!!.sessionInbound)
        assertFalse(gone.peerTrusted)
    }
}
