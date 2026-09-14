package com.gotkicry.audiolink.audio

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * [PcmLayout] 的索引换算测试。
 *
 * 这些换算看着像"乘 2 除 2"，但它是整个模块里最容易悄悄错的地方：帧/样本混用一次，
 * 结果就是音调不对或左右声道串位，而这两种现象在真机上很容易被误判成"网络问题"。
 */
class PcmLayoutTest {

    @Test
    fun samplesForFramesMultipliesByChannelCount() {
        assertEquals(0, PcmLayout.samplesForFrames(0, 2))
        assertEquals(2, PcmLayout.samplesForFrames(1, 2))
        assertEquals(960, PcmLayout.samplesForFrames(480, 2))
        assertEquals(480, PcmLayout.samplesForFrames(480, 1))
    }

    @Test
    fun framesForSamplesTruncatesPartialFrame() {
        assertEquals(0, PcmLayout.framesForSamples(1, 2))
        assertEquals(1, PcmLayout.framesForSamples(2, 2))
        assertEquals("5 个样本只有 2 个整帧，零头必须丢掉", 2, PcmLayout.framesForSamples(5, 2))
        assertEquals(480, PcmLayout.framesForSamples(960, 2))
    }

    @Test
    fun sampleOffsetOfFramePointsAtFrameHead() {
        assertEquals(0, PcmLayout.sampleOffsetOfFrame(0, 2))
        assertEquals(6, PcmLayout.sampleOffsetOfFrame(3, 2))
        assertEquals(3, PcmLayout.sampleOffsetOfFrame(3, 1))
    }

    @Test
    fun roundTripIsStableForWholeFrames() {
        for (frame in 0..1000 step 7) {
            val samples = PcmLayout.samplesForFrames(frame, 2)
            assertEquals(frame, PcmLayout.framesForSamples(samples, 2))
        }
    }

    @Test(expected = IllegalArgumentException::class)
    fun zeroChannelCountIsRejected() {
        PcmLayout.samplesForFrames(1, 0)
    }

    @Test(expected = IllegalArgumentException::class)
    fun negativeFramesIsRejected() {
        PcmLayout.samplesForFrames(-1, 2)
    }
}
