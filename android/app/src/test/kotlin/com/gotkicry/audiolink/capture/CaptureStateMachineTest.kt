package com.gotkicry.audiolink.capture

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [CaptureStateMachine] 的规则测试。
 *
 * 三条最值钱的断言：**权限类失败不自动重试**、**设备占用按退避重试**、**停止幂等**。
 */
class CaptureStateMachineTest {

    private val backoff = BackoffPolicy(initialMs = 100, maxMs = 800, multiplier = 2)

    private fun machine() = CaptureStateMachine(backoff)

    private fun busy() = CaptureFailure(CaptureErrorKind.DeviceBusy, "被占用")

    @Test
    fun requestGoesToAwaitingPermission() {
        val machine = machine()
        assertEquals(CaptureState.Idle, machine.state)
        assertEquals(CaptureState.AwaitingPermission, machine.onEvent(CaptureEvent.Request(CaptureSourceKind.Microphone), 0))
        assertEquals(CaptureSourceKind.Microphone, machine.source)
    }

    @Test
    fun grantedThenStartedLeadsToRunning() {
        val machine = machine()
        machine.onEvent(CaptureEvent.Request(CaptureSourceKind.SystemLoopback), 0)
        assertEquals(CaptureState.Starting, machine.onEvent(CaptureEvent.PermissionGranted, 10))
        assertEquals(CaptureState.Running, machine.onEvent(CaptureEvent.StartSucceeded, 20))
        assertNull(machine.nextRetryAtMs)
    }

    @Test
    fun deniedPermissionNeverRetriesAutomatically() {
        val machine = machine()
        machine.onEvent(CaptureEvent.Request(CaptureSourceKind.Microphone), 0)
        machine.onEvent(CaptureEvent.PermissionDenied("用户点了拒绝"), 5)
        assertEquals(CaptureState.Failed, machine.state)
        assertNull("权限被拒不该自动重试（否则会反复弹窗）", machine.nextRetryAtMs)
        assertTrue(machine.needsUserAction)
        assertFalse(machine.isReadyToRetry(1_000_000))
    }

    @Test
    fun deviceBusyRetriesWithExponentialBackoff() {
        val machine = machine()
        machine.onEvent(CaptureEvent.Request(CaptureSourceKind.Microphone), 0)
        machine.onEvent(CaptureEvent.PermissionGranted, 0)

        machine.onEvent(CaptureEvent.StartFailed(busy()), 1_000)
        assertEquals(1, machine.attempts)
        assertEquals(1_100L, machine.nextRetryAtMs)
        assertFalse("还没到退避点", machine.isReadyToRetry(1_099))
        assertTrue("到点了", machine.isReadyToRetry(1_100))

        // 再失败两次：100 → 200 → 400 → 封顶 800。
        machine.onEvent(CaptureEvent.StartFailed(busy()), 1_100)
        assertEquals(1_300L, machine.nextRetryAtMs)
        machine.onEvent(CaptureEvent.StartFailed(busy()), 1_300)
        assertEquals(1_700L, machine.nextRetryAtMs)
        machine.onEvent(CaptureEvent.StartFailed(busy()), 1_700)
        assertEquals(2_500L, machine.nextRetryAtMs)
        machine.onEvent(CaptureEvent.StartFailed(busy()), 2_500)
        assertEquals(3_300L, machine.nextRetryAtMs)
        machine.onEvent(CaptureEvent.StartFailed(busy()), 3_300)
        assertEquals("退避封顶后再失败仍然等 maxMs", 4_100L, machine.nextRetryAtMs)
    }

    @Test
    fun nonRetryableRuntimeFailureStopsBackoffAndKeepsUserAction() {
        val machine = machine()
        machine.onEvent(CaptureEvent.Request(CaptureSourceKind.SystemLoopback), 0)
        machine.onEvent(CaptureEvent.PermissionGranted, 0)
        machine.onEvent(CaptureEvent.StartSucceeded, 0)
        // Android 14+：内录授权被系统回收 → 要用户重新授权，不该自己退避重试。
        machine.onEvent(
            CaptureEvent.StartFailed(CaptureFailure(CaptureErrorKind.PermissionRevoked, "会话结束")),
            500,
        )
        assertEquals(CaptureState.Failed, machine.state)
        assertNull(machine.nextRetryAtMs)
        assertTrue(machine.needsUserAction)
    }

    @Test
    fun successClearsFailureAndCounters() {
        val machine = machine()
        machine.onEvent(CaptureEvent.Request(CaptureSourceKind.Microphone), 0)
        machine.onEvent(CaptureEvent.PermissionGranted, 0)
        machine.onEvent(CaptureEvent.StartFailed(busy()), 0)
        machine.onEvent(CaptureEvent.StartSucceeded, 500)
        assertEquals(CaptureState.Running, machine.state)
        assertEquals(0, machine.attempts)
        assertNull(machine.lastFailure)
    }

    @Test
    fun stopIsIdempotentAndReturnsToIdle() {
        val machine = machine()
        machine.onEvent(CaptureEvent.Request(CaptureSourceKind.Microphone), 0)
        machine.onEvent(CaptureEvent.PermissionGranted, 0)
        machine.onEvent(CaptureEvent.StartSucceeded, 0)
        assertEquals(CaptureState.Stopping, machine.onEvent(CaptureEvent.StopRequested, 10))
        assertEquals(CaptureState.Idle, machine.onEvent(CaptureEvent.Stopped, 11))
        // 重复的停止事件不能把状态推回去。
        assertEquals(CaptureState.Idle, machine.onEvent(CaptureEvent.StopRequested, 12))
        assertEquals(CaptureState.Idle, machine.onEvent(CaptureEvent.Stopped, 13))
    }

    @Test
    fun engineStoppedClearsEverything() {
        val machine = machine()
        machine.onEvent(CaptureEvent.Request(CaptureSourceKind.SystemLoopback), 0)
        machine.onEvent(CaptureEvent.PermissionGranted, 0)
        machine.onEvent(CaptureEvent.StartFailed(busy()), 0)
        machine.onEvent(CaptureEvent.EngineStopped, 1)
        assertEquals(CaptureState.Idle, machine.state)
        assertNull(machine.source)
        assertEquals(0, machine.attempts)
    }

    @Test
    fun outOfOrderEventsDoNotMoveState() {
        val machine = machine()
        // 还没请求就「授权到手」：不该凭空进入 Starting。
        machine.onEvent(CaptureEvent.PermissionGranted, 0)
        assertEquals(CaptureState.Idle, machine.state)
        // 没在跑就「启动成功」：同上。
        machine.onEvent(CaptureEvent.StartSucceeded, 1)
        assertEquals(CaptureState.Idle, machine.state)
    }

    @Test
    fun backoffPolicyValidatesArguments() {
        try {
            BackoffPolicy(initialMs = 0)
            throw AssertionError("initialMs 必须为正")
        } catch (_: IllegalArgumentException) {
            // 期望路径。
        }
        try {
            BackoffPolicy(initialMs = 500, maxMs = 100)
            throw AssertionError("maxMs 不能小于 initialMs")
        } catch (_: IllegalArgumentException) {
            // 期望路径。
        }
        // 第 1 次失败等 initialMs，逐次翻倍，到 maxMs 封顶（默认 200 → 3200）。
        assertEquals(200L, BackoffPolicy().delayMsFor(1))
        assertEquals(400L, BackoffPolicy().delayMsFor(2))
        assertEquals(3_200L, BackoffPolicy().delayMsFor(99))
        assertEquals(1L, BackoffPolicy(initialMs = 1, maxMs = 1, multiplier = 2).delayMsFor(99))
    }
}
