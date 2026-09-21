package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import com.gotkicry.audiolink.ui.components.FluentTextField
import androidx.compose.ui.Alignment
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.Spacer
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.service.SendGate
import com.gotkicry.audiolink.service.SenderStateMapper
import com.gotkicry.audiolink.service.SenderUiState
import com.gotkicry.audiolink.ui.components.AlButton
import com.gotkicry.audiolink.ui.components.AlTextButton
import com.gotkicry.audiolink.ui.components.AppIcons
import com.gotkicry.audiolink.ui.components.ExpandablePanel
import com.gotkicry.audiolink.ui.components.LampTone
import com.gotkicry.audiolink.ui.components.PanelCard
import com.gotkicry.audiolink.ui.components.StatusBadge
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import com.gotkicry.audiolink.ui.i18n.connectEntryGateHints
import com.gotkicry.audiolink.ui.i18n.connectPathError
import com.gotkicry.audiolink.ui.i18n.connectPathNote

/** 同一个表单按连接 / 配对两步呈现，保持服务侧门禁为唯一判据。 */
@Composable
fun ReceiverEntryCard(
    sender: SenderUiState,
    onConnect: (String) -> Unit,
    onSubmitPin: (String) -> Unit,
    modifier: Modifier = Modifier,
    compact: Boolean = false,
    connectedIds: List<String> = emptyList(),
) {
    val strings = LocalStrings.current
    var expanded by rememberSaveable { mutableStateOf(false) }
    var addr by rememberSaveable { mutableStateOf(sender.targetAddr) }
    var pin by rememberSaveable(sender.targetAddr, sender.awaitingPin) { mutableStateOf("") }
    var editAddress by rememberSaveable(sender.awaitingPin) { mutableStateOf(false) }
    val focus = LocalFocusManager.current
    LaunchedEffect(sender.targetAddr) {
        if (sender.targetAddr.isNotEmpty()) addr = sender.targetAddr
    }
    val connectGate = SenderStateMapper.canConnect(sender, addr)
    val pairing = sender.awaitingPin && !editAddress
    val connect: () -> Unit = {
        if (connectGate == SendGate.Allowed) {
            focus.clearFocus()
            editAddress = false
            onConnect(addr.trim())
        }
    }
    val submitPin: () -> Unit = {
        if (pin.length == 6 && !sender.connecting) {
            focus.clearFocus()
            onSubmitPin(pin)
        }
    }
    val form: @Composable () -> Unit = {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(
                if (pairing) strings.senderAwaitingPinNote else strings.receiverEntryHint,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (pairing) {
                Text(sender.targetAddr, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                FluentTextField(
                    value = pin,
                    onValueChange = { input -> pin = input.filter { it in '0'..'9' }.take(6) },
                    label = strings.pinLabel,
                    enabled = !sender.connecting,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword, imeAction = ImeAction.Done),
                    keyboardActions = KeyboardActions(onDone = { submitPin() }),
                    modifier = Modifier.fillMaxWidth(),
                )
                AlButton(
                    onClick = submitPin,
                    enabled = pin.length == 6 && !sender.connecting,
                    modifier = Modifier.align(Alignment.End).widthIn(min = 144.dp).heightIn(min = 48.dp),
                ) { Text(if (sender.connecting) strings.actionConnecting else strings.actionSubmitPin) }
                AlTextButton(onClick = { editAddress = true }, enabled = !sender.connecting) { Text(strings.changeHost) }
            } else {
                DiscoveredHosts(
                    connecting = sender.connecting,
                    connectedIds = connectedIds,
                    onConnect = { discoveredAddr ->
                        if (SenderStateMapper.canConnect(sender, discoveredAddr) == SendGate.Allowed) {
                            addr = discoveredAddr
                            focus.clearFocus()
                            editAddress = false
                            onConnect(discoveredAddr)
                        }
                    },
                )
                FluentTextField(
                    value = addr,
                    onValueChange = { addr = it },
                    label = strings.targetAddress,
                    placeholder = strings.targetAddressPlaceholder,
                    enabled = !sender.connecting,
                    isError = addr.isNotBlank() && connectGate == SendGate.AddressInvalid,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go),
                    keyboardActions = KeyboardActions(onGo = { connect() }),
                    modifier = Modifier.fillMaxWidth(),
                )
                AlButton(
                    onClick = connect,
                    enabled = connectGate == SendGate.Allowed,
                    modifier = Modifier.align(Alignment.End).widthIn(min = 144.dp).heightIn(min = 48.dp),
                ) {
                    Icon(AppIcons.Link, null, Modifier.size(18.dp))
                    Spacer(Modifier.size(8.dp))
                    Text(if (sender.connecting) strings.actionConnecting else strings.actionConnect)
                }
                // 空输入已有 label 与 placeholder，只有实际格式错误才追加说明。
                if (addr.isNotBlank() && connectGate != SendGate.Allowed && !sender.connecting) {
                    connectEntryGateHints(strings, sender, addr).firstOrNull()?.let {
                        Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                    }
                }
            }
            if (sender.connecting) LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
            if (!pairing) {
                connectPathNote(strings, sender)?.let {
                    Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            connectPathError(sender)?.let {
                StatusBadge(LampTone.Live, AppIcons.Warning, it, modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
            }
        }
    }
    // 会话存在时，新增连接收起；进行中的配对与失败信息绝不藏起来。
    if (compact && !sender.awaitingPin && !sender.connecting && connectPathError(sender) == null) {
        ExpandablePanel(
            title = strings.addHost,
            summary = null,
            expanded = expanded,
            onToggle = { expanded = !expanded },
            expandedStateText = strings.semanticsExpanded,
            collapsedStateText = strings.semanticsCollapsed,
            modifier = modifier,
        ) { form() }
    } else {
        PanelCard(modifier = modifier) {
            Text(
                if (pairing) strings.pairingEntryTitle else strings.deckReceiverEntry,
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.semantics { heading() },
            )
            form()
        }
    }
}
