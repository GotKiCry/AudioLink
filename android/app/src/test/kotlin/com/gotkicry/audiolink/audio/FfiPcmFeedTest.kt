package com.gotkicry.audiolink.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import com.gotkicry.audiolink.core.PcmBufferState

/**
 * [FfiPcmFeed] 的 JVM 测试。
 *
 * 这里覆盖的是**不需要 Android、也不需要 Rust 动态库**的部分：搬运正确性（含声道顺序）、
 * 口径防御（样本数不足）、环满时向上游回报"没全收"的约定，以及复用缓冲的扩容路径。
 * 真正跨 FFI 的行为（JNA 加载、内核回调线程、GC 抖动）**本机无法验证**，见类注释。
 */
class FfiPcmFeedTest {

    private companion object {
        const val CHANNELS = 2
    }

    /** 造 n 帧 2ch 交错样本：第 f 帧 = `[f*10+1, f*10+2]` —— 用帧号编码数值，便于验声道不串位。 */
    private fun interleaved(frames: Int, startFrame: Int = 0): List<Float> =
        List(PcmLayout.samplesForFrames(frames, CHANNELS)) { i ->
            val frame = startFrame + i / CHANNELS
            (frame * 10 + (i % CHANNELS) + 1).toFloat()
        }

    private fun readAll(ring: PcmRingBuffer, samples: Int): FloatArray {
        val out = FloatArray(samples)
        assertEquals("期望能读满", samples, ring.readInto(out))
        return out
    }

    @Test
    fun feedbackIncludesRingAndPendingOutputWithoutChangingDeviceTarget() {
        val ring = PcmRingBuffer(2880, CHANNELS)
        val feed = FfiPcmFeed(ring, outputBufferState = { PcmBufferState(1920u, 1440u) })
        feed.feedPcm(interleaved(960), 960)
        assertEquals(PcmBufferState(2880u, 1440u), feed.playoutBufferState())
        ring.readInto(FloatArray(960))
        assertEquals(PcmBufferState(2400u, 1440u), feed.playoutBufferState())
        assertEquals(null, FfiPcmFeed(ring).playoutBufferState())
    }

    @Test
    fun deliversInterleavedSamplesIntoRing() {
        val ring = PcmRingBuffer(capacityFrames = 16, channelCount = CHANNELS)
        val feed = FfiPcmFeed(ring)

        val accepted = feed.feedPcm(interleaved(8), frames = 8)

        assertEquals("正常路径必须全收", 8, accepted)
        assertEquals(8, ring.sizeFrames)
        val out = readAll(ring, PcmLayout.samplesForFrames(8, CHANNELS))
        val expected = interleaved(8)
        for (i in expected.indices) {
            assertEquals("sample[$i] 必须逐样本一致（声道不能串位）", expected[i], out[i], 0f)
        }
        assertEquals(0L, ring.overflowFrames)
    }

    @Test
    fun zeroOrNegativeFramesAreIgnored() {
        val ring = PcmRingBuffer(capacityFrames = 4, channelCount = CHANNELS)
        val feed = FfiPcmFeed(ring)

        assertEquals(0, feed.feedPcm(emptyList(), frames = 0))
        assertEquals(0, feed.feedPcm(interleaved(2), frames = -3))
        assertEquals("不该写进任何东西", 0, ring.sizeFrames)
    }

    @Test
    fun shortSampleListIsRejectedWithoutWriting() {
        val ring = PcmRingBuffer(capacityFrames = 16, channelCount = CHANNELS)
        val feed = FfiPcmFeed(ring)

        // 声明 8 帧却只给 4 个样本 → 上游口径破了，写进去只会变成整段声道错位。
        val accepted = feed.feedPcm(List(4) { 1f }, frames = 8)

        assertEquals("宁可丢一段，也不写错位数据", 0, accepted)
        assertEquals(0, ring.sizeFrames)
    }

    @Test
    fun ringOverflowIsReportedUpstreamAsPartialAcceptance() {
        val ring = PcmRingBuffer(capacityFrames = 4, channelCount = CHANNELS)
        val feed = FfiPcmFeed(ring)

        assertEquals("第一次写满 4 帧，不丢", 4, feed.feedPcm(interleaved(4, startFrame = 0), frames = 4))
        val accepted = feed.feedPcm(interleaved(4, startFrame = 100), frames = 4)

        // 环满：drop-oldest 丢掉 4 帧旧的，内核必须能从返回值看出"这段没全收"
        // （`accepted < frames` → 内核 `PlayoutStats.write_errors++`，见 audio_bridge.rs）。
        assertEquals("环满时必须报告未全收", 0, accepted)
        assertEquals("丢弃量记在环的 overflow 口径里", 4L, ring.overflowFrames)
        val out = readAll(ring, PcmLayout.samplesForFrames(4, CHANNELS))
        val expected = interleaved(4, startFrame = 100)
        for (i in expected.indices) {
            assertEquals("保住的是最新数据", expected[i], out[i], 0f)
        }
    }

    @Test
    fun partialOverflowReportsScaledBackAcceptance() {
        val ring = PcmRingBuffer(capacityFrames = 6, channelCount = CHANNELS)
        val feed = FfiPcmFeed(ring)

        assertEquals(6, feed.feedPcm(interleaved(6, startFrame = 0), frames = 6))
        // 再写 4 帧：容量只有 6，必然丢 4 帧旧的 → accepted = 4 - 4 = 0
        val accepted = feed.feedPcm(interleaved(4, startFrame = 50), frames = 4)

        assertEquals(0, accepted)
        assertEquals(4L, ring.overflowFrames)
        assertEquals("环容量是硬上界", 6, ring.sizeFrames)
    }

    @Test
    fun oversizedFrameGrowsScratchBufferAndKeepsCarryingData() {
        val ring = PcmRingBuffer(capacityFrames = 4_096, channelCount = CHANNELS)
        val feed = FfiPcmFeed(ring)

        // 2048 个样本 > 初始 scratch（1920）→ 走扩容分支
        val big = interleaved(1_024, startFrame = 0)
        assertEquals(1_024, feed.feedPcm(big, frames = 1_024))

        val out = readAll(ring, PcmLayout.samplesForFrames(1_024, CHANNELS))
        for (i in big.indices) {
            assertEquals("扩容后内容必须仍然正确", big[i], out[i], 0f)
        }

        // 扩容是一次性的：再来一小段，仍然正确。
        assertEquals(20, feed.feedPcm(interleaved(20, startFrame = 7), frames = 20))
        val small = readAll(ring, PcmLayout.samplesForFrames(20, CHANNELS))
        val expectedSmall = interleaved(20, startFrame = 7)
        for (i in expectedSmall.indices) {
            assertEquals(expectedSmall[i], small[i], 0f)
        }
    }

    @Test
    fun accumulatesAcrossRepeatedCalls() {
        val ring = PcmRingBuffer(capacityFrames = 512, channelCount = CHANNELS)
        val feed = FfiPcmFeed(ring)

        var acceptedTotal = 0
        for (round in 0 until 10) {
            acceptedTotal += feed.feedPcm(interleaved(48, startFrame = round * 48), frames = 48)
        }

        assertEquals(480, acceptedTotal)
        assertEquals(480, ring.sizeFrames)
        assertTrue("没有发生丢弃", ring.overflowFrames == 0L)
    }
}
