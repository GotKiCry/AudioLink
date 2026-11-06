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
 * [EventRing] 的计数口径测试：各 tag 分别计数、最近一次距今、回绕淘汰最旧事件。
 */
class EventRingTest {

    private val ms = 1_000_000L
    private val t0 = 1_000_000_000L

    @Test
    fun countsAndLastOccurrenceArePerTag() {
        val ring = EventRing()
        ring.record(ProbeEvent.PLAYOUT_RING_OVERFLOW, t0)
        ring.record(ProbeEvent.PLAYOUT_RING_OVERFLOW, t0 + 10L * ms)
        ring.record(ProbeEvent.SILENCE_FILL, t0 + 20L * ms)

        val snap = ring.snapshot(nowNanos = t0 + 100L * ms)

        assertEquals(ProbeEvent.entries.size, snap.size)
        assertEquals(2, snap.getValue(ProbeEvent.PLAYOUT_RING_OVERFLOW).count)
        assertEquals(90f, snap.getValue(ProbeEvent.PLAYOUT_RING_OVERFLOW).lastAgeMs!!, 0.001f)
        assertEquals(1, snap.getValue(ProbeEvent.SILENCE_FILL).count)
        assertEquals(80f, snap.getValue(ProbeEvent.SILENCE_FILL).lastAgeMs!!, 0.001f)
        assertEquals("没发生过的 tag 也要在报告里（计数 0，无时刻）", 0, snap.getValue(ProbeEvent.TRACK_UNDERRUN).count)
        assertNull(snap.getValue(ProbeEvent.TRACK_UNDERRUN).lastAgeMs)
    }

    @Test
    fun wraparoundEvictsOldestEvents() {
        // 容量 3 记 4 次：t0 那条 overflow 被淘汰。
        val ring = EventRing(capacity = 3)
        ring.record(ProbeEvent.PLAYOUT_RING_OVERFLOW, t0)
        ring.record(ProbeEvent.TRACK_UNDERRUN, t0 + 1L * ms)
        ring.record(ProbeEvent.PLAYOUT_RING_OVERFLOW, t0 + 2L * ms)
        ring.record(ProbeEvent.TRACK_UNDERRUN, t0 + 3L * ms)

        val snap = ring.snapshot(nowNanos = t0 + 3L * ms)

        assertEquals(1, snap.getValue(ProbeEvent.PLAYOUT_RING_OVERFLOW).count)
        assertEquals("环里仅剩 t0+2ms 那条 overflow，快照在 t0+3ms → 距今 1ms", 1f, snap.getValue(ProbeEvent.PLAYOUT_RING_OVERFLOW).lastAgeMs!!, 0.001f)
        assertEquals(2, snap.getValue(ProbeEvent.TRACK_UNDERRUN).count)
        assertEquals(0f, snap.getValue(ProbeEvent.TRACK_UNDERRUN).lastAgeMs!!, 0.001f)
    }

    @Test
    fun unwrittenSlotsAreNotReportedAsEvents() {
        // 容量 8 只记 1 条：其余 7 个槽从没写过，绝不能变成「发生过的事件」。
        // 无锁实现的读侧必须靠发布戳把手压在这里 —— 幽灵事件比少记一条更难查。
        val ring = EventRing(capacity = 8)
        ring.record(ProbeEvent.PLAYOUT_RING_UNDERRUN, t0)

        val snap = ring.snapshot(nowNanos = t0 + 5L * ms)

        assertEquals(1, snap.getValue(ProbeEvent.PLAYOUT_RING_UNDERRUN).count)
        assertEquals(0, snap.getValue(ProbeEvent.PLAYOUT_RING_OVERFLOW).count)
        val total = ProbeEvent.entries.sumOf { snap.getValue(it).count }
        assertEquals("环里只有 1 条事件", 1, total)
    }

    @Test
    fun concurrentRecordingFromThreeThreadsLosesNoEvent() {
        // 一把锁被三线程共用改成无锁之后，最怕的是「丢更新」：并发的 record 互相覆盖。
        // 容量远大于总量（没有回绕淘汰），所以每条都必须一条不少地留在环里。
        val perWriter = 400
        val tags = arrayOf(
            ProbeEvent.SILENCE_FILL,
            ProbeEvent.TRACK_UNDERRUN,
            ProbeEvent.CAPTURE_TRUNCATED,
        )
        val ring = EventRing(capacity = 4 * perWriter)
        val barrier = CountDownLatch(1)
        val threads = tags.mapIndexed { w, tag ->
            Thread({
                barrier.await()
                for (i in 0 until perWriter) ring.record(tag, t0 + (w * 100L + i * 10L) * ms)
            }, "event-writer-" + w)
        }

        threads.forEach { it.start() }
        barrier.countDown()
        threads.forEach { it.join() }

        val snap = ring.snapshot(nowNanos = t0)
        for (tag in tags) {
            assertEquals("并发记录不许丢更新：" + tag.label, perWriter, snap.getValue(tag).count)
        }
        assertEquals(0, snap.getValue(ProbeEvent.PLAYOUT_RING_OVERFLOW).count)
    }

    @Test
    fun concurrentSnapshotNeverReportsGhostEvents() {
        // 快照与写入真并发时允许弱一致，但不许读到「还没写入的 0」：
        // 注入时刻的基准取到 1 小时（3.6e12 ns），幽灵 0 会量出 ≈ 3.6e6 ms 的「距今」，
        // 与合法上界（几毫秒）之间差着六个数量级。
        val epoch = 3_600_000_000_000L
        val capacity = 64
        val ring = EventRing(capacity = capacity)
        val tags = arrayOf(
            ProbeEvent.PLAYOUT_RING_OVERFLOW,
            ProbeEvent.SILENCE_FILL,
            ProbeEvent.TRACK_UNDERRUN,
        )
        val perWriter = 2_000
        val barrier = CountDownLatch(1)
        val stopReader = AtomicBoolean(false)
        val readerFailure = AtomicReference<Throwable?>()
        val writers = tags.mapIndexed { w, tag ->
            Thread({
                barrier.await()
                for (i in 0 until perWriter) ring.record(tag, epoch + (w * 100L + i * 10L) * ms)
            }, "event-writer-" + w)
        }
        val reader = Thread({
            barrier.await()
            while (!stopReader.get()) {
                val snap = ring.snapshot(nowNanos = epoch)
                var total = 0
                for (tag in ProbeEvent.entries) {
                    val summary = snap.getValue(tag)
                    total += summary.count
                    val age = summary.lastAgeMs ?: continue
                    if (age > 1_000f) {
                        readerFailure.compareAndSet(
                            null,
                            AssertionError("幽灵事件时刻：" + tag.label + " age=" + age + "ms"),
                        )
                        return@Thread
                    }
                }
                if (total > capacity) {
                    readerFailure.compareAndSet(null, AssertionError("窗口计数超过容量：" + total))
                    return@Thread
                }
            }
        }, "event-reader")

        writers.forEach { it.start() }
        reader.start()
        barrier.countDown()
        writers.forEach { it.join() }
        stopReader.set(true)
        reader.join()

        readerFailure.get()?.let { throw it }
        val finalSnap = ring.snapshot(nowNanos = epoch)
        val finalTotal = ProbeEvent.entries.sumOf { finalSnap.getValue(it).count }
        // 写者全部退出后窗口必然贴满整圈，且每条都只算一次。
        assertEquals(capacity, finalTotal)
    }

    @Test
    fun recordTakesNoJavaMonitor() {
        // docs/77 §S4 #2 的回归护栏：record 落在实时线程上（FFI 回调 / 播放线程 / 采集线程），
        // 一旦被重新打上 @Synchronized（JVM 方法上就是 ACC_SYNCHRONIZED），三个线程又会在同一把
        // monitor 上排队，优先级反转照旧。
        val record = EventRing::class.java.getDeclaredMethod(
            "record",
            ProbeEvent::class.java,
            Long::class.javaPrimitiveType,
        )
        assertFalse("record 不许再上 monitor", Modifier.isSynchronized(record.modifiers))
        val snapshot = EventRing::class.java.getDeclaredMethod("snapshot", Long::class.javaPrimitiveType)
        assertFalse("snapshot 也不许上 monitor", Modifier.isSynchronized(snapshot.modifiers))
    }
}
