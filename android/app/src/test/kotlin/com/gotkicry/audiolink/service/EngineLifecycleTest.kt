package com.gotkicry.audiolink.service

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.yield
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class EngineLifecycleTest {
    private suspend fun drain(queue: EngineOperationQueue) {
        val drained = CompletableDeferred<Unit>()
        queue.enqueue { drained.complete(Unit) }
        withTimeout(5_000) { drained.await() }
    }

    @Test(timeout = 10_000)
    fun destroyedServiceCannotPublishAndNewServiceWaitsForItsCleanup() = runBlocking {
        val queue = EngineOperationQueue(this)
        val startEntered = CompletableDeferred<Unit>()
        val finishStart = CompletableDeferred<Unit>()
        val stopEntered = CompletableDeferred<Unit>()
        val finishStop = CompletableDeferred<Unit>()
        val operations = mutableListOf<String>()
        val published = mutableListOf<String>()
        val old = EngineLifecycle(
            queue,
            startEngine = {
                operations += "old start"
                startEntered.complete(Unit)
                finishStart.await()
                "old"
            },
            stopEngine = {
                operations += "old stop"
                stopEntered.complete(Unit)
                finishStop.await()
            },
            publish = { status, _ -> published += "old: $status" },
        )
        val fresh = EngineLifecycle(
            queue,
            startEngine = { operations += "new start"; "new" },
            stopEngine = { operations += "new stop" },
            publish = { status, _ -> published += "new: $status" },
        )
        old.start()
        startEntered.await()
        val oldToken = old.generation
        old.close()
        assertFalse(old.isCurrent(oldToken))
        fresh.start()
        yield()
        assertEquals(listOf("old start"), operations)
        finishStart.complete(Unit)
        stopEntered.await()
        assertTrue(published.isEmpty())
        assertEquals(listOf("old start", "old stop"), operations)
        finishStop.complete(Unit)
        drain(queue)
        assertEquals(listOf("old start", "old stop", "new start"), operations)
        assertEquals(listOf("new: new"), published)
        fresh.close()
        drain(queue)
        assertEquals("new stop", operations.last())
        assertEquals(listOf("new: new"), published)
    }

    @Test(timeout = 10_000)
    fun restartInOneServiceIgnoresOldResultsAndInvalidatesEarlierReads() = runBlocking {
        val queue = EngineOperationQueue(this)
        val finishStop = CompletableDeferred<Unit>()
        val published = mutableListOf<Pair<String?, String?>>()
        var starts = 0
        val lifecycle = EngineLifecycle(
            queue,
            startEngine = { "engine-${++starts}" },
            stopEngine = { finishStop.await(); error("old stop failure") },
            publish = { status, error -> published += status to error },
        )
        lifecycle.start()
        drain(queue)
        val staleToken = lifecycle.generation
        lifecycle.stop()
        lifecycle.start()
        assertFalse("旧代次拿到的那次查询结果必须作废", lifecycle.isCurrent(staleToken))
        yield()
        assertEquals(1, starts)
        finishStop.complete(Unit)
        drain(queue)
        assertEquals(listOf("engine-1" to null, null to null, "engine-2" to null), published)
        lifecycle.close()
        drain(queue)
    }

    @Test(timeout = 10_000)
    fun repeatedCommandsAreIdempotentEvenWhileStartIsPending() = runBlocking {
        val queue = EngineOperationQueue(this)
        val finishStart = CompletableDeferred<Unit>()
        var starts = 0
        var stops = 0
        val published = mutableListOf<String?>()
        val lifecycle = EngineLifecycle(
            queue,
            startEngine = { starts++; finishStart.await(); "running" },
            stopEngine = { stops++ },
            publish = { status, _ -> published += status },
        )
        lifecycle.start()
        lifecycle.start()
        yield()
        lifecycle.stop()
        lifecycle.stop()
        finishStart.complete(Unit)
        drain(queue)
        assertEquals(1, starts)
        assertEquals(1, stops)
        assertTrue(published.all { it == null })
        lifecycle.close()
        lifecycle.start()
        drain(queue)
        assertEquals(1, starts)
        assertEquals(1, stops)
    }

    @Test(timeout = 10_000)
    fun nativeErrorsAreVisibleAndDoNotPoisonLaterOperations() = runBlocking {
        val queue = EngineOperationQueue(this)
        val errors = mutableListOf<String>()
        val published = mutableListOf<String?>()
        var starts = 0
        val lifecycle = EngineLifecycle(
            queue,
            startEngine = {
                if (++starts == 1) throw UnsatisfiedLinkError("missing native library")
                "running"
            },
            stopEngine = { error("native stop failure") },
            publish = { status, error ->
                published += status
                if (error != null) errors += error
            },
        )
        lifecycle.start()
        drain(queue)
        assertTrue(errors.single().contains("UnsatisfiedLinkError"))
        lifecycle.start()
        drain(queue)
        assertEquals("running", published.last())
        lifecycle.stop()
        lifecycle.stop()
        drain(queue)
        assertTrue(errors.last().contains("native stop failure"))
        assertEquals(2, errors.size)
    }
}
