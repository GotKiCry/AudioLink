package com.gotkicry.audiolink.capture

import com.gotkicry.audiolink.audio.PcmLayout

/**
 * 采集侧的**单步推进器** —— 纯逻辑、不依赖 Android 框架、可直接 JVM 单测，
 * 与播放侧的 [com.gotkicry.audiolink.audio.PlayoutLoop] 对称。
 *
 * 它把「设备给的裸样本」变成「内核能直接用的一帧」：
 *
 * ```
 * AudioCaptureSource.read ──► 编码换算（S16 → f32）──► 声道展开（mono → 2ch 交错）
 *                          ──► 帧对齐装配（不足一帧的零头留在装配器里）──► CaptureRing.write
 * ```
 *
 * ## 三条纪律
 * 1. **不丢样本、不补零**：零头由 [PcmFrameAssembler] 缓存到下一块（丢零头会累积时序漂移）。
 * 2. **不重采样**：格式不符合协议就抛 [CaptureErrorKind.UnsupportedFormat]，绝不偷偷 SRC。
 * 3. **不猜声道布局**：只支持「源声道数 == 目标」与「1 → 2 复制」两条路径，别的明确报错。
 *
 * 线程模型：**非线程安全**，只允许采集线程调用（与 `PlayoutLoop` 同口径）。
 */
class CapturePump(
    private val source: AudioCaptureSource,
    private val ring: CaptureRing,
    /** 目标声道数：协议固定 2（`docs/11-m1-contract.md` §7）。 */
    private val targetChannels: Int = PcmFrameAssembler.DEFAULT_CHANNEL_COUNT,
    /** 每次从设备拉取的帧数；默认 20 ms，与内核一帧同长。 */
    private val framesPerRead: Int = DEFAULT_FRAMES_PER_READ,
) {

    init {
        require(targetChannels > 0) { "targetChannels 必须为正：$targetChannels" }
        require(framesPerRead > 0) { "framesPerRead 必须为正：$framesPerRead" }
        require(ring.channelCount == targetChannels) {
            "环的声道数（${ring.channelCount}）必须与目标声道数（$targetChannels）一致"
        }
        val sourceChannels = source.format.channelCount
        require(sourceChannels in 1..2) { "源的声道数只支持 1 或 2：$sourceChannels" }
        require(source.format.sampleRateHz == CaptureFormat.SAMPLE_RATE_HZ) {
            "源的采样率必须是 ${CaptureFormat.SAMPLE_RATE_HZ} Hz（不做重采样）：${source.format.sampleRateHz}"
        }
    }

    private val sourceChannels: Int = source.format.channelCount

    /** 一次读取的样本数（源声道数口径）。 */
    private val readSamples: Int = PcmLayout.samplesForFrames(framesPerRead, sourceChannels)

    private val buffer = CaptureBuffer(source.format.encoding, readSamples)

    /** S16 换算的落地缓冲（F32 源用不到，但只是一次分配）。 */
    private val converted = FloatArray(readSamples)

    /** mono → 2ch 展开的落地缓冲。 */
    private val expanded = FloatArray(readSamples * targetChannels)

    /** 装配输出（比 expanded 多留一帧，容纳上一轮残留的零头）。 */
    private val assembled = FloatArray(expanded.size + targetChannels)

    private val assembler = PcmFrameAssembler(targetChannels)

    /** 调用 [pumpOnce] 的次数（诊断）。 */
    var readCalls: Long = 0L
        private set

    /** 从设备实际读到的样本数（源声道数口径，诊断）。 */
    var sourceSamples: Long = 0L
        private set

    /** 写进环的帧数（诊断）。 */
    var framesDelivered: Long = 0L
        private set

    /** 源「返回样本数 > 缓冲容量」的次数 —— 源的实现 bug，被截断并计数（不崩、不越界）。 */
    var truncatedReads: Long = 0L
        private set

    /** 装配器里还没凑齐一帧的零头样本数（诊断；正常恒 0 或 1）。 */
    val pendingSamples: Int get() = assembler.pendingSamples

    private val encoding: PcmEncoding = source.format.encoding

    /**
     * 从设备拉一块并投进环。
     *
     * @return `true` = 本轮确实读到了样本（哪怕还没凑齐一整帧）；`false` = 设备这一轮没有数据。
     * @throws CaptureFailure 源报错（返回负值 / 不支持的声道布局）时。
     */
    fun pumpOnce(): Boolean {
        readCalls += 1
        val read = source.read(buffer)
        if (read < 0) {
            // AudioRecord 的错误码（ERROR_INVALID_OPERATION 等）走这条路：错误必须炸给上层，
            // 让它按分类决定「退避重试」还是「回界面要授权」。吞掉就等于采集静默死掉。
            throw CaptureFailure(
                CaptureErrorKind.Internal,
                "采集源返回错误码 $read（${source.javaClass.simpleName}）",
            )
        }
        if (read == 0) return false

        val count = read.coerceAtMost(buffer.capacitySamples)
        if (count != read) truncatedReads += 1
        sourceSamples += count.toLong()

        // 编码换算：F32 源零拷贝直接用缓冲，S16 源走纯函数换算（有单测）。
        val srcSamples: FloatArray
        val srcCount: Int
        when (encoding) {
            PcmEncoding.F32 -> {
                srcSamples = buffer.floats
                srcCount = count
            }

            PcmEncoding.S16 -> {
                srcCount = PcmConversion.int16ToFloat(buffer.shorts, count, converted)
                srcSamples = converted
            }
        }

        // 声道布局：等声道直通；mono → 2ch 复制展开；其余明确报错（不猜）。
        val laidOut: FloatArray
        val laidOutCount: Int
        if (sourceChannels == targetChannels) {
            laidOut = srcSamples
            laidOutCount = srcCount
        } else if (sourceChannels == 1 && targetChannels == 2) {
            laidOut = expanded
            laidOutCount = PcmConversion.expandMonoToStereo(srcSamples, srcCount, expanded)
        } else {
            throw CaptureFailure(
                CaptureErrorKind.UnsupportedFormat,
                "不支持的声道布局：$sourceChannels → $targetChannels",
            )
        }

        // 帧对齐：不足一帧的零头留在装配器里等下一块。
        val outSamples = assembler.append(laidOut, laidOutCount, assembled)
        if (outSamples == 0) return true

        val frames = PcmLayout.framesForSamples(outSamples, targetChannels)
        ring.write(assembled, frames)
        framesDelivered += frames.toLong()
        return true
    }

    companion object {
        /** 默认每次拉 20 ms：与内核一帧同长，开流后第一次 `readPcm` 就能拿到整帧。 */
        const val DEFAULT_FRAMES_PER_READ = 20
    }
}
