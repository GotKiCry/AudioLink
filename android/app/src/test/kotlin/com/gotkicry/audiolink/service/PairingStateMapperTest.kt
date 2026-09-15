package com.gotkicry.audiolink.service

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [PairingStateMapper] 的测试 —— 全部在纯 JVM 上跑（**不需要 `.so`、不需要 Android**）。
 *
 * 为什么这几条值得单独钉：真机验收时"手机上没显示 PIN"有三种完全不同的原因 ——
 * 内核没给 PIN、内核给了但外壳把它当空值丢了、以及外壳压根没问内核。
 * 这一层是第二种的判决点，所以"有 PIN / 无 PIN"两态必须逐字钉住，
 * 而不是靠"跑一次真机看看"。
 */
class PairingStateMapperTest {

    @Test
    fun noPinMeansNoPairingPanel() {
        val state = PairingStateMapper.map(pin = null, snapshots = emptyList())

        assertNull("内核没给 PIN 时外壳不得自己造一个", state.pin)
        assertFalse("没有 PIN 也没有对端时不该占位", state.hasAnything)
    }

    @Test
    fun pinIsCarriedVerbatim() {
        val state = PairingStateMapper.map(pin = "418906", snapshots = emptyList())

        assertEquals("PIN 是内核生成的原值，外壳不做任何加工", "418906", state.pin)
        assertTrue("有 PIN 就必须渲染出来", state.hasAnything)
    }

    @Test
    fun pinWithSurroundingWhitespaceIsTrimmed() {
        assertEquals("123456", PairingStateMapper.normalizePin("  123456\n"))
    }

    @Test
    fun blankPinIsTreatedAsAbsent() {
        assertNull("空串不是 PIN（否则会渲染出一个空的大字号框）", PairingStateMapper.normalizePin(""))
        assertNull("纯空白同理", PairingStateMapper.normalizePin("   "))
        assertNull(PairingStateMapper.normalizePin(null))
    }

    @Test
    fun pinLengthIsNotReValidatedInTheShell() {
        // 格式的权威是内核的 PinGate；外壳再判一次，只会在内核改长度时把真 PIN 藏起来
        // （"看不见的 PIN"比"格式不标准的 PIN"更糟）。
        assertEquals("12345", PairingStateMapper.normalizePin("12345"))
        assertEquals("1234567", PairingStateMapper.normalizePin("1234567"))
    }

    @Test
    fun peerKeepsShortIdNameAddressAndTrust() {
        val state = PairingStateMapper.map(
            pin = null,
            snapshots = listOf(
                PeerSnapshot(
                    idShort = "a255cf9c4230c64e",
                    name = "DESKTOP-7HH1JC8",
                    addr = "172.16.2.51:58290",
                    state = "streaming",
                    trusted = false,
                ),
            ),
        )

        val peer = state.peers.single()
        assertEquals("a255cf9c4230c64e", peer.idShort)
        assertEquals("DESKTOP-7HH1JC8", peer.name)
        assertEquals("172.16.2.51:58290", peer.addr)
        assertEquals("streaming", peer.state)
        assertEquals("已连接", peer.stateLabel)
        assertFalse("内核说没授信就是没授信，外壳不得自行推断", peer.trusted)
        assertTrue(state.hasAnything)
    }

    @Test
    fun trustedFlagIsPassedThrough() {
        val peer = PairingStateMapper.peers(
            listOf(PeerSnapshot("deadbeef", "PHONE", "10.0.0.9:1234", "idle", trusted = true)),
        ).single()

        assertTrue(peer.trusted)
        assertEquals("空闲", peer.stateLabel)
    }

    @Test
    fun allSixSessionStatesHaveLabels() {
        val expected = mapOf(
            "idle" to "空闲",
            "handshaking" to "握手中",
            "streaming" to "已连接",
            "degraded" to "质量下降",
            "reconnecting" to "重连中",
            "failed" to "失败",
        )

        expected.forEach { (raw, label) ->
            assertEquals("状态 $raw 的标签必须固定（UI 与验收术语要一致）", label, PairingStateMapper.stateLabel(raw))
        }
    }

    @Test
    fun unknownStateIsShownVerbatimInsteadOfSwallowed() {
        // 内核将来加了新状态，外壳宁可显示原始字面量，也不要显示成空白或"未知"二字
        assertEquals("quiescing", PairingStateMapper.stateLabel("quiescing"))
        assertEquals("DRAINING", PairingStateMapper.stateLabel("DRAINING"))
        // 已知状态大小写不敏感（内核口径是小写，但别因为大小写就掉进"未知"分支）
        assertEquals("已连接", PairingStateMapper.stateLabel("Streaming"))
    }

    @Test
    fun blankPeerNameFallsBackToShortId() {
        val peer = PairingStateMapper.peers(
            listOf(PeerSnapshot("a255cf9c", "", "172.16.2.54:58290", "handshaking", trusted = false)),
        ).single()

        assertEquals("名字是对端自报的、可以为空；短指纹永远有值，所以回落方向是名字→短码", "a255cf9c", peer.name)
    }

    @Test
    fun peerWithoutNameAndShortIdStillRendersSomething() {
        val peer = PairingStateMapper.peers(
            listOf(PeerSnapshot("", "   ", "", "", trusted = false)),
        ).single()

        assertEquals("（未知对端）", peer.name)
    }

    @Test
    fun errorBecomesHumanReadableNote() {
        val state = PairingStateMapper.map(
            pin = null,
            snapshots = emptyList(),
            error = "读取配对 PIN 失败：UnsatisfiedLinkError: libaudiolink_ffi.so 缺失",
        )

        assertTrue("失败原因必须带到 UI，不能静默吞掉", state.note!!.contains("libaudiolink_ffi.so"))
        assertNull("出错了不等于有 PIN", state.pin)
    }

    @Test
    fun pinWithoutAnyPeerIsMarkedStale() {
        // 真机实测出来的场景：对端断开后 FFI 仍返回旧 PIN（PinGate 已随连接销毁），
        // 外壳必须把"这个数字不作数了"标出来，否则用户会被骗着输一个必然失败的 PIN。
        val state = PairingStateMapper.map(pin = "415785", snapshots = emptyList())

        assertTrue("没有等待配对的对端 → 这个 PIN 是上一次连接留下的", state.pinIsStale)
        assertEquals("但内核原值仍然要显示，不许藏", "415785", state.pin)
    }

    @Test
    fun pinWithUntrustedPeerIsLive() {
        val state = PairingStateMapper.map(
            pin = "058081",
            snapshots = listOf(
                PeerSnapshot("1dcda2a4b2eb115b", "PC", "192.168.3.200:63092", "handshaking", trusted = false),
            ),
        )

        assertFalse("有未授信会话在等着提交 PIN → 这个 PIN 是活的", state.pinIsStale)
    }

    @Test
    fun pinWithOnlyTrustedPeersIsStillStale() {
        val state = PairingStateMapper.map(
            pin = "058081",
            snapshots = listOf(
                PeerSnapshot("1dcda2a4b2eb115b", "PC", "192.168.3.200:63092", "streaming", trusted = true),
            ),
        )

        assertTrue("已授信的对端不会在等 PIN，残留的 PIN 依然是死的", state.pinIsStale)
    }

    @Test
    fun noPinIsNeverStale() {
        assertFalse(PairingStateMapper.map(pin = null, snapshots = emptyList()).pinIsStale)
    }

    @Test
    fun peersPreserveKernelOrder() {
        val peers = PairingStateMapper.peers(
            listOf(
                PeerSnapshot("aaa", "A", "1.1.1.1:1", "streaming", true),
                PeerSnapshot("bbb", "B", "2.2.2.2:2", "idle", false),
            ),
        )

        assertEquals(listOf("aaa", "bbb"), peers.map { it.idShort })
    }
}
