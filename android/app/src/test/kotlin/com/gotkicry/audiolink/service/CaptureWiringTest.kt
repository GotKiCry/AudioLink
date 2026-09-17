package com.gotkicry.audiolink.service

import com.gotkicry.audiolink.capture.CaptureErrorKind
import com.gotkicry.audiolink.capture.CaptureFailure
import com.gotkicry.audiolink.capture.CaptureSourceKind
import com.gotkicry.audiolink.capture.CaptureState
import com.gotkicry.audiolink.capture.CaptureStateSnapshot
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 采集 → 服务接线的规则测试。
 *
 * 这里钉的是四类「写错了要真机才发现」的判定：
 * 1. **发送源关闭时不得把采集接进引擎**（否则等于无条件打开麦克风）；
 * 2. **只有「关闭 ⇄ 非关闭」才重启引擎**（内录 ⇄ 麦克风共享同一个 pull，重启会白掐断接收流）；
 * 3. **失败要分清「要用户动手」与「我们自己重试」**（两种提示不能混成一句）；
 * 4. **§13 能力位按平台真实能力给**：内录要 API 29、麦克风要 manifest 声明了 RECORD_AUDIO ——
 *    声明了做不到的事，对端会据此以为「这台设备能内录」，而运行期根本推不出来。
 *    它们与**当前发送源无关**（能力位说的是「能」而不是「正在」，与 CAPTURE/PLAYOUT 同口径）。
 */
class CaptureWiringTest {

    private fun snapshot(
        state: CaptureState,
        attempts: Int = 0,
        failure: CaptureFailure? = null,
        source: CaptureSourceKind? = CaptureSourceKind.Microphone,
    ) = CaptureStateSnapshot(
        state = state,
        source = source,
        attempts = attempts,
        nextRetryAtMs = if (attempts > 0) 1_000L else null,
        failure = failure,
    )

    @Test
    fun captureIsNotWiredWhenTheSourceIsOff() {
        assertFalse("发送源关闭时不能把采集出口交给引擎", CaptureWiring.engineCaptureEnabled(null))
        assertTrue(CaptureWiring.engineCaptureEnabled(CaptureSourceKind.Microphone))
        assertTrue(CaptureWiring.engineCaptureEnabled(CaptureSourceKind.SystemLoopback))
    }

    @Test
    fun onlyCrossingTheOffStateRestartsTheEngine() {
        assertFalse("关闭 → 关闭", CaptureWiring.requiresEngineRestart(null, null))
        assertFalse("麦克风 → 麦克风", CaptureWiring.requiresEngineRestart(CaptureSourceKind.Microphone, CaptureSourceKind.Microphone))
        assertFalse(
            "麦克风 → 内录：共用同一个 pull，不该重启引擎（重启会掐断正在播的接收流）",
            CaptureWiring.requiresEngineRestart(CaptureSourceKind.Microphone, CaptureSourceKind.SystemLoopback),
        )
        assertTrue("关闭 → 麦克风必须重启（capture 是引擎启动参数）", CaptureWiring.requiresEngineRestart(null, CaptureSourceKind.Microphone))
        assertTrue("内录 → 关闭必须重启", CaptureWiring.requiresEngineRestart(CaptureSourceKind.SystemLoopback, null))
    }

    @Test
    fun foregroundTypeFollowsTheSelectedSource() {
        assertEquals(CaptureServiceType.None, CaptureWiring.extraForegroundServiceType(null))
        assertEquals(CaptureServiceType.Microphone, CaptureWiring.extraForegroundServiceType(CaptureSourceKind.Microphone))
        assertEquals(
            "系统内录要的是 mediaProjection 类型位（不是 microphone）",
            CaptureServiceType.MediaProjection,
            CaptureWiring.extraForegroundServiceType(CaptureSourceKind.SystemLoopback),
        )
    }

    @Test
    fun noteIsSilentWhenTheSourceIsOff() {
        assertNull("关闭是正常态，通知里不该永久占一行", CaptureWiring.captureNote(null, snapshot(CaptureState.Idle)))
    }

    @Test
    fun noteDescribesTheHappyPath() {
        assertEquals("发送源：麦克风（等待启动）", CaptureWiring.captureNote(CaptureSourceKind.Microphone, null))
        val running = CaptureWiring.captureNote(CaptureSourceKind.Microphone, snapshot(CaptureState.Running))
        assertNotNull(running)
        assertTrue("运行态要说清在采集：$running", running!!.contains("正在采集"))
        assertTrue("等待授权要说清：$running", CaptureWiring.captureNote(
            CaptureSourceKind.SystemLoopback,
            snapshot(CaptureState.AwaitingPermission),
        )!!.contains("等待授权"))
    }

    @Test
    fun noteSeparatesUserActionFromAutoRetry() {
        val needsUser = CaptureWiring.captureNote(
            CaptureSourceKind.Microphone,
            snapshot(
                CaptureState.Failed,
                failure = CaptureFailure(CaptureErrorKind.PermissionDenied),
            ),
        )!!
        assertTrue("权限类失败必须让用户动手：$needsUser", needsUser.contains("需要你处理"))
        assertTrue("人话原因要带上：$needsUser", needsUser.contains("没有获得录音授权"))

        val autoRetry = CaptureWiring.captureNote(
            CaptureSourceKind.Microphone,
            snapshot(
                CaptureState.Failed,
                attempts = 3,
                failure = CaptureFailure(CaptureErrorKind.DeviceBusy),
            ),
        )!!
        assertTrue("设备占用是环境瞬态，应说明会自己重试：$autoRetry", autoRetry.contains("自动重试"))
        assertTrue("要带上第几次失败：$autoRetry", autoRetry.contains("第 3 次"))
    }

    @Test
    fun noteNamesTheRightSource() {
        assertTrue(
            CaptureWiring.captureNote(CaptureSourceKind.SystemLoopback, snapshot(CaptureState.Running))!!
                .contains("系统内录"),
        )
        assertTrue(
            CaptureWiring.captureNote(CaptureSourceKind.Microphone, snapshot(CaptureState.Running))!!
                .contains("麦克风"),
        )
    }

    // ---- §13 能力位：按平台真实能力给（task-28） ----

    @Test
    fun declaredCapabilitiesGatesLoopbackOnTheApi29Threshold() {
        // API 号写**字面量**，不要用 `SYSTEM_LOOPBACK_MIN_SDK ± 1`：那样测试与实现共享同一个
        // 假设，常量本身写错时两边一起错、测试永远绿。这条是实测踩出来的 —— 把门槛临时改成 28，
        // 用相对常量的版本照旧全绿；换成字面量 28/29 才抓得住。
        val below = CaptureWiring.declaredCapabilities(sdkInt = 28, declaresRecordAudio = true)
        assertEquals(
            "Android 9（API 28）没有 MediaProjection/AudioPlaybackCapture，不能声明内录位：$below",
            0u,
            below and CapabilityBits.SYSTEM_LOOPBACK,
        )

        val atThreshold = CaptureWiring.declaredCapabilities(sdkInt = 29, declaresRecordAudio = true)
        assertEquals(
            "Android 10（API 29）正是内录的门槛，必须声明：$atThreshold",
            CapabilityBits.SYSTEM_LOOPBACK,
            atThreshold and CapabilityBits.SYSTEM_LOOPBACK,
        )
    }

    @Test
    fun declaredCapabilitiesGatesMicrophoneOnTheManifestDeclaration() {
        val without = CaptureWiring.declaredCapabilities(sdkInt = 34, declaresRecordAudio = false)
        assertEquals(
            "manifest 没声明 RECORD_AUDIO 就不能声称能采麦克风：$without",
            0u,
            without and CapabilityBits.MICROPHONE,
        )
        // 内录位**不受** RECORD_AUDIO 影响：它俩是两个独立的平台前提。
        assertEquals(
            "没声明麦克风不影响内录位：$without",
            CapabilityBits.SYSTEM_LOOPBACK,
            without and CapabilityBits.SYSTEM_LOOPBACK,
        )

        val with = CaptureWiring.declaredCapabilities(sdkInt = 34, declaresRecordAudio = true)
        assertEquals(
            "声明了 RECORD_AUDIO 就要带上麦克风位：$with",
            CapabilityBits.MICROPHONE,
            with and CapabilityBits.MICROPHONE,
        )
    }

    @Test
    fun declaredCapabilitiesCombineTheTwoPlatformPremises() {
        assertEquals(
            "新设备 + 已声明麦克风：两位都在",
            CapabilityBits.SYSTEM_LOOPBACK or CapabilityBits.MICROPHONE,
            CaptureWiring.declaredCapabilities(sdkInt = 34, declaresRecordAudio = true),
        )
        assertEquals(
            "老设备（API 26）只有麦克风位",
            CapabilityBits.MICROPHONE,
            CaptureWiring.declaredCapabilities(sdkInt = 26, declaresRecordAudio = true),
        )
        assertEquals(
            "新设备但没声明麦克风：只有内录位",
            CapabilityBits.SYSTEM_LOOPBACK,
            CaptureWiring.declaredCapabilities(sdkInt = 29, declaresRecordAudio = false),
        )
    }
}
