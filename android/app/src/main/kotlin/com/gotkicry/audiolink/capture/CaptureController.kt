package com.gotkicry.audiolink.capture

import android.media.projection.MediaProjection
import android.os.SystemClock
import com.gotkicry.audiolink.core.PcmPull

/**
 * 采集的总编排：状态机 + 采集源 + 采集线程 + 采集环 + 给内核的 [PcmPull] 出口。
 *
 * ```text
 * UI/服务 ──startMicrophone()/startLoopback()──► 本类
 *           本类 ──采集线程──► CapturePump ──► CaptureRing ──► CapturePcmPull ──► 内核（FFI）
 * ```
 *
 * ## 服务侧接线（**service/ 与 UI 不在本模块写作用域内，需另行接线**）
 * ```kotlin
 * // AudioLinkService：字段 + 启动引擎时把 pull 交给内核（原先是 capture = null）
 * private val capture = CaptureController()
 * engineStart(config, playout = pcmFeed, capture = capture.pull)
 *
 * // 用户选「系统内录」→ 拉起 MediaProjectionRequestActivity；结果回传后：
 * capture.startLoopback(projection)
 * // 用户选「麦克风」→ 先取得 RECORD_AUDIO 运行时权限；拿到后：
 * capture.startMicrophone()
 * // 停止采集：
 * capture.stop()
 * // Service.onDestroy（**必须在引擎停止之前或与之并发**）：
 * capture.close()
 * ```
 * **顺序不变量**：`close()` 会唤醒阻塞在 `readPcm` 里的内核采集线程。若把它放在引擎停止之后，
 * 引擎会先等这个线程，于是最多卡 [CaptureRing] 的 `maxBlockMs`（有界阻塞兜底）—— 不会死锁，
 * 但那段时间停不下来。反过来（先 close）是干净的。
 *
 * ## 重试与环的关系（刻意设计）
 * 一次「采集会话」= 一个环 + 一个 pull，**跨重试复用**：设备被占用时我们重新 `start()` 源，
 * 把数据继续写进**同一个环**。只有 [close] 才关环 —— 因为 pull 的持有者是引擎，换环就等于
 * 让引擎拿着一个死引用（而那正是「重连成功却没声音」那一类故障的形状）。
 *
 * ## 仍需真机验证
 * * 真实内录是否真的抓到系统音频、麦克风实际采样与延迟、授权弹窗流程、MIUI 的后台限制；
 * * 装箱（UniFFI 把 `Vec<f32>` 映成 `List<Float>`）在真机上的 GC 抖动。
 * 这些**本模块无法验证**，不要当成已验证。
 */
class CaptureController(
    ringCapacityFrames: Int = DEFAULT_RING_FRAMES,
    backoff: BackoffPolicy = BackoffPolicy(),
    private val clock: () -> Long = { SystemClock.elapsedRealtime() },
    private val framesPerRead: Int = CapturePump.DEFAULT_FRAMES_PER_READ,
) {

    private val ring = CaptureRing(ringCapacityFrames)

    /**
     * 给内核的采集出口（`engineStart(..., capture = ...)` 用它）。
     *
     * 生命周期与引擎一致：引擎起来时创建、引擎停止前 [close]。
     */
    val pull: PcmPull = CapturePcmPull(ring)

    private val machine = CaptureStateMachine(backoff)

    /** 保护 [machine] / [worker] / [activeSource] / [running] / [sessionId]（它们是跨线程读写的）。 */
    private val guard = Any()

    private var worker: Thread? = null
    private var activeSource: AudioCaptureSource? = null
    private var running = false

    /** 会话代次：每次 start 自增；旧线程据此发现「我已经不是当前那次会话了」并退出。 */
    private var sessionId = 0

    /** 当前状态（UI 读它）。 */
    val state: CaptureState get() = synchronized(guard) { machine.state }

    /** 状态快照（UI 与日志用；含「该让用户去授权吗」）。 */
    fun snapshot(): CaptureStateSnapshot = synchronized(guard) { machine.snapshot() }

    /** 环的观测读数（诊断面板用）。 */
    fun ringStats(): CaptureRingStats = CaptureRingStats(
        availableFrames = ring.availableFrames,
        overflowFrames = ring.overflowFrames,
        underrunCount = ring.underrunCount,
    )

    /** 开始麦克风采集（调用方需已取得 `RECORD_AUDIO` 运行时授权）。 */
    fun startMicrophone(): CaptureState = start(CaptureSourceKind.Microphone) { MicrophoneCaptureSource() }

    /** 开始系统内录（调用方需已拿到 MediaProjection 授权结果）。 */
    fun startLoopback(projection: MediaProjection): CaptureState =
        start(CaptureSourceKind.SystemLoopback) { SystemLoopbackCaptureSource(projection) }

    /**
     * 停止采集但**不关环**：用户可能马上再点一次「开始」。
     *
     * 停止后会唤醒阻塞在源 `read` 上的采集线程（`source.stop()` 会让 `AudioRecord.read` 返回）。
     */
    fun stop(): CaptureState {
        val thread = synchronized(guard) {
            running = false
            machine.onEvent(CaptureEvent.StopRequested, clock())
            val current = worker
            worker = null
            current
        }
        thread?.interrupt()
        stopSourceQuietly()
        joinWorker(thread)
        return synchronized(guard) {
            machine.onEvent(CaptureEvent.Stopped, clock())
            machine.state
        }
    }

    /**
     * 彻底关闭：停采集 + 关环 + 广播 `EngineStopped`。
     *
     * **幂等**；关闭之后 [pull] 恒返回空（不阻塞），所以内核采集线程不会卡住。
     */
    fun close() {
        stop()
        synchronized(guard) { machine.onEvent(CaptureEvent.EngineStopped, clock()) }
        ring.close()
    }

    private fun start(kind: CaptureSourceKind, factory: () -> AudioCaptureSource): CaptureState {
        // 已经在跑（或正在重试）就先收干净：同一个控制器同时只服务一个采集源。
        stop()
        val id = synchronized(guard) {
            // `Request` 只表示「用户要这个源」，状态机会先落到 `AwaitingPermission`；而走到本行时
            // **授权已经到手**（麦克风：调用方已持 RECORD_AUDIO；内录：调用方已拿到投影），
            // 所以紧接着补一发 `PermissionGranted`。
            // 不补的后果（2026-09-18 真机定位，静态读到）：状态永远停在「等待授权」；
            // 而 `StartFailed` 只在 `Starting`/`Running`/`Failed` 被接受 ⇒ 采集失败**完全静默**，
            // 退避重试也形同不存在（`nextRetryAtMs` 恒为 null）。
            machine.onEvent(CaptureEvent.Request(kind), clock())
            machine.onEvent(CaptureEvent.PermissionGranted, clock())
            running = true
            sessionId += 1
            sessionId
        }
        val thread = Thread({ supervise(id, kind, factory) }, THREAD_NAME)
        synchronized(guard) { worker = thread }
        thread.start()
        return state
    }

    /**
     * 采集线程主体：打开源 → 泵循环 → 失败按策略退避重试。
     *
     * 退避与「能不能重试」的判定全在 [CaptureStateMachine]（可单测）；这里只负责执行。
     */
    private fun supervise(id: Int, kind: CaptureSourceKind, factory: () -> AudioCaptureSource) {
        var attempt = 0
        while (isCurrent(id)) {
            val source = try {
                factory()
            } catch (error: Throwable) {
                if (!handleFailure(id, CaptureFailure.fromException(error))) return
                if (!awaitBackoff(id)) return
                continue
            }
            synchronized(guard) { activeSource = source }
            try {
                source.start()
                synchronized(guard) { machine.onEvent(CaptureEvent.StartSucceeded, clock()) }
                attempt = 0
                val pump = CapturePump(source, ring, framesPerRead = framesPerRead)
                while (isCurrent(id)) {
                    pump.pumpOnce()
                }
                // 正常退出（被 stop 打断）：把源收干净，状态由 stop() 推到 Idle。
                stopSourceQuietly()
                return
            } catch (error: Throwable) {
                val failure = if (error is CaptureFailure) error else CaptureFailure.fromException(error)
                stopSourceQuietly()
                if (!isCurrent(id)) return
                val canRetry = handleFailure(id, failure)
                attempt += 1
                if (!canRetry || attempt >= MAX_START_ATTEMPTS) {
                    // 放弃：环**不关**（引擎还在跑，之后用户再授权还要用同一个 pull）。
                    return
                }
                if (!awaitBackoff(id)) return
            }
        }
    }

    /** 记录失败并回答「还值得自动重试吗」。 */
    private fun handleFailure(id: Int, failure: CaptureFailure): Boolean = synchronized(guard) {
        if (!isCurrentLocked(id)) return false
        machine.onEvent(CaptureEvent.StartFailed(failure), clock())
        machine.nextRetryAtMs != null
    }

    /** 等到退避点（可被 [stop] 的 interrupt 打断）；返回 false = 该退出了。 */
    private fun awaitBackoff(id: Int): Boolean {
        val next = synchronized(guard) { if (isCurrentLocked(id)) machine.nextRetryAtMs else null } ?: return false
        val waitMs = (next - clock()).coerceAtLeast(0L)
        if (waitMs == 0L) return true
        return try {
            Thread.sleep(waitMs)
            isCurrent(id)
        } catch (_: InterruptedException) {
            false
        }
    }

    private fun isCurrent(id: Int): Boolean = synchronized(guard) { isCurrentLocked(id) }

    private fun isCurrentLocked(id: Int): Boolean = running && sessionId == id

    private fun stopSourceQuietly() {
        val source = synchronized(guard) {
            val current = activeSource
            activeSource = null
            current
        }
        try {
            source?.stop()
        } catch (_: Throwable) {
            // 收尾路径：停止失败没有可做的补救，状态机已经把这次会话标成停止。
        }
    }

    private fun joinWorker(thread: Thread?) {
        if (thread == null || thread === Thread.currentThread()) return
        try {
            thread.join(JOIN_TIMEOUT_MS)
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        }
    }

    /** 环的观测读数。 */
    data class CaptureRingStats(
        val availableFrames: Int,
        val overflowFrames: Long,
        val underrunCount: Long,
    )

    companion object {
        /**
         * 采集环深度：200 ms。
         *
         * 为什么是它：要能吸收「采集线程被系统临时挂住」的量级（几十毫秒），
         * 又不该让积压变成端到端延迟的来源（200 ms 已经是「听得出来」的边界）。
         * 再深只会把延迟预算吃在本地。
         */
        const val DEFAULT_RING_FRAMES = 9_600

        /** 连续失败多少次后放弃自动重试（约 0.2 + 0.4 + 0.8 + 1.6 ≈ 3 s 的总退避）。 */
        const val MAX_START_ATTEMPTS = 5

        private const val JOIN_TIMEOUT_MS = 1_000L

        private const val THREAD_NAME = "audiolink-capture"
    }
}
