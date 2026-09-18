package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.service.PeerUi
import com.gotkicry.audiolink.service.PlaybackUiState
import com.gotkicry.audiolink.ui.components.DeckHeader
import com.gotkicry.audiolink.ui.components.LampTone
import com.gotkicry.audiolink.ui.components.MediaVolumeController
import com.gotkicry.audiolink.ui.components.PanelCard
import com.gotkicry.audiolink.ui.components.Readout
import com.gotkicry.audiolink.ui.components.ReadoutFlow
import com.gotkicry.audiolink.ui.components.SilkLabel
import com.gotkicry.audiolink.ui.components.StatusBadge
import com.gotkicry.audiolink.ui.components.StatusLamp
import com.gotkicry.audiolink.ui.components.VolumeFader
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import com.gotkicry.audiolink.ui.i18n.isPeerStateDegraded
import com.gotkicry.audiolink.ui.i18n.isPeerStateFailed
import com.gotkicry.audiolink.ui.components.AppIcons
import com.gotkicry.audiolink.ui.theme.AudioLinkType
import com.gotkicry.audiolink.ui.theme.statusColors
import java.util.Locale

/**
 * 接收台 —— 首屏的绝对主角，手机端三件事里的两件（开始接收 / 停止接收）加第三件（音量）都在这里。
 *
 * 信息层级（从上到下就是"一瞥"的顺序）：
 * 1. **状态灯 + 状态文字**：余光能捕捉的那一层，只有它在呼吸；
 * 2. 来源设备名：回答"在听谁"；
 * 3. 延迟 / 码率 / 丢包：回答"质量如何"，等宽数字，不跳；
 * 4. **大号音量推子**：唯一的连续控件，触控目标 ≥48 dp；
 * 5. 主按钮：整卡唯一的主色块，一键可达。
 *
 * 状态文字挂了 `liveRegion`：状态变化时 TalkBack 会主动播报，不必靠用户反复划动去找。
 */
@Composable
fun ReceiverDeck(
    state: PlaybackUiState,
    volume: MediaVolumeController,
    onTogglePlayback: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val strings = LocalStrings.current
    val source = primaryPeer(state.peers)
    val degraded = state.peers.any { isPeerStateDegraded(it.state) }
    val failed = state.peers.any { isPeerStateFailed(it.state) }
    val receiving = state.playing && source != null

    val tone = when {
        state.lastError != null || failed -> LampTone.Live
        receiving && degraded -> LampTone.Warn
        receiving -> LampTone.On
        else -> LampTone.Idle
    }
    val statusText = when {
        state.lastError != null -> strings.stateFailed
        receiving && degraded -> strings.stateDegraded
        receiving -> strings.stateReceiving
        // 大字只说「接收/未在接收」：服务运行与否已经在右上角那行小字里如实给出，
        // 同一件事说两遍在大字体（1.3x）下会把状态行挤成两行。
        else -> strings.stateNotReceiving
    }
    val statusIcon = when (tone) {
        LampTone.Live, LampTone.Warn -> AppIcons.Warning
        LampTone.On -> AppIcons.Check
        LampTone.Idle -> AppIcons.Info
    }

    PanelCard(modifier = modifier) {
        DeckHeader(
            text = strings.deckReceiver,
            trailing = {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    StatusLamp(
                        tone = if (state.serviceRunning) LampTone.On else LampTone.Idle,
                        diameter = 8.dp,
                    )
                    Text(
                        text = if (state.serviceRunning) strings.serviceRunning else strings.serviceStopped,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            },
        )

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
            value = source?.name ?: strings.receiverSourceNone,
            valueColor = if (source == null) {
                MaterialTheme.colorScheme.onSurfaceVariant
            } else {
                MaterialTheme.colorScheme.onSurface
            },
        )

        ReadoutFlow {
            Readout(strings.receiverLatency, latencyText(source, strings.readoutUnknown))
            Readout(strings.receiverBitrate, bitrateText(source, strings.readoutUnknown))
            Readout(strings.receiverLoss, lossText(source, strings.readoutUnknown))
        }

        Spacer(modifier = Modifier.size(4.dp))

        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            SilkLabel(strings.volume)
            Spacer(modifier = Modifier.weight(1f))
            Text(
                text = "${volume.percent}%",
                style = AudioLinkType.readout,
                color = MaterialTheme.colorScheme.onSurface,
            )
        }
        VolumeFader(controller = volume, contentDescription = strings.volume)

        Button(
            onClick = { onTogglePlayback(!state.serviceRunning) },
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 56.dp),
        ) {
            Icon(
                imageVector = if (state.serviceRunning) AppIcons.Stop else AppIcons.Play,
                contentDescription = null, // decorative：按钮文字已经说清动作
                modifier = Modifier.size(20.dp),
            )
            Spacer(modifier = Modifier.size(8.dp))
            Text(
                text = if (state.serviceRunning) strings.actionStopReceiving else strings.actionStartReceiving,
                style = MaterialTheme.typography.titleMedium,
            )
        }

        Text(
            text = if (receiving && degraded) strings.receiverDegradedHint else strings.receiverHint,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** 正在推流的那台（没有就退回第一台）—— 接收台只讲**一个**来源，不摊开整张会话表。 */
private fun primaryPeer(peers: List<PeerUi>): PeerUi? =
    peers.firstOrNull { it.state.equals("streaming", true) } ?: peers.firstOrNull()

/** 端到端延迟读数（µs → ms）。没有遥测就是"—"，不写 0 ms 骗人。 */
private fun latencyText(peer: PeerUi?, unknown: String): String {
    val us = peer?.e2eLatencyUs ?: 0L
    return if (us <= 0L) unknown else "${us / 1000} ms"
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
    if (peer == null) return unknown
    return String.format(Locale.US, "%.1f %%", peer.lossPct)
}
