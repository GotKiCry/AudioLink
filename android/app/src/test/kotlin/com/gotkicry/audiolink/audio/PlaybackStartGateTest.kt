package com.gotkicry.audiolink.audio

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PlaybackStartGateTest {
    @Test
    fun fillsLegacyStartThresholdEvenWhenThirtyMsTargetWasReached() {
        val gate = PlaybackStartGate()
        assertTrue(gate.shouldPrime(0, 1440, 3844, 0))
        // 门槛不是 chunk 的整数倍；最后 4 帧也必须写，不能停在 3840。
        assertTrue(gate.shouldPrime(0, 3840, 3844, 1_000_000))
        assertFalse(gate.shouldPrime(0, 3844, 3844, 2_000_000))
        assertFalse(gate.shouldPrime(480, 3364, 3844, 10_000_000))
        assertFalse(gate.shouldPrime(960, 2884, 3844, 20_000_000))
    }

    @Test
    fun stalledPlaybackCanReachRestartThresholdAgain() {
        val gate = PlaybackStartGate()
        assertFalse(gate.shouldPrime(480, 0, 3844, 0))
        assertFalse(gate.shouldPrime(480, 1440, 3844, 99_000_000))
        assertTrue(gate.shouldPrime(480, 1440, 3844, 100_000_000))
        assertFalse(gate.shouldPrime(960, 960, 3844, 110_000_000))
    }

    @Test
    fun modernSmallThresholdDoesNotRequireFillingDeviceCapacity() {
        val gate = PlaybackStartGate()
        assertTrue(gate.shouldPrime(0, 0, 480, 0))
        assertFalse(gate.shouldPrime(0, 480, 480, 1_000_000))
    }
}
