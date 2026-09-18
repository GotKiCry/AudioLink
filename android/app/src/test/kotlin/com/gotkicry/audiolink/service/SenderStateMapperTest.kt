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

    @Test
    fun connectBlockedUntilEngineRuns() {
        assertEquals(
            SendGate.EngineDown,
            SenderStateMapper.canConnect(sender(), engineRunning = false, targetAddr = "192.168.1.5"),
        )
    }

    @Test
    fun connectBlockedWhileConnectingAndWhenAddressBad() {
        assertEquals(
            SendGate.Connecting,
            SenderStateMapper.canConnect(sender(connecting = true), engineRunning = true, targetAddr = "192.168.1.5"),
        )
        assertEquals(
            SendGate.AddressEmpty,
            SenderStateMapper.canConnect(sender(), engineRunning = true, targetAddr = ""),
        )
        assertEquals(
            SendGate.Allowed,
            SenderStateMapper.canConnect(sender(), engineRunning = true, targetAddr = "192.168.1.5"),
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
            SenderStateMapper.canConnect(fresh, engineRunning = true, targetAddr = "192.168.3.200"),
        )
        assertEquals(
            SendGate.AddressEmpty,
            SenderStateMapper.canConnect(fresh, engineRunning = true, targetAddr = "   "),
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
}
