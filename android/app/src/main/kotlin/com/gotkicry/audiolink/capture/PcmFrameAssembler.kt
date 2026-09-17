package com.gotkicry.audiolink.capture

/**
 * 帧对齐装配器 —— **纯逻辑，可直接 JVM 单测**。
 *
 * 为什么必须有它：采集设备一次 `read()` 返回多少样本是**设备说了算**（常见是 10 ms 或 20 ms 的整块，
 * 但容量、格式、调度都会让它变成别的数字，**甚至不是整数帧**）。而内核拿到的每个
 * `CapturedPacket.frames` 都必须是「整数帧」—— 半帧交给编码器就是整段声道错位（听起来像噪音）。
 *
 * 做法：把「不足一帧的零头样本」留在装配器里，等下一块来了拼齐再交付。**丢弃零头是错的**
 * （每块丢 1 个样本 = 每块少半个采样时刻 = 累积的时序漂移），所以这里必须缓存。
 *
 * 缓存上界 = `channelCount - 1` 个样本（结构上不可能更多），所以不需要「缓存爆了怎么办」的分支。
 *
 * 线程模型：**非线程安全**，只允许采集线程调用。
 */
class PcmFrameAssembler(private val channelCount: Int = DEFAULT_CHANNEL_COUNT) {

    init {
        require(channelCount > 0) { "channelCount 必须为正：$channelCount" }
    }

    /** 跨块残留的零头样本；容量恒为 [channelCount]，实际使用 [pendingSamples] 个。 */
    private val carry = FloatArray(channelCount)

    /** [carry] 里有效的样本数（0 .. channelCount-1）。 */
    var pendingSamples: Int = 0
        private set

    /**
     * 交付一块**整帧对齐**的样本。
     *
     * @param src 采集到的交错样本（已是目标声道数、已是 float）。
     * @param srcCount 本次有效的样本数（可能小于 `src.size` —— 设备只填了这么多）。
     * @param dst 输出缓冲（调用方复用；容量必须 ≥ 本次可交付的样本数）。
     * @return 写进 [dst] 的样本数（恒为 [channelCount] 的整数倍）。
     * @throws IllegalArgumentException [srcCount] 越界、或 [dst] 装不下时（调用方 bug，必须炸）。
     */
    fun append(src: FloatArray, srcCount: Int, dst: FloatArray): Int {
        require(srcCount in 0..src.size) { "srcCount 越界：$srcCount（src.size=${src.size}）" }

        val total = pendingSamples + srcCount
        val usable = total - total % channelCount
        require(dst.size >= usable) { "dst 只有 ${dst.size} 个样本，装不下 $usable 个" }

        // 段 1：先吃完 carry 里的零头（它一定排在拼接流的最前面）。
        var written = 0
        if (usable > 0 && pendingSamples > 0) {
            written = minOf(pendingSamples, usable)
            System.arraycopy(carry, 0, dst, 0, written)
        }
        // 段 2：再从 src 开头取，凑满 usable。
        val fromSrc = usable - written
        if (fromSrc > 0) {
            System.arraycopy(src, 0, dst, written, fromSrc)
        }

        // 重新计算 carry：拼接流里「还没交付」的尾部，长度 total - usable ∈ 0 .. channelCount-1。
        val tail = total - usable
        if (tail == 0) {
            pendingSamples = 0
            return usable
        }
        var kept = 0
        // (a) carry 自身还有没被交付的尾部（只在 usable == 0 时可能，例如本块一个样本都没填）。
        val carryLeft = pendingSamples - usable
        if (carryLeft > 0) {
            System.arraycopy(carry, usable, carry, 0, carryLeft)
            kept = carryLeft
        }
        // (b) 其余尾部来自 src 的末尾。
        val srcKeep = tail - kept
        if (srcKeep > 0) {
            System.arraycopy(src, srcCount - srcKeep, carry, kept, srcKeep)
            kept += srcKeep
        }
        pendingSamples = kept
        return usable
    }

    /** 丢掉残留零头（采集重启时用）。改了 `channelCount` 要重建装配器，而不是调它。 */
    fun reset() {
        pendingSamples = 0
    }

    companion object {
        /** 协议固定 2ch（`docs/11-m1-contract.md` §7）。 */
        const val DEFAULT_CHANNEL_COUNT = 2
    }
}
