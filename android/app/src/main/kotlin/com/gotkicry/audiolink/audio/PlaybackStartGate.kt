package com.gotkicry.audiolink.audio

/** 小队列也必须达到 AudioTrack 的起播门槛；停滞后允许重新灌到该门槛。 */
class PlaybackStartGate {
    private var lastConsumed = 0L
    private var lastAdvanceNanos: Long? = null
    private var priming = true

    fun shouldPrime(
        consumedFrames: Long,
        queuedFrames: Long,
        thresholdFrames: Int,
        nowNanos: Long,
    ): Boolean {
        val lastAdvance = lastAdvanceNanos
        if (lastAdvance == null || consumedFrames != lastConsumed) {
            lastAdvanceNanos = nowNanos
            if (consumedFrames > lastConsumed) priming = false
            lastConsumed = consumedFrames
        } else if (nowNanos - lastAdvance >= 100_000_000L) {
            priming = true
        }
        return priming && queuedFrames < thresholdFrames.coerceAtLeast(1)
    }
}
