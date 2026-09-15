package com.gotkicry.audiolink.audio

/**
 * 输出设备队列水位的**纯逻辑记账器** —— 不依赖 Android 框架，可 JVM 单测。
 *
 * 它回答两个问题（task-8 的核心）：
 * 1. **现在输出设备（`AudioTrack`）里还压着多少帧没播完？** = [queuedFrames]
 * 2. **这一拍该不该往里写？** = [shouldWrite]
 *
 * ## 为什么需要它（而不是直接看 `getBufferSizeInFrames()`）
 * `getBufferSizeInFrames()` 是**容量**，不是**水位**。真机实测：容量 3844 帧（80.08 ms），
 * 而我们此前每拍都尽力写满 —— 于是稳态下队列里长期压着 ≈3844 帧，flinger 对该 track 报
 * Latency=101.00 ms。**延迟地板不是容量给的，是"把容量灌满"这个写入策略给的。**
 * 要压延迟就必须知道"里面还有多少"，而这只由「已送入帧数 − 已消费帧数」得到。
 *
 * ## 已消费帧数从哪来：`getPlaybackHeadPosition()`
 * 它返回 `u32`（帧），**会回绕**（48 kHz 下约 24.85 小时一圈）。回绕时新值会比旧值小，
 * 直接相减得到负数、queued 瞬间跳到 2^32 ⇒ UI 上会看到"水位突然爆表"这种最难解释的现象。
 * 所以这里用 `(new - old) and 0xFFFF_FFFF` 在 32 位空间里取正向增量再累加（见 [advanceHead]）。
 *
 * ## 线程模型
 * **非线程安全**：只允许被播放线程访问。所有设备读数（head position）都必须在播放线程采，
 * 这与 [LowLatencyPlayer] 的线程亲和纪律一致。
 */
class QueueWatermark {

    /** 上一次读到的 `playbackHeadPosition` 原始值（已归一成 0..2^32-1）。 */
    private var lastHeadRaw: Long = 0

    /** 是否已经采过第一次 head position（第一次只做基准，不产生增量）。 */
    private var initialized = false

    /** 累计被设备消费（播出去）的帧数。 */
    var consumedFrames: Long = 0L
        private set

    /** 累计送进设备的帧数（有效音频 + 补的静音都算 —— 设备都真的收了）。 */
    var fedFrames: Long = 0L
        private set

    /** 设备里还压着没播的帧数。**理论下界 0**：设备可能已经播掉我们还没记账的部分。 */
    val queuedFrames: Long
        get() = (fedFrames - consumedFrames).coerceAtLeast(0L)

    /**
     * 采一次 `playbackHeadPosition`（`u32`，回绕安全），返回累计已消费帧数。
     *
     * 第一次调用只建立基准（`play()` 之前该值恒为 0；`play()` 之后开始增长）。
     */
    fun advanceHead(rawHeadPosition: Int): Long {
        val raw = rawHeadPosition.toLong() and U32_MASK
        if (!initialized) {
            initialized = true
            lastHeadRaw = raw
            return consumedFrames
        }
        // 回绕安全：差值在 32 位空间里取模，永远是"正向前进"。
        val delta = (raw - lastHeadRaw) and U32_MASK
        lastHeadRaw = raw
        consumedFrames += delta
        return consumedFrames
    }

    /** 记一次实际送进设备的帧数（`AudioTrack.write` 的返回值）。 */
    fun recordFed(frames: Int) {
        if (frames > 0) fedFrames += frames.toLong()
    }

    /**
     * 这一拍该不该写。
     *
     * 口径：**只在队列水位低于「目标 − 一个 chunk」时才写**。
     * 为什么留一个 chunk 的余量：写是一次一个 chunk（[LowLatencyPlayer] 的 `chunkFrames`，10 ms），
     * 若判据是 `queued < target`，写完就会冲到 `target + chunk`，摆幅反而更大；
     * 留一个 chunk 余量让水位稳定落在 `[target − chunk, target]`。
     *
     * [targetFrames] `<= 0` 表示"不设目标、尽力写满"（旧的满灌行为，A/B 的「改前」那一侧）。
     */
    fun shouldWrite(targetFrames: Int, chunkFrames: Int): Boolean {
        if (targetFrames <= 0) return true
        val threshold = (targetFrames - chunkFrames).coerceAtLeast(1)
        return queuedFrames <= threshold
    }

    /** 播放器重开/重建 track 时归零（`playbackHeadPosition` 的新一轮从 0 起算）。 */
    fun reset() {
        lastHeadRaw = 0
        initialized = false
        consumedFrames = 0
        fedFrames = 0
    }

    private companion object {
        const val U32_MASK = 0xFFFF_FFFFL
    }
}
