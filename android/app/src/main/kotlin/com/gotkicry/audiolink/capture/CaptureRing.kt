package com.gotkicry.audiolink.capture

import com.gotkicry.audiolink.audio.PcmLayout
import com.gotkicry.audiolink.audio.PcmRingBuffer
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.withLock

/**
 * 采集环：采集线程**写**、内核采集线程**读**（经 [CapturePcmPull]）。
 *
 * ## 为什么不直接让 FFI 回调里去读 `AudioRecord`
 * 两侧节奏不一样：设备按自己的块大小出数据（10 ms / 20 ms / 别的），内核按 20 ms 一帧来拉。
 * 更要紧的是 **FFI 契约明写「返回空 = 暂无数据，不要忙等返回 0」** —— 内核拿到空返回时
 * 只做一次最多 5 ms 的睡眠（`core/crates/audiolink-ffi/src/audio_bridge.rs` 的
 * `KotlinCaptureSource::read`）就再来拉，所以「立刻返回空」等于把内核采集线程变成热循环：
 * 占满一个核却一个样本都没搬。因此**阻塞必须发生在这里**（条件变量等数据），
 * 而不是把空返回丢给内核去轮询。
 *
 * ## 为什么复用 [PcmRingBuffer] 而不是另写一个环
 * 它与播放侧同一个环、同一套 drop-oldest 论证与同一套统计口径：采集侧积压同样等于
 * **端到端延迟增长**，所以「丢最旧、保最新」让延迟有上界。
 * **已知限制（诚实记账）**：`KotlinCaptureSource` 恒把 `CapturedPacket.discontinuity` 填 `false`，
 * 所以这里丢帧造成的**不连续无法经 FFI 上报给内核** —— 现阶段只有本端的 [overflowFrames] 看得见。
 * 要真正修，需要 FFI 契约增加字段（core/ 侧改动，不在本模块权限内）。
 *
 * ## 关闭语义（关键不变量）
 * 源一旦停止/失败**必须** [close]：它唤醒所有阻塞中的读取者并让它们立刻返回空。
 * 否则内核采集线程会永远停在 [readBlocking] 里，引擎连停都停不下来。
 * 关闭之后 [readBlocking] 恒返回 0（不阻塞）—— 内核会把它记成 `read_timeouts`，
 * 「采集源没了」在引擎遥测里因此也是可见的。
 *
 * 线程模型：线程安全（内部锁 + 条件变量；环自身另有锁，两者无反向获取路径）。
 */
class CaptureRing(
    capacityFrames: Int,
    val channelCount: Int = PcmFrameAssembler.DEFAULT_CHANNEL_COUNT,
    /**
     * 「凑满一帧」的额外等待上限（毫秒）。
     *
     * 为什么要有这一段：设备可能按 10 ms 出块，而内核按 20 ms 帧来拉 —— 不等就会长期交付「半帧」，
     * 让内核侧帧切分粒度变碎。等一小会儿（默认 5 ms）能显著提高「一次拉满」的比例，
     * 最坏代价只是 5 ms 额外延迟（远小于内核 20 ms 一帧的节奏）。单测传 0 可让等待变成确定性的。
     */
    private val topUpWaitMs: Long = DEFAULT_TOP_UP_WAIT_MS,

    /**
     * 单次读取的最长阻塞时间（毫秒）—— **这是一道防死锁的闸，不是性能参数**。
     *
     * 为什么必须有：内核采集线程会阻塞在 [readBlocking] 里；而「引擎停止要不要等这个线程退出」
     * 与「谁来关这个环」的顺序不在本模块手里。一旦前者先发生，无限阻塞就是死锁。
     * 有界等待把最坏情况退化成「最多 200 ms 后返回空」，内核侧会记成 `read_timeouts`
     * —— 采集没数据在引擎遥测里本来就是可见的，不会静默。
     */
    private val maxBlockMs: Long = DEFAULT_MAX_BLOCK_MS,
) {

    init {
        require(topUpWaitMs >= 0) { "topUpWaitMs 不能为负：$topUpWaitMs" }
        require(maxBlockMs > 0) { "maxBlockMs 必须为正：$maxBlockMs" }
    }

    /** 底层存储与统计（drop-oldest、overflowFrames 等口径全部复用播放环）。 */
    private val ring = PcmRingBuffer(capacityFrames, channelCount)

    private val lock = ReentrantLock()

    /** 「有新数据」与「已关闭」共用一个条件：两者的等待者都该在同一时刻被唤醒。 */
    private val hasData = lock.newCondition()

    private var closed = false

    /** 环容量（帧）。 */
    val capacityFrames: Int get() = ring.capacityFrames

    /** 当前可读帧数（诊断用）。 */
    val availableFrames: Int get() = ring.sizeFrames

    /** 因环满被丢弃的帧数（drop-oldest 口径）。它涨 = 内核读得比设备产得慢。 */
    val overflowFrames: Long get() = ring.overflowFrames

    /** 读不满的次数（内核一次没拉满一帧就 +1）。 */
    val underrunCount: Long get() = ring.underrunCount

    /** 是否已关闭（关闭后读取者不再阻塞）。 */
    val isClosed: Boolean get() = lock.withLock { closed }

    /**
     * 采集线程投递数据（[samples] 前 [frames] 帧，交错）。
     *
     * 已关闭时**静默丢弃**：停止路径上采集线程与停止调用并发是正常竞态，不该在那里炸。
     */
    fun write(samples: FloatArray, frames: Int) {
        if (frames <= 0) return
        lock.withLock {
            if (closed) return
            ring.write(samples, frames)
            hasData.signalAll()
        }
    }

    /**
     * 阻塞读取：等到**至少一帧**可用（或环被关闭），再尽力凑满 [dst]。
     *
     * @return 写进 [dst] 的**样本数**（[channelCount] 的整数倍）；0 = 已关闭且无数据。
     */
    fun readBlocking(dst: FloatArray): Int {
        val wantFrames = PcmLayout.framesForSamples(dst.size, channelCount)
        if (wantFrames <= 0) return 0

        lock.withLock {
            // 有界等待：数据 / 关闭 / maxBlockMs 三者任一到达就返回（见构造参数里的死锁说明）。
            var waitNanos = maxBlockMs * 1_000_000L
            while (!closed && ring.sizeFrames == 0 && waitNanos > 0) {
                waitNanos = hasData.awaitNanos(waitNanos)
            }
            if (ring.sizeFrames == 0) return 0

            var remainingNanos = topUpWaitMs * 1_000_000L
            while (!closed && remainingNanos > 0 && ring.sizeFrames < wantFrames) {
                remainingNanos = hasData.awaitNanos(remainingNanos)
            }
            return ring.readInto(dst)
        }
    }

    /** 关闭并唤醒所有等待者。幂等。 */
    fun close() {
        lock.withLock {
            closed = true
            hasData.signalAll()
        }
    }

    /** 清空数据但**不动统计**（口径同 [PcmRingBuffer.clear]）。 */
    fun clear() {
        lock.withLock { ring.clear() }
    }

    companion object {
        /** 「凑满一帧」的默认额外等待（见构造参数说明）。 */
        const val DEFAULT_TOP_UP_WAIT_MS = 5L

        /** 单次读取的默认最长阻塞（毫秒）：5 倍于内核一帧（20 ms），又远小于任何人类感知的卡顿。 */
        const val DEFAULT_MAX_BLOCK_MS = 100L
    }
}
