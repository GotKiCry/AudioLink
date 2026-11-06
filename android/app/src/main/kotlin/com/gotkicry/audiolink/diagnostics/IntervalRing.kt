package com.gotkicry.audiolink.diagnostics

import java.util.concurrent.atomic.AtomicIntegerArray
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicLongArray
import kotlin.math.ceil

/**
 * 时序探针：定长环里存「每次回调的时刻 + 交付帧数」—— **纯逻辑，可 JVM 单测**。
 *
 * 它回答的问题是「供给节拍稳不稳」：诊断面板上的累计计数器全绿，也可能声音在断 ——
 * 计数器只告诉你「少了多少」，说不出「什么时候、以什么节奏断」。本环把每次供给的
 * 到达间隔留下来，间隙检测（[IntervalSummary.gapCount] / [IntervalSummary.lastGapAgeMs]）
 * 就是「断断续续」的直接证据。
 *
 * ## 纪律
 * - **record 路径零锁、零分配**：存储是预分配的 [AtomicLongArray] / [AtomicIntegerArray]，回绕复用；
 *   统计（分位数、间隙）只在 [snapshot] 时算 —— 快照走 dumpsys / 5 s 心跳，不在热路径上。
 * - 间隙阈值：间隔 > max(2×基准, 基准+20 ms)，基准 = 窗口内间隔中位数；
 *   **中位数为 0 时**（同一 tick 连记、或只有一条间隔）按构造传入的
 *   [expectedIntervalMs] 兜底 —— 否则阈值是 0，任何间隔都会被误判成间隙。
 *
 * ## 为什么改成原子数组而不是 `@Synchronized`（docs/77 §S4 #2，2026-10-20）
 * record 落在**实时线程**上：FFI 回调线程（内核推 PCM）、采集线程、播放线程。
 * Kotlin 的 `@Synchronized` 是实例 monitor，持锁线程一旦被调度走，实时线程就只能停在入口等 ——
 * 优先级反转在这里等价于「供给停摆」。`android/AGENTS.md` 对埋点本来就要求
 * 「热路径只写预分配数组 / 原子量」，探针自身不该是例外。
 *
 * ## 无锁带来的弱一致性（刻意接受，但要挡住最坏那一种）
 * 快照是**取证**用途（看趋势、找「什么时候断的」），不是计费口径，允许读到写读交错的中间态；
 * 但**绝不允许把还没写过的槽当成真实时刻** —— 一个 0 会同时污染分位数与间隙判定。做法：
 * 每个槽带一个发布戳 [slotSeq]（= 记录序号 + 1，`0` 表示从未写过），写侧「先写数据、最后写戳」，
 * 读侧「先验戳、再读值」。戳对不上 = 这条还没写完、或已被下一圈覆盖 → 丢弃这一条。
 * 常规路径（[ProbeHub] 里每个探针只有一个写入线程）上丢弃从不发生。
 *
 * 线程安全：record 可在任意线程调；[snapshot] 与 record 并发安全（弱一致，见上）。
 */
class IntervalRing(
    /** 基准节拍的兜底值（毫秒），只在中位数不可用时参与间隙判定（见类注释）。 */
    private val expectedIntervalMs: Float,
    private val capacity: Int = DEFAULT_CAPACITY,
) {

    init {
        require(capacity > 0) { "capacity 必须为正：$capacity" }
        require(expectedIntervalMs > 0f) { "expectedIntervalMs 必须为正：$expectedIntervalMs" }
    }

    /** 每个槽的时刻（纳秒）。 */
    private val slotNanos = AtomicLongArray(capacity)

    /**
     * 每个槽交付的帧数。
     *
     * 当前 [IntervalSummary] 不读它（口径只看时刻），但写入点保留：它属于埋点契约，
     * 将来要加「每次回调交付多少帧」的统计时不必再动 [record] 的签名。
     */
    private val slotFrames = AtomicIntegerArray(capacity)

    /**
     * 槽的发布戳 = 该槽这条记录的序号 + 1；`0` = 该槽从未写过。
     *
     * 为什么非它不可：只凭 [reservations] 裁剪窗口挡不住「读到未写入的 0」——
     * 序号在写数据**之前**就已发出，落在窗口尾部的那一两个在途槽读出来就是 0。
     */
    private val slotSeq = AtomicLongArray(capacity)

    /**
     * 已发出的记录序号，单调递增、不回绕（50~100 次/s 的记录频率下 `Long` 用不完）。
     *
     * 用「发给下一笔记录的序号」而不是「已写条数」：序号必须先发给写者才有槽位可写，
     * 而序号→槽位的换算只依赖容量这一定值，不需要任何共享游标 —— 这正是 record 能全程无锁的原因。
     */
    private val reservations = AtomicLong()

    /** 记一次供给（[frames] = 本次交付的帧数/样本数，口径由埋点处决定）。 */
    fun record(frames: Int, nowNanos: Long = System.nanoTime()) {
        val seq = reservations.getAndIncrement()
        val index = (seq % capacity).toInt()
        slotNanos.set(index, nowNanos)
        slotFrames.set(index, frames)
        // 最后写戳：戳可见 ⇒ 上面两次 volatile 写一定已可见（JMM 保证 volatile 写之间不重排）。
        slotSeq.set(index, seq + 1)
    }

    /** 取一份不可变摘要；快照路径允许分配（调用节奏是 dumpsys / 秒级心跳）。 */
    fun snapshot(nowNanos: Long = System.nanoTime()): IntervalSummary {
        val times = collectWindow() ?: return IntervalSummary.EMPTY

        val intervals = LongArray(times.size - 1) { times[it + 1] - times[it] }
        val sorted = intervals.copyOf().apply { sort() }
        val medianNs = percentileNs(sorted, 50.0)
        val baselineNs = if (medianNs > 0L) {
            medianNs.toDouble()
        } else {
            expectedIntervalMs.toDouble() * NANOS_PER_MILLI
        }
        val gapThresholdNs = maxOf(2.0 * baselineNs, baselineNs + GAP_SLACK_MS * NANOS_PER_MILLI)

        var gapCount = 0
        var lastGapStartNanos = Long.MIN_VALUE
        for (i in intervals.indices) {
            if (intervals[i] > gapThresholdNs) {
                gapCount += 1
                // 间隙的起点 = 前一个事件的时刻：那才是「声音从这里开始断」的时间点。
                lastGapStartNanos = times[i]
            }
        }

        return IntervalSummary(
            eventCount = times.size,
            windowSeconds = (times[times.size - 1] - times[0]).toFloat() / NANOS_PER_SECOND_F,
            p50Ms = nsToMs(percentileNs(sorted, 50.0)),
            p95Ms = nsToMs(percentileNs(sorted, 95.0)),
            p99Ms = nsToMs(percentileNs(sorted, 99.0)),
            maxMs = nsToMs(if (sorted.isEmpty()) 0L else sorted[sorted.size - 1]),
            gapCount = gapCount,
            lastGapAgeMs = if (gapCount == 0) null else ageMs(nowNanos, lastGapStartNanos),
            lastEventAgeMs = ageMs(nowNanos, times[times.size - 1]),
        )
    }

    /**
     * 按记录顺序拷出窗口内**已发布**的时刻（窗口 = 最近 min(已发序号数, capacity) 条）。
     *
     * 返回的时刻保持记录顺序（正常即时间序）；一条都没验过时返回 `null`（等价于空环，
     * 调用方据此报 [IntervalSummary.EMPTY]）。被丢掉的只可能是「还没写完」、或在拷贝期间
     * 被下一圈覆盖的极个别槽 —— 那是取证快照可以承受的损失，而 0 值不是。
     */
    private fun collectWindow(): LongArray? {
        val reserved = reservations.get()
        val window = minOf(reserved, capacity.toLong()).toInt()
        if (window == 0) return null

        val oldest = reserved - window
        var times = LongArray(window)
        var count = 0
        for (i in 0 until window) {
            val seq = oldest + i
            val index = (seq % capacity).toInt()
            // 先验戳再读值：戳对不上说明这一条还没发布（或已被覆盖），读出来的时刻没有意义。
            if (slotSeq.get(index) != seq + 1L) continue
            val nanos = slotNanos.get(index)
            // 读完再看一眼戳：拷贝期间正好被写者按下一圈覆盖的话，值可能已不属于本序号。
            if (slotSeq.get(index) != seq + 1L) continue
            times[count++] = nanos
        }
        if (count == 0) return null
        if (count != window) times = times.copyOf(count)
        return times
    }

    companion object {
        const val DEFAULT_CAPACITY = 256

        /** 间隙判定的松弛量（见类注释的阈值公式）。 */
        const val GAP_SLACK_MS = 20.0

        private const val NANOS_PER_MILLI = 1_000_000.0
        private const val NANOS_PER_SECOND_F = 1_000_000_000f

        /** 最近秩分位数：rank = ceil(p/100 × n) − 1（空数组返回 0）。 */
        private fun percentileNs(sortedNs: LongArray, p: Double): Long {
            if (sortedNs.isEmpty()) return 0L
            val rank = (ceil(p / 100.0 * sortedNs.size).toInt() - 1).coerceIn(0, sortedNs.size - 1)
            return sortedNs[rank]
        }

        private fun nsToMs(nanos: Long): Float = nanos.toFloat() / NANOS_PER_MILLI.toFloat()

        private fun ageMs(nowNanos: Long, thenNanos: Long): Float =
            ((nowNanos - thenNanos).coerceAtLeast(0L)).toFloat() / NANOS_PER_MILLI.toFloat()
    }
}

/**
 * [IntervalRing.snapshot] 的不可变结果。
 *
 * [lastEventAgeMs] 与 [lastGapAgeMs] 分工：间隙要等「下一个事件到达」才记得上，
 * 所以**正在进行**的停供只体现在 [lastEventAgeMs] 上 —— 它很大而 gapCount 不涨，
 * 就是「现在正断着」。
 */
data class IntervalSummary(
    /** 窗口内事件数（≤ 环容量）。 */
    val eventCount: Int,
    /** 窗口时长（秒）：环内最早到最晚事件的跨度。 */
    val windowSeconds: Float,
    val p50Ms: Float,
    val p95Ms: Float,
    val p99Ms: Float,
    val maxMs: Float,
    /** 窗口内超出间隙阈值的间隔个数（判定见 [IntervalRing] 类注释）。 */
    val gapCount: Int,
    /** 最近一次间隙起点距今（毫秒）；`null` = 窗口内没有间隙。 */
    val lastGapAgeMs: Float?,
    /** 最近一次事件距今（毫秒）；`null` = 还没有任何事件。 */
    val lastEventAgeMs: Float?,
) {
    companion object {
        val EMPTY = IntervalSummary(0, 0f, 0f, 0f, 0f, 0f, 0, null, null)
    }
}
