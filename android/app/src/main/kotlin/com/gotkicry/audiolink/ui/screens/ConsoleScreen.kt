package com.gotkicry.audiolink.ui.screens

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.capture.CaptureSourceKind
import com.gotkicry.audiolink.service.AudioLinkService
import com.gotkicry.audiolink.service.PlaybackUiState
import com.gotkicry.audiolink.ui.components.AlTextButton
import com.gotkicry.audiolink.ui.components.AppIcons
import com.gotkicry.audiolink.ui.components.ConsoleContent
import com.gotkicry.audiolink.ui.components.ConsoleRoleSwitch
import com.gotkicry.audiolink.ui.components.ConsolePageHeading
import com.gotkicry.audiolink.ui.components.ConnectionGuide
import com.gotkicry.audiolink.ui.components.LampTone
import com.gotkicry.audiolink.ui.components.NoticePanel
import com.gotkicry.audiolink.ui.i18n.LocalStrings

/** 单页控制台：连接和播放优先，发送是同一页中的独立视图。 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConsoleScreen(
    state: PlaybackUiState,
    onTogglePlayback: (Boolean) -> Unit,
    onConnect: (String) -> Unit,
    onStartSend: () -> Unit,
    onStopSend: () -> Unit,
    onSelectCapture: (CaptureSourceKind?) -> Unit,
    onOpenSettings: () -> Unit,
    scrollState: ScrollState,
    modifier: Modifier = Modifier,
) {
    val strings = LocalStrings.current
    val context = LocalContext.current
    var sendingView by rememberSaveable { mutableStateOf(false) }
    val senderScroll = rememberScrollState()
    val viewState = rememberSaveableStateHolder()
    BackHandler(enabled = sendingView) { sendingView = false }

    Scaffold(
        modifier = modifier,
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                title = {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Icon(AppIcons.Link, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary)
                        Text("AudioLink", style = MaterialTheme.typography.titleMedium)
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
                actions = {
                    AlTextButton(onClick = onOpenSettings) {
                        Icon(AppIcons.Settings, null, Modifier.size(20.dp))
                        Spacer(Modifier.size(8.dp))
                        Text(strings.settingsTitle)
                    }
                },
            )
        },
    ) { padding ->
        ConsoleContent(padding, if (sendingView) senderScroll else scrollState) {
            ConsoleRoleSwitch(sending = sendingView, onSelect = { sendingView = it })
            ConsolePageHeading(
                title = if (sendingView) strings.sendTab else strings.receiveTab,
                description = if (sendingView) strings.sendIntro else strings.receiveIntro,
            )

            if (state.lastError != null || state.engineError != null) {
                NoticePanel(strings.errorTitle, strings.errorRecoveryPlayback, AppIcons.Warning, LampTone.Live)
                AlTextButton(onClick = onOpenSettings) { Text(strings.showDiagnostics) }
            }
            if (state.peersNote != null) {
                NoticePanel(strings.errorPeersReadFailed, strings.errorRecoveryGeneric, AppIcons.Warning, LampTone.Warn)
            }

            viewState.SaveableStateProvider(sendingView) {
                Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    if (sendingView) {
                        HostAddressCard(listenAddr = state.engineAddr)
                        SenderDeck(
                            sender = state.sender,
                            captureSelection = state.captureSelection,
                            captureState = state.captureState,
                            engineRunning = state.engineRunning,
                            serviceRunning = state.serviceRunning,
                            onToggleService = onTogglePlayback,
                            onSelectCapture = onSelectCapture,
                            onStartSend = onStartSend,
                            onStopSend = onStopSend,
                        )
                    } else {
                        val connected = state.peers.isNotEmpty()
                        if (connected) {
                            ReceiverDeck(state = state, onDisconnect = { onTogglePlayback(false) })
                        }
                        ReceiverEntryCard(
                            sender = state.sender,
                            onConnect = onConnect,
                            compact = connected,
                            connectedIds = state.peers.map { it.idShort },
                        )
                        if (!connected && !state.sender.connecting) {
                            ConnectionGuide()
                        }
                    }
                    DeviceDeck(
                        peers = state.peers,
                        onSetLocalGain = { id, gain -> AudioLinkService.setLocalGain(context, id, gain) },
                        onToggleMute = { id -> AudioLinkService.toggleMute(context, id) },
                        onDisconnect = { id -> AudioLinkService.disconnectPeer(context, id) },
                    )
                }
            }
            Spacer(Modifier.height(8.dp))
        }
    }
}
