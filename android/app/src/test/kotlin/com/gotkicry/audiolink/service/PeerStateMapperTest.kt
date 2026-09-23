package com.gotkicry.audiolink.service

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * [PeerStateMapper] 的测试 —— 全部在纯 JVM 上跑（**不需要 `.so`、不需要 Android**）。
 *
 * 为什么这几条值得单独钉：内核 `peers()` 的字段是**跨层**传到 UI 的（遥测读数被首屏直接显示、
 * 状态字面量被映射成中文标签、名字缺失时要回落），任何一处漏拷或错拷都只会在真机上表现为
 * "某个读数一直是占位符"或"某台设备显示空白"—— 而那种缺陷在界面上是**静默**的，只有这一层盯得住。
 */
class PeerStateMapperTest {

    @Test
    fun networkLatencySurvivesMappingWithoutEndToEndMeasurement() {
        // RTT（网络往返）是内核真给的读数；端到端延迟在未推流时**没有测量值** —— 两者必须分开：
        // 映射不许拿一个顶上另一个（那会把"没测过"显示成一个看起来很正常的延迟）。
        val peer = PeerStateMapper.peers(
            listOf(
                PeerSnapshot(
                    idShort = "a255cf9c",
                    name = "PC",
                    addr = "192.168.3.200:63092",
                    state = "streaming",
                    e2eLatencyUs = 0L,
                    bitrateBps = 192_000L,
                    lossPct = 1.5,
                    rttUs = 3_000L,
                    negotiatedFrameMs = 10,
                ),
            ),
        ).single()

        assertEquals("网络往返延迟必须原样穿过映射", 3_000L, peer.rttUs)
        assertEquals("没测过端到端延迟就是 0，不许拿 RTT 冒充", 0L, peer.e2eLatencyUs)
        assertEquals(192_000L, peer.bitrateBps)
        assertEquals(1.5, peer.lossPct, 0.0)
        assertEquals("协商帧长也要跟着过来（诊断行靠它）", 10, peer.negotiatedFrameMs)
    }

    @Test
    fun peerKeepsShortIdNameAddressAndState() {
        val peer = PeerStateMapper.peers(
            listOf(
                PeerSnapshot(
                    idShort = "a255cf9c4230c64e",
                    name = "DESKTOP-7HH1JC8",
                    addr = "172.16.2.51:58290",
                    state = "streaming",
                    idHex = "a255cf9c4230c64e00",
                ),
            ),
        ).single()

        assertEquals("a255cf9c4230c64e", peer.idShort)
        assertEquals("DESKTOP-7HH1JC8", peer.name)
        assertEquals("172.16.2.51:58290", peer.addr)
        assertEquals("streaming", peer.state)
        assertEquals("已连接", peer.stateLabel)
        assertEquals("完整指纹要留着：按设备控制（音量/断开）传的是它", "a255cf9c4230c64e00", peer.idHex)
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
            assertEquals("状态 $raw 的标签必须固定（UI 与验收术语要一致）", label, PeerStateMapper.stateLabel(raw))
        }
    }

    @Test
    fun unknownStateIsShownVerbatimInsteadOfSwallowed() {
        // 内核将来加了新状态，外壳宁可显示原始字面量，也不要显示成空白或"未知"二字
        assertEquals("quiescing", PeerStateMapper.stateLabel("quiescing"))
        assertEquals("DRAINING", PeerStateMapper.stateLabel("DRAINING"))
        // 已知状态大小写不敏感（内核口径是小写，但别因为大小写就掉进"未知"分支）
        assertEquals("已连接", PeerStateMapper.stateLabel("Streaming"))
    }

    @Test
    fun blankPeerNameFallsBackToShortId() {
        val peer = PeerStateMapper.peers(
            listOf(PeerSnapshot("a255cf9c", "", "172.16.2.54:58290", "handshaking")),
        ).single()

        assertEquals("名字是对端自报的、可以为空；指纹短码永远有值，所以回落方向是名字→短码", "a255cf9c", peer.name)
    }

    @Test
    fun peerWithoutNameAndShortIdStillRendersSomething() {
        val peer = PeerStateMapper.peers(
            listOf(PeerSnapshot("", "   ", "", "")),
        ).single()

        assertEquals("（未知对端）", peer.name)
    }

    @Test
    fun peersPreserveKernelOrder() {
        val peers = PeerStateMapper.peers(
            listOf(
                PeerSnapshot("aaa", "A", "1.1.1.1:1", "streaming"),
                PeerSnapshot("bbb", "B", "2.2.2.2:2", "idle"),
            ),
        )

        assertEquals(listOf("aaa", "bbb"), peers.map { it.idShort })
    }

    @Test
    fun mapCarriesThePeerList() {
        val state = PeerStateMapper.map(listOf(PeerSnapshot("aaa", "A", "1.1.1.1:1", "idle")))

        assertEquals(listOf("aaa"), state.peers.map { it.idShort })
        assertNull("取数失败的原因由调用方补（壳侧那一行 `.copy(note = ...)`），映射本身不假设成功", state.note)
    }
}
