package com.gotkicry.audiolink.audio

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * [PlaybackCounters] 的累计口径测试。
 *
 * 这些数字就是本轮交付里唯一能被验证的"指标"（真机指标需要真机），所以口径必须在这里被钉死：
 * 有效音频帧、静音填充帧、丢弃帧、错误次数、供给欠载次数/缺失帧数，六条口径逐条断言。
 */
class PlaybackCountersTest {

    @Test
    fun recordWriteAccumulatesFrames() {
        val c = PlaybackCounters()
        c.recordWrite(100)
        c.recordWrite(64)
        c.recordWrite(0)
        assertEquals("0 帧不产生累计（缓冲满不是写入）", 164L, c.framesWritten)
    }

    @Test
    fun negativeFramesNeverLowerTheCounter() {
        val c = PlaybackCounters()
        c.recordWrite(50)
        c.recordWrite(-7)
        assertEquals("负值是错误路径，不许污染已写入帧数", 50L, c.framesWritten)
    }

    @Test
    fun recordWriteErrorCountsAndKeepsLastCode() {
        val c = PlaybackCounters()
        assertEquals(PlaybackCounters.ERROR_NONE, c.lastWriteError)

        c.recordWriteError(-3) // ERROR_INVALID_OPERATION
        c.recordWriteError(-6) // ERROR_DEAD_OBJECT
        assertEquals(2, c.writeErrors)
        assertEquals("留最近一次错误码，便于 UI 直接展示", -6, c.lastWriteError)
    }

    @Test
    fun silenceAndDropAreCountedSeparatelyFromWrittenFrames() {
        val c = PlaybackCounters()
        c.recordWrite(480)
        c.recordSilence(240)
        c.recordDrop(120)

        assertEquals("设备实收 = written + silence", 480L, c.framesWritten)
        assertEquals(240L, c.silenceFrames)
        assertEquals(120L, c.framesDropped)
    }

    @Test
    fun recordReadFullChunkIsNotAnUnderrun() {
        val c = PlaybackCounters()
        c.recordRead(returnedSamples = 960, requestedSamples = 960, channelCount = 2)
        assertEquals(0L, c.sourceUnderruns)
        assertEquals(0L, c.starvedFrames)
    }

    @Test
    fun recordReadShortChunkCountsUnderrunAndMissingFrames() {
        val c = PlaybackCounters()
        c.recordRead(returnedSamples = 480, requestedSamples = 960, channelCount = 2)
        assertEquals(1L, c.sourceUnderruns)
        assertEquals("缺的 480 个样本 = 240 帧", 240L, c.starvedFrames)

        c.recordRead(returnedSamples = 0, requestedSamples = 960, channelCount = 2)
        assertEquals(2L, c.sourceUnderruns)
        assertEquals(720L, c.starvedFrames)
    }

    @Test
    fun recordReadToleratesNegativeReturnFromSource() {
        val c = PlaybackCounters()
        c.recordRead(returnedSamples = -5, requestedSamples = 8, channelCount = 2)
        assertEquals("异常源返回负值时按零处理", 1L, c.sourceUnderruns)
        assertEquals(4L, c.starvedFrames)
    }

    @Test
    fun resetClearsEveryCounter() {
        val c = PlaybackCounters()
        c.recordWrite(10)
        c.recordSilence(20)
        c.recordDrop(30)
        c.recordWriteError(-1)
        c.recordRead(0, 8, 2)

        c.reset()

        assertEquals(0L, c.framesWritten)
        assertEquals(0L, c.silenceFrames)
        assertEquals(0L, c.framesDropped)
        assertEquals(0, c.writeErrors)
        assertEquals(PlaybackCounters.ERROR_NONE, c.lastWriteError)
        assertEquals(0L, c.sourceUnderruns)
        assertEquals(0L, c.starvedFrames)
    }
}
