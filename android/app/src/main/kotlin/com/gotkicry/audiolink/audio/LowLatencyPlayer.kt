package com.gotkicry.audiolink.audio

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.os.Process
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * 低延迟播放器：**Kotlin 侧**持有 `AudioTrack` 并消费 PCM。
 *
 * ## 架构纪律（`docs/02-architecture.md` §1 第 3 条，不可反转）
 * `AudioTrack` 的**生命周期与线程亲和性在这里**，内核只输出"PCM + 目标播放时刻 + 期望速率"，
 * 不做"内核托管播放线程"。具体落法是：本类自己起一个专用线程（[THREAD_NAME]），
 * `build()` → `play()` → 循环写入 → `stop()`/`release()` **全部发生在同一个线程上**；
 * 内核/FFI 只通过 [setSource] 挂一个 [PcmSource] 进来，永远不碰 `AudioTrack`。
 * 这样既避开"JNI 跨线程播放"的经典崩溃（旧版 `mAudioTrack` 的教训），也不再需要任何 native 侧同步。
 *
 * ## 契约（`docs/11-m1-contract.md` §7，形状冻结）
 * 构造参数 `(sampleRate = 48_000, channelCount = 2, bufferFrames)`、
 * [start] → [PlaybackReport]、[write]、[stop]、[stats]。
 * 在契约之外**只增加**了两个接缝：[setSource]（拉模式，内核 PCM 经环缓冲进来）与
 * [tryWrite]（[PcmSink] 实现，供 [PlayoutLoop] 使用）。
 *
 * ## 几处刻意的选择（都写清"为什么"）
 * 1. **请求低延迟 ≠ 拿到低延迟**：`start()` 只负责把 `getPerformanceMode()` **实测值**回报出去
 *    （[PlaybackReport.lowLatency]）。设备/厂商可以直接拒绝（返回 `PERFORMANCE_MODE_NONE`），
 *    这是**设备能力**而不是调用方的 bug —— 因此这里"上报"而不抛异常：抛异常会让不支持低延迟的
 *    设备连降级播放都做不到，那比"延迟高一点"糟得多。
 * 2. **48 kHz 硬断言**：零重采样是本项目的硬纪律（契约 §5），降级到别的采样率必须由调用方显式决定，
 *    不能在播放器里"顺手"处理 —— 顺手处理出来的是静默的音调错误。
 * 3. **缓冲帧数取 `max(请求, getMinBufferSize)`**：请求值小于设备下限时系统会自己抬高，
 *    与其拿到一个对不上的数字，不如先夹紧，再把**实际生效值**（`getBufferSizeInFrames()`）读出来回报。
 * 4. **统计只从 volatile 快照读**：UI（主）线程调 [stats] 时绝不触碰 `AudioTrack` 对象，
 *    所有设备读数都在播放线程里采下来 —— 亲和性纪律在读取路径上同样成立。
 */
class LowLatencyPlayer(
    private val sampleRate: Int = SAMPLE_RATE_48K,
    private val channelCount: Int = CHANNEL_COUNT_STEREO,
    /** 请求的输出缓冲（帧）。实际生效值见 [PlaybackStats.actualBufferFrames]。 */
    private val bufferFrames: Int,
) : PcmSink {

    companion object {
        /** 零重采样：本项目只做 48 kHz（`docs/11-m1-contract.md` §5）。 */
        const val SAMPLE_RATE_48K = 48_000
        const val CHANNEL_COUNT_MONO = 1
        const val CHANNEL_COUNT_STEREO = 2

        /** `PERFORMANCE_MODE_LOW_LATENCY` 自 API 26 起（minSdk 也是 26，这是低延迟的硬门槛）。 */
        const val PERFORMANCE_MODE_UNKNOWN = -1

        /**
         * "播放器还没启动"的本地错误标记。刻意取远离 `AudioTrack` 错误码（-1…-6x）的值，
         * 避免在 UI 上被误读成设备错误。
         */
        const val ERROR_NOT_STARTED = -1000

        private const val BYTES_PER_FLOAT = 4
        private const val THREAD_NAME = "audiolink-playout"
        private const val START_TIMEOUT_MS = 3_000L
        private const val JOIN_TIMEOUT_MS = 1_000L

        /** 无事可做时的休眠（只在输出缓冲满、源也没新数据时走到）。 */
        private const val IDLE_SLEEP_MS = 1L

        /** 缓冲快照的采样间隔（轮次）：`getBufferSizeInFrames` 是 JNI 调用，没必要每轮都问。 */
        private const val BUFFER_SNAPSHOT_INTERVAL = 32

        /** `getPerformanceMode()` → 可读名字（UI 用；不要让它显示裸数字）。 */
        fun performanceModeName(mode: Int): String = when (mode) {
            AudioTrack.PERFORMANCE_MODE_LOW_LATENCY -> "LOW_LATENCY"
            AudioTrack.PERFORMANCE_MODE_POWER_SAVING -> "POWER_SAVING"
            AudioTrack.PERFORMANCE_MODE_NONE -> "NONE"
            PERFORMANCE_MODE_UNKNOWN -> "UNKNOWN"
            else -> "UNRECOGNIZED($mode)"
        }

        /** 声道数 → `AudioFormat` 通道掩码。M1 只支持 1/2 声道。 */
        fun channelMaskOf(channelCount: Int): Int = when (channelCount) {
            CHANNEL_COUNT_MONO -> AudioFormat.CHANNEL_OUT_MONO
            CHANNEL_COUNT_STEREO -> AudioFormat.CHANNEL_OUT_STEREO
            else -> throw IllegalArgumentException("M1 只支持 1 或 2 声道，收到 $channelCount")
        }
    }

    init {
        require(sampleRate == SAMPLE_RATE_48K) {
            "零重采样纪律：播放采样率必须是 $SAMPLE_RATE_48K Hz（收到 $sampleRate）"
        }
        require(channelCount == CHANNEL_COUNT_MONO || channelCount == CHANNEL_COUNT_STEREO) {
            "M1 只支持 1 或 2 声道（收到 $channelCount）"
        }
        require(bufferFrames > 0) { "bufferFrames 必须为正：$bufferFrames" }
    }

    private val counters = PlaybackCounters()

    /**
     * 保护 `AudioTrack` 的创建/写入/释放。
     * 播放线程与（可选的）推模式调用线程都从这里过 —— 锁的存在让"释放"与"写入"不可能交错。
     */
    private val writeLock = Any()

    /** 每次拉取的帧数：10 ms，且不超过缓冲的一半（否则永远写不满一块）。 */
    private val chunkFrames: Int = maxOf(1, minOf(sampleRate / 100, bufferFrames / 2))

    @Volatile
    private var source: PcmSource = EmptyPcmSource

    /** 只在播放线程创建/释放，读侧只取快照 —— 见类注释第 4 条。 */
    @Volatile
    private var track: AudioTrack? = null

    @Volatile
    private var running = false

    /** 播放线程句柄；只被 [start]/[stop] 的调用线程访问（不再向外泄）。 */
    @Volatile
    private var worker: Thread? = null

    @Volatile
    private var performanceMode: Int = PERFORMANCE_MODE_UNKNOWN

    @Volatile
    private var actualBufferFrames: Int = 0

    @Volatile
    private var trackUnderruns: Int = 0

    @Volatile
    private var lastFailure: String? = null

    /** 最近一次内部失败原因（启动失败 / 释放异常 / 线程未按时退出）；`null` = 没出过问题。 */
    val lastFailureReason: String? get() = lastFailure

    /**
     * 挂上 PCM 数据源（拉模式）。**这是内核/FFI 的唯一接缝**：内核把解码后的 PCM 送进
     * [PcmRingBuffer]，播放线程从这里读走。不设置时用 [EmptyPcmSource]（= 一直欠载、输出静音），
     * 于是"内核还没接上"会变成一个持续增长的欠载计数，而不是无从判断的静音。
     */
    fun setSource(source: PcmSource) {
        this.source = source
    }

    /**
     * 启动播放（同步返回启动结果）。
     *
     * 为什么用 `CountDownLatch` 同步等待：`AudioTrack` 必须由播放线程自己创建（纪律），
     * 但契约要求 `start()` 把 [PlaybackReport] **同步**交回调用方；latch 同时充当内存屏障，
     * 保证线程内的赋值对调用方可见。
     *
     * @throws IllegalStateException 重复调用、启动超时、或设备拒绝创建（含信号了原始原因）。
     */
    fun start(): PlaybackReport {
        check(worker == null) { "播放器已在运行；请先 stop()" }

        val ready = CountDownLatch(1)
        var report: PlaybackReport? = null
        var failure: Throwable? = null

        val thread = Thread({
            Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
            var localTrack: AudioTrack? = null
            try {
                localTrack = buildAudioTrack()
                // play() 之后才采设备读数：缓冲大小与性能模式都以"已进入播放态"的值为准。
                localTrack.play()
                val mode = localTrack.performanceMode
                val bufferFramesActual = localTrack.bufferSizeInFrames
                synchronized(writeLock) {
                    track = localTrack
                    performanceMode = mode
                    actualBufferFrames = bufferFramesActual
                }
                report = PlaybackReport(
                    lowLatency = mode == AudioTrack.PERFORMANCE_MODE_LOW_LATENCY,
                    actualBufferFrames = bufferFramesActual,
                    sampleRate = sampleRate,
                    channelCount = channelCount,
                    performanceMode = mode,
                )
                ready.countDown()
                runPlayoutLoop(localTrack)
            } catch (t: Throwable) {
                failure = t
                lastFailure = "播放线程异常：${t.javaClass.simpleName}: ${t.message}"
                ready.countDown()
            } finally {
                releaseTrack(localTrack)
                running = false
            }
        }, THREAD_NAME)

        running = true
        worker = thread
        thread.start()

        if (!ready.await(START_TIMEOUT_MS, TimeUnit.MILLISECONDS)) {
            stop()
            throw IllegalStateException("AudioTrack 启动超时（${START_TIMEOUT_MS} ms）")
        }
        failure?.let { cause ->
            stop()
            throw IllegalStateException("AudioTrack 启动失败：${cause.message}", cause)
        }
        val result = report
        if (result == null) {
            stop()
            throw IllegalStateException("AudioTrack 启动失败：未产生 PlaybackReport")
        }
        return result
    }

    /**
     * 推模式写入（契约 API）：把 [frames] 帧从调用线程直接交给输出设备。
     *
     * 与拉模式（[setSource] + 播放线程）**二选一**：两个入口同时用会让两股数据在 `AudioTrack`
     * 缓冲里交错。服务走拉模式；这个方法留给"上层自己驱动"的场景（例如未来 JNI 回调直推）。
     *
     * 非阻塞，写不满的部分**丢弃并计入** [PlaybackStats.framesDropped] —— 调用方在主线程上，
     * 不允许在这里等输出设备。
     */
    fun write(samples: FloatArray, frames: Int) {
        require(frames >= 0) { "frames 不能为负：$frames" }
        val wantedSamples = PcmLayout.samplesForFrames(frames, channelCount)
        require(samples.size >= wantedSamples) {
            "samples 只有 ${samples.size} 个样本，装不下 $frames 帧 × $channelCount 声道"
        }
        if (wantedSamples == 0) return

        var notStarted = false
        var writtenSamples = 0
        synchronized(writeLock) {
            val target = track
            if (target == null) {
                notStarted = true
            } else {
                writtenSamples = target.write(samples, 0, wantedSamples, AudioTrack.WRITE_NON_BLOCKING)
            }
        }

        if (notStarted) {
            counters.recordWriteError(ERROR_NOT_STARTED)
            return
        }
        if (writtenSamples < 0) {
            counters.recordWriteError(writtenSamples)
            return
        }
        val writtenFrames = PcmLayout.framesForSamples(writtenSamples, channelCount)
        counters.recordWrite(writtenFrames)
        if (writtenFrames < frames) {
            counters.recordDrop(frames - writtenFrames)
        }
    }

    /** 停止播放并释放 `AudioTrack`（幂等）。释放由播放线程自己完成，符合线程亲和纪律。 */
    fun stop() {
        val thread = worker
        if (thread != null) {
            running = false
            try {
                thread.join(JOIN_TIMEOUT_MS)
            } catch (e: InterruptedException) {
                Thread.currentThread().interrupt()
                lastFailure = "等待播放线程退出时被中断：${e.message}"
            }
            if (thread.isAlive) {
                lastFailure = "播放线程未在 ${JOIN_TIMEOUT_MS} ms 内退出（AudioTrack 可能仍然持有）"
            }
            worker = null
        }
    }

    /** 统计快照（线程安全，可从任意线程调用）。 */
    fun stats(): PlaybackStats = PlaybackStats(
        running = running,
        underruns = trackUnderruns,
        sourceUnderruns = counters.sourceUnderruns,
        framesWritten = counters.framesWritten,
        silenceFrames = counters.silenceFrames,
        framesDropped = counters.framesDropped,
        writeErrors = counters.writeErrors,
        lastWriteError = counters.lastWriteError,
        actualBufferFrames = actualBufferFrames,
        requestedBufferFrames = bufferFrames,
        performanceMode = performanceMode,
        sampleRate = sampleRate,
        channelCount = channelCount,
    )

    /** [PcmSink] 实现：非阻塞写一块，`0` 表示"这轮写不进"（输出缓冲满，调用方稍后再来）。 */
    override fun tryWrite(samples: FloatArray, offsetSamples: Int, frames: Int): Int {
        if (frames <= 0) return 0
        synchronized(writeLock) {
            val target = track ?: return 0
            val writtenSamples = target.write(
                samples,
                offsetSamples,
                PcmLayout.samplesForFrames(frames, channelCount),
                AudioTrack.WRITE_NON_BLOCKING,
            )
            if (writtenSamples < 0) {
                // 记录真实错误码后按"写不进"返回：PlayoutLoop 会保留数据下轮重试，
                // 而错误本身已经进了统计，UI 看得见，不需要在这里抛。
                counters.recordWriteError(writtenSamples)
                return 0
            }
            return PcmLayout.framesForSamples(writtenSamples, channelCount)
        }
    }

    // ---------------------------------------------------------------- 播放线程内部

    /** 播放线程主循环：拉 → 写 →（写不进就歇 1 ms）→ 采设备快照。 */
    private fun runPlayoutLoop(localTrack: AudioTrack) {
        val loop = PlayoutLoop(source, this, channelCount, chunkFrames, counters)
        var ticks = 0
        while (running) {
            val progressed = loop.pumpOnce()
            trackUnderruns = localTrack.underrunCount
            if (++ticks % BUFFER_SNAPSHOT_INTERVAL == 0) {
                actualBufferFrames = localTrack.bufferSizeInFrames
            }
            if (!progressed) {
                try {
                    Thread.sleep(IDLE_SLEEP_MS)
                } catch (e: InterruptedException) {
                    Thread.currentThread().interrupt()
                    lastFailure = "播放线程被中断，已退出循环：${e.message}"
                    return
                }
            }
        }
    }

    /** 在**播放线程**内构建 `AudioTrack`：低延迟模式 + 非阻塞写 + FLOAT/48 kHz/2ch。 */
    private fun buildAudioTrack(): AudioTrack {
        val channelMask = channelMaskOf(channelCount)
        val minBytes = AudioTrack.getMinBufferSize(
            sampleRate,
            channelMask,
            AudioFormat.ENCODING_PCM_FLOAT,
        )
        if (minBytes <= 0) {
            throw IllegalStateException(
                "AudioTrack.getMinBufferSize 返回 $minBytes：设备不支持 " +
                    "$sampleRate Hz / ${channelCount}ch / ENCODING_PCM_FLOAT 输出",
            )
        }
        val frameBytes = channelCount * BYTES_PER_FLOAT
        // 请求值小于设备下限时先夹紧：拿到一个"我请求 960 实际 3840"的落差不如一开始就说明白。
        val bufferBytes = maxOf(bufferFrames * frameBytes, minBytes)

        val format = AudioFormat.Builder()
            .setEncoding(AudioFormat.ENCODING_PCM_FLOAT)
            .setSampleRate(sampleRate)
            .setChannelMask(channelMask)
            .build()

        val attributes = AudioAttributes.Builder()
            // USAGE_MEDIA：走媒体音量与音频焦点，符合"把 PC 的声音播出来"的语义。
            .setUsage(AudioAttributes.USAGE_MEDIA)
            // CONTENT_TYPE_MUSIC：官方低延迟示例的搭配；部分厂商会按内容类型叠 DSP，
            // 若真机实测发现额外音染/延迟，这里切到 CONTENT_TYPE_UNKNOWN 是第一个该试的开关。
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .build()

        return AudioTrack.Builder()
            .setAudioAttributes(attributes)
            .setAudioFormat(format)
            .setTransferMode(AudioTrack.MODE_STREAM)
            .setBufferSizeInBytes(bufferBytes)
            .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
            .build()
    }

    /** 释放：先摘掉引用（写入路径立刻看到"没设备了"），再 stop/release。 */
    private fun releaseTrack(localTrack: AudioTrack?) {
        synchronized(writeLock) {
            track = null
            if (localTrack == null) return
            try {
                if (localTrack.playState != AudioTrack.PLAYSTATE_STOPPED) {
                    localTrack.stop()
                }
            } catch (e: IllegalStateException) {
                // 已停止/未初始化都走这里：不算致命，但必须留痕（旧版的教训是吞掉一切异常）。
                lastFailure = "停止 AudioTrack 时状态异常：${e.message}"
            }
            try {
                localTrack.release()
            } catch (e: IllegalStateException) {
                lastFailure = "释放 AudioTrack 失败：${e.message}"
            }
        }
    }
}
