package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.capture.CaptureSourceKind
import com.gotkicry.audiolink.service.PlaybackUiState
import com.gotkicry.audiolink.ui.components.AppIcons
import com.gotkicry.audiolink.ui.components.LampTone
import com.gotkicry.audiolink.ui.components.NoticePanel
import com.gotkicry.audiolink.ui.components.SilkLabel
import com.gotkicry.audiolink.ui.components.rememberMediaVolumeController
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import com.gotkicry.audiolink.ui.i18n.UiStrings
import com.gotkicry.audiolink.ui.theme.ThemeMode

/**
 * 首屏 —— **听筒端**的单页（产品真相：手机上只做三件事：开始接收、停止接收、调音量）。
 *
 * 分区顺序就是信息层级，不是审美排序：
 * 1. 顶部应用栏（标题 + 主题切换 + 许可入口）；
 * 2. 配对 PIN（**有才出现**，且排最前：配对是有时限的窗口，错过就得让对端重连）；
 * 3. **接收台**：状态灯 + 三个读数 + 音量推子 + 主按钮；
 * 4. **发送台**：采集源 → 连接 → 推流（顺序在卡内写死，避免"先连接后选源"把连接掐断）；
 * 5. 设备通道；
 * 6. 省电白名单引导（服务能不能长期活下去与此直接相关）；
 * 7. **诊断**（默认折叠，一个数字都不删）；
 * 8. 错误/提示（人话 + 恢复路径）。
 *
 * 全屏唯一的持续动画是接收台那盏灯的呼吸（≤1 Hz）。
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConsoleScreen(
    state: PlaybackUiState,
    onTogglePlayback: (Boolean) -> Unit,
    onConnect: (String) -> Unit,
    onSubmitPin: (String) -> Unit,
    onStartSend: () -> Unit,
    onStopSend: () -> Unit,
    onSelectCapture: (CaptureSourceKind?) -> Unit,
    themeMode: ThemeMode,
    onThemeModeChange: (ThemeMode) -> Unit,
    onOpenLicenses: () -> Unit,
    scrollState: ScrollState,
    modifier: Modifier = Modifier,
) {
    val strings = LocalStrings.current
    // 音量是听筒端的第三件事：推子直接读写系统媒体音量（见 rememberMediaVolumeController）。
    val volume = rememberMediaVolumeController()
    var diagnosticsExpanded by rememberSaveable { mutableStateOf(false) }

    Scaffold(
        modifier = modifier,
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            ConsoleTopBar(
                themeMode = themeMode,
                onThemeModeChange = onThemeModeChange,
                onOpenLicenses = onOpenLicenses,
            )
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                // 键盘 insets（2026-09-18 真机定位）：targetSdk 36 + Android 15 起强制 edge-to-edge，
                // Manifest 的 adjustResize 不再生效 —— 不加这一行，软键盘会**整块盖住**发送台的按钮。
                .imePadding()
                .padding(padding)
                .verticalScroll(scrollState)
                .padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Spacer(modifier = Modifier.height(4.dp))

            // PIN 卡排在最前：此刻用户唯一该做的动作就是把这 6 位数字打进电脑端。
            state.pairingPin?.let { pin ->
                PairingPanel(pin = pin, stale = state.pinIsStale, note = state.pairingNote)
            }

            ReceiverDeck(
                state = state,
                volume = volume,
                onTogglePlayback = onTogglePlayback,
            )

            SenderDeck(
                sender = state.sender,
                captureSelection = state.captureSelection,
                captureState = state.captureState,
                engineRunning = state.engineRunning,
                onSelectCapture = onSelectCapture,
                onConnect = onConnect,
                onSubmitPin = onSubmitPin,
                onStartSend = onStartSend,
                onStopSend = onStopSend,
            )

            DeviceDeck(peers = state.peers)

            PowerWhitelistDeck()

            DiagnosticsDeck(
                state = state,
                expanded = diagnosticsExpanded,
                onToggle = { diagnosticsExpanded = !diagnosticsExpanded },
            )

            // 错误卡：**只讲人话与恢复路径**，内核原文在诊断区（产品原则 3 + 5）。
            state.lastError?.let {
                NoticePanel(
                    title = strings.errorTitle,
                    message = strings.errorRecoveryPlayback,
                    icon = AppIcons.Warning,
                    tone = LampTone.Live,
                )
            }

            // 取配对状态失败（不是"没有配对"）：有 PIN 时它已经在 PIN 卡里显示了，这里只在没有 PIN 时兜底，
            // 免得同一句话出现两次。
            if (state.pairingNote != null && state.pairingPin == null) {
                NoticePanel(
                    title = strings.errorPairingReadFailed,
                    message = strings.errorRecoveryGeneric,
                    icon = AppIcons.Warning,
                    tone = LampTone.Warn,
                    detail = state.pairingNote,
                    detailLabel = strings.errorKernelDetail,
                )
            }

            Spacer(modifier = Modifier.height(24.dp))
        }
    }
}

/** 顶部应用栏。主题切换用文字（丝印大写）而不是图标：**说得出名字的控件不需要猜**。 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ConsoleTopBar(
    themeMode: ThemeMode,
    onThemeModeChange: (ThemeMode) -> Unit,
    onOpenLicenses: () -> Unit,
) {
    val strings = LocalStrings.current
    var menuExpanded by remember { mutableStateOf(false) }

    TopAppBar(
        title = {
            Text(
                text = "AudioLink",
                style = MaterialTheme.typography.titleLarge,
                color = MaterialTheme.colorScheme.onSurface,
            )
        },
        actions = {
            Box {
                TextButton(
                    onClick = { menuExpanded = true },
                    modifier = Modifier.heightIn(min = 48.dp),
                ) {
                    SilkLabel(strings.themeAction)
                }
                DropdownMenu(
                    expanded = menuExpanded,
                    onDismissRequest = { menuExpanded = false },
                ) {
                    ThemeMode.entries.forEach { mode ->
                        DropdownMenuItem(
                            text = { Text(themeModeLabel(strings, mode)) },
                            onClick = {
                                onThemeModeChange(mode)
                                menuExpanded = false
                            },
                            leadingIcon = {
                                if (mode == themeMode) {
                                    Icon(
                                        imageVector = AppIcons.Check,
                                        contentDescription = null,
                                    )
                                } else {
                                    Spacer(modifier = Modifier.height(24.dp))
                                }
                            },
                            modifier = Modifier.heightIn(min = 48.dp),
                        )
                    }
                }
            }
            TextButton(
                onClick = onOpenLicenses,
                modifier = Modifier.heightIn(min = 48.dp),
            ) {
                SilkLabel(strings.licensesAction)
            }
        },
    )
}

/** 主题模式的当前语言标签。 */
private fun themeModeLabel(strings: UiStrings, mode: ThemeMode): String = when (mode) {
    ThemeMode.System -> strings.themeSystem
    ThemeMode.Light -> strings.themeLight
    ThemeMode.Dark -> strings.themeDark
}
