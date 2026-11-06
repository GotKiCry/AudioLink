package com.gotkicry.audiolink.diagnostics

import java.lang.reflect.Modifier
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * [IntervalRing] 的统计口径测试：分位数、间隙判定（含 expectedIntervalMs 兜底）、回绕、停供中的读数。
 *
 * 全部用显式注入的时刻驱动，不碰真实时钟。
 */
class IntervalRingTest {

    private val ms = 1_000_000L
    private val t0 = 1_000_000_000L

    /** 在 [t0] 起按给定毫秒偏移各记一次。 */
    private fun ringAt(offsetsMs: LongArray, capacity: Int = IntervalRing.DEFAULT_CAPACITY): IntervalRing {
        val ring = IntervalRing(expectedIntervalMs = 10f, capacity = capacity)
        for (offset in offsetsMs) ring.record(frames = 480, nowNanos = t0 + offset * ms)
        return ring
    }

    @Test
    fun emptyRingReportsEmptySummary() {
        val s = IntervalRing(expectedIntervalMs = 10f).snapshot(nowNanos = t0)
        assertEquals(0, s.eventCount)
        assertEquals(0, s.gapCount)
        assertNull(s.lastGapAgeMs)
        assertNull(s.lastEventAgeMs)
        assertEquals(0f, s.p50Ms, 0.001f)
    }

    @Test
    fun steadyCadenceHasFlatPercentilesAndNoGaps() {
        // 11 个事件、10 ms 等间隔 → 10 条间隔全是 10 ms。
        val ring = ringAt(LongArray(11) { it * 10L })
        val s = ring.snapshot(nowNanos = t0 + 100L * ms)

        assertEquals(11, s.eventCount)
        assertEquals(0.1f, s.windowSeconds, 0.001f)
        assertEquals(10f, s.p50Ms, 0.001f)
        assertEquals(10f, s.p95Ms, 0.001f)
        assertEquals(10f, s.p99Ms, 0.001f)
        assertEquals(10f, s.maxMs, 0.001f)
        assertEquals("阈值 = max(2×10, 10+20) = 30 ms，稳态不该有间隙", 0, s.gapCount)
        assertNull(s.lastGapAgeMs)
        assertEquals(0f, s.lastEventAgeMs!!, 0.001f)
    }

    @Test
    fun intervalAboveMedianRuleIsAGap() {
        // 60 ms 的那一跳 > max(2×10, 10+20) = 30 ms → 一次间隙；起点是 t0+20ms 那个事件。
        val ring = ringAt(longArrayOf(0, 10, 20, 80, 90, 100))
        val s = ring.snapshot(nowNanos = t0 + 100L * ms)

        assertEquals(1, s.gapCount)
        assertEquals("间隙起点 = 前一个事件的时刻", 80f, s.lastGapAgeMs!!, 0.001f)
        assertEquals(60f, s.maxMs, 0.001f)
    }

    @Test
    fun medianZeroFallsBackToExpectedInterval() {
        // 三条 0 间隔（同一 tick 连记）把中位数压成 0 → 阈值按 expected=10 ms 兜底成 30 ms：
        // 25 ms 不算间隙，40 ms 算。
        val ring = ringAt(longArrayOf(0, 0, 0, 40, 65))
        val s = ring.snapshot(nowNanos = t0 + 65L * ms)

        assertEquals("中位数为 0 时间隙判定必须还有意义", 1, s.gapCount)
        // 唯一的间隙是 0→40ms 那一跳，起点 = 前一个事件（t0+0ms）；快照时刻 t0+65ms → 距今 65ms。
        assertEquals(65f, s.lastGapAgeMs!!, 0.001f)
    }

    @Test
    fun wraparoundKeepsOnlyNewestEvents() {
        // 容量 4 记 6 次：最早两次被回绕覆盖，统计只剩最近 4 个事件。
        val ring = ringAt(LongArray(6) { it * 10L }, capacity = 4)
        val s = ring.snapshot(nowNanos = t0 + 50L * ms)

        assertEquals(4, s.eventCount)
        assertEquals("窗口只剩 t0+20ms .. t0+50ms", 0.03f, s.windowSeconds, 0.001f)
        assertEquals(10f, s.maxMs, 0.001f)
        assertEquals(0, s.gapCount)
    }

    @Test
    fun ongoingStallShowsInLastEventAgeNotInGapCount() {
        // 间隙要等下一个事件到达才记得上：正在进行的停供只能从 lastEventAgeMs 看出来。
        val ring = ringAt(longArrayOf(0, 10))
        val s = ring.snapshot(nowNanos = t0 + 100L * ms)

        assertEquals(0, s.gapCount)
        assertEquals(90f, s.lastEventAgeMs!!, 0.001f)
    }

    @Test
    fun invalidParametersAreRejected() {
        org.junit.Assert.assertThrows(IllegalArgumentException::class.java) {
            IntervalRing(expectedIntervalMs = 10f, capacity = 0)
        }
        org.junit.Assert.assertThrows(IllegalArgumentException::class.java) {
            IntervalRing(expectedIntervalMs = 0f)
        }
    }

    @Test
    fun singleEventHasNoIntervalButIsStillReported() {
        // 只有一条记录 → 没有间隔，分位数只能是 0；但它确实发生过，n 与 lastEventAgeMs 必须如实。
        val ring = ringAt(longArrayOf(0))
        val s = ring.snapshot(nowNanos = t0 + 40L * ms)

        assertEquals(1, s.eventCount)
        assertEquals(0f, s.windowSeconds, 0.001f)
        assertEquals(0f, s.p50Ms, 0.001f)
        assertEquals(0, s.gapCount)
        assertNull(s.lastGapAgeMs)
        assertEquals(40f, s.lastEventAgeMs!!, 0.001f)
    }

    @Test
    fun unwrittenSlotsNeverEnterStatistics() {
        // 容量 8 只写 3 条：窗口必须就只算这 3 条。
        // 若把环里还没写过的槽当真实时刻（= 0）读进来，分位数会被 0 拉平、间隙数会暴涨 —— 这条就是护栏。
        val ring = ringAt(longArrayOf(0, 10, 20), capacity = 8)
        val s = ring.snapshot(nowNanos = t0 + 20L * ms)

        assertEquals("未写入的槽不许参与统计", 3, s.eventCount)
        assertEquals(0.02f, s.windowSeconds, 0.001f)
        assertEquals(10f, s.p50Ms, 0.001f)
        assertEquals(10f, s.maxMs, 0.001f)
        assertEquals("3 条记录之间没有间隙", 0, s.gapCount)
    }

    @Test
    fun concurrentRecordAndSnapshotKeepStatisticsPlausible() {
        // 无锁化之后 record 与 snapshot 会真并发（record 在实时线程，snapshot 在 dumpsys / 心跳）：
        // 弱一致性可以接受，但「读到未写入的 0」不行 —— 那会让 max 飙到无意义的量级。
        // 所有时刻都是注入的、整体跨度 ≈ 20 s；基准取到 1 小时（3.6e12 ns）远大于跨度，
        // 于是「幽灵 0」量出的间隔 ≈ 3.6e6 ms，与合法上界（20 s 出头）之间差着两个数量级。
        val epoch = 3_600_000_000_000L
        val capacity = 64
        val ring = IntervalRing(expectedIntervalMs = 10f, capacity = capacity)
        val writers = 3
        val perWriter = 2_000
        val barrier = CountDownLatch(1)
        val stopReader = AtomicBoolean(false)
        val readerFailure = AtomicReference<Throwable?>()
        val writerThreads = List(writers) { w ->
            Thread({
                barrier.await()
                for (i in 0 until perWriter) {
                    // 每个写者自己单调递增，写者之间错开 100 ms。
                    ring.record(frames = 480, nowNanos = epoch + (w * 100L + i * 10L) * ms)
                }
            }, "interval-writer-" + w)
        }
        val reader = Thread({
            barrier.await()
            while (!stopReader.get()) {
                val s = ring.snapshot(nowNanos = epoch)
                // 注意：多写者交错下相邻两条的时刻不保证单调，间隔**可以**合法地为负，
                // 所以这里不断言 p50 的正负 —— 只钉「窗口大小」与「不出现幽灵 0」。
                if (s.eventCount !in 0..capacity) {
                    readerFailure.compareAndSet(null, AssertionError("eventCount 越界：" + s.eventCount))
                    return@Thread
                }
                if (s.maxMs > 60_000f) {
                    readerFailure.compareAndSet(null, AssertionError("max 出现幽灵值：" + s.maxMs + " ms"))
                    return@Thread
                }
            }
        }, "interval-reader")

        writerThreads.forEach { it.start() }
        reader.start()
        barrier.countDown()
        writerThreads.forEach { it.join() }
        stopReader.set(true)
        reader.join()

        readerFailure.get()?.let { throw it }
        val finalSnapshot = ring.snapshot(nowNanos = epoch)
        // 记录总量 6000 条 >> 容量，写者全部退出后窗口必然贴满整圈。
        assertEquals("容量写满后窗口就是整圈", capacity, finalSnapshot.eventCount)
    }

    @Test
    fun recordTakesNoJavaMonitor() {
        // docs/77 §S4 #2 的回归护栏：record 落在实时线程上，一旦被重新打上 @Synchronized
        // （在 JVM 方法上就是 ACC_SYNCHRONIZED），实时线程会再次被 Java monitor 挡住。
        val record = IntervalRing::class.java.getDeclaredMethod(
            "record",
            Int::class.javaPrimitiveType,
            Long::class.javaPrimitiveType,
        )
        assertFalse("record 不许再上 monitor", Modifier.isSynchronized(record.modifiers))
        val snapshot = IntervalRing::class.java.getDeclaredMethod("snapshot", Long::class.javaPrimitiveType)
        assertFalse("snapshot 也不许上 monitor", Modifier.isSynchronized(snapshot.modifiers))
    }
}
