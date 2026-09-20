package com.gotkicry.audiolink.ui.screens

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.rememberScrollState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
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
import com.gotkicry.audiolink.update.UpdateController

/**
 * 主题、系统语言和二级导航。设置、更新与许可逐级返回；保存每个页面的表单与滚动位置。
 * 页面数量有限，使用轻量状态导航，不额外引入导航依赖。
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
    var showUpdate by rememberSaveable { mutableStateOf(false) }
    var showSettings by rememberSaveable { mutableStateOf(false) }
    val screenState = rememberSaveableStateHolder()
    val consoleScroll = rememberScrollState()
    val licensesScroll = rememberScrollState()
    val updateScroll = rememberScrollState()
    val settingsScroll = rememberScrollState()
    // 更新控制器由本层持有（而不是 UpdateScreen 自己 remember）：
    // 下载中途切回控制台再回来时，进度与「已下载待安装」的状态都还在。
    val updateController = remember { UpdateController(context.applicationContext) }
    DisposableEffect(updateController) {
        onDispose { updateController.dispose() }
    }

    AudioLinkTheme(mode = themeMode) {
        CompositionLocalProvider(LocalStrings provides strings) {
            if (showLicenses) {
                BackHandler { showLicenses = false }
                LicensesScreen(
                    onBack = { showLicenses = false },
                    scrollState = licensesScroll,
                    modifier = modifier,
                )
            } else if (showUpdate) {
                BackHandler { showUpdate = false }
                UpdateScreen(
                    controller = updateController,
                    onBack = { showUpdate = false },
                    scrollState = updateScroll,
                    modifier = modifier,
                )
            } else if (showSettings) {
                BackHandler { showSettings = false }
                screenState.SaveableStateProvider("settings") {
                    SettingsScreen(
                        state = state,
                        themeMode = themeMode,
                        onThemeModeChange = { mode ->
                            themeMode = mode
                            ThemePreference.write(context, mode)
                            onThemeModeChanged(mode)
                        },
                        onBack = { showSettings = false },
                        onOpenLicenses = { showLicenses = true },
                        onOpenUpdate = { showUpdate = true },
                        scrollState = settingsScroll,
                        modifier = modifier,
                    )
                }
            } else {
                screenState.SaveableStateProvider("console") {
                    ConsoleScreen(
                        state = state,
                        onTogglePlayback = onTogglePlayback,
                        onConnect = onConnect,
                        onSubmitPin = onSubmitPin,
                        onStartSend = onStartSend,
                        onStopSend = onStopSend,
                        onSelectCapture = onSelectCapture,
                        onOpenSettings = { showSettings = true },
                        scrollState = consoleScroll,
                        modifier = modifier,
                    )
                }
            }
        }
    }
}
