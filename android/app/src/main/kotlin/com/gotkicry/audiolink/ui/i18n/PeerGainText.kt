package com.gotkicry.audiolink.ui.i18n

import com.gotkicry.audiolink.service.PeerUi
import kotlin.math.roundToInt

/** 本地增益的满值（千分点）：与内核一致，0 = 静音、1000 = 原声、2000 = 两倍。 */
private const val GAIN_FULL_PERMILLE = 1_000

/**
 * 某一路音量的显示口径 —— **三态**，每一态对用户的含义都不同：
 *
 * * `null` = 用户从没设过（FFI 的 `localPeerGain` 用 null 与"设成 1000"区分这两件事）→「默认」；
 * * `0` →「已静音」；
 * * 其余 → 百分比。
 *
 * 抽成纯函数是为了能被单测穷举：多设备场景下**每行都要独立算**，
 * 一旦把"null 当 0"，用户会看到某台明明没静音却写着已静音。
 */
internal fun gainLabel(strings: UiStrings, gain: UInt?): String = when {
    gain == null -> strings.peerVolumeDefault
    gain == 0u -> strings.peerMuted
    else -> "${(gain.toInt() * 100f / GAIN_FULL_PERMILLE).roundToInt()}%"
}

/**
 * 接收端「来源」读数：**零台说未连接、一台说名字、多台说台数 + 首台**。
 *
 * 为什么值得一条测试：多设备是 FR-12 的核心场景，而旧实现写死了单数
 * （`source?.name ?: receiverSourceNone`）—— 多台同时接入时会**少报**：
 * 用户看到的只有一台，实际两台都在放。宁可说"2 台设备（A）"也不要漏。
 *
 * 名字缺失时回落到短指纹（与 PairingStateMapper.displayName 同口径）。
 */
internal fun receiverSourceText(strings: UiStrings, peers: List<PeerUi>): String = when {
    peers.isEmpty() -> strings.receiverSourceNone
    peers.size == 1 -> peers.first().name.ifEmpty { peers.first().idShort }
    else -> String.format(
        java.util.Locale.US,
        strings.receiverSourceManyFormat,
        peers.size,
        peers.first().name.ifEmpty { peers.first().idShort },
    )
}
