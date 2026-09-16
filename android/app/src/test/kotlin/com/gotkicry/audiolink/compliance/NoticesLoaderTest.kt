package com.gotkicry.audiolink.compliance

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [NoticesLoader] 的测试 —— 全部在纯 JVM 上跑（不需要 Android、不需要真机）。
 *
 * 为什么这几条值得钉住：「找不到声明」这条路径只在**打包漏了资源**时才走到，
 * 而那正是最不容易在开发机上遇到的情况；如果那时给的是空白页，用户只会以为应用坏了。
 */
class NoticesLoaderTest {

    @Test
    fun missingTextExplainsHowToGenerate() {
        val state = NoticesLoader.fromText(null)
        assertFalse(state.available)
        assertEquals(0, state.bytes)
        assertEquals(NoticesLoader.MISSING_HINT, state.text)
        assertTrue(state.text.contains("license-audit.ps1 -Notices"))
    }

    @Test
    fun emptyTextCountsAsMissing() {
        // 空文件与没有文件对用户是同一件事：都看不到内容。
        assertFalse(NoticesLoader.fromText("").available)
    }

    @Test
    fun presentTextReportsUtf8Size() {
        val state = NoticesLoader.fromText("# 声明\n组件清单\n")
        assertTrue(state.available)
        // UTF-8 字节数：中文一个字 3 字节 —— 用字符数会低估。
        assertEquals("# 声明\n组件清单\n".toByteArray(Charsets.UTF_8).size, state.bytes)
        assertTrue(state.text.startsWith("# 声明"))
    }
}
