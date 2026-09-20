package com.gotkicry.audiolink.ui.i18n

import com.gotkicry.audiolink.capture.CaptureSourceKind
import com.gotkicry.audiolink.capture.CaptureState
import com.gotkicry.audiolink.service.SenderStateMapper
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

/**
 * [captureNoteText] 的口径测试（JVM，无 Android 依赖）。
 *
 * 这一层只换"怎么说"，**绝不换"要不要说"** —— 所以这里的第一条断言是拿它与 service 层的
 * [SenderStateMapper.captureNote] 逐格对比：每个「发送源 × 采集状态」组合上，两边必须同时
 * 出提示或同时不出。这条不变量一旦破了，界面就会在采集正常时挂着"正在等待授权"，
 * 或者在采集出错时一声不吭。
 */
class CaptureNoteTextTest {

    @Test
    fun notePresenceMatchesTheServiceSideDecision() {
        val selections = listOf(null, CaptureSourceKind.Microphone, CaptureSourceKind.SystemLoopback)
        for (selection in selections) {
            for (state in CaptureState.entries) {
                val serviceSays = SenderStateMapper.captureNote(selection, state) != null
                for (strings in listOf(AudioLinkStrings.zh, AudioLinkStrings.en)) {
                    assertEquals(
                        "「$selection × $state」的提示口径必须与 service 层一致",
                        serviceSays,
                        captureNoteText(strings, selection, state) != null,
                    )
                }
            }
        }
    }

    /** 英文界面不许漏中文：这一层的全部意义就是把 service 的中文常量换掉。 */
    @Test
    fun englishNotesContainNoChineseCharacters() {
        val chinese = Regex("[\\u4e00-\\u9fff]")
        for (state in CaptureState.entries) {
            val note = captureNoteText(AudioLinkStrings.en, CaptureSourceKind.SystemLoopback, state).orEmpty()
            assertFalse("英文文案里混进了中文：$state → $note", chinese.containsMatchIn(note))
        }
    }
}
