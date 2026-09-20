package com.gotkicry.audiolink.ui.screens

import android.content.ClipData
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.ui.components.AlTextButton
import com.gotkicry.audiolink.ui.components.AppIcons
import com.gotkicry.audiolink.ui.components.DeckHeader
import com.gotkicry.audiolink.ui.components.PanelCard
import com.gotkicry.audiolink.ui.host.LocalAddresses
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import com.gotkicry.audiolink.ui.theme.AudioLinkType
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** 主机出示自己的地址，接收端主动连接。地址与复制在同一行，长地址允许换行。 */
@Composable
fun HostAddressCard(
    listenAddr: String,
    modifier: Modifier = Modifier,
    discover: (() -> List<String>)? = null,
) {
    val strings = LocalStrings.current
    val context = LocalContext.current
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    var refreshKey by remember { mutableStateOf(0) }
    var copied by remember { mutableStateOf(false) }
    val candidates = remember(refreshKey) { discover?.invoke() ?: LocalAddresses.lanAddrs(context) }
    val port = LocalAddresses.portOf(listenAddr)
    val address = remember(candidates, port) { LocalAddresses.displayAddr(candidates, port) }
    val ready = address.isNotEmpty()

    PanelCard(modifier = modifier) {
        DeckHeader(strings.deckHost, trailing = {
            AlTextButton(onClick = { refreshKey++ }) { Text(strings.hostAddrRefresh) }
        })
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = if (ready) address else strings.hostAddrMissing,
                style = if (ready) AudioLinkType.readoutLarge else MaterialTheme.typography.titleMedium,
                color = if (ready) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.weight(1f).semantics { liveRegion = LiveRegionMode.Polite },
            )
            AlTextButton(
                onClick = {
                    scope.launch {
                        try {
                            clipboard.setClipEntry(ClipEntry(ClipData.newPlainText("AudioLink", address)))
                            copied = true
                            delay(1_600L)
                            copied = false
                        } catch (_: Exception) {
                            // 系统拒绝剪贴板时不报告复制成功，完整地址仍可手动输入。
                        }
                    }
                },
                enabled = ready,
                modifier = Modifier.heightIn(min = 48.dp),
            ) {
                Icon(
                    if (copied && ready) AppIcons.Check else AppIcons.Copy,
                    if (copied && ready) strings.hostAddrCopied else strings.hostAddrCopy,
                    Modifier.size(20.dp),
                )
            }
        }
        Text(
            text = if (ready) strings.hostAddrHint else strings.hostAddrMissingHint,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (candidates.size > 1) {
            Text(strings.hostAddrMultiHint, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}
