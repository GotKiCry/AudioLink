package com.gotkicry.audiolink.ui.i18n

import com.gotkicry.audiolink.service.SenderUiState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 拆卡之后"谁说什么"的口径测试（JVM，无 Android 依赖）。
 *
 * 为什么值得单独测：[SenderUiState] 的 `note` / `error` 是**一条**通道，里面混着两个角色的消息。
 * 拆成「接收端入口卡」与「主机卡」之后，归属判错的表现是**静默**的 —— 用户不会看到报错，
 * 只会看到同一句话说两遍，或者某类失败一句话都不说。Compose 在 JVM 单测里跑不起来，
 * 所以归属判定被抽成了这里能穷举的纯函数。
 */
class SenderFeedbackTextTest {

    private fun sender(
        peerIdShort: String? = null,
        note: String? = null,
        error: String? = null,
        awaitingPin: Boolean = false,
        sessionDropped: Boolean = false,
    ) = SenderUiState(
        peerIdShort = peerIdShort,
        note = note,
        error = error,
        awaitingPin = awaitingPin,
        sessionDropped = sessionDropped,
    )

    /** 没有会话 ⇒ 错误只可能来自这一次连接尝试 ⇒ 归接收端入口卡。 */
    @Test
    fun errorsBeforeASessionBelongToTheConnectEntry() {
        val failed = sender(error = "连接失败（1001）；可以再试一次")
        assertEquals("连接失败（1001）；可以再试一次", connectPathError(failed))
        assertNull("还没连上，推流那一侧没有话可说", hostPathError(failed))
    }

    /** 有了会话 ⇒ 失败必然出在推流 / 停流 / 门禁上 ⇒ 归主机卡。 */
    @Test
    fun errorsAfterASessionBelongToTheHostDeck() {
        val failed = sender(peerIdShort = "aaaa0001", error = "发送操作失败（1001）")
        assertEquals("发送操作失败（1001）", hostPathError(failed))
        assertNull("连接已经成了，入口卡不该再报错", connectPathError(failed))
    }

    /** 输码与断开：两条都归接收端入口卡（主机卡那边状态徽章会自己退回「未连接」）。 */
    @Test
    fun pinAndDisconnectNotesBelongToTheConnectEntry() {
        val strings = AudioLinkStrings.zh

        val awaiting = sender(awaitingPin = true, note = "service 层的中文提示")
        assertEquals(strings.senderAwaitingPinNote, connectPathNote(strings, awaiting))
        assertNull(hostPathNote(awaiting))

        val dropped = sender(sessionDropped = true)
        assertEquals(strings.senderSessionDropped, connectPathNote(strings, dropped))
        assertNull(hostPathNote(dropped))
    }

    /** 推流路径的提示（例如「已停止发送」）留在主机卡。 */
    @Test
    fun streamingNotesStayOnTheHostDeck() {
        val stopped = sender(peerIdShort = "aaaa0001", note = "已停止发送（连接保持）")
        assertEquals("已停止发送（连接保持）", hostPathNote(stopped))
        assertNull(connectPathNote(AudioLinkStrings.zh, stopped))
    }

    /**
     * **不重复、不遗漏**：一条 note 在两张卡上恰好出现一次。
     * 这条不变量一旦破了就是静默缺陷 —— 用户会看到同一句话说两遍，或者一句都不说。
     */
    @Test
    fun everyNoteLandsOnExactlyOneCard() {
        val strings = AudioLinkStrings.zh
        for (hasSession in listOf(false, true)) {
            for (awaitingPin in listOf(false, true)) {
                for (dropped in listOf(false, true)) {
                    val state = sender(
                        peerIdShort = if (hasSession) "aaaa0001" else null,
                        note = "推流那一侧的话",
                        awaitingPin = awaitingPin,
                        sessionDropped = dropped,
                    )
                    val shown = listOf(connectPathNote(strings, state), hostPathNote(state)).count { it != null }
                    assertEquals(
                        "hasSession=$hasSession awaitingPin=$awaitingPin dropped=$dropped：" +
                            "提示必须恰好出现在一张卡上",
                        1,
                        shown,
                    )
                }
            }
        }
    }

    /**
     * 接收端入口的角色文案：说清"这一格改的是本机角色"，且方向不写反（地址来自**主机**）。
     * 这条防的是旧命名回潮 —— 退回"添加设备"就又把角色切换讲成了设备清单。
     */
    @Test
    fun receiverEntryTextsStateTheRoleNotADeviceListEntry() {
        for (strings in listOf(AudioLinkStrings.zh, AudioLinkStrings.en)) {
            assertTrue(strings.deckReceiverEntry.isNotBlank())
            assertTrue(strings.receiverRoleTag.isNotBlank())
            assertTrue(strings.receiverEntryHint.isNotBlank())
        }
        assertTrue(
            "中文角色标签要说「接收端」：${AudioLinkStrings.zh.receiverRoleTag}",
            AudioLinkStrings.zh.receiverRoleTag.contains("接收端"),
        )
        assertTrue(
            "英文角色标签要说 receiver：${AudioLinkStrings.en.receiverRoleTag}",
            AudioLinkStrings.en.receiverRoleTag.contains("receiver", ignoreCase = true),
        )
        assertTrue(
            "代价说明要指明地址来自主机：${AudioLinkStrings.zh.receiverEntryHint}",
            AudioLinkStrings.zh.receiverEntryHint.contains("主机"),
        )
        // 冻结术语：主机不叫「电脑」。
        assertFalse(AudioLinkStrings.zh.receiverEntryHint.contains("电脑"))
        assertFalse(AudioLinkStrings.zh.receiverRoleTag.contains("电脑"))
        assertFalse(AudioLinkStrings.zh.deckReceiverEntry.contains("电脑"))
    }

    // ---- 门禁提示**恰好一次**（真机评审的收口项）----

    /**
     * 连接入口卡上最多一条门禁提示。
     *
     * 真机缺陷：地址为空时 [com.gotkicry.audiolink.service.SenderStateMapper.addressGate] 与
     * [com.gotkicry.audiolink.service.SenderStateMapper.canConnect] 返回的是**同一个** SendGate，
     * 输入框下渲染一次、按钮下再渲染一次 —— 同一句「先填主机地址（例如 192.168.1.5）」印了两遍。
     * 现在候选集中在 [connectEntryGateHints]，界面只取第一条；这条不变量在此穷举。
     */
    @Test
    fun theConnectEntryShowsAtMostOneGateHint() {
        val addresses = listOf("", "   ", "192.168.1.5", "192.168.1.5:0", "192.168.1.5:58290", "999.1.1.1")
        val states = listOf(
            sender(),
            sender().copy(connecting = true),
            sender(peerIdShort = "aaaa0001"),
            sender(awaitingPin = true),
        )
        for (state in states) {
            for (raw in addresses) {
                val hints = connectEntryGateHints(AudioLinkStrings.zh, state, raw)
                assertTrue(
                    "连接入口卡的提示最多一条（addr=$raw）：$hints",
                    hints.size <= 1,
                )
            }
        }
    }

    /** 地址为空时必须**恰好**给出一条：一句都没有的话，用户按不动按钮还不知道为什么。 */
    @Test
    fun anEmptyAddressExplainsItselfExactlyOnce() {
        for (strings in listOf(AudioLinkStrings.zh, AudioLinkStrings.en)) {
            val hints = connectEntryGateHints(strings, sender(), "")
            assertEquals("空地址要恰好给出一条提示：$hints", 1, hints.size)
            assertTrue(hints.single().isNotBlank())
        }
        // 地址合法时不该有任何提示（按钮是亮的，没什么要解释）。
        assertTrue(connectEntryGateHints(AudioLinkStrings.zh, sender(), "192.168.1.5").isEmpty())
    }
}
