package com.gotkicry.audiolink.capture

import com.gotkicry.audiolink.core.PcmPull
import com.gotkicry.audiolink.diagnostics.EventRing
import com.gotkicry.audiolink.diagnostics.IntervalRing
import com.gotkicry.audiolink.diagnostics.ProbeEvent
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * FFI 采集契约的 Kotlin 实现：`com.gotkicry.audiolink.core.PcmPull`。
 *
 * 内核侧消费方是 `KotlinCaptureSource::read`（`core/crates/audiolink-ffi/src/audio_bridge.rs`），
 * 它给出的三条硬约束在这里逐条对应：
 *
 * 1. 「返回 48 kHz / 小端 f32 / 2ch 交错字节块，样本数 ≤ `max_samples`」——
 *    长度先向下取整到声道边界（[CaptureRing] 只会给出整帧），绝不超过 `max_samples`；
 * 2. 「返回空 = 暂无数据（不要忙等返回 0，那会在采集线程里变成热循环）」——
 *    数据不足时**阻塞在 [CaptureRing.readBlocking] 的条件变量上**（有界阻塞：环关闭或等满
 *    `maxBlockMs` 才返回空，后者是一道防死锁闸，见 [CaptureRing] 的构造参数说明）；
 * 3. 「可能从 Rust 音频线程调用，实现必须线程安全」—— `@Synchronized`
 *    （复用 `scratch`，与 `audio/FfiPcmFeed` 同一风格）。
 *
 * **停止路径的不变量**：采集停止**必须** `ring.close()`，否则内核线程会永远停在阻塞读上
 * （见 [CaptureRing] 的关闭语义）。
 *
 * 返回 `ByteArray` 而非逐样本装箱的 `List<Float>`；小端 f32 由 Rust 侧批量解码。
 */
class CapturePcmPull(
    private val ring: CaptureRing,
    /** 内核拉取节拍探针（常开诊断，`null` = 不记；每次调用一记）。 */
    private val probe: IntervalRing? = null,
    /** 离散事件探针：返回空（只该在环已关闭后发生）记一条。 */
    private val events: EventRing? = null,
) : PcmPull {

    /**
     * 复用的拉取缓冲。
     *
     * **长度必须精确等于本次请求量**：环按 `dst.size` 决定「拉多少帧」，
     * 若把一个大缓冲整块递进去，就会一次从环里取走远超 `max_samples` 的样本 ——
     * 既违反 FFI 契约（长度 ≤ `max_samples`），又会让多出来的样本**出队后被直接丢掉**。
     * 内核每次按同一个 `max_samples` 来拉，所以实际只在第一次分配。
     */
    private var scratch = FloatArray(0)

    /** 拉取次数（诊断）。 */
    var readCalls: Long = 0L
        private set

    /** 返回过「空」的次数：只该在环已关闭后增长。它涨说明内核在采集已停时还在拉。 */
    var emptyReturns: Long = 0L
        private set

    @Synchronized
    override fun readPcm(maxSamples: Int): ByteArray {
        val out = readFromRing(maxSamples)
        // 探针：每次调用一记（间隔 = 内核拉取节拍），空返回单独记事件。
        probe?.record(out.size)
        if (out.isEmpty()) events?.record(ProbeEvent.CAPTURE_PULL_EMPTY)
        return out
    }

    private fun readFromRing(maxSamples: Int): ByteArray {
        readCalls += 1
        if (maxSamples <= 0) {
            emptyReturns += 1
            return ByteArray(0)
        }
        // 契约要求整数帧：maxSamples 是奇数时只能少给一个样本 —— 宁少给，
        // 不给半帧（内核虽会截断，但那样每次都多记一次「读不满」，遥测噪声更大）。
        val usable = maxSamples - maxSamples % ring.channelCount
        if (usable <= 0) {
            emptyReturns += 1
            return ByteArray(0)
        }

        val dst = scratchFor(usable)
        val got = ring.readBlocking(dst)
        if (got <= 0) {
            emptyReturns += 1
            return ByteArray(0)
        }

        val out = ByteArray(got * 4)
        ByteBuffer.wrap(out)
            .order(ByteOrder.LITTLE_ENDIAN)
            .asFloatBuffer()
            .put(dst, 0, got)
        return out
    }

    /** 取一块**长度恰好等于本次请求量**的缓冲（长度变了才重分配这个 scratch）。 */
    private fun scratchFor(size: Int): FloatArray {
        if (scratch.size != size) {
            scratch = FloatArray(size)
        }
        return scratch
    }


}
