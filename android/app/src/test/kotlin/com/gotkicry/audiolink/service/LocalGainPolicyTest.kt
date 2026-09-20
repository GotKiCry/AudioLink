package com.gotkicry.audiolink.service

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * 本地增益的**纯策略**测试。
 *
 * 这四条不是凑数：第一条钉的正是真机上抓到的缺陷 —— 用户从没设过时读回 `null`，
 * 旧实现把它当成"调用失败"直接早退，于是**第一次点静音等于没点**。
 * 后面三条守住"取消静音回到多少"的记忆语义（内核不记，壳侧记）。
 */
class LocalGainPolicyTest {

    /** 从没设过（null）→ 静音就该是 0，不是"什么都不做"。 */
    @Test
    fun firstMuteFromUnsetGoesToZero() {
        assertEquals(0, nextGainAfterMuteToggle(current = null, remembered = null))
    }

    /** 已经调过音量的那一路 → 再点静音 = 0。 */
    @Test
    fun mutingAnAdjustedChannelGoesToZero() {
        assertEquals(0, nextGainAfterMuteToggle(current = 800, remembered = null))
    }

    /** 取消静音 → 回到上一次的非零值（不是回到 100%）。 */
    @Test
    fun unmuteRestoresTheRememberedValue() {
        assertEquals(800, nextGainAfterMuteToggle(current = 0, remembered = 800))
    }

    /** 没记过任何非零值 → 回到默认 100%。 */
    @Test
    fun unmuteWithoutMemoryFallsBackToDefault() {
        assertEquals(DEFAULT_GAIN_PERMILLE, nextGainAfterMuteToggle(current = 0, remembered = null))
        assertEquals(1_000, DEFAULT_GAIN_PERMILLE)
        assertEquals(2_000, LOCAL_GAIN_MAX_PERMILLE)
    }
}
