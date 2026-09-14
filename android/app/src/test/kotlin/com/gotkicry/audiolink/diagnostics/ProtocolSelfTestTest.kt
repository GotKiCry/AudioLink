package com.gotkicry.audiolink.diagnostics

import com.gotkicry.audiolink.core.FfiException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [ProtocolSelfTest.map] 的测试 —— 全部在纯 JVM 上跑（**不需要 `.so`、不需要 Android**）。
 *
 * 为什么专门测"异常怎么变成人话"：真机验收那天它能决定排查方向。
 * 把 `UnsatisfiedLinkError` 显示成"自检失败"和显示成"原生库未加载（ABI 不匹配）"，
 * 是两条完全不同的排查路径。这里逐条钉住映射口径。
 */
class ProtocolSelfTestTest {

    @Test
    fun passedKeepsKernelSummaryVerbatim() {
        val result = ProtocolSelfTest.map { "protocol self-test … RESULT: PASS (5/5)" }

        assertTrue("必须是通过态", result is SelfTestResult.Passed)
        assertEquals(
            "内核返回的摘要要原样显示，不做二次加工",
            "protocol self-test … RESULT: PASS (5/5)",
            (result as SelfTestResult.Passed).summary,
        )
    }

    @Test
    fun kernelRejectionCarriesCodeShortNameAndContext() {
        val result = ProtocolSelfTest.map {
            throw FfiException.Failure(
                code = 1002u,
                messageText = "NOT_PAIRED",
                context = "peer requires pin pairing",
            )
        }

        assertTrue("必须是内核拒绝态", result is SelfTestResult.Rejected)
        val rejected = result as SelfTestResult.Rejected
        // 注意：Kotlin 里 `1002u` 字面量的类型是 UInt，与 UShort 字段装箱比较会因类型不同而不相等
        // （会得到 "expected: kotlin.UInt<1002> but was: kotlin.UShort<1002>"），必须显式转换。
        assertEquals(1002u.toUShort(), rejected.code)
        assertEquals("NOT_PAIRED", rejected.shortName)
        assertEquals("peer requires pin pairing", rejected.context)
    }

    @Test
    fun unsatisfiedLinkErrorBecomesActionableReason() {
        val result = ProtocolSelfTest.map {
            throw UnsatisfiedLinkError("dlopen failed: library \"libaudiolink_ffi.so\" not found")
        }

        assertTrue("这属于跑不起来的形态，不是内核拒绝", result is SelfTestResult.Unavailable)
        val reason = (result as SelfTestResult.Unavailable).reason
        assertTrue("要说清是原生库没加载", reason.contains("原生库未加载"))
        assertTrue("要保留原始报错，便于定位", reason.contains("libaudiolink_ffi.so"))
        assertTrue("要点出 ABI 这条常见原因", reason.contains("ABI"))
    }

    @Test
    fun otherThrowableKeepsTypeNameAndMessage() {
        val result = ProtocolSelfTest.map { throw IllegalStateException("引擎还没起来") }

        assertTrue(result is SelfTestResult.Unavailable)
        val reason = (result as SelfTestResult.Unavailable).reason
        assertTrue("异常类型名必须留着（否则无从下手）", reason.contains("IllegalStateException"))
        assertTrue(reason.contains("引擎还没起来"))
    }

    @Test
    fun mapActuallyInvokesTheCall() {
        var invoked = false
        val result = ProtocolSelfTest.map {
            invoked = true
            "ok"
        }

        assertTrue("映射层不能把调用吞掉", invoked)
        assertTrue(result is SelfTestResult.Passed)
    }
}
