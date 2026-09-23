package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.foundation.layout.size
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.service.PeerUi
import com.gotkicry.audiolink.ui.components.AlTextButton
import com.gotkicry.audiolink.ui.components.AudioLinkSlider
import com.gotkicry.audiolink.ui.components.AppIcons
import com.gotkicry.audiolink.ui.components.DeckHeader
import com.gotkicry.audiolink.ui.components.KeyValueRow
import com.gotkicry.audiolink.ui.components.LampTone
import com.gotkicry.audiolink.ui.components.PanelCard
import com.gotkicry.audiolink.ui.components.StatusBadge
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import com.gotkicry.audiolink.ui.i18n.gainLabel
import com.gotkicry.audiolink.ui.i18n.isPeerStateDegraded
import com.gotkicry.audiolink.ui.i18n.isPeerStateFailed
import com.gotkicry.audiolink.ui.i18n.peerStateText
import com.gotkicry.audiolink.ui.theme.AudioLinkType
import kotlin.math.roundToInt

/** 本机增益的取值范围（千分点）：与内核一致，0 = 静音、1000 = 原声、2000 = 两倍。 */
private const val GAIN_MAX_PERMILLE = 2_000

/**
 * 设备通道 —— 对端列表 **+ 每台的独立控制**（FR-12：每路可独立调音量/静音）。
 *
 * 每条仍然是**三通道状态**（灯 + 图标 + 文字），并且**同时**显示自报名与短指纹：
 * 名字是对端自己写的（内核文档明说"不参与信任判定"），光看名字判断不了"连上的是不是我要的那台机器"；
 * 短指纹来自证书 SHA-256，才是信任库里的锚点。信任一列直接读内核的信任位 ——
 * 外壳不自己推断（不因为"连上了"就显示已信任）。
 *
 * 层级刻意**不是"每行一张卡"**：行内用四段（状态 / 音量 / 字段 / 动作），行与行之间只用一条分隔线。
 * 多设备时卡片数量不随设备数增长，滚动长度也可预期。
 *
 * 控制的键一律用**完整指纹**（[PeerUi.idHex]）：内核在多会话时拒绝短码（"matches more than
 * one session; pass the full idHex"），而这张卡天生就是多设备场景。
 *
 * 三档动作的权重（产品定）：**静音**（可逆、会话还在，就在每行）→ **断开**（可逆、只断连，
 * 收在行末的次级位置）→ 撤销信任（破坏性，只在设置面板里，这里不提供）。
 *
 * @param onSetLocalGain 设置本机听到的音量（千分点）。**不走网络**，不影响对端。
 * @param onToggleMute 本地静音开关（0 ⇄ 上一次的非零值；"回到多少"由服务记）。
 * @param onDisconnect 断开这一台（只断连、保留信任）。
 */
@Composable
fun DeviceDeck(
    peers: List<PeerUi>,
    onSetLocalGain: (String, Int) -> Unit,
    onToggleMute: (String) -> Unit,
    onDisconnect: (String) -> Unit,
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
            key(peer.idHex) {
                var detailsExpanded by rememberSaveable { mutableStateOf(false) }
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

                    PeerVolumeRow(
                        peer = peer,
                        onSetLocalGain = onSetLocalGain,
                        onToggleMute = onToggleMute,
                    )

                    Text(peerStateText(peer.state), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    if (detailsExpanded) {
                        KeyValueRow(strings.labelFingerprint, peer.idShort)
                        KeyValueRow(strings.labelAddress, peer.addr)
                        KeyValueRow(strings.labelState, peerStateText(peer.state))
                    }

                    // 断开放在行末的次级位置（用户认可的权重：静音每行可见，断开下沉一档）——
                    // 文案不能说成"永久踢掉"：只断这一条会话，对端若是发起方会自己重拨回来。
                    Row(modifier = Modifier.fillMaxWidth()) {
                        AlTextButton(
                            onClick = { detailsExpanded = !detailsExpanded },
                            modifier = Modifier.semantics {
                                stateDescription = if (detailsExpanded) strings.semanticsExpanded else strings.semanticsCollapsed
                            },
                        ) {
                            Text(strings.connectionDetails)
                            Icon(if (detailsExpanded) AppIcons.ChevronUp else AppIcons.ChevronDown, null, Modifier.size(16.dp))
                        }
                        Spacer(modifier = Modifier.weight(1f))
                        AlTextButton(
                            onClick = { onDisconnect(peer.idHex) },
                            // 没有完整指纹就没法按设备操作（内核拒绝空 peer id）——宁可灰着也不发一次注定失败的请求。
                            enabled = peer.idHex.isNotEmpty(),
                        ) {
                            Text(strings.actionDisconnect, style = MaterialTheme.typography.labelLarge)
                        }
                    }
                }
            }
        }
        Text(
            text = strings.devicesControlNote,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/**
 * 一行里的音量块：标签 + 当前值 + 静音开关 + 滑块。
 *
 * 值的三态（读回口径来自 FFI）：`null` = 用户没设过（显示"默认"，不打"已调整"标记）、
 * 0 = 静音、其余按千分点显示百分比。**只显示本机这一层**：主机下发的增益内核没有读回接口，
 * 拿不到就绝不编一个数出来（缺口已回报）。
 */
@Composable
private fun PeerVolumeRow(
    peer: PeerUi,
    onSetLocalGain: (String, Int) -> Unit,
    onToggleMute: (String) -> Unit,
) {
    val strings = LocalStrings.current
    val gain = peer.localGain?.toInt()
    val muted = gain == 0
    val fraction = ((gain ?: 1_000).coerceIn(0, GAIN_MAX_PERMILLE)) / GAIN_MAX_PERMILLE.toFloat()

    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = strings.peerVolumeLabel,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(modifier = Modifier.weight(1f))
        Text(
            // 三态（null = 没设过 / 0 = 已静音 / 其余 = 百分比）见 [gainLabel]：纯函数，单测穷举。
            text = gainLabel(strings, peer.localGain),
            style = AudioLinkType.readout,
            color = MaterialTheme.colorScheme.onSurface,
        )
        AlTextButton(onClick = { onToggleMute(peer.idHex) }, enabled = peer.idHex.isNotEmpty()) {
            Text(if (muted) strings.peerUnmute else strings.peerMute)
        }
    }
    AudioLinkSlider(
        value = fraction,
        onValueChange = { onSetLocalGain(peer.idHex, (it * GAIN_MAX_PERMILLE).roundToInt()) },
        enabled = peer.idHex.isNotEmpty(),
        contentDescription = "${peer.name} · ${strings.peerVolumeLabel}",
        modifier = Modifier.fillMaxWidth(),
    )
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
