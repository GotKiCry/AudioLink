package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.gotkicry.audiolink.service.LanDiscoveryState
import com.gotkicry.audiolink.service.discoverLanHosts
import com.gotkicry.audiolink.ui.components.AlTextButton
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.buffer

@OptIn(ExperimentalCoroutinesApi::class)
@Composable
fun DiscoveredHosts(
    connecting: Boolean,
    connectedIds: List<String>,
    onConnect: (String) -> Unit,
) {
    val strings = LocalStrings.current
    val context = LocalContext.current.applicationContext
    val refreshes = remember { MutableStateFlow(0) }
    // flatMapLatest waits for the previous scan to release its socket and Wi-Fi lock.
    val discovery = remember(context) { refreshes.flatMapLatest { discoverLanHosts(context) }.buffer(0) }
    val state by discovery.collectAsStateWithLifecycle(initialValue = LanDiscoveryState())
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(strings.discoveryTitle, style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f).semantics { heading() })
            AlTextButton(
                onClick = { refreshes.value += 1 },
                enabled = !state.scanning && !connecting,
                modifier = Modifier.heightIn(min = 48.dp),
            ) { Text(if (state.scanning) strings.discoveryRefreshing else strings.discoveryRefresh) }
        }
        Text(
            when {
                state.failed -> strings.discoveryError
                state.scanning -> strings.discoverySearching
                state.hosts.isEmpty() -> strings.discoveryEmpty
                else -> strings.discoveryHint
            },
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        state.hosts.forEach { host ->
            val connected = host.id in connectedIds
            AlTextButton(
                onClick = { onConnect(host.addr) },
                enabled = !connecting && !connected && host.compatible,
                modifier = Modifier.fillMaxWidth().heightIn(min = 64.dp),
            ) {
                Column(Modifier.weight(1f), horizontalAlignment = Alignment.Start) {
                    Text(host.name, maxLines = 2, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Start)
                    Text(host.addr, style = MaterialTheme.typography.bodySmall)
                    if (!host.compatible) Text(strings.discoveryIncompatible, style = MaterialTheme.typography.bodySmall, textAlign = TextAlign.Start)
                }
                Text(if (connected) strings.stateConnected else strings.actionConnect, style = MaterialTheme.typography.labelMedium)
            }
        }
    }
}
