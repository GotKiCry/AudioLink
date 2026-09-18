package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.service.PeerUi
import com.gotkicry.audiolink.ui.components.AppIcons
import com.gotkicry.audiolink.ui.components.DeckHeader
import com.gotkicry.audiolink.ui.components.KeyValueRow
import com.gotkicry.audiolink.ui.components.LampTone
import com.gotkicry.audiolink.ui.components.PanelCard
import com.gotkicry.audiolink.ui.components.StatusBadge
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import com.gotkicry.audiolink.ui.i18n.isPeerStateDegraded
import com.gotkicry.audiolink.ui.i18n.isPeerStateFailed
import com.gotkicry.audiolink.ui.i18n.peerStateText

/**
 * 设备通道 —— 对端列表。
 *
 * 每条都是**三通道状态**（灯 + 图标 + 文字），并且**同时**显示自报名与短指纹：
 * 名字是对端自己写的（内核文档明说"不参与信任判定"），光看名字判断不了"连上的是不是我要的那台机器"；
 * 短指纹来自证书 SHA-256，才是信任库里的锚点。信任一列直接读内核的信任位 ——
 * 外壳不自己推断（不因为"连上了"就显示已信任）。
 */
@Composable
fun DeviceDeck(
    peers: List<PeerUi>,
    modifier: Modifier = Modifier,
) {
    val strings = LocalStrings.current
    if (peers.isEmpty()) {
        return
    }
    PanelCard(modifier = modifier) {
        DeckHeader(
            text = strings.deckDevices,
            trailing = {
                Text(
                    text = String.format(strings.devicesCountFormat, peers.size),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            },
        )
        peers.forEachIndexed { index, peer ->
            if (index > 0) {
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            }
            Column(
                modifier = Modifier.fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                StatusBadge(
                    tone = peerTone(peer),
                    icon = peerIcon(peer),
                    text = peer.name,
                    textStyle = MaterialTheme.typography.titleSmall,
                )
                KeyValueRow(strings.labelFingerprint, peer.idShort)
                KeyValueRow(strings.labelAddress, peer.addr)
                KeyValueRow(strings.labelState, peerStateText(peer.state))
                KeyValueRow(
                    label = strings.labelTrust,
                    value = if (peer.trusted) strings.trustYes else strings.trustNo,
                )
            }
        }
    }
}

private fun peerTone(peer: PeerUi): LampTone = when {
    isPeerStateFailed(peer.state) -> LampTone.Live
    isPeerStateDegraded(peer.state) -> LampTone.Warn
    peer.state.equals("streaming", ignoreCase = true) -> LampTone.On
    else -> LampTone.Idle
}

private fun peerIcon(peer: PeerUi) = when {
    isPeerStateFailed(peer.state) -> AppIcons.Close
    isPeerStateDegraded(peer.state) -> AppIcons.Warning
    peer.state.equals("streaming", ignoreCase = true) -> AppIcons.Check
    else -> AppIcons.Info
}
