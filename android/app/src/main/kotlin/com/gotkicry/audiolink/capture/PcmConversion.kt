package com.gotkicry.audiolink.capture

import com.gotkicry.audiolink.audio.PcmLayout

/**
 * 采集设备能给出的样本编码。
 *
 * 只列 `AudioRecord` 在 `AudioFormat.ENCODING_*` 里、且本协议能直接消化的两种：
 * 设备若连这两种都给不出 48 kHz 双声道，就报 [CaptureErrorKind.UnsupportedFormat]，
 * **不**在中间插一层格式协商（协议 §3 锁死 48 kHz / 2ch）。
 */
enum class PcmEncoding {
    /** `AudioFormat.ENCODING_PCM_FLOAT`（API 21+ 可读，无需转换）。 */
    F32,

    /** `AudioFormat.ENCODING_PCM_16BIT`（兼容性最好，需要一次 int16 → float 换算）。 */
    S16,
}

/**
 * 采集缓冲的样本换算与声道布局 —— **纯函数，可直接 JVM 单测**（不依赖 Android 框架）。
 *
 * ## 三条刻意的「不做」
 * 1. **不做重采样**。48 kHz 是协议红线（`docs/03-protocol.md` §3），内核的「零重采样」硬闸
 *    （`format_guard`）会在接收侧把它断言出来。谁想在这里 SRC，先去看那条闸门。
 * 2. **不做增益/限幅**。采集侧看到的样本就是设备的真值；音量是接收侧的事（§4.1 的 SET_GAIN）。
 *    float 路径超出 ±1 也原样保留 —— 削顶会把「设备给的数据本来就很响」这件事从遥测里抹掉。
 * 3. **不做补零**。补零属于消费方（[CapturePump] / 内核），这里只做等长搬运。
 *
 * ## 为什么 int16 除以 32768 而不是 32767
 * 32768 让 `Short.MIN_VALUE` 恰好映射到 -1.0，正负极值对称、无直流偏移；
 * 除以 32767 会让 -32768 溢出到 -1.00003（超出 [-1,1] 的名义范围）。
 * 代价是正极值只到 0.99997（-0.0003 dB），听不出来。
 */
object PcmConversion {

    /** 单个 int16 → float（[-1, 1]，见类注释的 32768 论证）。 */
    fun int16ToFloat(value: Short): Float = value / 32768f

    /**
     * 把 [src] 的前 [srcCount] 个 int16 样本换算成 float 写进 [dst] 的开头。
     *
     * @return 写入 [dst] 的样本数（= [srcCount]）。
     * @throws IllegalArgumentException 当 [srcCount] 越界、或 [dst] 装不下时（调用方 bug，必须炸）。
     */
    fun int16ToFloat(src: ShortArray, srcCount: Int, dst: FloatArray): Int {
        require(srcCount in 0..src.size) { "srcCount 越界：$srcCount（src.size=${src.size}）" }
        require(dst.size >= srcCount) { "dst 只有 ${dst.size} 个样本，装不下 $srcCount 个" }
        for (index in 0 until srcCount) {
            dst[index] = int16ToFloat(src[index])
        }
        return srcCount
    }

    /**
     * 单声道 → 双声道交错展开（`L R L R …`）：把每个样本复制一份。
     *
     * 为什么必须有它：协议要 2ch，而**大量手机的内置麦克风只有 1 个物理通道** ——
     * 请求 `CHANNEL_IN_STEREO` 时设备要么直接初始化失败，要么给回单声道数据。
     * 复制而不是「另一声道补零」：补零会让接收侧听到偏移到一边的声音，复制是等功率的居中。
     *
     * @param srcFrames 源里的**帧数**（单声道下帧数 = 样本数）。
     * @return 写入 [dst] 的**样本数**（= srcFrames × 2）。
     */
    fun expandMonoToStereo(src: FloatArray, srcFrames: Int, dst: FloatArray): Int {
        require(srcFrames in 0..src.size) { "srcFrames 越界：$srcFrames（src.size=${src.size}）" }
        val outSamples = PcmLayout.samplesForFrames(srcFrames, 2)
        require(dst.size >= outSamples) { "dst 只有 ${dst.size} 个样本，装不下 $outSamples 个" }
        for (frame in 0 until srcFrames) {
            val sample = src[frame]
            dst[frame * 2] = sample
            dst[frame * 2 + 1] = sample
        }
        return outSamples
    }
}
