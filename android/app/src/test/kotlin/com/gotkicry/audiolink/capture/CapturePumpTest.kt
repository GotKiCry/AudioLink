package com.gotkicry.audiolink.capture

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [CapturePump] 的行为测试：把假采集源接到真环上，验证「读 → 换算 → 声道展开 → 帧装配 → 投环」全链。
 *
 * 这些正是**没有真机也能钉住**的部分；真机独有的东西（设备真的出声音、授权弹窗）不在这里。
 */
class CapturePumpTest {

    private class FakeFloatSource(
        override val format: CaptureFormat = CaptureFormat.DEFAULT,
        private val blocks: List<FloatArray> = emptyList(),
        private val forcedReturn: Int? = null,
    ) : AudioCaptureSource {
        var startCalls = 0
        var stopCalls = 0
        private var index = 0

        override fun start() {
            startCalls += 1
        }

        override fun read(buffer: CaptureBuffer): Int {
            forcedReturn?.let { return it }
            if (index >= blocks.size) return 0
            val block = blocks[index]
            index += 1
            val count = minOf(block.size, buffer.floats.size)
            block.copyInto(buffer.floats, 0, 0, count)
            return count
        }

        override fun stop() {
            stopCalls += 1
        }
    }

    private class FakeShortSource(
        override val format: CaptureFormat = CaptureFormat(encoding = PcmEncoding.S16),
        private val blocks: List<ShortArray> = emptyList(),
    ) : AudioCaptureSource {
        private var index = 0

        override fun start() = Unit

        override fun read(buffer: CaptureBuffer): Int {
            if (index >= blocks.size) return 0
            val block = blocks[index]
            index += 1
            val count = minOf(block.size, buffer.shorts.size)
            block.copyInto(buffer.shorts, 0, 0, count)
            return count
        }

        override fun stop() = Unit
    }

    private fun ring(capacity: Int = 64) = CaptureRing(capacity, channelCount = 2, topUpWaitMs = 0L)

    @Test
    fun floatBlockLandsInRingAsWholeFrames() {
        val ring = ring()
        val source = FakeFloatSource(blocks = listOf(floatArrayOf(1f, 2f, 3f, 4f)))
        val pump = CapturePump(source, ring)

        assertTrue(pump.pumpOnce())
        assertEquals(2L, pump.framesDelivered)
        assertEquals(0, pump.pendingSamples)

        val dst = FloatArray(4)
        assertEquals(4, ring.readBlocking(dst))
        assertEquals(1f, dst[0], 0f)
        assertEquals(4f, dst[3], 0f)
    }

    @Test
    fun shortBlockIsConvertedToFloat() {
        val ring = ring()
        val source = FakeShortSource(blocks = listOf(shortArrayOf(Short.MIN_VALUE, Short.MAX_VALUE)))
        val pump = CapturePump(source, ring)

        pump.pumpOnce()
        val dst = FloatArray(2)
        assertEquals(2, ring.readBlocking(dst))
        assertEquals(-1f, dst[0], 0f)
        assertEquals(32767f / 32768f, dst[1], 0f)
    }

    @Test
    fun monoSourceIsExpandedToStereo() {
        val ring = ring()
        val source = FakeFloatSource(
            format = CaptureFormat(channelCount = 1),
            blocks = listOf(floatArrayOf(0.25f, -0.25f)),
        )
        val pump = CapturePump(source, ring)

        pump.pumpOnce()
        val dst = FloatArray(4)
        assertEquals(4, ring.readBlocking(dst))
        assertEquals(0.25f, dst[0], 0f)
        assertEquals(0.25f, dst[1], 0f)
        assertEquals(-0.25f, dst[2], 0f)
        assertEquals(-0.25f, dst[3], 0f)
    }

    @Test
    fun oddTailIsHeldForTheNextBlock() {
        val ring = ring()
        val source = FakeFloatSource(
            blocks = listOf(
                floatArrayOf(1f, 2f, 3f),
                floatArrayOf(4f),
            ),
        )
        val pump = CapturePump(source, ring)

        assertTrue(pump.pumpOnce())
        assertEquals(1L, pump.framesDelivered)
        assertEquals(1, pump.pendingSamples)

        assertTrue(pump.pumpOnce())
        assertEquals(2L, pump.framesDelivered)
        assertEquals(0, pump.pendingSamples)

        val dst = FloatArray(4)
        assertEquals(4, ring.readBlocking(dst))
        assertEquals(3f, dst[2], 0f)
        assertEquals(4f, dst[3], 0f)
    }

    @Test
    fun zeroReadReportsNoData() {
        val ring = ring()
        val pump = CapturePump(FakeFloatSource(forcedReturn = 0), ring)
        assertFalse(pump.pumpOnce())
        assertEquals(0L, pump.framesDelivered)
        assertEquals(1L, pump.readCalls)
    }

    @Test
    fun negativeReadIsReportedAsFailure() {
        val pump = CapturePump(FakeFloatSource(forcedReturn = -3), ring())
        try {
            pump.pumpOnce()
            throw AssertionError("源的负返回必须抛 CaptureFailure（否则采集会静默死掉）")
        } catch (failure: CaptureFailure) {
            assertEquals(CaptureErrorKind.Internal, failure.kind)
        }
    }

    @Test
    fun oversizedReadIsTruncatedAndCounted() {
        val ring = ring()
        val pump = CapturePump(FakeFloatSource(forcedReturn = 10_000), ring)
        pump.pumpOnce()
        assertEquals("源声称超量必须被记账", 1L, pump.truncatedReads)
        assertTrue(
            "截断到缓冲容量后，容量内的那部分数据仍应投递（实测 ${pump.framesDelivered} 帧）",
            pump.framesDelivered > 0L,
        )
    }

    @Test
    fun unsupportedSourceChannelCountIsRejected() {
        try {
            CapturePump(FakeFloatSource(format = CaptureFormat(channelCount = 3)), ring())
            throw AssertionError("3 声道源应当被拒绝（不猜布局）")
        } catch (_: IllegalArgumentException) {
            // 期望路径。
        }
    }

    @Test
    fun resamplingIsRejectedAtConstruction() {
        try {
            CapturePump(FakeFloatSource(format = CaptureFormat(sampleRateHz = 44_100)), ring())
            throw AssertionError("44.1 kHz 源必须被拒绝（全链路禁重采样）")
        } catch (_: IllegalArgumentException) {
            // 期望路径。
        }
    }
}
