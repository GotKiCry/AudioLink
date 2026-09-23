package com.gotkicry.audiolink.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [PlayoutLoop] 的行为测试 —— 这是"欠载检测"真正落地的地方。
 *
 * 用假 [PcmSource] / 假 [PcmSink] 驱动（`AudioTrack` 在 JVM 上只有 stub，一调就抛），
 * 覆盖四种现实里会遇到的形态：数据充足、完全没数据、数据不足、输出侧写不进。
 */
private const val CHANNELS = 2

/** 小 chunk 便于逐样本核对。 */
private const val CHUNK_FRAMES = 4

class PlayoutLoopTest {

    /** 可编程的假数据源；记录被读了几次（用于验证"pending 未写完时不拉新数据"）。 */
    private class FakeSource(private val produce: (FloatArray) -> Int) : PcmSource {
        var readCount = 0
            private set

        override fun readInto(dst: FloatArray): Int {
            readCount++
            return produce(dst).coerceIn(0, dst.size)
        }
    }

    /** 可编程的假输出：返回"愿意接受多少帧"，并记录真正被写出去的样本内容。 */
    private class FakeSink(private val accept: (requestedFrames: Int) -> Int) : PcmSink {
        val writes = mutableListOf<FloatArray>()

        override fun tryWrite(samples: FloatArray, offsetSamples: Int, frames: Int): Int {
            val accepted = accept(frames).coerceIn(0, frames)
            if (accepted > 0) {
                writes += samples.copyOfRange(
                    offsetSamples,
                    offsetSamples + PcmLayout.samplesForFrames(accepted, CHANNELS),
                )
            }
            return accepted
        }
    }

    /** 往 dst 里灌一串非零斜坡：静音填 0，所以"是不是静音"一眼可辨。 */
    private fun fillRamp(dst: FloatArray) {
        for (i in dst.indices) dst[i] = (i + 1).toFloat()
    }

    @Test
    fun feedbackIncludesPartialWritePendingUntilDeviceAcceptsIt() {
        val ring = PcmRingBuffer(8, CHANNELS)
        ring.write(FloatArray(16) { 0.4f }, 8)
        val sink = FakeSink { minOf(it, 2) }
        val loop = PlayoutLoop(ring, sink, CHANNELS, CHUNK_FRAMES, PlaybackCounters())
        assertTrue(loop.pumpOnce())
        assertEquals(4, ring.sizeFrames)
        assertEquals(2, loop.bufferedFrames)
        assertTrue(loop.pumpOnce())
        assertEquals(4, ring.sizeFrames)
        assertEquals(0, loop.bufferedFrames)
    }

    @Test
    fun fullChunkIsWrittenThroughAndCountedAsAudio() {
        val counters = PlaybackCounters()
        val source = FakeSource { dst -> fillRamp(dst); dst.size }
        val sink = FakeSink { it }
        val loop = PlayoutLoop(source, sink, CHANNELS, CHUNK_FRAMES, counters)

        assertTrue(loop.pumpOnce())

        assertEquals("一块只拉一次", 1, source.readCount)
        assertEquals(CHUNK_FRAMES.toLong(), counters.framesWritten)
        assertEquals(0L, counters.silenceFrames)
        assertEquals("数据充足不该记欠载", 0L, counters.sourceUnderruns)
        assertEquals(1, sink.writes.size)
        assertEquals(
            PcmLayout.samplesForFrames(CHUNK_FRAMES, CHANNELS),
            sink.writes[0].size,
        )
    }

    @Test
    fun emptySourceBecomesSilenceAtTheSinkAndCountsOneUnderrun() {
        val counters = PlaybackCounters()
        val source = FakeSource { 0 }
        val sink = FakeSink { it }
        val loop = PlayoutLoop(source, sink, CHANNELS, CHUNK_FRAMES, counters)

        assertTrue("补了静音也算推进了输出流", loop.pumpOnce())

        assertEquals("没有有效音频帧", 0L, counters.framesWritten)
        assertEquals(CHUNK_FRAMES.toLong(), counters.silenceFrames)
        assertEquals(1L, counters.sourceUnderruns)
        assertEquals(CHUNK_FRAMES.toLong(), counters.starvedFrames)
        assertTrue("欠载时必须补 0，不能把缓冲里的旧数据播出去", sink.writes[0].all { it == 0f })
    }

    @Test
    fun partialChunkIsPaddedWithSilenceAndSplitCorrectly() {
        val counters = PlaybackCounters()
        // 只给一半样本 = 一半帧。
        val halfSamples = PcmLayout.samplesForFrames(CHUNK_FRAMES / 2, CHANNELS)
        val source = FakeSource { dst -> fillRamp(dst); halfSamples }
        val sink = FakeSink { it }
        val loop = PlayoutLoop(source, sink, CHANNELS, CHUNK_FRAMES, counters)

        assertTrue(loop.pumpOnce())

        assertEquals((CHUNK_FRAMES / 2).toLong(), counters.framesWritten)
        assertEquals((CHUNK_FRAMES - CHUNK_FRAMES / 2).toLong(), counters.silenceFrames)
        assertEquals(1L, counters.sourceUnderruns)
        assertEquals((CHUNK_FRAMES / 2).toLong(), counters.starvedFrames)

        val written = sink.writes[0]
        assertEquals(1f, written[0], 0f)
        assertEquals(halfSamples.toFloat(), written[halfSamples - 1], 0f)
        assertEquals("后半块必须是静音", 0f, written[halfSamples], 0f)
        assertEquals(0f, written[written.size - 1], 0f)
    }

    @Test
    fun fullSinkReturnsFalseWithoutLosingData() {
        val counters = PlaybackCounters()
        val source = FakeSource { dst -> fillRamp(dst); dst.size }
        var acceptAll = false
        val sink = FakeSink { if (acceptAll) it else 0 }
        val loop = PlayoutLoop(source, sink, CHANNELS, CHUNK_FRAMES, counters)

        assertFalse("输出缓冲满：这一轮写不进，但不该丢数据", loop.pumpOnce())
        assertEquals("没写出去就不许记账", 0L, counters.framesWritten)

        acceptAll = true
        assertTrue(loop.pumpOnce())
        assertEquals(CHUNK_FRAMES.toLong(), counters.framesWritten)
        assertEquals("pending 未写完前不拉新数据，源只该被读一次", 1, source.readCount)
    }

    @Test
    fun repeatedBackpressureEventuallyWritesEverythingExactlyOnce() {
        val counters = PlaybackCounters()
        val source = FakeSource { dst -> fillRamp(dst); dst.size }
        val sink = FakeSink { minOf(1, it) } // 每次只接受 1 帧 = 被回压拆成 CHUNK_FRAMES 次
        val loop = PlayoutLoop(source, sink, CHANNELS, CHUNK_FRAMES, counters)

        repeat(CHUNK_FRAMES) { assertTrue(loop.pumpOnce()) }

        assertEquals("拆成多次写，累计口径只能记一次", CHUNK_FRAMES.toLong(), counters.framesWritten)
        assertEquals(CHUNK_FRAMES, sink.writes.size)
        assertEquals(1, source.readCount)
    }

    @Test
    fun persistentBackpressureDoesNotSpinTheSource() {
        val counters = PlaybackCounters()
        val source = FakeSource { dst -> fillRamp(dst); dst.size }
        val sink = FakeSink { 0 } // 永远写不进
        val loop = PlayoutLoop(source, sink, CHANNELS, CHUNK_FRAMES, counters)

        repeat(5) { assertFalse(loop.pumpOnce()) }

        assertEquals("写不进时反复调用也不该重复拉取源", 1, source.readCount)
        assertEquals(0L, counters.framesWritten)
        assertEquals(0, sink.writes.size)
    }
}
