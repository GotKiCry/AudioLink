package com.gotkicry.audiolink.ui.i18n

import com.gotkicry.audiolink.service.SendGate
import com.gotkicry.audiolink.service.SenderStateMapper
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [sendGateText] 的口径测试（JVM，无 Android 依赖）。
 *
 * 为什么值得测：这些字符串是**灰按钮唯一的解释**。用户看不到「SendGate.AddressEmpty」，
 * 只看到一句人话 —— 少一条文案就是一片空白，多一句「电脑」就把冻结术语捅出一个洞
 * （主机也可能是另一部手机，而术语按约定是全项目统一的）。
 *
 * 枚举是封闭的，所以这里能**穷举**：新增一条门禁却忘了给文案，这个测试会当场抓住。
 */
class SendGateTextTest {

    /** 每条门禁在两种语言下都必须有非空文案；只有 [SendGate.Allowed] 没有原因。 */
    @Test
    fun everyGateHasTextInBothLanguages() {
        for (gate in SendGate.entries) {
            val zh = sendGateText(AudioLinkStrings.zh, gate)
            val en = sendGateText(AudioLinkStrings.en, gate)

            if (gate == SendGate.Allowed) {
                assertNull("放行没有禁用原因：$gate", zh)
                assertNull("放行没有禁用原因：$gate", en)
                continue
            }
            assertFalse("中文文案不能为空：$gate", zh.isNullOrBlank())
            assertFalse("英文文案不能为空：$gate", en.isNullOrBlank())
        }
    }

    /**
     * 冻结术语：主机不叫「电脑」。这条不是文字洁癖 —— 手机当主机时（内录推给 PC），
     * 屏幕上写着「先填电脑的地址」会让人以为拿错了设备。
     */
    @Test
    fun noGateTextCallsTheHostAPc() {
        for (gate in SendGate.entries) {
            if (gate == SendGate.Allowed) continue
            val zh = sendGateText(AudioLinkStrings.zh, gate).orEmpty()
            val en = sendGateText(AudioLinkStrings.en, gate).orEmpty()
            assertFalse("中文文案不能出现「电脑」：$gate → $zh", zh.contains("电脑"))
            assertFalse("英文文案不能出现 PC：$gate → $en", en.contains("PC"))
        }
    }

    /** 端口示例必须来自内核的缺省端口常量，不能手写一个数字（否则哪天内核改了端口，界面在撒谎）。 */
    @Test
    fun invalidAddressQuotesTheKernelDefaultPort() {
        val rendered = sendGateText(AudioLinkStrings.zh, SendGate.AddressInvalid).orEmpty()
        assertTrue(
            "地址格式提示必须带上内核的缺省端口：$rendered",
            rendered.contains(SenderStateMapper.DEFAULT_PORT_HINT.toString()),
        )
    }
}
