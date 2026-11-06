package com.gotkicry.audiolink.diagnostics

import java.util.EnumMap
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicLongArray
import java.util.concurrent.atomic.AtomicReferenceArray

/**
 * 离散事件探针：定长环存「事件 tag + 发生时刻」—— 与 [IntervalRing] 同为纯逻辑、可 JVM 单测。
 *
 * 它回答的是「稀有但致命的事发生过没有、最近一次是什么时候」：播放环 overflow、
 * 供给欠载、补静音、AudioTrack 欠载这类事件在稳态下**应该是 0**，涨一次就是一条线索。
 *
 * tag 用 [ProbeEvent] 枚举而不是任意字符串：集合封闭，报告里每种事件的口径才有唯一解
 * （散落的字面量迟早漂移出两个名字数同一件事）。
 *
 * record 路径零锁、零分配（枚举单例 + 预分配原子数组）；统计只在 [snapshot] 时算。
 *
 * ## 为什么改成原子数组而不是 `@Synchronized`（docs/77 §S4 #2，2026-10-20）
 * 旧实现一把实例 monitor 被**三个线程**共用：FFI 回调线程（播放环 overflow / 欠载）、
 * 播放线程（补静音 / AudioTrack 欠载）、采集线程（截断 / 拉空）。共用 monitor 加上
 * 实时线程 = 优先级反转的教科书构图（持锁线程被降优先级，其余两个实时线程一起等）。
 * 而 `android/AGENTS.md` 对埋点的纪律本来就只有一句「热路径只写预分配数组 / 原子量」。
 *
 * 弱一致性口径与 [IntervalRing] 完全一致：每个槽带发布戳 [slotSeq]（序号 + 1，`0` = 从未写过），
 * 写侧「先写数据、最后写戳」，读侧「先验戳、再读值」—— 于是**快照永远不会把未写入的槽
 * 当成事件计入**（否则「没发生过的事件」会凭空出现，比少记一条更坏）。
 */
class EventRing(private val capacity: Int = DEFAULT_CAPACITY) {

    init {
        require(capacity > 0) { "capacity 必须为正：$capacity" }
    }

    /** 每个槽的事件 tag（枚举单例，无分配）。 */
    private val slotTags = AtomicReferenceArray<ProbeEvent>(capacity)

    /** 每个槽的发生时刻（纳秒）。 */
    private val slotNanos = AtomicLongArray(capacity)

    /**
     * 槽的发布戳 = 该槽这条记录的序号 + 1；`0` = 该槽从未写过（理由同 [IntervalRing.slotSeq]）。
     */
    private val slotSeq = AtomicLongArray(capacity)

    /** 已发出的记录序号（单调递增、不回绕；见 [IntervalRing] 同名说明）。 */
    private val reservations = AtomicLong()

    fun record(tag: ProbeEvent, nowNanos: Long = System.nanoTime()) {
        val seq = reservations.getAndIncrement()
        val index = (seq % capacity).toInt()
        slotNanos.set(index, nowNanos)
        slotTags.set(index, tag)
        // 最后写戳 = 发布：戳可见 ⇒ 上面两次 volatile 写一定已可见（JMM 保证 volatile 写之间不重排）。
        slotSeq.set(index, seq + 1)
    }

    /** 各 tag 在**当前环内容**里的计数与最近一次发生距今（快照路径允许分配）。 */
    fun snapshot(nowNanos: Long = System.nanoTime()): Map<ProbeEvent, EventTagSummary> {
        val counts = IntArray(ProbeEvent.entries.size)
        val lastNanos = LongArray(ProbeEvent.entries.size) { Long.MIN_VALUE }

        // 窗口 = 最近 min(已发序号数, capacity) 条；尾部可能在途，靠发布戳筛掉。
        val reserved = reservations.get()
        val window = minOf(reserved, capacity.toLong()).toInt()
        val oldest = reserved - window
        for (i in 0 until window) {
            val seq = oldest + i
            val index = (seq % capacity).toInt()
            if (slotSeq.get(index) != seq + 1L) continue
            val nanos = slotNanos.get(index)
            if (slotSeq.get(index) != seq + 1L) continue
            // 双保险：戳验过之后 tag 仍为 null 只可能是被下一圈清过，这一条不算事件。
            val tag = slotTags.get(index) ?: continue
            counts[tag.ordinal] += 1
            if (nanos > lastNanos[tag.ordinal]) lastNanos[tag.ordinal] = nanos
        }

        val out = EnumMap<ProbeEvent, EventTagSummary>(ProbeEvent::class.java)
        for (tag in ProbeEvent.entries) {
            val last = lastNanos[tag.ordinal]
            out[tag] = EventTagSummary(
                count = counts[tag.ordinal],
                lastAgeMs = if (last == Long.MIN_VALUE) {
                    null
                } else {
                    (nowNanos - last).coerceAtLeast(0L).toFloat() / NANOS_PER_MILLI_F
                },
            )
        }
        return out
    }

    companion object {
        const val DEFAULT_CAPACITY = 128

        private const val NANOS_PER_MILLI_F = 1_000_000f
    }
}

/** [EventRing] 里单个 tag 的摘要；[lastAgeMs] 为 `null` = 窗口内没发生过。 */
data class EventTagSummary(val count: Int, val lastAgeMs: Float?)

/** 探针事件的封闭集合（label 是报告/logcat 里的短名，改它就是改报告格式）。 */
enum class ProbeEvent(val label: String) {
    /** 播放环 drop-oldest 丢弃（生产快于消费 / 播放被卡住）。 */
    PLAYOUT_RING_OVERFLOW("ringOvf"),

    /** 播放环读不满且生产者活跃（供给欠载，口径同 [PcmRingBuffer] 的 underrunCount）。 */
    PLAYOUT_RING_UNDERRUN("ringUdr"),

    /** 供给活跃期间的欠载补静音（[PlaybackCounters.recordSilence]）。 */
    SILENCE_FILL("silence"),

    /** `AudioTrack.getUnderrunCount()` 出现正增量（设备侧真的断了一口）。 */
    TRACK_UNDERRUN("trkUdr"),

    /** 采集源返回样本数超过缓冲容量（源的实现 bug，被截断）。 */
    CAPTURE_TRUNCATED("capTrunc"),

    /** 内核拉采集拿到空（只该在环已关闭后发生；采集中出现 = 供给断了）。 */
    CAPTURE_PULL_EMPTY("pullEmpty"),
}
