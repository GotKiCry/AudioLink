package com.gotkicry.audiolink.capture

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

/**
 * [CaptureRing] 的行为测试。
 *
 * 这里的重点不是「能存能取」，而是两条**只有并发才暴露**的纪律：
 * 1. 没数据时必须**阻塞**（而不是返回 0 让内核去热循环）；
 * 2. [CaptureRing.close] 必须**唤醒**阻塞中的读取者 —— 否则内核采集线程会永远停在 `readPcm` 里，
 *    引擎连停都停不下来。
 */
class CaptureRingTest {

    private fun ring(capacity: Int = 32) = CaptureRing(capacity, channelCount = 2, topUpWaitMs = 0L)

    @Test
    fun writtenFramesComeBackAsInterleavedSamples() {
        val ring = ring()
        ring.write(floatArrayOf(1f, 2f, 3f, 4f), 2)
        val dst = FloatArray(4)
        val got = ring.readBlocking(dst)
        assertEquals(4, got)
        assertEquals(1f, dst[0], 0f)
        assertEquals(4f, dst[3], 0f)
    }

    @Test
    fun readNeverExceedsDestinationCapacity() {
        val ring = ring()
        repeat(10) { ring.write(floatArrayOf(1f, 2f), 1) }
        val dst = FloatArray(4)
        val got = ring.readBlocking(dst)
        assertEquals("只能给 dst 装得下的量", 4, got)
        assertEquals("读走的是 4 个样本 = 2 帧，10 帧里应当还剩 8 帧", 8, ring.availableFrames)
    }

    @Test
    fun blockingReadWakesUpWhenDataArrives() {
        val ring = ring()
        val got = AtomicInteger(-1)
        val reader = Thread {
            val dst = FloatArray(2)
            got.set(ring.readBlocking(dst))
        }
        reader.start()
        Thread.sleep(30) // 让读者进入等待
        assertEquals("还没写就必须还在等", -1, got.get())
        ring.write(floatArrayOf(0.5f, 0.5f), 1)
        reader.join(2_000)
        assertFalse("读者必须被唤醒并退出", reader.isAlive)
        assertEquals(2, got.get())
    }

    @Test
    fun closeWakesUpBlockingReaderAndReturnsZero() {
        val ring = ring()
        val got = AtomicInteger(-1)
        val reader = Thread {
            val dst = FloatArray(2)
            got.set(ring.readBlocking(dst))
        }
        reader.start()
        Thread.sleep(30)
        ring.close()
        reader.join(2_000)
        assertFalse("close 必须唤醒阻塞中的读取者（否则引擎停不下来）", reader.isAlive)
        assertEquals(0, got.get())
    }

    @Test
    fun afterCloseWritesAreDroppedAndReadsReturnImmediately() {
        val ring = ring()
        ring.close()
        ring.write(floatArrayOf(1f, 2f), 1)
        val startedAt = System.nanoTime()
        val got = ring.readBlocking(FloatArray(4))
        val elapsedMs = (System.nanoTime() - startedAt) / 1_000_000
        assertEquals(0, got)
        assertTrue("关闭后不该再阻塞（实测 ${elapsedMs} ms）", elapsedMs < 50)
    }

    @Test
    fun overflowIsCountedWhenProducerOutrunsConsumer() {
        val ring = ring(capacity = 4)
        repeat(20) { ring.write(floatArrayOf(1f, 2f), 1) }
        assertTrue("drop-oldest 口径下溢出必须被计数：${ring.overflowFrames}", ring.overflowFrames > 0)
        assertEquals("环里最多留容量的那一份", 4, ring.availableFrames)
    }

    @Test
    fun closedFlagReflectsLifecycle() {
        val ring = ring()
        assertFalse(ring.isClosed)
        ring.close()
        assertTrue(ring.isClosed)
        ring.close() // 幂等
        assertTrue(ring.isClosed)
    }
}
