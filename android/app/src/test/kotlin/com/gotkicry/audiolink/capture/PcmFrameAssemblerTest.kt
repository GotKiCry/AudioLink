package com.gotkicry.audiolink.capture

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * [PcmFrameAssembler] 的行为测试 —— 它守着「内核拿到的每个 packet 都是整数帧」这条纪律。
 */
class PcmFrameAssemblerTest {

    @Test
    fun wholeFramesPassThroughAndLeaveNoCarry() {
        val assembler = PcmFrameAssembler(channelCount = 2)
        val dst = FloatArray(8)
        val written = assembler.append(floatArrayOf(1f, 2f, 3f, 4f), 4, dst)
        assertEquals(4, written)
        assertEquals(0, assembler.pendingSamples)
        assertArrayEquals(floatArrayOf(1f, 2f, 3f, 4f), dst.copyOf(4), 0f)
    }

    @Test
    fun oddTailIsCarriedIntoTheNextBlock() {
        val assembler = PcmFrameAssembler(channelCount = 2)
        val dst = FloatArray(8)

        // 第一块 3 个样本（1.5 帧）：只交付 2 个，剩下 1 个进 carry。
        val first = assembler.append(floatArrayOf(1f, 2f, 3f), 3, dst)
        assertEquals(2, first)
        assertEquals(1, assembler.pendingSamples)
        assertArrayEquals(floatArrayOf(1f, 2f), dst.copyOf(2), 0f)

        // 第二块 2 个样本：与 carry 拼成 1.5 帧 → 交付 2 个，再留 1 个。
        val second = assembler.append(floatArrayOf(4f, 5f), 2, dst)
        assertEquals(2, second)
        assertEquals("carry 必须补上第一块的零头", 3f, dst[0], 0f)
        assertEquals(4f, dst[1], 0f)
        assertEquals(1, assembler.pendingSamples)

        // 第三块 1 个样本：拼齐最后一帧。
        val third = assembler.append(floatArrayOf(6f), 1, dst)
        assertEquals(2, third)
        assertEquals(5f, dst[0], 0f)
        assertEquals(6f, dst[1], 0f)
        assertEquals(0, assembler.pendingSamples)
    }

    @Test
    fun noSamplesDeliveredWhileCarryIsIncomplete() {
        val assembler = PcmFrameAssembler(channelCount = 2)
        val dst = FloatArray(4)
        assertEquals(0, assembler.append(floatArrayOf(0.1f), 1, dst))
        assertEquals(1, assembler.pendingSamples)
        assertEquals("一块都没有交付时不该写 dst", 0, assembler.append(FloatArray(0), 0, dst))
        assertEquals(1, assembler.pendingSamples)
    }

    @Test
    fun emptyBlockKeepsCarryIntact() {
        val assembler = PcmFrameAssembler(channelCount = 2)
        val dst = FloatArray(4)
        assembler.append(floatArrayOf(7f), 1, dst)
        assertEquals(0, assembler.append(floatArrayOf(9f, 9f), 0, dst))
        assertEquals("空块不能吃掉 carry", 1, assembler.pendingSamples)
    }

    @Test
    fun carryNeverExceedsOneFrameMinusOneSample() {
        val assembler = PcmFrameAssembler(channelCount = 2)
        val dst = FloatArray(4)
        repeat(50) { index ->
            assembler.append(floatArrayOf(index.toFloat()), 1, dst)
            assert(assembler.pendingSamples < 2) { "carry 上界被打破：${assembler.pendingSamples}" }
        }
    }

    @Test
    fun resetDropsCarry() {
        val assembler = PcmFrameAssembler(channelCount = 2)
        val dst = FloatArray(4)
        assembler.append(floatArrayOf(1f), 1, dst)
        assembler.reset()
        assertEquals(0, assembler.pendingSamples)
    }

    @Test
    fun invalidArgumentsThrow() {
        val assembler = PcmFrameAssembler(channelCount = 2)
        val dst = FloatArray(4)
        try {
            assembler.append(floatArrayOf(1f), 2, dst)
            throw AssertionError("越界的 srcCount 应当抛 IllegalArgumentException")
        } catch (_: IllegalArgumentException) {
            // 期望路径。
        }
        try {
            assembler.append(floatArrayOf(1f, 2f, 3f, 4f), 4, FloatArray(2))
            throw AssertionError("装不下的 dst 应当抛 IllegalArgumentException")
        } catch (_: IllegalArgumentException) {
            // 期望路径。
        }
    }
}
