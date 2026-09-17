package com.gotkicry.audiolink.capture

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [PcmConversion] 的口径测试。
 *
 * 这些数字不是「随手写的期望值」：int16 → float 的除数决定了整条链路的电平，
 * 选错了会让接收侧听到 0.03 dB 的偏差或让 -32768 溢出到 [-1,1] 之外。
 */
class PcmConversionTest {

    @Test
    fun int16ToFloatMapsFullScaleWithoutOverflow() {
        assertEquals(0f, PcmConversion.int16ToFloat(0), 0f)
        assertEquals(1f / 32768f, PcmConversion.int16ToFloat(1), 0f)
        assertEquals(-1f / 32768f, PcmConversion.int16ToFloat(-1), 0f)
        // 负极值必须**恰好**是 -1.0（除以 32767 会溢出到 -1.00003）。
        assertEquals(-1f, PcmConversion.int16ToFloat(Short.MIN_VALUE), 0f)
        // 正极值不到 1.0（-0.0003 dB），但绝不超过 1.0。
        assertTrue(PcmConversion.int16ToFloat(Short.MAX_VALUE) < 1f)
        assertEquals(32767f / 32768f, PcmConversion.int16ToFloat(Short.MAX_VALUE), 0f)
    }

    @Test
    fun int16ToFloatCopiesOnlyRequestedCount() {
        val src = shortArrayOf(0, 32767, -32768, 1000)
        val dst = FloatArray(4) { -2f }
        val written = PcmConversion.int16ToFloat(src, 2, dst)
        assertEquals(2, written)
        assertEquals(0f, dst[0], 0f)
        assertEquals(32767f / 32768f, dst[1], 0f)
        assertEquals("没被请求的尾巴不该被动", -2f, dst[2], 0f)
    }

    @Test
    fun int16ToFloatRejectsInvalidArguments() {
        val dst = FloatArray(2)
        assertThrows { PcmConversion.int16ToFloat(shortArrayOf(1), 2, dst) }
        assertThrows { PcmConversion.int16ToFloat(shortArrayOf(1), -1, dst) }
        assertThrows { PcmConversion.int16ToFloat(shortArrayOf(1, 2), 2, FloatArray(1)) }
    }

    @Test
    fun expandMonoToStereoDuplicatesEachSample() {
        val src = floatArrayOf(0.5f, -0.5f, 1f)
        val dst = FloatArray(6)
        val written = PcmConversion.expandMonoToStereo(src, 3, dst)
        assertEquals(6, written)
        // 交错布局：L R L R …，两个声道都是同一个样本（等功率居中，而不是补零偏到一边）。
        assertEquals(0.5f, dst[0], 0f)
        assertEquals(0.5f, dst[1], 0f)
        assertEquals(-0.5f, dst[2], 0f)
        assertEquals(-0.5f, dst[3], 0f)
        assertEquals(1f, dst[4], 0f)
        assertEquals(1f, dst[5], 0f)
    }

    @Test
    fun expandMonoToStereoRejectsInvalidArguments() {
        assertThrows { PcmConversion.expandMonoToStereo(floatArrayOf(1f), 1, FloatArray(1)) }
        assertThrows { PcmConversion.expandMonoToStereo(floatArrayOf(1f), 2, FloatArray(4)) }
    }

    /** 断言块抛 [IllegalArgumentException]（调用方 bug 必须炸，不能悄悄截断）。 */
    private fun assertThrows(block: () -> Unit) {
        try {
            block()
        } catch (_: IllegalArgumentException) {
            return
        }
        throw AssertionError("期望 IllegalArgumentException，但没有抛")
    }
}
