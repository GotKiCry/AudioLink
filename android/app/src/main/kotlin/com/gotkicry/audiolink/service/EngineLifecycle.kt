package com.gotkicry.audiolink.service

import android.util.Log
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/**
 * 进程内的引擎操作队列。入口和协程均在同一线程（生产环境为 Main.immediate）。
 * 队列独立于 Service 实例；旧服务销毁后，已登记的停止仍须在新服务启动之前完成。
 * 真正的 FFI 调用由调用方切到 IO，不能把主线程阻塞在 native 调用上。
 */
/** 日志 tag：`adb logcat -s AudioLink` 就能看到引擎启动链路。 */
private const val TAG = "AudioLink"

/**
 * 引擎启动链路的痕迹 —— 真机上唯一的现场。
 *
 * 为什么包在 runCatching 里：单测跑在 JVM 上，`android.util.Log` 是 stub（一调用就抛），
 * 日志是尽力而为的现场记录，不该因为它把业务路径带塌（2026-09 实测：直接调 Log 让
 * EngineLifecycleTest 四条全红）。
 */
private fun logEngineStart(message: String, error: Throwable? = null) {
    runCatching {
        if (error == null) Log.i(TAG, message) else Log.i(TAG, message, error)
    }
}

internal class EngineOperationQueue(private val scope: CoroutineScope) {
    private var tail: Job? = null

    fun enqueue(operation: suspend () -> Unit) {
        val previous = tail
        tail = scope.launch {
            previous?.join()
            operation()
        }
    }
}

/**
 * 一个 Service 实例拥有的一代引擎状态。所有入口/发布均在队列所在线程调用。
 * 停止/销毁立即使旧结果失效，但保留底层操作：启动完成后仍须执行对应的停止。
 */
internal class EngineLifecycle<S>(
    private val queue: EngineOperationQueue,
    private val startEngine: suspend () -> S,
    private val stopEngine: suspend () -> Unit,
    private val publish: (S?, String?) -> Unit,
) {
    private var requested = false
    private var closed = false
    var generation: Long = 0
        private set

    fun isCurrent(token: Long): Boolean = !closed && token == generation

    fun start() {
        if (closed || requested) return
        requested = true
        val token = ++generation
        queue.enqueue {
            var status: S? = null
            var error: String? = null
            // 正式日志（不是临时的）：引擎启动失败的原因在界面上一度只写进 state.engineError，
            // 而那块折叠区在真机上没人展开 —— 排查绕了一大圈才看到 "UniFFI API checksum mismatch"。
            // 从此这条链路自带痕迹：begin 之后没有 done/failed = 挂死在 FFI 调用里（catch 到不了）；
            // 有 failed 就有**完整堆栈**，能直接指出是哪个导出函数不匹配。
            logEngineStart("engine start: begin")
            try {
                status = startEngine()
                logEngineStart("engine start: done")
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                error = "引擎启动失败：${e.javaClass.simpleName}: ${e.message}"
                logEngineStart("engine start: failed", e)
            }
            if (isCurrent(token)) {
                if (error != null) requested = false
                publish(status, error)
            }
        }
    }

    fun stop() {
        if (!requested) return
        requested = false
        val token = ++generation
        if (!closed) publish(null, null)
        queue.enqueue {
            var error: String? = null
            try {
                stopEngine()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                error = "引擎停止失败：${e.javaClass.simpleName}: ${e.message}"
            }
            if (isCurrent(token)) publish(null, error)
        }
    }

    fun close() {
        closed = true
        stop()
    }
}
