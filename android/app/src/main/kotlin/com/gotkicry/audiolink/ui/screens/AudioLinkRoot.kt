package com.gotkicry.audiolink.ui.screens

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.rememberScrollState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import com.gotkicry.audiolink.capture.CaptureSourceKind
import com.gotkicry.audiolink.service.PlaybackUiState
import com.gotkicry.audiolink.ui.i18n.AudioLinkStrings
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import com.gotkicry.audiolink.ui.theme.AudioLinkTheme
import com.gotkicry.audiolink.ui.theme.ThemeMode
import com.gotkicry.audiolink.ui.theme.ThemePreference

/**
 * 应用根：**主题 + 语言 + 导航**三件事，一层解决。
 *
 * 导航为什么不引 navigation-compose：全应用只有两个目的地（控制台 / 开源许可），
 * 为两屏引入一整套导航图、返回栈与序列化契约，是拿一个依赖换一次 if —— 不划算。
 * 但**必须**有 [BackHandler]：许可页是真正的二级目的地，按系统返回要回控制台，
 * 而不是像旧版那样直接退出 App（旧版用一个布尔替换整屏且没有返回处理，这是真机上的返回键陷阱）。
 *
 * 滚动位置由这一层持有（[consoleScroll] / [licensesScroll]）：切走再回来时**位置还在**，
 * 不会被重置到顶部。
 */
@Composable
fun AudioLinkRoot(
    state: PlaybackUiState,
    onTogglePlayback: (Boolean) -> Unit,
    onConnect: (String) -> Unit,
    onSubmitPin: (String) -> Unit,
    onStartSend: () -> Unit,
    onStopSend: () -> Unit,
    onSelectCapture: (CaptureSourceKind?) -> Unit,
    modifier: Modifier = Modifier,
    /** 主题模式变化时的回调（宿主用它同步系统栏图标的深浅）。 */
    onThemeModeChanged: (ThemeMode) -> Unit = {},
) {
    val context = LocalContext.current
    // 用 LocalConfiguration（configuration-aware）而不是 context.resources.configuration：
    // Manifest 声明了 uiMode/screenSize 等由本 Activity 自己处理，换语言、换深浅时
    // Activity **不重建** —— 走 context.resources 读到的是那一份没被更新过的 Configuration，
    // 语言切换后界面会停在旧文案。这条是 lint 的 LocalContextConfigurationRead（预先存在的告警，
    // 不是本轮形状改造引入的），因为这轮把「配置变化不让 Activity 重建」这条路径用实了，顺手修掉。
    val language = LocalConfiguration.current.locales[0].language
    val strings = remember(language) { AudioLinkStrings.forLanguage(language) }
    var themeMode by remember { mutableStateOf(ThemePreference.read(context)) }
    var showLicenses by rememberSaveable { mutableStateOf(false) }
    val consoleScroll = rememberScrollState()
    val licensesScroll = rememberScrollState()

    AudioLinkTheme(mode = themeMode) {
        CompositionLocalProvider(LocalStrings provides strings) {
            if (showLicenses) {
                BackHandler { showLicenses = false }
                LicensesScreen(
                    onBack = { showLicenses = false },
                    scrollState = licensesScroll,
                    modifier = modifier,
                )
            } else {
                ConsoleScreen(
                    state = state,
                    onTogglePlayback = onTogglePlayback,
                    onConnect = onConnect,
                    onSubmitPin = onSubmitPin,
                    onStartSend = onStartSend,
                    onStopSend = onStopSend,
                    onSelectCapture = onSelectCapture,
                    themeMode = themeMode,
                    onThemeModeChange = { mode ->
                        themeMode = mode
                        ThemePreference.write(context, mode)
                        onThemeModeChanged(mode)
                    },
                    onOpenLicenses = { showLicenses = true },
                    scrollState = consoleScroll,
                    modifier = modifier,
                )
            }
        }
    }
}
