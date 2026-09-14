package com.gotkicry.audiolink.audio

/**
 * PCM 数据的**索引换算**（交错采样：`L R L R …`）。
 *
 * 为什么单独抽一个对象：
 *  - "帧"（一个采样时刻的所有声道）与"样本"（一个 Float）在本模块里同时出现（环缓冲按帧、
 *    `FloatArray` 按样本、AudioTrack 的 `write` 既收样本数又按帧返回），如果不集中换算，
 *    同一个 `× 2` 会散落在五六个地方，改声道数时必然漏改；
 *  - 它是纯函数，可以直接被 JVM 单测钉住（测试放在 `android/app/src/test/` 下，不依赖 Android 框架）。
 *
 * 所有函数都**不做取整补偿**：`framesForSamples` 用整除向下取整（不足一帧的零头样本由调用方丢弃），
 * 这是刻意的 —— 实时路径不允许"猜"半个帧的语义。
 */
object PcmLayout {

    /** 帧数 → 样本数（= 帧数 × 声道数）。 */
    fun samplesForFrames(frames: Int, channelCount: Int): Int {
        require(channelCount > 0) { "channelCount 必须为正：$channelCount" }
        require(frames >= 0) { "frames 不能为负：$frames" }
        return frames * channelCount
    }

    /** 样本数 → 帧数（向下取整：不足一帧的零头样本被丢弃）。 */
    fun framesForSamples(samples: Int, channelCount: Int): Int {
        require(channelCount > 0) { "channelCount 必须为正：$channelCount" }
        require(samples >= 0) { "samples 不能为负：$samples" }
        return samples / channelCount
    }

    /** 帧号 → 该帧首样本在交错 `FloatArray` 中的下标。 */
    fun sampleOffsetOfFrame(frame: Int, channelCount: Int): Int = samplesForFrames(frame, channelCount)
}

/**
 * PCM 数据源（**拉模式**）：由 Kotlin 侧的播放线程主动来读；内核/FFI 只负责把 PCM 灌进环缓冲。
 *
 * 规格：`docs/11-m1-contract.md` §7（Kotlin 接口形状冻结）。
 * 纪律：`docs/02-architecture.md` §1 第 3 条 —— `AudioTrack` 的生命周期与线程亲和性必须在 Kotlin 侧，
 *       内核**不**托管播放线程；所以这里只描述"数据从哪来"，不描述"谁驱动播放"。
 *
 * 返回值口径（刻意定义成**样本数**，而不是帧数，避免调用方各自猜）：
 *  - 返回真正写进 [dst] 的样本数，取值区间 `0..dst.size`；
 *  - 返回 `0`（或小于 `dst.size`）= 源里当前就这么多 = **欠载**，此时 `dst` 尾部元素内容未定义，
 *    实现**不得**假装它已经被清零（补零策略属于消费方 [PlayoutLoop] 的职责）；
 *  - 交错采样，故 `样本数 = 帧数 × channelCount`，换算统一走 [PcmLayout]。
 */
interface PcmSource {
    fun readInto(dst: FloatArray): Int
}

/**
 * "永远没有数据"的源：读多少次都是 0。
 *
 * 用途：`LowLatencyPlayer` 在没人 [com.gotkicry.audiolink.audio.LowLatencyPlayer.setSource] 时用它，
 * 语义上等于"内核还没接上" —— 此时播放链路照样运转（补静音），但欠载计数会持续增长，
 * 这正好把"没数据"这件事变成可观测的数字，而不是静默的无声。
 */
object EmptyPcmSource : PcmSource {
    override fun readInto(dst: FloatArray): Int = 0
}

/**
 * PCM 数据汇（**写侧**抽象）：播放线程把 PCM 交给它。
 *
 * 为什么不让 `PlayoutLoop` 直接依赖 `LowLatencyPlayer`：
 *  - 拉取循环（欠载判定、补零、回压）是本轮最值得单测的逻辑，而 `AudioTrack` 在 JVM 单测里
 *    只有 stub（一调就抛 `RuntimeException("Stub!")`）；面向接口后可用假实现把循环钉住。
 *
 * 返回值口径：返回**实际写入的帧数**，`0` 表示"这一轮写不进"（输出缓冲已满）。
 * 实现**不得**返回负数 —— 错误由实现自己记账（见 [LowLatencyPlayer.stats]）。
 */
interface PcmSink {
    fun tryWrite(samples: FloatArray, offsetSamples: Int, frames: Int): Int
}
