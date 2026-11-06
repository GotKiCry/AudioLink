package com.gotkicry.audiolink.diagnostics

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [ProbeHub] 两个出口的形状测试：report 多行可扫、oneLine 单行可 grep。
 */
class ProbeHubTest {

    private val t0 = 1_000_000_000L

    @Test
    fun reportCoversEveryProbe() {
        val report = ProbeHub().report(nowNanos = t0)
        val lines = report.lines().filter { it.isNotBlank() }

        assertEquals("三条节拍探针 + 一条事件行", 4, lines.size)
        assertTrue(lines[0].startsWith("pcmFeed: n=0"))
        assertTrue(lines[1].startsWith("captureRead: n=0"))
        assertTrue(lines[2].startsWith("capturePull: n=0"))
        assertTrue(lines[3].startsWith("events:"))
        // 事件行覆盖全部 tag（含没发生过的），agent 不用猜字段集。
        for (tag in ProbeEvent.entries) {
            assertTrue("events 行缺 ${tag.label}", lines[3].contains("${tag.label}=0"))
        }
    }

    @Test
    fun reportReflectsRecordedProbes() {
        val hub = ProbeHub()
        hub.pcmFeed.record(frames = 960, nowNanos = t0)
        hub.pcmFeed.record(frames = 960, nowNanos = t0 + 20_000_000L)
        hub.events.record(ProbeEvent.SILENCE_FILL, nowNanos = t0)

        val report = hub.report(nowNanos = t0 + 20_000_000L)

        assertTrue(report.contains("pcmFeed: n=2"))
        assertTrue(report.contains("p50=20.0ms"))
        assertTrue(report.contains("silence=1"))
    }

    @Test
    fun oneLineIsSingleLineAndCarriesAllSections() {
        val hub = ProbeHub()
        hub.captureRead.record(frames = 480, nowNanos = t0)
        hub.events.record(ProbeEvent.TRACK_UNDERRUN, nowNanos = t0)

        val line = hub.oneLine(nowNanos = t0)

        assertFalse("logcat 心跳必须单行", line.contains('\n'))
        assertTrue(line.startsWith("pcmFeed["))
        assertTrue(line.contains("captureRead[n=1"))
        assertTrue(line.contains("capturePull["))
        assertTrue(line.contains("ev["))
        assertTrue(line.contains("trkUdr=1"))
    }
}
