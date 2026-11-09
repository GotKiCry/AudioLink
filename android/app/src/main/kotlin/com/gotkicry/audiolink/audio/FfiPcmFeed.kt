package com.gotkicry.audiolink.audio

import com.gotkicry.audiolink.core.PcmFeed
import com.gotkicry.audiolink.core.PcmBufferState
import com.gotkicry.audiolink.diagnostics.IntervalRing
import java.nio.ByteBuffer
import java.nio.ByteOrder

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
 * 2. **格式**：`samples` 恒为 48 kHz / 小端 f32 / 2ch 交错的字节块，长度应为 `frames * 2 * 4`。
 * 3. **返回值**：`accepted < frames` 在内核侧会被记为"Kotlin 侧环满"（`PlayoutStats.write_errors++`），
 *    所以环满时**不能**闷声返回 `frames` —— 那会让上游遥测里这一段彻底消失。
 *
 * `ByteArray` 避免逐样本装箱；`FloatBuffer` 批量读入预分配 scratch，避免逐元素 JVM 方法调用。
 *
 * @param ring 目标播放环（其 `channelCount` 决定交错布局）。
 */
class FfiPcmFeed(
    private val ring: PcmRingBuffer,
    private val channelCount: Int = ring.channelCount,
    private val outputBufferState: () -> PcmBufferState? = { null },
    /** 供给节拍探针（常开诊断，`null` = 不记；每次回调一记，路径上零分配）。 */
    private val probe: IntervalRing? = null,
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

    /** 复用的搬运缓冲：`FloatBuffer` 直接写入这里，不再逐样本装箱或复制出第二个样本数组。 */
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
    override fun feedPcm(samples: ByteArray, frames: Int): Int {
        // 探针记每一次回调（含被拒绝的）：内核供给的间隔抖动/停摆正是「计数器全绿但声音断」要找的东西。
        probe?.record(frames)
        if (frames <= 0) return 0

        val wanted = PcmLayout.samplesForFrames(frames, channelCount)
        val wantedBytes = wanted * 4
        if (samples.size < wantedBytes) {
            // 声明的帧数与实际字节数不符 = 上游口径破了。**宁可不写**：
            // 写进去就是整段声道错位（听起来像噪音），比丢一段糟得多；返回 0 也让内核把这次算成异常。
            return 0
        }

        val buffer = scratchFor(wanted)
        ByteBuffer.wrap(samples)
            .order(ByteOrder.LITTLE_ENDIAN)
            .asFloatBuffer()
            .get(buffer, 0, wanted)

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
