package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.capture.CaptureSourceKind
import com.gotkicry.audiolink.capture.CaptureState
import com.gotkicry.audiolink.service.SendGate
import com.gotkicry.audiolink.service.SenderStateMapper
import com.gotkicry.audiolink.service.SenderUiState
import com.gotkicry.audiolink.ui.components.AlButton
import com.gotkicry.audiolink.ui.components.AlOutlinedButton
import com.gotkicry.audiolink.ui.components.AppIcons
import com.gotkicry.audiolink.ui.components.Choice
import com.gotkicry.audiolink.ui.components.ChoiceGroup
import com.gotkicry.audiolink.ui.components.DeckHeader
import com.gotkicry.audiolink.ui.components.LampTone
import com.gotkicry.audiolink.ui.components.PanelCard
import com.gotkicry.audiolink.ui.components.SilkLabel
import com.gotkicry.audiolink.ui.components.StatusLamp
import com.gotkicry.audiolink.ui.components.StatusBadge
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import com.gotkicry.audiolink.ui.i18n.captureNoteText
import com.gotkicry.audiolink.ui.i18n.hostPathError
import com.gotkicry.audiolink.ui.i18n.hostPathNote
import com.gotkicry.audiolink.ui.i18n.peerStateText
import com.gotkicry.audiolink.ui.i18n.sendGateText

/**
 * 主机 —— 本机提供声音、被连接的一方：**采集源 + 推流控制**。
 *
 * 为什么只管这两件事（归位）：本机也可以当**接收端**（去听另一台主机），但那部分已经搬到
 * [ReceiverEntryCard] —— 填地址、输码是接收端的动作，和采集源、推流并排在一张卡里，
 * 用户会不知道"我该填还是该等"。拆开之后每张卡只讲一个角色，而且各连着对应的面板：
 * 接收端 = [ReceiverEntryCard] → [ReceiverDeck]；主机 = [HostAddressCard] → 本卡。
 *
 * 一条既有联动没有变（也不由本卡决定）：切换采集源（关闭 ⇄ 非关闭）会**重启引擎**
 * （见 CaptureWiring），因此它同样会掐断已经建立的会话 —— 无论那条会话来自哪一侧。
 *
 * 门禁一字不改：能不能推全部由 [SenderStateMapper] 判定，UI 只渲染它给的理由
 * （连"为什么按钮是灰的"都是它给的文案）。文案走 [sendGateText]（跟随系统语言），
 * 与 [peerStateText] 覆盖会话状态标签是同一套做法。
 */
@Composable
fun SenderDeck(
    sender: SenderUiState,
    captureSelection: CaptureSourceKind?,
    captureState: CaptureState,
    engineRunning: Boolean,
    serviceRunning: Boolean,
    /** 主机角色的服务开关：起服务 = 等对方接入（接收端路径不需要它）。 */
    onToggleService: (Boolean) -> Unit,
    onSelectCapture: (CaptureSourceKind?) -> Unit,
    onStartSend: () -> Unit,
    onStopSend: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val strings = LocalStrings.current

    val startGate = SenderStateMapper.canStartSend(sender, engineRunning, captureSelection, captureState)
    val stopGate = SenderStateMapper.canStopSend(sender, engineRunning)

    val tone = when {
        sender.error != null -> LampTone.Live
        sender.sending -> LampTone.On
        sender.hasSession -> LampTone.Warn
        else -> LampTone.Idle
    }
    val sessionText = when {
        !sender.hasSession -> strings.senderSessionDisconnected
        sender.sending -> strings.senderSessionSending
        else -> strings.senderSessionConnected
    }
    val sessionDetail = sender.peerState.takeIf { it.isNotEmpty() }?.let { peerStateText(it) }

    PanelCard(modifier = modifier) {
        DeckHeader(
            text = strings.deckSender,
            trailing = {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    StatusLamp(
                        tone = if (serviceRunning) LampTone.On else LampTone.Idle,
                        diameter = 8.dp,
                    )
                    Text(
                        text = if (serviceRunning) strings.serviceRunning else strings.serviceStopped,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            },
        )
        // 主机角色的第一步：让服务跑起来等对方接入。**这不叫"开始接收"** —— 接收端那边的
        // 连接动作会自己把服务拉起来，用户在这里点的是一件只有主机才需要的事。
        AlOutlinedButton(
            onClick = { onToggleService(!serviceRunning) },
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 48.dp),
        ) {
            Text(
                text = if (serviceRunning) strings.actionStopWaiting else strings.actionWaitForDevice,
                style = MaterialTheme.typography.titleMedium,
            )
        }
        Text(
            text = strings.hostServiceHint,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        if (sender.hasSession || sender.error != null) {
            StatusBadge(
                tone = tone,
                icon = when (tone) {
                    LampTone.On -> AppIcons.Check
                    LampTone.Live, LampTone.Warn -> AppIcons.Warning
                    LampTone.Idle -> AppIcons.Info
                },
                text = sessionText,
                modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
            )
            if (sessionDetail != null) {
                Text(
                    text = sessionDetail,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

        }
        SilkLabel(strings.senderCaptureLabel)
        ChoiceGroup(
            options = listOf(
                Choice<CaptureSourceKind?>(null, strings.captureOff),
                Choice<CaptureSourceKind?>(CaptureSourceKind.Microphone, strings.captureMic),
                Choice<CaptureSourceKind?>(CaptureSourceKind.SystemLoopback, strings.captureLoopback),
            ),
            selected = captureSelection,
            onSelect = onSelectCapture,
        )
        captureNoteText(strings, captureSelection, captureState)?.let { hint ->
            Text(
                text = hint,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Text(
            text = strings.captureHint,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        if (!sender.sending) {
            AlButton(
                onClick = onStartSend,
                enabled = startGate == SendGate.Allowed,
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 48.dp),
            ) {
                Text(strings.actionStartSend, style = MaterialTheme.typography.titleSmall)
            }
        } else {
            AlOutlinedButton(
                onClick = onStopSend,
                enabled = stopGate == SendGate.Allowed,
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 48.dp),
            ) {
                Text(strings.actionStopSend, style = MaterialTheme.typography.titleSmall)
            }
        }
        if (!sender.sending && startGate != SendGate.Allowed) {
            sendGateText(strings, startGate)?.let { hint ->
                Text(
                    text = hint,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        // 主机路径的提示与错误：输码、断开那两类已经归了接收端入口卡（见 [hostPathNote] /
        // [hostPathError]），这里只说推流这一侧的事 —— 两张卡各管一段，同一句话不会出现两遍，
        // 也不会两边都不出现。
        hostPathNote(sender)?.let { hint ->
            Text(text = hint, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary)
        }
        hostPathError(sender)?.let { message ->
            StatusBadge(
                tone = LampTone.Live,
                icon = AppIcons.Warning,
                text = message,
                modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
            )
        }
        Text(
            text = strings.senderHint,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        // 自动行为的边界必须写在用户看得见的地方：他得知道"为什么会自己开始"，
        // 以及"怎么让它别再自己开始"（停一次，服务端就会把那台设备记成拒绝）。
        Text(
            text = strings.senderAutoSendHint,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
