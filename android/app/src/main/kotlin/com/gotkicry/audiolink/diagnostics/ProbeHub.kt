package com.gotkicry.audiolink.diagnostics

import java.util.Locale

/**
 * 全部时序探针的命名注册表 —— `AudioLinkService` 持有一个实例，埋点组件各取所需。
 *
 * 两个出口：
 * - [report]：多行文本，给 `dumpsys activity service`（事后取证，人读）；
 * - [oneLine]：单行压缩，给 logcat 心跳（`adb logcat -s AudioLink:I`，可 grep / 可机器扫）。
 *
 * 取值语义：三个 [IntervalRing] 分别回答「内核推播放的节拍」「设备读的节拍」「内核拉采集的节拍」，
 * [events] 收全部离散异常事件。探针**常开**（稳态零分配），所有统计都在这两个方法里现算。
 */
class ProbeHub(
    /** 内核 → 播放环的供给节拍（FfiPcmFeed.feedPcm；标准档 20 ms / 低延迟档 10 ms）。 */
    val pcmFeed: IntervalRing = IntervalRing(expectedIntervalMs = 20f),
    /** 设备采集读的节拍（CapturePump.pumpOnce；10 ms 一块）。 */
    val captureRead: IntervalRing = IntervalRing(expectedIntervalMs = 10f),
    /** 内核拉采集的节拍（CapturePcmPull.readPcm；跟随内核帧长）。 */
    val capturePull: IntervalRing = IntervalRing(expectedIntervalMs = 20f),
    val events: EventRing = EventRing(),
) {

    /** dumpsys 用多行报告；每行一个探针，字段对齐方便人扫。 */
    fun report(nowNanos: Long = System.nanoTime()): String = buildString {
        appendIntervalLine("pcmFeed", pcmFeed.snapshot(nowNanos))
        appendIntervalLine("captureRead", captureRead.snapshot(nowNanos))
        appendIntervalLine("capturePull", capturePull.snapshot(nowNanos))
        appendEventsLine("events", events.snapshot(nowNanos), verbose = true)
    }

    /** logcat 心跳用单行：无换行、无多余空白。 */
    fun oneLine(nowNanos: Long = System.nanoTime()): String = buildString {
        appendIntervalCompact("pcmFeed", pcmFeed.snapshot(nowNanos))
        append(' ')
        appendIntervalCompact("captureRead", captureRead.snapshot(nowNanos))
        append(' ')
        appendIntervalCompact("capturePull", capturePull.snapshot(nowNanos))
        append(' ')
        appendEventsCompact(events.snapshot(nowNanos))
    }

    private fun StringBuilder.appendIntervalLine(name: String, s: IntervalSummary) {
        if (s.eventCount == 0) {
            append(name).appendLine(": n=0（无事件）")
            return
        }
        append(name).append(": n=").append(s.eventCount)
        append(" win=").append(fmt(s.windowSeconds)).append('s')
        append(" p50=").append(fmt(s.p50Ms)).append("ms")
        append(" p95=").append(fmt(s.p95Ms)).append("ms")
        append(" p99=").append(fmt(s.p99Ms)).append("ms")
        append(" max=").append(fmt(s.maxMs)).append("ms")
        append(" gaps=").append(s.gapCount)
        append(" lastGapAgo=").append(s.lastGapAgeMs?.let { fmt(it) + "ms" } ?: "-")
        append(" lastEventAgo=").append(s.lastEventAgeMs?.let { fmt(it) + "ms" } ?: "-")
        appendLine()
    }

    private fun StringBuilder.appendIntervalCompact(name: String, s: IntervalSummary) {
        append(name).append("[n=").append(s.eventCount)
        if (s.eventCount > 0) {
            append(" p50=").append(fmt(s.p50Ms))
            append(" p95=").append(fmt(s.p95Ms))
            append(" max=").append(fmt(s.maxMs))
            append(" g=").append(s.gapCount)
            s.lastGapAgeMs?.let { append(" lastGap=").append(fmt(it)) }
        }
        append(']')
    }

    private fun StringBuilder.appendEventsLine(
        name: String,
        snap: Map<ProbeEvent, EventTagSummary>,
        verbose: Boolean,
    ) {
        append(name).append(':')
        for (tag in ProbeEvent.entries) {
            val s = snap.getValue(tag)
            append(' ').append(tag.label).append('=').append(s.count)
            if (verbose) {
                append('(').append(s.lastAgeMs?.let { fmt(it) + "ms前" } ?: "-").append(')')
            }
        }
        appendLine()
    }

    private fun StringBuilder.appendEventsCompact(snap: Map<ProbeEvent, EventTagSummary>) {
        append("ev[")
        ProbeEvent.entries.forEachIndexed { i, tag ->
            if (i > 0) append(' ')
            append(tag.label).append('=').append(snap.getValue(tag).count)
        }
        append(']')
    }

    private companion object {
        fun fmt(v: Float): String = String.format(Locale.US, "%.1f", v)
    }
}
