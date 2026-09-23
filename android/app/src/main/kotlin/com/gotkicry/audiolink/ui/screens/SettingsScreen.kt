package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.BuildConfig
import com.gotkicry.audiolink.service.AudioLinkService
import com.gotkicry.audiolink.service.LowLatencyPreference
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
            // 低延迟模式：音频链路的档位，独立一张卡（与「外观」这种纯 UI 偏好分开，
            // 因为它有真实代价与兼容要求，不该混在主题选择里被顺手改掉）。
            LowLatencyDeck()
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

/**
 * 「低延迟模式」开关。
 *
 * ## 为什么状态用 `remember` 初值而不进 [PlaybackUiState]
 * 这个档位**不是运行期状态**，它是下次启动引擎才消费的一份偏好（契约）。把它塞进每 500 ms
 * 推一次的播放状态流，会让 UI 误以为"改了立刻生效" —— 那正是文案要澄清的误解。
 * 这里读一次、改一次都是同步的本地读写，不需要协程。
 *
 * ## 为什么不用 `rememberSaveable`
 * 唯一真相在 SharedPreferences（[LowLatencyPreference]），`remember` 只是一个渲染缓存。
 * 用 Saveable 反而制造第二份真相 —— 配置变更后用存档值覆盖盘上的值，会把"在别处改过"的状态吞掉。
 * 每次进入设置页重新读盘，才是与 [PowerWhitelistDeck] 一致的立场（那一侧也是 onResume 重读）。
 *
 * ## 为什么初值读**盘**，而不是 [AudioLinkService.isLowLatencyEnabled] 那个会话镜像
 * 这是个反直觉但必须的取舍 —— 镜像听起来更快，但它在"冷启动先开设置页"这一路是**假的**：
 * 服务只在用户点「开始接收」或连接主机时才被拉起（见 MainActivity 的 onTogglePlayback），
 * 设置页完全可能在一个**从未跑过服务**的进程里打开。此时镜像还停在类初始化值 `false`，
 * 而盘上明明是 `true` —— 开关会显示成关，用户以为设置丢了。
 * 读盘没有这个死角：**盘是本开关唯一的真相**，镜像只是给引擎启动省一次 IO 的加速层。
 *
 * 这不会引入"两个真相"：写路径（[AudioLinkService.setLowLatency]）把盘与镜像**一起**更新，
 * 而镜像的初值同样来自读盘，两者只会因为"写盘异步失败"而分歧 —— 那时以能持久化的一方为准，
 * 恰恰也是用户重开 App 后会看到的值。
 */
@Composable
private fun LowLatencyDeck(modifier: Modifier = Modifier) {
    val strings = LocalStrings.current
    val context = LocalContext.current
    var enabled by remember { mutableStateOf(LowLatencyPreference.isEnabled(context)) }

    PanelCard(modifier = modifier) {
        DeckHeader(strings.lowLatencyTitle)
        Row(
            // toggleable 挂在整行上（而不是只让 Switch 可点）：48 dp 的可点区域是安全底线，
            // 而 Switch 本体只有约 52×32 dp，把标题也算进热区才是这块卡片的正常手感。
            modifier = Modifier.fillMaxWidth()
                .toggleable(
                    value = enabled,
                    role = Role.Switch,
                    onValueChange = { next ->
                        // 顺序要紧：先落盘 + 更新服务里的会话镜像与水位的下一次默认值，
                        // 再改本地 UI 状态。反过来的话（先改本地）若写盘失败，开关会停在盘上没有的位置。
                        AudioLinkService.setLowLatency(context, next)
                        enabled = next
                    },
                )
                .heightIn(min = 48.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = strings.lowLatencyTitle,
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.weight(1f),
            )
            Switch(
                checked = enabled,
                // 触控/无障碍语义由外层 toggleable 提供；Switch 自身不再接 onCheckedChange，
                // 否则同一行会有两个独立的点击源（点 Switch 与点整行走不同分支，语义读数会重复）。
                onCheckedChange = null,
            )
        }
        // 说明文字必须常显（不是展开才看）："两端需同时开启"是使用前就要知道的事，
        // 藏进对话框等于让用户在"听不清"的时候才去找原因。
        Text(
            text = strings.lowLatencyHint,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
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
