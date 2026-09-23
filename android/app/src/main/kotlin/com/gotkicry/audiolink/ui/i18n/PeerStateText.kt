package com.gotkicry.audiolink.ui.i18n

import androidx.compose.runtime.Composable
import java.util.Locale

/**
 * 内核会话状态字面量 → 当前语言的标签。
 *
 * 为什么不在 UI 里直接用 `PeerUi.stateLabel`：那个标签由 [com.gotkicry.audiolink.service.PeerStateMapper]
 * 生成，是**中文常量**且有 JVM 单测钉住。语言跟随系统时英文界面会被它拖回中文，所以这里按同一口径
 * （对应 docs/03-protocol.md 的 state 字面量，未知值原样透传、不吞）重新给一份当前语言的映射。
 * 判定口径不变，只是换了语言。
 */
@Composable
fun peerStateText(state: String): String {
    val strings = LocalStrings.current
    return when (state.trim().lowercase(Locale.ROOT)) {
        "idle" -> strings.peerStateIdle
        "handshaking" -> strings.peerStateHandshaking
        "streaming" -> strings.peerStateStreaming
        "degraded" -> strings.peerStateDegraded
        "reconnecting" -> strings.peerStateReconnecting
        "failed" -> strings.peerStateFailed
        else -> state
    }
}

/** 会话状态是否属于"质量下降/重连中"这一类（灯色用 Warn）。 */
fun isPeerStateDegraded(state: String): Boolean =
    when (state.trim().lowercase(Locale.ROOT)) {
        "degraded", "reconnecting" -> true
        else -> false
    }

/** 会话状态是否已被内核判为失败（灯色用 Live/error）。 */
fun isPeerStateFailed(state: String): Boolean = state.trim().lowercase(Locale.ROOT) == "failed"
