package com.gotkicry.audiolink.capture

import com.gotkicry.audiolink.core.PcmPull
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

/**
 * [CapturePcmPull] 的契约测试 —— 断言的每一条都来自 FFI 侧的白纸黑字
 * （`core/crates/audiolink-ffi/src/audio_bridge.rs` 的 `PcmPull` 文档与 `KotlinCaptureSource::read` 的实现）。
 *
 * 内核消费方的三条硬约束：长度**偶数**且 ≤ `max_samples`；空返回只表示「暂无数据」；
 * **绝不忙等**（内核拿到空返回只睡最多 5 ms 就再来拉）。
 */
class CapturePcmPullTest {

    private fun ring(capacity: Int = 64) = CaptureRing(capacity, channelCount = 2, topUpWaitMs = 0L)

    @Test
    fun implementsTheGeneratedPcmPullInterface() {
        val pull: PcmPull = CapturePcmPull(ring())
        assertTrue("必须是生成绑定里的 PcmPull 实现（引擎按这个类型收）", pull is PcmPull)
    }

    @Test
    fun neverExceedsMaxSamplesAndKeepsLengthEven() {
        val ring = ring()
        repeat(10) { ring.write(floatArrayOf(1f, 2f), 1) }
        val pull = CapturePcmPull(ring)

        // 内核一次要 4 个样本：正好两帧。
        assertEquals(4, pull.readPcm(4).size)

        // 奇数上限：宁少给一个，也不给半帧（契约要求偶数长度）。
        val odd = pull.readPcm(3)
        assertEquals("奇数上限要退到偶数", 2, odd.size)
        assertEquals(0, odd.size % 2)
    }

    @Test
    fun nonPositiveMaxSamplesReturnsEmpty() {
        val ring = ring()
        ring.write(floatArrayOf(1f, 2f), 1)
        val pull = CapturePcmPull(ring)
        assertTrue(pull.readPcm(0).isEmpty())
        assertTrue(pull.readPcm(-8).isEmpty())
        assertEquals("数据不能因为一次非法请求就消失", 1, ring.availableFrames)
    }

    @Test
    fun blocksUntilDataArrivesThenReturnsIt() {
        val ring = ring()
        val pull = CapturePcmPull(ring)
        val returned = AtomicInteger(-1)
        val reader = Thread { returned.set(pull.readPcm(4).size) }

        reader.start()
        Thread.sleep(30)
        assertEquals("没数据时必须阻塞（返回空会让内核热循环）", -1, returned.get())

        ring.write(floatArrayOf(0.5f, 0.5f, 0.25f, 0.25f), 2)
        reader.join(2_000)
        assertFalse("写数据后必须被唤醒", reader.isAlive)
        assertEquals(4, returned.get())
    }

    @Test
    fun returnsEmptyAfterCloseWithoutBlocking() {
        val ring = ring()
        ring.close()
        val pull = CapturePcmPull(ring)

        val startedAt = System.nanoTime()
        assertTrue(pull.readPcm(1_920).isEmpty())
        val elapsedMs = (System.nanoTime() - startedAt) / 1_000_000
        assertTrue("关闭后必须立刻返回（实测 ${elapsedMs} ms）", elapsedMs < 50)
        assertEquals("这就是内核侧 read_timeouts 的来源，必须被计数", 1L, pull.emptyReturns)
    }

    @Test
    fun samplesKeepInterleavedOrder() {
        val ring = ring()
        val pull = CapturePcmPull(ring)
        ring.write(floatArrayOf(1f, 2f, 3f, 4f), 2)
        val got = pull.readPcm(4)
        assertEquals(listOf(1f, 2f, 3f, 4f), got)
    }
}
