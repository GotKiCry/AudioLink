package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.capture.CaptureSourceKind
import com.gotkicry.audiolink.capture.CaptureState
import com.gotkicry.audiolink.service.SendGate
import com.gotkicry.audiolink.service.SenderStateMapper
import com.gotkicry.audiolink.service.SenderUiState
import com.gotkicry.audiolink.ui.components.AppIcons
import com.gotkicry.audiolink.ui.components.Choice
import com.gotkicry.audiolink.ui.components.ChoiceGroup
import com.gotkicry.audiolink.ui.components.DeckHeader
import com.gotkicry.audiolink.ui.components.LampTone
import com.gotkicry.audiolink.ui.components.PanelCard
import com.gotkicry.audiolink.ui.components.SilkLabel
import com.gotkicry.audiolink.ui.components.StatusBadge
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import com.gotkicry.audiolink.ui.i18n.peerStateText

/**
 * 发送台 —— 原有的「发送源」与「发送」两张卡合并成一张。
 *
 * 为什么合并：这两件事在用户心里是**一件事**（"我要把手机的声音给电脑"），拆成两张卡时
 * 中间隔着一张服务卡，用户按「先连接、后选源」的自然顺序操作会把刚建立的连接掐断
 * （切换 关闭 ⇄ 非关闭 要重启引擎，见 CaptureWiring）。合成一张后顺序在**卡内**就写死了：
 * 先选源 → 再连接 → 再推流。
 *
 * 门禁一字不改：能不能连、能不能发全部由 [SenderStateMapper] 判定，UI 只渲染它给的理由
 * （连"为什么按钮是灰的"都是它给的文案）。
 */
@Composable
fun SenderDeck(
    sender: SenderUiState,
    captureSelection: CaptureSourceKind?,
    captureState: CaptureState,
    engineRunning: Boolean,
    onSelectCapture: (CaptureSourceKind?) -> Unit,
    onConnect: (String) -> Unit,
    onSubmitPin: (String) -> Unit,
    onStartSend: () -> Unit,
    onStopSend: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val strings = LocalStrings.current
    var addr by remember { mutableStateOf(sender.targetAddr) }
    var pin by remember { mutableStateOf("") }

    // 服务侧规范化后的地址回填到输入框（用户只填 IP 时，服务会补上默认端口）。
    LaunchedEffect(sender.targetAddr) {
        if (sender.targetAddr.isNotEmpty()) addr = sender.targetAddr
    }

    // 门禁读**输入框**（addr），不是服务侧的回填副本 —— 否则按钮会死在"地址没进服务"这个死锁里。
    val connectGate = SenderStateMapper.canConnect(sender, engineRunning, addr)
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
        DeckHeader(text = strings.deckSender)

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
        SenderStateMapper.captureNote(captureSelection, captureState)?.let { hint ->
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

        OutlinedTextField(
            value = addr,
            onValueChange = { addr = it },
            label = { Text(strings.targetAddress) },
            placeholder = { Text(strings.targetAddressPlaceholder) },
            singleLine = true,
            enabled = !sender.connecting,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
            modifier = Modifier.fillMaxWidth(),
        )
        SenderStateMapper.gateNote(SenderStateMapper.addressGate(addr))?.let { hint ->
            Text(
                text = hint,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        Button(
            onClick = { onConnect(addr) },
            enabled = connectGate == SendGate.Allowed,
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 48.dp),
        ) {
            Text(
                text = if (sender.connecting) strings.actionConnecting else strings.actionConnect,
                style = MaterialTheme.typography.titleMedium,
            )
        }
        // 连接中：文案变化之外再给一条细进度条（M3 标准），让"正在做事"不必靠读字。
        if (sender.connecting) {
            LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
        }
        if (connectGate != SendGate.Allowed) {
            SenderStateMapper.gateNote(connectGate)?.let { hint ->
                Text(
                    text = hint,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        // 需要本机输码时才出现输入框（connect 返回 1002 之后）—— 不是错误态，是流程中的一步。
        if (sender.awaitingPin) {
            OutlinedTextField(
                value = pin,
                onValueChange = { input -> pin = input.filter { it.isDigit() }.take(6) },
                label = { Text(strings.pinLabel) },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
                modifier = Modifier.fillMaxWidth(),
            )
            Button(
                onClick = { onSubmitPin(pin) },
                enabled = pin.length == 6,
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 48.dp),
            ) {
                Text(strings.actionSubmitPin, style = MaterialTheme.typography.titleMedium)
            }
        }

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Button(
                onClick = onStartSend,
                enabled = startGate == SendGate.Allowed,
                modifier = Modifier
                    .weight(1f)
                    .heightIn(min = 48.dp),
            ) {
                Text(strings.actionStartSend, style = MaterialTheme.typography.titleSmall)
            }
            OutlinedButton(
                onClick = onStopSend,
                enabled = stopGate == SendGate.Allowed,
                modifier = Modifier
                    .weight(1f)
                    .heightIn(min = 48.dp),
            ) {
                Text(strings.actionStopSend, style = MaterialTheme.typography.titleSmall)
            }
        }
        if (startGate != SendGate.Allowed) {
            SenderStateMapper.gateNote(startGate)?.let { hint ->
                Text(
                    text = hint,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        sender.note?.let { hint ->
            Text(text = hint, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary)
        }
        sender.error?.let { message ->
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
    }
}
