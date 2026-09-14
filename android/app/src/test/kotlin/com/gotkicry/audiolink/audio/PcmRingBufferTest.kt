package com.gotkicry.audiolink.audio

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * [PcmRingBuffer] 的口径测试（JVM，无 Android 依赖）。
 *
 * 这里刻意把每个数字都断言死（容量 / 溢出 / 欠载 / 缺失帧数 / 环绕顺序），因为真机上一旦这些
 * 计数错了，看到的只是"声音偶尔卡一下"，根本追不到根因。
 */
class PcmRingBufferTest {

    /**
     * 造 n 帧 2ch 交错样本：第 f 帧 = `[f*10+1, f*10+2]`。
     * 用帧号编码数值，是为了让"读到几号帧、有没有串声道"一眼可辨。
     */
    private fun interleaved2(frames: Int, startFrame: Int = 0): FloatArray =
        FloatArray(PcmLayout.samplesForFrames(frames, 2)) { i ->
            val frame = startFrame + PcmLayout.framesForSamples(i, 2)
            (frame * 10 + (i % 2) + 1).toFloat()
        }

    @Test
    fun writeAndReadBackPreservesInterleavedOrder() {
        val ring = PcmRingBuffer(capacityFrames = 8, channelCount = 2)
        val data = interleaved2(8)

        assertEquals("写满容量应全部写入", 8, ring.write(data, 8))
        assertEquals(8, ring.sizeFrames)
        assertEquals(0, ring.availableFrames)

        val out = FloatArray(16)
        assertEquals("读满 8 帧应返回 16 个样本", 16, ring.readInto(out))
        assertArrayEquals("读回内容必须与写入逐样本一致（含左右声道不串位）", data, out, 0f)
        assertEquals(0, ring.sizeFrames)
        assertEquals("读满不算欠载", 0L, ring.underrunCount)
        assertEquals(0L, ring.overflowFrames)
    }

    @Test
    fun writeBeyondCapacityDropsOldestFramesAndCountsOverflow() {
        val ring = PcmRingBuffer(capacityFrames = 8, channelCount = 2)
        ring.write(interleaved2(8, startFrame = 0), 8)

        // 环已满，再写 3 帧 → 最旧的 3 帧（帧 0/1/2）被丢弃。
        val written = ring.write(interleaved2(3, startFrame = 100), 3)

        assertEquals("drop-oldest 下写入永不失败", 3, written)
        assertEquals("容量是硬上界", 8, ring.sizeFrames)
        assertEquals("丢弃的正是 3 帧最旧数据", 3L, ring.overflowFrames)

        val out = FloatArray(16)
        assertEquals(16, ring.readInto(out))
        // 保留的是 帧3..帧7 + 帧100..帧102（丢的是最旧，即"保证延迟不累积"）。
        val expected = interleaved2(5, startFrame = 3) + interleaved2(3, startFrame = 100)
        assertArrayEquals(expected, out, 0f)
    }

    @Test
    fun singleWriteLargerThanCapacityKeepsNewestFrames() {
        val ring = PcmRingBuffer(capacityFrames = 8, channelCount = 2)

        assertEquals(20, ring.write(interleaved2(20, startFrame = 0), 20))
        assertEquals(8, ring.sizeFrames)
        assertEquals("一次性超容量写入丢弃 12 帧", 12L, ring.overflowFrames)

        val out = FloatArray(16)
        ring.readInto(out)
        assertArrayEquals(interleaved2(8, startFrame = 12), out, 0f)
    }

    @Test
    fun readFromEmptyRingCountsOneUnderrun() {
        val ring = PcmRingBuffer(capacityFrames = 8, channelCount = 2)
        val out = FloatArray(8)

        assertEquals("空环读不到任何样本", 0, ring.readInto(out))
        assertEquals("读不满一次 = 一次欠载", 1L, ring.underrunCount)
        assertEquals("缺失帧数按帧计", 4L, ring.starvedFramesTotal)
    }

    @Test
    fun partialReadCountsUnderrunAndMissingFrames() {
        val ring = PcmRingBuffer(capacityFrames = 8, channelCount = 2)
        ring.write(interleaved2(2, startFrame = 7), 2)

        val out = FloatArray(8) // 请求 4 帧，环里只有 2 帧
        assertEquals("只有 2 帧可读 → 返回 4 个样本", 4, ring.readInto(out))
        assertEquals(1L, ring.underrunCount)
        assertEquals("缺 2 帧", 2L, ring.starvedFramesTotal)
        assertArrayEquals(
            interleaved2(2, startFrame = 7),
            out.copyOfRange(0, 4),
            0f,
        )
        assertEquals("读空后环内无残留", 0, ring.sizeFrames)
    }

    @Test
    fun readIntoLeavesTailUntouchedWhenStarved() {
        val ring = PcmRingBuffer(capacityFrames = 8, channelCount = 2)
        ring.write(interleaved2(2, startFrame = 1), 2)

        val out = FloatArray(8) { -1f } // 4 帧请求
        ring.readInto(out)

        // 环缓冲**不补零**（补零是消费方 PlayoutLoop 的职责），未覆盖的尾部必须保持原值。
        assertEquals(-1f, out[4], 0f)
        assertEquals(-1f, out[7], 0f)
    }

    @Test
    fun wrapAroundKeepsFrameOrder() {
        val ring = PcmRingBuffer(capacityFrames = 5, channelCount = 2)
        ring.write(interleaved2(3, startFrame = 0), 3)
        ring.readInto(FloatArray(4)) // 读 2 帧 → 环内剩 1 帧，读指针前进

        ring.write(interleaved2(4, startFrame = 10), 4) // 这一写必然跨越环绕点
        assertEquals("1 + 4 = 5，恰好写满且不丢", 5, ring.sizeFrames)
        assertEquals(0L, ring.overflowFrames)

        val out = FloatArray(10)
        assertEquals(10, ring.readInto(out))
        val expected = interleaved2(1, startFrame = 2) + interleaved2(4, startFrame = 10)
        assertArrayEquals("环绕后顺序不能错位", expected, out, 0f)
    }

    @Test
    fun clearDropsDataButKeepsCounters() {
        val ring = PcmRingBuffer(capacityFrames = 4, channelCount = 2)
        ring.write(interleaved2(6, startFrame = 0), 6) // 溢出 2 帧
        ring.readInto(FloatArray(4)) // 读 2 帧（环满 4 帧）

        ring.clear()
        assertEquals("clear 后数据清空", 0, ring.sizeFrames)
        assertEquals("清数据≠没发生过溢出：统计是刻意保留的", 2L, ring.overflowFrames)

        ring.resetStats()
        assertEquals(0L, ring.overflowFrames)
        assertEquals(0L, ring.underrunCount)
    }

    @Test(expected = IllegalArgumentException::class)
    fun writeWithTooSmallBufferIsRejected() {
        val ring = PcmRingBuffer(capacityFrames = 8, channelCount = 2)
        ring.write(FloatArray(2), frames = 4) // 4 帧需要 8 个样本
    }

    @Test(expected = IllegalArgumentException::class)
    fun zeroCapacityIsRejected() {
        PcmRingBuffer(capacityFrames = 0, channelCount = 2)
    }

    @Test
    fun readIntoIgnoresPartialFrameTailInDstSize() {
        val ring = PcmRingBuffer(capacityFrames = 8, channelCount = 2)
        ring.write(interleaved2(3, startFrame = 0), 3)

        val out = FloatArray(7) { -1f } // 7 个样本 = 3 整帧 + 1 个零头
        assertEquals("只按整帧读取 → 3 帧 = 6 个样本", 6, ring.readInto(out))
        assertEquals("零头元素不属于任何帧，保持原值", -1f, out[6], 0f)
        assertEquals("没有欠载：请求的 3 帧都在", 0L, ring.underrunCount)
    }
}
