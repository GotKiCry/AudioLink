package com.gotkicry.audiolink.ui.i18n

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * 诊断面板副标题的口径测试。
 *
 * 钉住的行为很具体：**内核一报错，折叠态那句副标题就必须改口**。
 * 真机踩过：引擎起不来时主路径只说"服务没能启动；请再试一次"，而真正的原因
 * （UniFFI checksum mismatch）埋在折叠区里没人展开 —— 排查绕了一大圈。
 */
class DiagnosticsTextTest {

    @Test
    fun anEngineErrorChangesTheCollapsedSummary() {
        val zh = AudioLinkStrings.zh
        assertEquals("没有报错时用常规副标题", zh.diagnosticsSummary, diagnosticsSummary(zh, null))
        assertEquals("空白不算报错", zh.diagnosticsSummary, diagnosticsSummary(zh, "   "))
        assertEquals(
            "有内核报错时必须改口，否则用户不会展开",
            zh.diagnosticsSummaryError,
            diagnosticsSummary(zh, "引擎启动失败：RuntimeException: UniFFI API checksum mismatch"),
        )
    }

    @Test
    fun theErrorSummaryExistsInBothLanguages() {
        for (strings in listOf(AudioLinkStrings.zh, AudioLinkStrings.en)) {
            assertEquals(
                "报错副标题必须与常规副标题不同（否则等于没提示）",
                false,
                strings.diagnosticsSummaryError == strings.diagnosticsSummary,
            )
            assertEquals(false, strings.diagnosticsSummaryError.isBlank())
        }
    }
}
