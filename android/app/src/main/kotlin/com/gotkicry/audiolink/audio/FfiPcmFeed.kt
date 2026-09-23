package com.gotkicry.audiolink.audio

import com.gotkicry.audiolink.core.PcmFeed
import com.gotkicry.audiolink.core.PcmBufferState

/**
 * 内核 PCM → 播放环的搬运器：UniFFI 回调接口 [`PcmFeed`] 的 Kotlin 实现。
 *
 * 位置（`docs/02-architecture.md` §1 第 3 条）：
 * ```
 * 网络 → engine 解码 → PcmFeed.feedPcm ──► 本类 ──► PcmRingBuffer ──► AudioTrack（Kotlin 侧播放线程）
 * ```
 * 内核只把 PCM 推过来，**不托管播放线程**；`AudioTrack` 的一切仍由 [LowLatencyPlayer] 在自己的线程上管。
 *
 * ## 三条必须守住的约定（来自 `core/crates/audiolink-ffi/src/audio_bridge.rs`）
 * 1. **线程**：`feedPcm` 由内核的 Rust 播放线程调用（不是 UI 线程、也不是 AudioTrack 线程），
 *    所以本类必须线程安全 —— 环自身有锁，[feedPcm] 另加 `@Synchronized` 保护复用缓冲（见下）。
 * 2. **格式**：`samples` 恒为 48 kHz / f32 / 2ch 交错，长度应为 `frames * 2`。
 * 3. **返回值**：`accepted < frames` 在内核侧会被记为"Kotlin 侧环满"（`PlayoutStats.write_errors++`），
 *    所以环满时**不能**闷声返回 `frames` —— 那会让上游遥测里这一段彻底消失。
 *
 * ## 已知代价（**未验证，不要当成"性能可接受"**）
 * UniFFI 0.29 把 Rust `Vec<f32>` 映成 `List<Float>`（没有 `FloatArray` 映射），
 * 每帧 20 ms / 1920 个样本会在 Kotlin 侧产生约 1920 个装箱对象 ≈ 30 KB/帧。
 * 本类只能做"少分配一次"：用**预分配 scratch 数组**逐元素搬运，而不是每个回调再 `toFloatArray()`。
 * **真机 GC 抖动未验证**（本机无真机）。若真机实测进不了预算，逃生通道是把这条 PCM 通道换成
 * UDL `bytes`（`ByteArray` + `ByteBuffer.asFloatBuffer()` 零拷贝视图）—— 属 Rust 侧改动，找 ffi 流。
 *
 * @param ring 目标播放环（其 `channelCount` 决定交错布局）。
 */
class FfiPcmFeed(
    private val ring: PcmRingBuffer,
    private val channelCount: Int = ring.channelCount,
    private val outputBufferState: () -> PcmBufferState? = { null },
) : PcmFeed {

    override fun playoutBufferState(): PcmBufferState? {
        val output = outputBufferState() ?: return null
        return output.copy(queuedFrames = (
            output.queuedFrames.toULong() + ring.sizeFrames.toULong()
        ).coerceAtMost(UInt.MAX_VALUE.toULong()).toUInt())
    }

    private companion object {
        /** 初始搬运缓冲：一帧 20 ms / 2ch 是 1920 个样本（`DEFAULT_FRAME_INTERLEAVED`）。 */
        const val INITIAL_SCRATCH_SAMPLES = 1_920
    }

    /** 复用的搬运缓冲：实时路径不再产生新数组（装箱已躲不掉，至少别再复制一份）。 */
    private var scratch = FloatArray(INITIAL_SCRATCH_SAMPLES)

    /**
     * 交付一段 PCM（内核回调，见类注释的三条约定）。
     *
     * `@Synchronized` 的原因：`scratch` 是实例级复用缓冲，若内核从多个线程回调同一个实例，
     * 并发搬运会让两段音频交错 —— 表现为"声音像被撕碎"，且只在负载高时复现。
     * 加锁后这种竞争不可能发生（真正的热路径仍是每 20 ms 一次，锁竞争代价可忽略）。
     *
     * @return 实际接收的帧数（仅供诊断）；环满导致丢帧时返回 `< frames`。
     */
    @Synchronized
    override fun feedPcm(samples: List<Float>, frames: Int): Int {
        if (frames <= 0) return 0

        val wanted = PcmLayout.samplesForFrames(frames, channelCount)
        if (samples.size < wanted) {
            // 声明的帧数与实际样本数不符 = 上游口径破了。**宁可不写**：
            // 写进去就是整段声道错位（听起来像噪音），比丢一段糟得多；返回 0 也让内核把这次算成异常。
            return 0
        }

        val buffer = scratchFor(wanted)
        for (i in 0 until wanted) {
            buffer[i] = samples[i]
        }

        val overflowBefore = ring.overflowFrames
        ring.write(buffer, frames)
        val dropped = ring.overflowFrames - overflowBefore

        // 环满时如实回报"没全收"：内核按 `accepted < frames` 记账（audio_bridge.rs 的约定），
        // 于是"Kotlin 侧跟不上"在内核遥测里同样可见，而不是只在我这边增长的一个私有计数。
        return if (dropped > 0) maxOf(0, frames - dropped.toInt()) else frames
    }

    /** 取够 [size] 个样本的搬运缓冲（只增不减：扩容一次之后长期复用）。 */
    private fun scratchFor(size: Int): FloatArray {
        if (scratch.size < size) {
            scratch = FloatArray(size)
        }
        return scratch
    }
}
