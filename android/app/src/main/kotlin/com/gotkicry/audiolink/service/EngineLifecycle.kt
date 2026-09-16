package com.gotkicry.audiolink.service

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/**
 * 进程内的引擎操作队列。入口和协程均在同一线程（生产环境为 Main.immediate）。
 * 队列独立于 Service 实例；旧服务销毁后，已登记的停止仍须在新服务启动之前完成。
 * 真正的 FFI 调用由调用方切到 IO，不能把主线程阻塞在 native 调用上。
 */
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
            try {
                status = startEngine()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                error = "引擎启动失败：${e.javaClass.simpleName}: ${e.message}"
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
