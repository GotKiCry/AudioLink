package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.service.PeerUi
import com.gotkicry.audiolink.service.PlaybackUiState
import com.gotkicry.audiolink.ui.components.AlOutlinedButton
import com.gotkicry.audiolink.ui.components.DeckHeader
import com.gotkicry.audiolink.ui.components.LampTone
import com.gotkicry.audiolink.ui.components.PanelCard
import com.gotkicry.audiolink.ui.components.Readout
import com.gotkicry.audiolink.ui.components.ReadoutFlow
import com.gotkicry.audiolink.ui.components.StatusBadge
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import com.gotkicry.audiolink.ui.i18n.isPeerStateDegraded
import com.gotkicry.audiolink.ui.i18n.isPeerStateFailed
import com.gotkicry.audiolink.ui.i18n.receiverSourceText
import com.gotkicry.audiolink.ui.components.AppIcons
import com.gotkicry.audiolink.ui.theme.AudioLinkType
import java.util.Locale

/**
 * 接收端 —— 首屏的绝对主角：手机端两件事（开始接收 / 停止接收）都在这里。
 *
 * **音量不在这张卡**：总响度归系统音量键（本机播放走 `USAGE_MEDIA`，本来就被媒体流音量控制），
 * App 只在设备通道里按主机调**每一路**的本地增益 —— 一张卡只讲一件事，不做第二个总音量。
 *
 * 信息层级（从上到下就是"一瞥"的顺序）：
 * 1. **状态灯 + 状态文字**：余光能捕捉的那一层，只有它在呼吸；
 * 2. 来源设备名：回答"在听谁"；
 * 3. 延迟 / 码率 / 丢包：回答"质量如何"，等宽数字，不跳；
 * 4. 主按钮：整卡唯一的主色块，一键可达。
 *
 * 状态文字挂了 `liveRegion`：状态变化时 TalkBack 会主动播报，不必靠用户反复划动去找。
 */
@Composable
fun ReceiverDeck(
    state: PlaybackUiState,
    /** 断开当前会话（结束会话 + 停止播放）。"开始接收"这个动作已经不存在 —— 连接即接收。 */
    onDisconnect: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val strings = LocalStrings.current
    val source = primaryPeer(state.peers)
    val degraded = state.peers.any { isPeerStateDegraded(it.state) }
    val failed = state.peers.any { isPeerStateFailed(it.state) }
    val connected = state.peers.isNotEmpty()
    // 「接收中」= **真的有音频在收**：内核把这条会话报成 streaming 才算，而"连着一条会话"不算 ——
    // 握手阶段的会话也能让大字说"接收中"，那是假话。
    val streamingPeers = state.peers.filter { it.state.equals("streaming", ignoreCase = true) }
    val receiving = state.playing && streamingPeers.isNotEmpty()

    val tone = when {
        state.lastError != null || failed -> LampTone.Live
        receiving && degraded -> LampTone.Warn
        receiving -> LampTone.On
        connected -> LampTone.Warn
        else -> LampTone.Idle
    }
    val statusText = when {
        state.lastError != null || failed -> strings.stateFailed
        receiving && degraded -> strings.stateDegraded
        // 多路时如实报数量（FR-12：接收端可以同时收多台）。
        receiving && streamingPeers.size > 1 ->
            String.format(Locale.US, strings.stateReceivingManyFormat, streamingPeers.size)
        receiving -> strings.stateReceiving
        // 连上了但对端还没推流：说"接收中"是假的（此刻没有声音），说"未在接收"又会让人以为连接断了。
        connected -> strings.stateConnected
        else -> strings.stateNotReceiving
    }
    val statusIcon = when (tone) {
        LampTone.Live, LampTone.Warn -> AppIcons.Warning
        LampTone.On -> AppIcons.Check
        LampTone.Idle -> AppIcons.Info
    }

    PanelCard(modifier = modifier) {
        // 服务状态与它的开关都搬到主机卡了：服务存在的意义是"等对方接入"（主机角色）。
        // 接收端这边连接动作会自动拉起服务，用户不需要知道它，也不该在这里再看到一盏"服务已停止"。
        DeckHeader(text = strings.deckReceiver)

        StatusBadge(
            tone = tone,
            icon = statusIcon,
            text = statusText,
            breathing = receiving && !degraded,
            textStyle = AudioLinkType.stateHeadline,
            modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
        )

        Readout(
            label = strings.receiverSource,
            // 多台时把数量说出来：单数来源会在多路场景下撒谎（用户原话："应该按照设备停止"）。
            // 口径与理由见 [receiverSourceText]（纯函数，单测穷举零/一/多台）。
            value = receiverSourceText(strings, state.peers),
            valueStyle = MaterialTheme.typography.bodyMedium,
            valueColor = if (!connected) {
                MaterialTheme.colorScheme.onSurfaceVariant
            } else {
                MaterialTheme.colorScheme.onSurface
            },
        )

        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        ReadoutFlow {
            Readout(strings.receiverLatency, latencyText(source, strings.readoutUnknown))
            Readout(strings.receiverBitrate, bitrateText(source, strings.readoutUnknown))
            Readout(strings.receiverLoss, lossText(source, strings.readoutUnknown))
        }

        // 连接建立**就是**开始接收（连接动作会自动拉起服务并启动播放），所以这张卡不再有"开始"按钮 ——
        // 那一个曾经和卡 1 的连接重复，用户会问"为什么连上了还要再点一次"。
        // 已连接时只提供**断开**：结束会话 + 停止播放。未连接时不出现按钮，路径由卡 1 承担。
        if (connected) {
            AlOutlinedButton(
                onClick = onDisconnect,
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 48.dp),
            ) {
                Icon(AppIcons.Stop, null, Modifier.size(16.dp))
                Spacer(Modifier.size(8.dp))
                Text(if (state.peers.size > 1) strings.stopAll else strings.actionDisconnect, style = MaterialTheme.typography.titleMedium)
            }
        }

        Text(
            text = if (receiving && degraded) strings.receiverDegradedHint else strings.receiverHint,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** 正在推流的那台（没有就退回第一台）—— 接收端面板只讲**一个**来源，不摊开整张会话表。 */
private fun primaryPeer(peers: List<PeerUi>): PeerUi? =
    peers.firstOrNull { it.state.equals("streaming", true) } ?: peers.firstOrNull()

/** 网络往返延迟（µs → ms）；未采样时显示占位。 */
private fun latencyText(peer: PeerUi?, unknown: String): String {
    val us = peer?.rttUs ?: 0L
    return if (us <= 0L) unknown else String.format(Locale.US, "%.1f ms", us / 1000.0)
}

/** 码率读数（bps → Mbps/kbps）。 */
private fun bitrateText(peer: PeerUi?, unknown: String): String {
    val bps = peer?.bitrateBps ?: 0L
    if (bps <= 0L) return unknown
    return if (bps >= 1_000_000L) {
        String.format(Locale.US, "%.2f Mbps", bps / 1_000_000.0)
    } else {
        "${bps / 1000} kbps"
    }
}

/** 丢包读数。0.0 % 是**有效**读数（真的没丢），与"没有遥测"要分开。 */
private fun lossText(peer: PeerUi?, unknown: String): String {
    if (peer == null || peer.bitrateBps <= 0L ||
        (!peer.state.equals("streaming", true) && !peer.state.equals("degraded", true))) return unknown
    return String.format(Locale.US, "%.2f %%", peer.lossPct)
}
