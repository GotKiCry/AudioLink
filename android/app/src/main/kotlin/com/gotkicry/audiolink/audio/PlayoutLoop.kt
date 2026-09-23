package com.gotkicry.audiolink.audio

/**
 * 播放拉取循环的**单步推进器** —— 纯逻辑，不依赖 Android 框架，可 JVM 单测。
 *
 * 它回答一个问题：**这一轮该给输出设备喂什么、缺数据时怎么办**。
 * `LowLatencyPlayer` 的专用播放线程只是 `while (running) pumpOnce()` 的外壳，
 * 这样"欠载判定 / 补零 / 回压"这些真正会出错的地方全都可以在没有真机的情况下被单测覆盖。
 *
 * ## 三个关键设计
 * 1. **不丢数据**：非阻塞写没接受的部分留在 pending 缓冲里，下一轮继续写（[pumpOnce] 返回 `false`
 *    只是告诉调用方"这轮没事干，可以睡一下"）。丢数据在音频里表现为爆音，代价比多睡 1 ms 大得多。
 * 2. **欠载补静音而不是空转**：源没数据时写静音，让输出流保持连续。这样设备侧不会因为缓冲被抽空而
 *    进入欠载恢复（那会带来额外的恢复延迟和爆音）；"我这边缺数据"这件事改由
 *    [PlaybackCounters.sourceUnderruns] 与 [PlaybackCounters.starvedFrames] 如实记录，指标更干净。
 * 3. **记账发生在"整块写完"之后**：一轮 chunk 可能被拆成多次非阻塞写，若每次只写一半就记账，
 *    音频帧与静音帧的构成会算不准；因此 [fill] 时先记下这一轮的构成，等 `pendingFrames` 归零再一次性入账。
 *
 * 线程模型：**非线程安全**，只允许被同一个播放线程调用。
 */
class PlayoutLoop(
    private val source: PcmSource,
    private val sink: PcmSink,
    private val channelCount: Int,
    /** 每次拉取的帧数（chunk）。播放线程的调度粒度就是它。 */
    private val chunkFrames: Int,
    private val counters: PlaybackCounters,
) {

    init {
        require(channelCount > 0) { "channelCount 必须为正：$channelCount" }
        require(chunkFrames > 0) { "chunkFrames 必须为正：$chunkFrames" }
    }

    /** 复用的拉取缓冲（实时路径零分配：不在循环里 new）。 */
    private val buffer = FloatArray(PcmLayout.samplesForFrames(chunkFrames, channelCount))

    /** 还没写进输出设备的帧数；> 0 表示上一轮被回压卡住了。 */
    private var pendingFrames = 0

    /** 已从源取走、尚未被设备接收的帧数；只在播放线程读。 */
    val bufferedFrames: Int get() = pendingFrames

    /** [buffer] 中尚未写出的起始样本下标。 */
    private var pendingOffsetSamples = 0

    /** 本块 pending 里属于"有效音频"的帧数（写完时入账）。 */
    private var pendingAudioFrames = 0

    /** 本块 pending 里属于"欠载补零"的帧数（写完时入账）。 */
    private var pendingSilenceFrames = 0

    /**
     * 推进一步。
     *
     * @return `true` = 本轮确实往输出设备写了东西；
     *         `false` = 输出设备这一轮写不进（缓冲满），调用方应当短暂休眠后再来，
     *         **不要**在这里忙等 —— 回压是设备侧给的，不是错误。
     */
    fun pumpOnce(): Boolean {
        if (pendingFrames == 0) {
            fill()
        }
        val written = sink.tryWrite(buffer, pendingOffsetSamples, pendingFrames)
        if (written <= 0) {
            // 输出缓冲满：数据留在 pending 里，下一轮继续（不丢、不忙等）。
            return false
        }
        val advancedFrames = minOf(written, pendingFrames)
        pendingOffsetSamples += PcmLayout.samplesForFrames(advancedFrames, channelCount)
        pendingFrames -= advancedFrames
        if (pendingFrames == 0) {
            counters.recordWrite(pendingAudioFrames)
            counters.recordSilence(pendingSilenceFrames)
            pendingAudioFrames = 0
            pendingSilenceFrames = 0
        }
        return true
    }

    /** 从源拉一块到 [buffer]，缺口补零，并记下这一块的音频/静音构成。 */
    private fun fill() {
        // 源返回超量（实现 bug）时按缓冲容量截断，绝不让它写出数组边界。
        val gotSamples = source.readInto(buffer).coerceIn(0, buffer.size)
        counters.recordRead(gotSamples, buffer.size, channelCount)
        val audioFrames = PcmLayout.framesForSamples(gotSamples, channelCount)

        if (gotSamples < buffer.size) {
            // 欠载：不足的部分补零（保持输出流连续）；尾部可能是"半帧"零头，一并清零。
            buffer.fill(0f, gotSamples, buffer.size)
        }

        pendingFrames = chunkFrames
        pendingOffsetSamples = 0
        pendingAudioFrames = audioFrames
        pendingSilenceFrames = chunkFrames - audioFrames
    }
}
