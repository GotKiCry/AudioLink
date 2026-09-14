package com.gotkicry.audiolink.audio

import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

/**
 * 播放统计的**累计口径**（纯逻辑，可 JVM 单测）。
 *
 * 为什么要单独一个类而不是散在播放器里加几个字段：
 *  - 统计口径是"验收指标"的一部分（`docs/11-m1-contract.md` §7 要求 `underruns / framesWritten /
 *    writeErrors / actualBufferFrames`），口径一旦含糊，指标就没法跟内核/桌面端对齐；
 *  - 它同时被**两个线程**碰：播放线程写、UI（主）线程读 → 用原子量，读侧不上锁；
 *  - 抽出来以后，"累加规则"可以被单测逐条钉住，而不是只能靠真机观察。
 *
 * ## 口径定义（每个数字都要能回答"它数的是什么"）
 *  - [framesWritten]：**来自数据源的有效音频帧**成功写进输出设备的总量（不含补的静音）；
 *  - [silenceFrames]：源欠载时为保持输出流连续而补写的静音帧（**不计入** [framesWritten]）；
 *    故"设备实际收到的帧数 = framesWritten + silenceFrames"；
 *  - [framesDropped]：推模式（`LowLatencyPlayer.write`）下非阻塞写没接受、调用方也不重试而丢弃的帧；
 *    拉模式（[PlayoutLoop]）**永不丢**（未写完的数据留在 pending 缓冲里下轮继续），所以它只反映推模式；
 *  - [writeErrors] / [lastWriteError]：输出设备返回负值错误的次数与最近一次错误码；
 *  - [sourceUnderruns]：一次读取拿不满请求量 = 一次供给欠载（次数口径）；
 *  - [starvedFrames]：这些欠载合计缺了多少帧（严重度口径）。
 */
class PlaybackCounters {

    private val writtenFrames = AtomicLong()
    private val silence = AtomicLong()
    private val dropped = AtomicLong()
    private val errorCount = AtomicInteger()
    private val lastErrorCode = AtomicInteger(ERROR_NONE)
    private val underruns = AtomicLong()
    private val starved = AtomicLong()

    companion object {
        /** [recordWriteError] 之外的"没有错误"哨兵值（0 不是合法错误码，负数才是）。 */
        const val ERROR_NONE = 0
    }

    val framesWritten: Long get() = writtenFrames.get()

    val silenceFrames: Long get() = silence.get()

    val framesDropped: Long get() = dropped.get()

    val writeErrors: Int get() = errorCount.get()

    val lastWriteError: Int get() = lastErrorCode.get()

    val sourceUnderruns: Long get() = underruns.get()

    val starvedFrames: Long get() = starved.get()

    /** 记一次**成功**写入的有效音频帧（负数/0 由调用方先行判掉，这里只做非负防御）。 */
    fun recordWrite(frames: Int) {
        if (frames > 0) writtenFrames.addAndGet(frames.toLong())
    }

    /** 记一次欠载补写的静音帧。 */
    fun recordSilence(frames: Int) {
        if (frames > 0) silence.addAndGet(frames.toLong())
    }

    /** 记一次"拿到了数据但写不进去、调用方放弃"的丢弃（推模式专用）。 */
    fun recordDrop(frames: Int) {
        if (frames > 0) dropped.addAndGet(frames.toLong())
    }

    /** 记一次输出设备错误（`AudioTrack.write` 返回的负值原样带进来）。 */
    fun recordWriteError(code: Int) {
        errorCount.incrementAndGet()
        lastErrorCode.set(code)
    }

    /**
     * 记一次数据源读取结果。
     *
     * 口径：`returnedSamples >= requestedSamples` 视为读满（不计欠载）；
     * 否则欠载次数 +1，并把缺的**帧**数累加进 [starvedFrames]。
     */
    fun recordRead(returnedSamples: Int, requestedSamples: Int, channelCount: Int) {
        require(channelCount > 0) { "channelCount 必须为正：$channelCount" }
        require(requestedSamples >= 0) { "requestedSamples 不能为负：$requestedSamples" }
        if (returnedSamples >= requestedSamples) return
        underruns.incrementAndGet()
        val missingSamples = requestedSamples - returnedSamples.coerceAtLeast(0)
        val missingFrames = PcmLayout.framesForSamples(missingSamples, channelCount)
        if (missingFrames > 0) starved.addAndGet(missingFrames.toLong())
    }

    /** 全部清零（例如播放器重开）。 */
    fun reset() {
        writtenFrames.set(0)
        silence.set(0)
        dropped.set(0)
        errorCount.set(0)
        lastErrorCode.set(ERROR_NONE)
        underruns.set(0)
        starved.set(0)
    }
}
