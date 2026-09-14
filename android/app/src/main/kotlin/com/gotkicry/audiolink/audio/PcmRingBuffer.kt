package com.gotkicry.audiolink.audio

import java.util.concurrent.atomic.AtomicLong

/**
 * 固定深度的 PCM 环缓冲（播放环）—— **纯逻辑，不依赖任何 Android 框架**，可直接 JVM 单测。
 *
 * 位置：内核（未来经 FFI）把解码后的 PCM 写进来 → 播放线程（[PlayoutLoop]）从这里读走喂给 `AudioTrack`。
 * 它不关心时间戳、不关心播放调度 —— "PCM + 目标播放时刻 + 期望速率"里的**时刻**由内核侧决定
 * （迟到帧在写进来之前就该被丢），环缓冲只保证"最新的 PCM 一定在"。
 *
 * ## 为什么满的时候丢**最旧**的（drop-oldest）而不是丢最新的
 * 实时音频的坏数据是"延迟累积"：环一旦写满，说明消费侧（播放）追不上生产侧，
 * 此时保留最旧数据 = 把已经过期的音频播出去 = 端到端延迟永久变大且再也降不回来。
 * 丢最旧（保留最新）使延迟有上界，代价是一次可听出来的断点 —— 这笔账对低延迟分发是划算的。
 * 丢弃量记入 [overflowFrames]，它是"网络/解码给太快或播放被卡住"的直接证据。
 *
 * ## 为什么用 `synchronized` 而不是手写无锁 SPSC
 * 本轮是**单生产（内核/FFI 线程）+ 单消费（播放线程）**，周期 10 ms 级别：
 * 锁竞争概率极低、持锁时间在微秒级（一次 `arraycopy`）。而无锁实现要自己保证内存序
 * （索引与数据之间的 publish/consume 关系），错了是"偶发读出脏数据"这类最难复现的 bug，
 * 且没有真机压测就无法证明它是对的。正确性优先，低竞争锁是这里的工程最优解；
 * 等 M2 上真机、有 jcstress 级别的验证手段后再评估无锁版本。
 *
 * 线程安全：所有索引与数组访问都在内部锁下；统计计数用 `AtomicLong`（读侧不取锁）。
 */
class PcmRingBuffer(
    /** 容量（帧）。1 帧 = 1 个采样时刻的 channelCount 个样本。 */
    val capacityFrames: Int,
    /** 声道数（M1 固定 2：`docs/11-m1-contract.md` §7 的 48 kHz / 2ch / FLOAT）。 */
    val channelCount: Int = DEFAULT_CHANNEL_COUNT,
) : PcmSource {

    companion object {
        const val DEFAULT_CHANNEL_COUNT = 2
    }

    init {
        require(capacityFrames > 0) { "capacityFrames 必须为正：$capacityFrames" }
        require(channelCount > 0) { "channelCount 必须为正：$channelCount" }
    }

    /** 交错样本存储：`[L R L R …]`，容量恒为 `capacityFrames * channelCount`。 */
    private val buffer = FloatArray(PcmLayout.samplesForFrames(capacityFrames, channelCount))

    private val lock = Any()

    /**
     * 绝对帧号（只增不减），数组下标 = `frame % capacityFrames`。
     *
     * 为什么用 `Long` 而不是 `Int`：48000 Hz 下 `Int` 约 12.4 小时就溢出，而这是一个
     * 常驻前台服务会跑满的东西 —— 溢出后 `%` 会变成负数并把索引打到数组外。
     * 12.4 小时才出问题 = 正好是最难复现的时长。
     */
    private var readFrame = 0L
    private var writeFrame = 0L

    private val overflow = AtomicLong()
    private val underrunEvents = AtomicLong()
    private val starvedFrames = AtomicLong()

    /** 当前可读帧数。 */
    val sizeFrames: Int
        get() = synchronized(lock) { (writeFrame - readFrame).toInt() }

    /** 当前还能写多少帧（不触发丢弃）。 */
    val availableFrames: Int
        get() = capacityFrames - sizeFrames

    /** 因环满而被丢弃的帧数（drop-oldest 口径：丢掉的都是**最旧**的帧）。 */
    val overflowFrames: Long get() = overflow.get()

    /** 读不满的次数（一次 [readInto] 拿不够请求量 = 一次供给欠载）。 */
    val underrunCount: Long get() = underrunEvents.get()

    /** 欠载时一共缺了多少帧（比 [underrunCount] 更能反映"断得有多狠"）。 */
    val starvedFramesTotal: Long get() = starvedFrames.get()

    /**
     * 写入 [frames] 帧（从 `samples[0]` 起，交错）。
     *
     * 返回实际写入帧数：drop-oldest 策略下**写入永不失败**，因此返回值等于 [frames]
     * （`frames <= 0` 时为 0）。写不下时最旧的帧被覆盖并计入 [overflowFrames]。
     *
     * @throws IllegalArgumentException 当 [frames] 超出 [samples] 的容量时（这是调用方的 bug，
     *         必须炸出来，不能悄悄截断成"少写一点"）。
     */
    fun write(samples: FloatArray, frames: Int): Int {
        require(frames >= 0) { "frames 不能为负：$frames" }
        if (frames == 0) return 0
        require(samples.size >= PcmLayout.samplesForFrames(frames, channelCount)) {
            "samples 只有 ${samples.size} 个样本，装不下 $frames 帧 × $channelCount 声道"
        }

        // 超容量的一次性写入：只保留最后 capacityFrames 帧（丢的都是最旧的）。
        var srcOffsetFrames = 0
        var framesToWrite = frames
        if (framesToWrite > capacityFrames) {
            val dropped = framesToWrite - capacityFrames
            overflow.addAndGet(dropped.toLong())
            srcOffsetFrames = dropped
            framesToWrite = capacityFrames
        }

        synchronized(lock) {
            val used = (writeFrame - readFrame).toInt()
            val free = capacityFrames - used
            if (framesToWrite > free) {
                // 腾空间：把读指针往前推 = 丢弃最旧的 (framesToWrite - free) 帧。
                val drop = framesToWrite - free
                overflow.addAndGet(drop.toLong())
                readFrame += drop
            }
            val dstFrame = (writeFrame % capacityFrames).toInt()
            val firstChunkFrames = minOf(framesToWrite, capacityFrames - dstFrame)
            System.arraycopy(
                samples,
                PcmLayout.sampleOffsetOfFrame(srcOffsetFrames, channelCount),
                buffer,
                PcmLayout.sampleOffsetOfFrame(dstFrame, channelCount),
                PcmLayout.samplesForFrames(firstChunkFrames, channelCount),
            )
            val restFrames = framesToWrite - firstChunkFrames
            if (restFrames > 0) {
                // 跨越环绕点：剩下的写到数组开头。
                System.arraycopy(
                    samples,
                    PcmLayout.sampleOffsetOfFrame(srcOffsetFrames + firstChunkFrames, channelCount),
                    buffer,
                    0,
                    PcmLayout.samplesForFrames(restFrames, channelCount),
                )
            }
            writeFrame += framesToWrite
        }
        return frames
    }

    /**
     * 读取：请求量 = `dst.size / channelCount` 帧（不足一帧的零头元素**不写**，保持原值）。
     *
     * 返回实际写进 [dst] 的**样本数**（见 [PcmSource] 的口径约定）：
     * 返回 `dst.size` = 读满；小于 `dst.size` = 供给不足，本方法**不补零**，
     * 并要求 [underrunCount] 递增一次。补零/丢弃策略属于消费方 [PlayoutLoop]。
     */
    override fun readInto(dst: FloatArray): Int {
        val wantFrames = PcmLayout.framesForSamples(dst.size, channelCount)
        if (wantFrames == 0) return 0

        val taken: Int
        synchronized(lock) {
            val used = (writeFrame - readFrame).toInt()
            taken = minOf(wantFrames, used)
            if (taken > 0) {
                val srcFrame = (readFrame % capacityFrames).toInt()
                val firstChunkFrames = minOf(taken, capacityFrames - srcFrame)
                System.arraycopy(
                    buffer,
                    PcmLayout.sampleOffsetOfFrame(srcFrame, channelCount),
                    dst,
                    0,
                    PcmLayout.samplesForFrames(firstChunkFrames, channelCount),
                )
                val restFrames = taken - firstChunkFrames
                if (restFrames > 0) {
                    System.arraycopy(
                        buffer,
                        0,
                        dst,
                        PcmLayout.samplesForFrames(firstChunkFrames, channelCount),
                        PcmLayout.samplesForFrames(restFrames, channelCount),
                    )
                }
                readFrame += taken
            }
        }

        if (taken < wantFrames) {
            underrunEvents.incrementAndGet()
            starvedFrames.addAndGet((wantFrames - taken).toLong())
        }
        return PcmLayout.samplesForFrames(taken, channelCount)
    }

    /** 丢弃当前全部数据（不清统计；统计口径见 [resetStats]）。用于会话重建/停止播放后重置状态。 */
    fun clear() {
        synchronized(lock) {
            readFrame = writeFrame
        }
    }

    /** 统计清零（[clear] 不动统计，是刻意的：数据清空不等于"从未发生过欠载"）。 */
    fun resetStats() {
        overflow.set(0)
        underrunEvents.set(0)
        starvedFrames.set(0)
    }
}
