package com.gotkicry.audiolink.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [QueueWatermark] 的测试 —— 全部在纯 JVM 上跑（不需要 Android、不需要真机）。
 *
 * 为什么这几条必须钉住：队列水位是 task-8 唯一的新判据，它错了会以两种最难查的方式出现 ——
 *  · `playbackHeadPosition` 回绕没处理：queued 瞬间跳到 2^32，门控永久失效（退化成满灌，
 *    表现为"改了但延迟没降"）；
 *  · 门控阈值搞反：要么永不写（静音）、要么每拍都写（等于没改）。
 * 两者都不会报错，只会让数字看起来"没变化"。
 */
class QueueWatermarkTest {

    @Test
    fun startsEmptyBeforeAnyHeadSample() {
        val watermark = QueueWatermark()

        assertEquals("还没送过数据，水位为 0", 0L, watermark.queuedFrames)
        assertEquals(0L, watermark.fedFrames)
        assertEquals(0L, watermark.consumedFrames)
    }

    @Test
    fun firstHeadSampleOnlyBuildsTheBaseline() {
        val watermark = QueueWatermark()

        // play() 之后 head position 从 0 开始；第一拍只建基准，不产生"已消费"增量。
        watermark.advanceHead(0)
        assertEquals("第一拍不能凭空算出一大段消费", 0L, watermark.consumedFrames)

        watermark.recordFed(960)
        assertEquals(960L, watermark.queuedFrames)
    }

    @Test
    fun consumingFramesLowersTheQueue() {
        val watermark = QueueWatermark()
        watermark.advanceHead(0)
        watermark.recordFed(1_920)

        watermark.advanceHead(480)
        assertEquals(480L, watermark.consumedFrames)
        assertEquals(1_440L, watermark.queuedFrames)

        watermark.advanceHead(1_920)
        assertEquals(1_920L, watermark.consumedFrames)
        assertEquals("该播的都播完了，水位回到 0", 0L, watermark.queuedFrames)
    }

    @Test
    fun headPositionWrapAroundDoesNotExplodeTheQueue() {
        // 复现现场：head 已经涨到 u32 顶端，再消费 300 帧就回绕到接近 0。
        // 若不处理回绕，raw 差值会是 -4294966996，queued 直接跳到 2^32 量级 —— 门控从此永久失效。
        val watermark = QueueWatermark()
        watermark.advanceHead(0xFFFFFF00.toInt()) // u32 = 4294967040
        watermark.recordFed(1_000)

        val consumed = watermark.advanceHead(0x0000002C) // 回绕后 300 帧

        assertEquals("回绕必须当作 +300 帧，而不是巨大负数", 300L, consumed)
        assertEquals(700L, watermark.queuedFrames)
    }

    @Test
    fun queueNeverGoesNegative() {
        // 设备比我们的记账跑得快（例如清空缓冲/seek）时，queued 不能变成负数，
        // 否则 shouldWrite 的判据会永远为真（又退回满灌）。
        val watermark = QueueWatermark()
        watermark.advanceHead(10_000)
        watermark.recordFed(100)

        watermark.advanceHead(20_000)

        assertEquals("理论下界是 0", 0L, watermark.queuedFrames)
    }

    @Test
    fun unlimitedTargetAlwaysWrites() {
        val watermark = QueueWatermark()
        watermark.advanceHead(0)
        watermark.recordFed(100_000)

        assertTrue(
            "target=0 是 A/B 的对照侧（旧的满灌行为），必须与 task-3/5 基线逐字一致",
            watermark.shouldWrite(LowLatencyPlayer.QUEUE_TARGET_UNLIMITED, 480),
        )
    }

    @Test
    fun gatedTargetKeepsOneChunkOfHeadroom() {
        val watermark = QueueWatermark()
        watermark.advanceHead(0)
        watermark.recordFed(1_920) // queued = 1920 = 目标本身

        assertFalse("水位已经到位，这一拍不该再写", watermark.shouldWrite(1_920, 480))

        watermark.advanceHead(479) // queued = 1441，仍高于阈值 1440
        assertFalse(watermark.shouldWrite(1_920, 480))

        watermark.advanceHead(481) // queued = 960-1+... → 1439 ≤ 1440
        assertTrue("掉到一个 chunk 以外就该补上", watermark.shouldWrite(1_920, 480))
    }

    @Test
    fun tinyTargetStillWritesOnceQueueDrains() {
        val watermark = QueueWatermark()
        watermark.advanceHead(0)

        // target 小于一个 chunk 时阈值会被夹到 1：只要队列空了就写（否则永远不会写 → 静音）。
        assertTrue(watermark.shouldWrite(240, 480))
    }

    @Test
    fun lowLatencyTargetRefillsBeforeTheLastChunkIsConsumed() {
        val watermark = QueueWatermark()
        watermark.advanceHead(0)
        watermark.recordFed(960)
        assertFalse(watermark.shouldWrite(960, 480))

        watermark.advanceHead(480)
        assertTrue(watermark.shouldWrite(960, 480))
        watermark.recordFed(480)
        assertEquals(960L, watermark.queuedFrames)
    }

    @Test
    fun resetClearsEverythingForANewTrack() {
        val watermark = QueueWatermark()
        watermark.advanceHead(0)
        watermark.recordFed(5_000)
        watermark.advanceHead(1_000)

        watermark.reset()

        assertEquals(0L, watermark.queuedFrames)
        assertEquals(0L, watermark.fedFrames)
        assertEquals(0L, watermark.consumedFrames)
        // 重建 track 后 head position 从 0 重新起算，第一次采样仍只建基准。
        watermark.advanceHead(0)
        assertEquals(0L, watermark.consumedFrames)
    }
}
