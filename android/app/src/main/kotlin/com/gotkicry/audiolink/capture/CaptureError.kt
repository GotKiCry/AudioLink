package com.gotkicry.audiolink.capture

/**
 * 采集失败的**分类** —— 它决定三件事，所以是枚举而不是一个错误字符串：
 *
 * 1. **能不能自动重试**（[isRetryable]）：设备被占用值得退避重试；权限被拒重试一百次也没用，
 *    只会让用户看见一个反复弹窗的面板；
 * 2. **要不要用户动作**（[needsUserAction]）：权限类失败必须回到界面，其余可以在后台自己恢复；
 * 3. **日志里能不能一眼分清「环境给的」与「我们的 bug」**：只有 [Internal] 是我们自己的缺陷。
 *
 * 与 `audio/` 侧一致的口径：错误不吞、不编数字（见 `FfiPcmFeed` 与 `PlayoutLoop` 的类注释）。
 */
enum class CaptureErrorKind {
    /** 用户拒绝了授权，或从未授权（麦克风走运行时 `RECORD_AUDIO`）。 */
    PermissionDenied,

    /**
     * 运行中授权被回收。
     *
     * **Android 14+ 的内录每次会话都要重新授权**（系统行为，无法绕过）—— 所以它是一条
     * **正常**的运行期路径，而不是异常：界面必须能再拉起一次授权页。
     */
    PermissionRevoked,

    /** 采集设备被别的应用占着（`AudioRecord` 初始化失败 / 已释放）。 */
    DeviceBusy,

    /**
     * 设备拿不出协议要求的格式（48 kHz / 2ch / FLOAT 或 16bit）。
     *
     * **不做重采样兜底**：全链路 48 kHz 是硬约束（`docs/03-protocol.md`），
     * 在本机偷偷 SRC 会让「零重采样」断言在接收侧变成一句谎话。宁可明确报不支持。
     */
    UnsupportedFormat,

    /** 本机没有可用的采集设备；API < 29 之上的「系统内录」也归这一类。 */
    NoCaptureSource,

    /** 源在停止路径上被释放 —— 这是**正常竞态**（stop 与 pump 并发），不是故障。 */
    SourceReleased,

    /** 其它，含我们自己的实现缺陷。 */
    Internal,
}

/**
 * 该失败是否值得**自动**退避重试。
 *
 * 只有 [CaptureErrorKind.DeviceBusy] 是「等一会儿可能就好」的：另一个应用放开麦克风、
 * 或者我们上一轮释放设备还没回收完。其余全是**要人**或**要环境**的条件，自动重试只会烧电。
 */
val CaptureErrorKind.isRetryable: Boolean
    get() = this == CaptureErrorKind.DeviceBusy

/** 该失败是否必须让用户在界面上做点什么（授权、换设备、升级系统）。 */
val CaptureErrorKind.needsUserAction: Boolean
    get() = when (this) {
        CaptureErrorKind.PermissionDenied,
        CaptureErrorKind.PermissionRevoked,
        CaptureErrorKind.NoCaptureSource,
        CaptureErrorKind.UnsupportedFormat,
        -> true

        CaptureErrorKind.DeviceBusy,
        CaptureErrorKind.SourceReleased,
        CaptureErrorKind.Internal,
        -> false
    }

/**
 * 采集失败：分类 + 机械上下文 + 人话说明。
 *
 * [describe] 是**给用户看的**那一句（UI 规格 §4：错误必说人话）；[detail] 是给日志看的技术细节。
 * 两者都在这里生成，避免每个调用点各写一套中文。
 */
class CaptureFailure(
    val kind: CaptureErrorKind,
    val detail: String = "",
    cause: Throwable? = null,
) : Exception(
    if (detail.isEmpty()) kind.describe() else "${kind.describe()}($detail)",
    cause,
) {

    /** 是否值得自动退避重试（见 [isRetryable]）。 */
    val isRetryable: Boolean get() = kind.isRetryable

    /** 是否必须回到界面让用户处理（见 [needsUserAction]）。 */
    val needsUserAction: Boolean get() = kind.needsUserAction

    companion object {
        /**
         * 把平台异常收敛成分类。
         *
         * 映射依据是 `AudioRecord` 的实际行为（`docs/04-tech-stack.md` 的 Android 音频栈）：
         * * `SecurityException` —— 没授权就 `startRecording()`；
         * * `IllegalStateException` —— `AudioRecord` 未初始化成功（设备被占用是最常见的原因）；
         * * `IllegalArgumentException` —— 请求的 `AudioFormat` 设备不认（例如 48 kHz 单声道）；
         * * `UnsupportedOperationException` —— 该设备/系统版本根本不提供这条采集路径。
         *
         * 已经是我们自己的 [CaptureFailure] 就原样返回：不要把它降级成 `Internal`
         * —— 那会把「权限被拒」在上层变成「未知错误」，用户再也看不到该点的那颗按钮。
         */
        fun fromException(error: Throwable): CaptureFailure = when (error) {
            is CaptureFailure -> error
            is SecurityException -> CaptureFailure(
                CaptureErrorKind.PermissionDenied,
                error.message ?: error.javaClass.simpleName,
                error,
            )

            is IllegalStateException -> CaptureFailure(
                CaptureErrorKind.DeviceBusy,
                error.message ?: error.javaClass.simpleName,
                error,
            )

            is IllegalArgumentException -> CaptureFailure(
                CaptureErrorKind.UnsupportedFormat,
                error.message ?: error.javaClass.simpleName,
                error,
            )

            is UnsupportedOperationException -> CaptureFailure(
                CaptureErrorKind.NoCaptureSource,
                error.message ?: error.javaClass.simpleName,
                error,
            )

            else -> CaptureFailure(
                CaptureErrorKind.Internal,
                "${error.javaClass.simpleName}: ${error.message}",
                error,
            )
        }
    }
}

/** 给用户看的那一句话（UI 直接可用，不需要再拼字符串）。 */
internal fun CaptureErrorKind.describe(): String = when (this) {
    CaptureErrorKind.PermissionDenied -> "没有获得录音授权"
    CaptureErrorKind.PermissionRevoked -> "录音授权已被系统收回，需要重新授权"
    CaptureErrorKind.DeviceBusy -> "采集设备正被其它应用占用"
    CaptureErrorKind.UnsupportedFormat -> "这台设备不支持 48 kHz 双声道的采集格式"
    CaptureErrorKind.NoCaptureSource -> "这台设备没有可用的采集源"
    CaptureErrorKind.SourceReleased -> "采集源已释放"
    CaptureErrorKind.Internal -> "采集内部错误"
}
