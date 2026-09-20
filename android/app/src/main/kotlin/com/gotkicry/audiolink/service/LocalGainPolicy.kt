package com.gotkicry.audiolink.service

/**
 * 「本机音量」（每台设备一路的本地增益）的**纯策略**，从 AudioLinkService 里抽出来。
 *
 * 为什么要抽：这块逻辑第一版写错过一次 —— 把"读回 null（用户从没设过）"和"调用失败"
 * 混成一件事直接早退，结果**第一次点静音等于没点**（真机上抓到）。它是纯决策，
 * 就该能在 JVM 单测里穷举，而不是只能在真机上试。
 */

/** 本地增益的上限（千分点）：与内核一致，0 = 静音、1000 = 原声、2000 = 两倍。 */
internal const val LOCAL_GAIN_MAX_PERMILLE = 2_000

/** 默认增益（千分点）：用户没设过时等价 1.0。 */
internal const val DEFAULT_GAIN_PERMILLE = 1_000

/**
 * 点一次静音开关之后，这一路该设成多少。
 *
 * **三态**是这里的关键（第一版把它们混成两态，于是静音失效）：
 * * [current] = `null`：用户从没设过 → 当作 1.0，"静音"就该是 0；
 * * [current] = 0：已经在静音 → 回到 [remembered]（上一次的非零值），没记过就回 [fallback]；
 * * 其余：正常值 → 置 0（并把当前值记进"回到多少"）。
 *
 * [remembered] 由壳侧维护：内核不记"取消静音回到多少"（冻结语义：本地静音 = 增益 0）。
 */
internal fun nextGainAfterMuteToggle(
    current: Int?,
    remembered: Int?,
    fallback: Int = DEFAULT_GAIN_PERMILLE,
): Int = if (current == 0) (remembered ?: fallback) else 0
