package com.gotkicry.audiolink.capture

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [CaptureFailure.fromException] 的映射 —— 纯 JVM 可测的那一半。
 *
 * 为什么值得单独钉：这条映射**决定了用户在界面上看到什么、以及系统会不会自动重试**。
 * 权限被拒若被降级成 Internal，用户看到的是"采集内部错误"（找不到该按的那颗授权按钮），
 * 而退避策略会把它当成"等一会儿可能就好"反复重试 —— 一个不存在的授权是不会自己出现的。
 *
 * 与真机的分工：`AudioRecord` 在 JVM 上只有 stub（一调就抛），所以"哪一行真的会抛 SecurityException"
 * 只能靠真机验证（见 [MicrophoneCaptureSource] / [AudioRecordSupport] 的注释）；
 * 但"抛出来之后被归成哪一类、要不要重试"完全在这一层，必须能在没有真机时钉住。
 */
class CaptureFailureTest {

    @Test
    fun securityExceptionIsReportedAsPermissionDeniedAndNeverRetried() {
        val failure = CaptureFailure.fromException(
            SecurityException("Permission Denial: recording audio requires RECORD_AUDIO"),
        )

        assertEquals(CaptureErrorKind.PermissionDenied, failure.kind)
        assertTrue("权限类失败必须回到界面：用户能做的只有去授权", failure.needsUserAction)
        assertFalse("重试一百次也不会长出授权 —— 不许自动退避重试", failure.isRetryable)
        assertTrue(
            "人话要能看出是授权问题，而不是把平台的英文原句甩给用户",
            failure.message.orEmpty().contains("录音授权"),
        )
    }

    @Test
    fun alreadyClassifiedFailureIsNotDowngraded() {
        // 纪律（见 fromException 的注释）：已经是 CaptureFailure 就原样返回 ——
        // 重新包一层会把它降级成 Internal，上层就再也看不到"该去授权"这个结论。
        val original = CaptureFailure(CaptureErrorKind.PermissionRevoked, "系统收回了授权")

        val mapped = CaptureFailure.fromException(original)

        assertTrue("同一个对象就够，不必重新包装", mapped === original)
        assertEquals(CaptureErrorKind.PermissionRevoked, mapped.kind)
    }

    @Test
    fun deviceBusyAndFormatFailuresKeepTheirOwnKinds() {
        val busy = CaptureFailure.fromException(IllegalStateException("init failed"))
        assertEquals(
            "设备被占用：唯一值得自动重试的一类",
            CaptureErrorKind.DeviceBusy,
            busy.kind,
        )
        assertTrue(busy.isRetryable)

        val format = CaptureFailure.fromException(IllegalArgumentException("bad sample rate"))
        assertEquals(
            "格式不被设备接受：全链路 48 kHz 是硬约束，重试没用",
            CaptureErrorKind.UnsupportedFormat,
            format.kind,
        )
        assertFalse(format.isRetryable)
    }
}
