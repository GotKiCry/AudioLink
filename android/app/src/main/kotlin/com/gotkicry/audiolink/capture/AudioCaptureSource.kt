package com.gotkicry.audiolink.capture

/**
 * 协议要求的采集格式：**48 kHz / 2ch**（`docs/03-protocol.md` §3）。
 *
 * 编码是「设备给我们的原始编码」，不是最终交付编码 —— FFI 契约（`PcmPull.readPcm`）规定
 * 交付必须是 48 kHz / f32 / 2ch 交错，所以 [PcmEncoding.S16] 的设备要经
 * [PcmConversion.int16ToFloat] 转一次（纯函数，有单测）。
 */
data class CaptureFormat(
    val sampleRateHz: Int = SAMPLE_RATE_HZ,
    val channelCount: Int = 2,
    val encoding: PcmEncoding = PcmEncoding.F32,
) {
    /** 是否满足协议硬约束（不满足就不该开采集，见 [CaptureErrorKind.UnsupportedFormat]）。 */
    val isProtocolCompliant: Boolean
        get() = sampleRateHz == SAMPLE_RATE_HZ && channelCount in 1..2

    companion object {
        /** 全链路唯一的采样率；任何重采样都是缺陷，不是特性。 */
        const val SAMPLE_RATE_HZ = 48_000

        /** 默认形态：48 kHz / 2ch / FLOAT —— 无需转换。 */
        val DEFAULT = CaptureFormat()
    }
}

/**
 * 一次读取的目标缓冲：按源的编码给出 `FloatArray` 或 `ShortArray`。
 *
 * 存在的理由：`AudioRecord` 的浮点与 16 位路径读的是**不同数组类型**（`read(FloatArray,…)` /
 * `read(ShortArray,…)`），而 [AudioCaptureSource] 又必须是一个能在 JVM 上被假实现替换的接口。
 * 让调用方（[CapturePump]）持有一个缓冲并把它交给实现，两边都不必为类型写分支。
 */
class CaptureBuffer(val encoding: PcmEncoding, capacitySamples: Int) {

    init {
        require(capacitySamples > 0) { "capacitySamples 必须为正：$capacitySamples" }
    }

    /** F32 源用；S16 源下为空数组（不浪费一半内存）。 */
    val floats: FloatArray = if (encoding == PcmEncoding.F32) FloatArray(capacitySamples) else FloatArray(0)

    /** S16 源用；F32 源下为空数组。 */
    val shorts: ShortArray = if (encoding == PcmEncoding.S16) ShortArray(capacitySamples) else ShortArray(0)

    /** 本缓冲能装的样本数。 */
    val capacitySamples: Int = capacitySamples
}

/**
 * 采集源（**可替换接缝**）：麦克风、系统内录、以及测试用的假源都实现它。
 *
 * 为什么要把 `AudioRecord` / `MediaProjection` 挡在这个接口后面：JVM 单测里那些类只有 stub
 * （一调就抛 `RuntimeException("Stub!")`），而「读多少 → 怎么转换 → 怎么装配 → 怎么投进环」
 * 这段最容易写错的逻辑与设备无关，必须能在没有真机的情况下被钉住（同 `audio/PcmSource` 的取舍）。
 *
 * 线程模型：`start` / `stop` 由控制线程调用，`read` 由**采集线程**调用；实现可以要求
 * 「`read` 与 `stop` 不自旋竞争」，但不能假设二者同线程。
 */
interface AudioCaptureSource {

    /** 本源的格式；[read] 交付的样本必须与它一致。 */
    val format: CaptureFormat

    /**
     * 打开设备并开始采集。
     *
     * @throws CaptureFailure 平台异常必须收敛成它（用 [CaptureFailure.fromException]）。
     */
    fun start()

    /**
     * 读一块进 [buffer]（用与 [format] 匹配的那个数组），返回**样本数**（交错）。
     *
     * 契约：
     * 1. 返回 `0` = 这一轮没有数据；实现**可以**阻塞等待，也**可以**立刻返回 0
     *    （驱动节奏由 [CapturePump] 决定），但**不得**忙等空转；
     * 2. 返回值为负 = 出错，`CapturePump` 会把它抛给上层（`AudioRecord` 的错误码就是这样）；
     * 3. 返回值不得大于 `buffer.capacitySamples`（越界会被 `CapturePump` 截断并计数）。
     */
    fun read(buffer: CaptureBuffer): Int

    /** 停止并释放设备。幂等；停止路径上抛异常是允许的（调用方只记日志）。 */
    fun stop()
}

/**
 * 「永远没有数据」的采集源。
 *
 * 用途：给 `AudioLinkService` 一个安全的默认值 —— 在授权结果回来之前、或用户在界面选了
 * 「关闭」时，引擎照样可以启动，只是采集侧一直空（内核侧表现为 `read_timeouts` 增长）。
 * 语义与 `audio/EmptyPcmSource` 对齐：把「没有源」变成**可观测的数字**，而不是静默无声。
 */
object NoCaptureSource : AudioCaptureSource {

    override val format: CaptureFormat = CaptureFormat.DEFAULT

    override fun start() = Unit

    override fun read(buffer: CaptureBuffer): Int = 0

    override fun stop() = Unit
}
