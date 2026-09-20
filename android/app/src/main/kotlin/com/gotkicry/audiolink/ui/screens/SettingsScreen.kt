package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
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
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.BuildConfig
import com.gotkicry.audiolink.service.PlaybackUiState
import com.gotkicry.audiolink.ui.components.AppIcons
import com.gotkicry.audiolink.ui.components.AlTextButton
import com.gotkicry.audiolink.ui.components.ConsoleContent
import com.gotkicry.audiolink.ui.components.ConsolePageHeading
import com.gotkicry.audiolink.ui.components.AppearancePicker
import com.gotkicry.audiolink.ui.components.DeckHeader
import com.gotkicry.audiolink.ui.components.PanelCard
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import com.gotkicry.audiolink.ui.theme.ThemeMode

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    state: PlaybackUiState,
    themeMode: ThemeMode,
    onThemeModeChange: (ThemeMode) -> Unit,
    onBack: () -> Unit,
    onOpenLicenses: () -> Unit,
    onOpenUpdate: () -> Unit,
    scrollState: ScrollState,
    modifier: Modifier = Modifier,
) {
    val strings = LocalStrings.current
    var diagnosticsExpanded by rememberSaveable { mutableStateOf(false) }
    Scaffold(
        modifier = modifier,
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                title = { Text("AudioLink", style = MaterialTheme.typography.bodyMedium) },
                navigationIcon = {
                    AlTextButton(onClick = onBack) { Icon(AppIcons.Back, strings.actionBack) }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
            )
        },
    ) { padding ->
        ConsoleContent(padding, scrollState) {
            ConsolePageHeading(strings.settingsTitle)
            PanelCard {
                DeckHeader(strings.appearanceTitle)
                Text(strings.appearanceHint, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                AppearancePicker(selected = themeMode, onSelect = onThemeModeChange)
            }
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(strings.backgroundTitle, style = MaterialTheme.typography.titleSmall, modifier = Modifier.semantics { heading() })
                PowerWhitelistDeck()
            }
            PanelCard {
                DeckHeader(strings.aboutTitle, trailing = {
                    Text(BuildConfig.VERSION_NAME, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                })
                SettingsLink(strings.updateTitle, onOpenUpdate)
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                SettingsLink(strings.licensesTitle, onOpenLicenses)
            }
            DiagnosticsDeck(
                state = state,
                expanded = diagnosticsExpanded,
                onToggle = { diagnosticsExpanded = !diagnosticsExpanded },
            )
        }
    }
}

@Composable
private fun SettingsLink(label: String, onClick: () -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onClick).heightIn(min = 48.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(label, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        Icon(AppIcons.ChevronRight, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}
