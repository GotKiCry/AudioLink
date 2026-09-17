package com.gotkicry.audiolink.capture

/** 采集的生命周期状态（FR-06/07 的「请求 → 授权 → 运行 → 停止」）。 */
enum class CaptureState {
    /** 没有采集，也没有待处理的请求。 */
    Idle,

    /** 已提出请求，等用户/系统给授权（内录拉 MediaProjection 授权页，麦克风要 RECORD_AUDIO）。 */
    AwaitingPermission,

    /** 已授权，正在打开设备（AudioRecord 构造 + startRecording）。 */
    Starting,

    /** 正常出数据。 */
    Running,

    /** 正在收尾（停源、关环）。 */
    Stopping,

    /** 出错了；能不能自动重试由 CaptureStateMachine.nextRetryAtMs 决定（null = 不能）。 */
    Failed,
}

/** 用户选中的采集源（FR-38 的「系统内录 / 麦克风 / 关闭」里的前两项）。 */
enum class CaptureSourceKind {
    /** 麦克风（AudioRecord，FR-07）。 */
    Microphone,

    /** 系统内录（AudioPlaybackCapture + MediaProjection，FR-06，API 29+）。 */
    SystemLoopback,
}

/** 驱动状态机的事件。 */
sealed interface CaptureEvent {

    /** 用户点了「开始采集」。 */
    data class Request(val source: CaptureSourceKind) : CaptureEvent

    /** 授权到手（麦克风运行时权限 / 内录的 MediaProjection 结果）。 */
    data object PermissionGranted : CaptureEvent

    /** 授权被拒。 */
    data class PermissionDenied(val detail: String = "") : CaptureEvent

    /** 设备打开成功、开始出数据。 */
    data object StartSucceeded : CaptureEvent

    /** 打开或运行中失败。 */
    data class StartFailed(val failure: CaptureFailure) : CaptureEvent

    /** 用户点了「停止采集」。 */
    data object StopRequested : CaptureEvent

    /** 收尾完成。 */
    data object Stopped : CaptureEvent

    /** 引擎停了（Service 销毁）：采集必须一起停。 */
    data object EngineStopped : CaptureEvent
}

/**
 * 采集失败后的退避策略 —— 纯数据 + 纯函数。
 *
 * 取值理由：设备被占用通常是**几秒级**瞬态（另一个应用正在录音、或来电占着麦克风），
 * 所以 200 ms 起步、翻倍、3.2 s 封顶：既不在一秒内连撞十次，也不会让用户等半分钟。
 * 上限刻意压到 3.2 s（而不是常见退避的几十秒）：采集是用户**当场按了按钮**的动作，
 * 后台静默等半分钟等于功能坏了。
 */
class BackoffPolicy(
    val initialMs: Long = 200L,
    val maxMs: Long = 3_200L,
    val multiplier: Int = 2,
) {

    init {
        require(initialMs > 0) { "initialMs 必须为正：$initialMs" }
        require(maxMs >= initialMs) { "maxMs 不能小于 initialMs：$maxMs < $initialMs" }
        require(multiplier >= 2) { "multiplier 至少要 2：$multiplier" }
    }

    /** 第 attempt 次失败（从 1 算起）之后该等多久（毫秒）。 */
    fun delayMsFor(attempt: Int): Long {
        require(attempt >= 1) { "attempt 从 1 算起：$attempt" }
        var delay = initialMs
        var index = 1
        while (index < attempt) {
            // 先判断是否已封顶，再乘 —— 否则 attempt 很大时乘法会绕回负数。
            if (delay >= maxMs) return maxMs
            delay *= multiplier.toLong()
            index += 1
        }
        return delay.coerceAtMost(maxMs)
    }
}

/** 状态的只读快照（给 UI 与日志用，避免它们各自拼字符串）。 */
data class CaptureStateSnapshot(
    val state: CaptureState,
    val source: CaptureSourceKind?,
    val attempts: Int,
    val nextRetryAtMs: Long?,
    val failure: CaptureFailure?,
) {
    /** 是否必须让用户在界面上做点什么（授权 / 换设备）。 */
    val needsUserAction: Boolean get() = failure?.needsUserAction == true
}

/**
 * 采集状态机 —— **纯逻辑、时间可注入、可直接 JVM 单测**（不依赖 Android 框架）。
 *
 * 这里有三条只有真机才容易踩、但没有真机也能测对的规则：
 *
 * 1. **权限类失败绝不自动重试**：用户拒绝授权后每 200 ms 再弹一次是最容易被投诉的形态；
 *    自动重试只对「设备被占用」这类环境瞬态开放（见 CaptureErrorKind.isRetryable）。
 * 2. **Android 14+ 的内录每次会话都要重新授权**，所以 Running 状态下收到
 *    PermissionRevoked 是**正常路径**，要回到「等授权」而不是「永久失败」。
 * 3. **停止必须幂等**：用户的 stop 与源自己的失败回调会并发到达，
 *    重复事件不能把状态推回 Running。
 *
 * 时间由调用方传入（onEvent 的 nowMs），所以退避与「到点没到点」完全可测。
 */
class CaptureStateMachine(private val backoff: BackoffPolicy = BackoffPolicy()) {

    var state: CaptureState = CaptureState.Idle
        private set

    var source: CaptureSourceKind? = null
        private set

    /** 连续失败次数（用于退避；成功后归零）。 */
    var attempts: Int = 0
        private set

    /** 下一次可以重试的时刻（毫秒，单调时钟）；null = 不该自动重试。 */
    var nextRetryAtMs: Long? = null
        private set

    var lastFailure: CaptureFailure? = null
        private set

    /** 是否必须用户动作（UI 据此显示「去授权」而不是「重试」）。 */
    val needsUserAction: Boolean get() = lastFailure?.needsUserAction == true

    /** 推进状态机；返回迁移后的状态（与 [state] 相同）。 */
    fun onEvent(event: CaptureEvent, nowMs: Long): CaptureState {
        when (event) {
            is CaptureEvent.Request -> {
                source = event.source
                attempts = 0
                nextRetryAtMs = null
                lastFailure = null
                state = CaptureState.AwaitingPermission
            }

            CaptureEvent.PermissionGranted -> when (state) {
                CaptureState.AwaitingPermission, CaptureState.Failed -> state = CaptureState.Starting
                else -> Unit
            }

            is CaptureEvent.PermissionDenied -> when (state) {
                CaptureState.AwaitingPermission, CaptureState.Starting -> {
                    lastFailure = CaptureFailure(CaptureErrorKind.PermissionDenied, event.detail)
                    nextRetryAtMs = null
                    state = CaptureState.Failed
                }

                else -> Unit
            }

            CaptureEvent.StartSucceeded -> when (state) {
                CaptureState.Starting, CaptureState.Failed -> {
                    attempts = 0
                    nextRetryAtMs = null
                    lastFailure = null
                    state = CaptureState.Running
                }

                else -> Unit
            }

            is CaptureEvent.StartFailed -> when (state) {
                // Failed 也必须接受：重试路径上「又打开失败一次」是连续投递的，
                // 若不接受，attempts 不再增长、退避会永远停在第一档（等于没有退避）。
                CaptureState.Starting, CaptureState.Running, CaptureState.Failed -> {
                    lastFailure = event.failure
                    if (event.failure.isRetryable) {
                        attempts += 1
                        nextRetryAtMs = nowMs + backoff.delayMsFor(attempts)
                    } else {
                        nextRetryAtMs = null
                    }
                    state = CaptureState.Failed
                }

                else -> Unit
            }

            CaptureEvent.StopRequested -> when (state) {
                CaptureState.Idle, CaptureState.Stopping -> Unit
                else -> state = CaptureState.Stopping
            }

            CaptureEvent.Stopped, CaptureEvent.EngineStopped -> {
                attempts = 0
                nextRetryAtMs = null
                lastFailure = null
                if (event is CaptureEvent.EngineStopped) source = null
                state = CaptureState.Idle
            }
        }
        return state
    }

    /** 第 attempt 次失败后的退避时长（透传策略，便于日志写清「等多久」）。 */
    fun retryDelayMs(attempt: Int): Long = backoff.delayMsFor(attempt)

    /** 现在能不能自动重试（可重试失败 + 已过退避点）。 */
    fun isReadyToRetry(nowMs: Long): Boolean {
        if (state != CaptureState.Failed) return false
        val at = nextRetryAtMs ?: return false
        return nowMs >= at
    }

    /** 只读快照。 */
    fun snapshot(): CaptureStateSnapshot = CaptureStateSnapshot(
        state = state,
        source = source,
        attempts = attempts,
        nextRetryAtMs = nextRetryAtMs,
        failure = lastFailure,
    )
}
