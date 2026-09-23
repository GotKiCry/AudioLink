package com.gotkicry.audiolink.ui.i18n

import com.gotkicry.audiolink.service.PeerUi
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 多设备场景下**逐台**呈现口径的测试（纯逻辑，JVM 可跑）。
 *
 * 这两块逻辑都在真机上有过代价：
 * 「每行音量」的三态（null / 0 / 正常）混过一次会让"没设过"与"已静音"看起来一样；
 * 「来源」写死单数时，多台同时接入只会报一台 —— 用户以为只连了一台。
 */
class PeerGainTextTest {

    private fun peer(id: String, name: String = "PC") = PeerUi(
        idShort = id,
        name = name,
        addr = "192.168.3.1:58290",
        state = "streaming",
        stateLabel = "已连接",
    )

    /** 三态必须各自可辨：没设过 ≠ 已静音 ≠ 某个百分比。 */
    @Test
    fun gainLabelDistinguishesUnsetFromMuted() {
        val zh = AudioLinkStrings.zh
        assertEquals("没设过说「默认」", zh.peerVolumeDefault, gainLabel(zh, null))
        assertEquals("0 说「已静音」", zh.peerMuted, gainLabel(zh, 0u))
        assertEquals("100%", gainLabel(zh, 1_000u))
        assertEquals("40%", gainLabel(zh, 400u))
        assertEquals("200%", gainLabel(zh, 2_000u))
    }

    @Test
    fun onePeerShowsItsName() {
        assertEquals("PC", receiverSourceText(AudioLinkStrings.zh, listOf(peer("abc123"))))
    }

    /** 多台必须报数量（旧实现只报一台，等于少报）。 */
    @Test
    fun manyPeersStateTheCount() {
        val text = receiverSourceText(
            AudioLinkStrings.zh,
            listOf(peer("aaa", "MACBOOK"), peer("bbb", "GOTKICRY_WORK")),
        )
        assertTrue("要说出台数：$text", text.contains("2"))
        assertTrue("也要给出一台的名字当作锚点：$text", text.contains("MACBOOK"))
    }

    @Test
    fun noPeerSaysNotConnected() {
        assertEquals(AudioLinkStrings.zh.receiverSourceNone, receiverSourceText(AudioLinkStrings.zh, emptyList()))
    }

    /** 名字是对端自报、可留空 → 回落到短指纹（不显示空行）。 */
    @Test
    fun namelessPeerFallsBackToFingerprint() {
        assertEquals("abc123", receiverSourceText(AudioLinkStrings.zh, listOf(peer("abc123", name = ""))))
    }
}
