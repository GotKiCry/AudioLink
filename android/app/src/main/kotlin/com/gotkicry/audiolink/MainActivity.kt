package com.gotkicry.audiolink

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.gotkicry.audiolink.capture.CaptureSourceKind
import com.gotkicry.audiolink.capture.MediaProjectionRequestActivity
import com.gotkicry.audiolink.service.AudioLinkService
import com.gotkicry.audiolink.ui.screens.AudioLinkRoot
import com.gotkicry.audiolink.ui.theme.ThemeMode
import com.gotkicry.audiolink.ui.theme.ThemePreference

/**
 * 单页信息架构（docs/08-ui-spec.md §3.1）：M1 落地「本机状态」这一半 ——
 * 服务启停 + 播放可观测项（低延迟是否生效 / 采样率 / 声道 / 缓冲帧数 / 欠载）；
 * 发送方向（连接电脑 + 推流，FR-17/§8）与发送源切换（FR-06/07）已接线，
 * 节点自动发现列表仍属 M2+。
 *
 * 注意：Activity 只负责 UI。前台服务（AudioLinkService）由用户操作显式启动，
 * 不存在「必须先打开 App 才能工作」的隐式依赖；状态经 `AudioLinkService.state` 收流，
 * 所以"服务先起、UI 后开"也能立刻显示正确状态。
 *
 * 界面本体全部在 `ui/` 下：主题与设计 token 在 `ui/theme`，屏幕在 `ui/screens`，
 * 共享控件在 `ui/components`，文案在 `ui/i18n`。这里只剩"把用户动作转成服务请求"。
 */
class MainActivity : ComponentActivity() {

    /** 主题偏好 → 是否深色（`System` 时读系统 `uiMode`）。 */
    private fun resolveDark(mode: ThemeMode): Boolean = when (mode) {
        ThemeMode.Dark -> true
        ThemeMode.Light -> false
        ThemeMode.System ->
            (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
                Configuration.UI_MODE_NIGHT_YES
    }

    /**
     * 系统栏：**透明 + 不加 scrim + 图标深浅跟随 App 主题**。
     *
     * 为什么不再用无参 `enableEdgeToEdge()`（2026-09-18）：
     * 1. 无参调用给导航栏传的是 `SystemBarStyle.auto(DefaultLightScrim, DefaultDarkScrim)`
     *    ——两个 scrim 都**不透明为 0**（浅色 `#E6FFFFFF`、深色 `#801B1B1B`），
     *    会在导航栏区域叠一层不属于本设计语言的颜色。这里显式给 `TRANSPARENT`：
     *    App 自己画到边，边缘的颜色只该来自界面本身（配 `res/values/themes.xml` 的同名声明）。
     * 2. 无参调用判断深浅读的是**系统** `uiMode`，而本 App 允许在界面里强制浅/深
     *    （[ThemePreference]）。系统浅色 + App 深色时，系统栏图标会按浅色画成深色图标，
     *    压在深色界面上等于看不见。这里按 **App 的实际主题**决定图标深浅。
     *
     * `ThemeMode.System` 是唯一还读系统的地方 —— 那正是"跟随系统"的定义。
     */
    private fun applySystemBars(dark: Boolean) {
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.auto(Color.TRANSPARENT, Color.TRANSPARENT) { dark },
            navigationBarStyle = SystemBarStyle.auto(Color.TRANSPARENT, Color.TRANSPARENT) { dark },
        )
    }

    /**
     * Manifest 声明了 `uiMode` 由本 Activity 自己处理（不重建），
     * 所以系统切换深浅时不会走 `onCreate`：系统栏必须在这里重算一次。
     */
    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        applySystemBars(resolveDark(ThemePreference.read(this)))
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        applySystemBars(resolveDark(ThemePreference.read(this)))
        setContent {
            val state by AudioLinkService.state.collectAsStateWithLifecycle()

            // Android 13+ 通知权限：被拒**不影响**播放（前台服务照样能起来），
            // 只是通知不可见 —— 与 ADR-012 的取舍一致，所以拒绝时不做任何降级动作。
            val notificationPermissionLauncher = rememberLauncherForActivityResult(
                contract = ActivityResultContracts.RequestPermission(),
            ) { _: Boolean -> }

            // 麦克风权限：**拿到之后才切源**。用户拒绝时什么都不做 —— 服务的发送源保持原值，
            // 不制造「选了麦克风却没有权限、界面还显示正在采集」这种假状态。
            var microphonePending by remember { mutableStateOf(false) }
            val audioPermissionLauncher = rememberLauncherForActivityResult(
                contract = ActivityResultContracts.RequestPermission(),
            ) { granted: Boolean ->
                if (granted && microphonePending) {
                    AudioLinkService.setCaptureSource(this, CaptureSourceKind.Microphone)
                }
                microphonePending = false
            }

            AudioLinkRoot(
                state = state,
                onTogglePlayback = { enable ->
                    if (enable) {
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) !=
                            PackageManager.PERMISSION_GRANTED
                        ) {
                            notificationPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
                        }
                        AudioLinkService.start(this)
                    } else {
                        AudioLinkService.stop(this)
                    }
                },
                // 发送方向：四个动作分别对应内核的 connect / submitPin / startSend / stopSend。
                // 这里只把「用户点了什么」转成服务请求；能不能连、能不能发由 service 判定并回写状态。
                // 连接会顺带把前台服务拉起来（真机评审 P0：用户不该先去找另一个按钮），
                // 所以通知权限也在这一步问 —— 与「开始接收」那条路径同一套处理。
                onConnect = { addr ->
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                        checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) !=
                        PackageManager.PERMISSION_GRANTED
                    ) {
                        notificationPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
                    }
                    AudioLinkService.connect(this, addr)
                },
                onSubmitPin = { pin -> AudioLinkService.submitPin(this, pin) },
                onStartSend = { AudioLinkService.startSend(this) },
                onStopSend = { AudioLinkService.stopSend(this) },
                // 发送源（FR-06/07）：三个分支各自把「还差什么前置」补齐再交给服务。
                onSelectCapture = { source ->
                    when (source) {
                        null -> AudioLinkService.setCaptureSource(this, null)

                        CaptureSourceKind.Microphone -> {
                            val granted = checkSelfPermission(Manifest.permission.RECORD_AUDIO) ==
                                PackageManager.PERMISSION_GRANTED
                            if (granted) {
                                AudioLinkService.setCaptureSource(this, CaptureSourceKind.Microphone)
                            } else {
                                microphonePending = true
                                audioPermissionLauncher.launch(Manifest.permission.RECORD_AUDIO)
                            }
                        }

                        CaptureSourceKind.SystemLoopback -> {
                            // 顺序不能反：先让服务登记「用户想要内录」并补上 mediaProjection 前台类型位
                            // （Android 14+ 要求应用已处于该类型的**前台服务**中才拿得到投影），
                            // 再拉起系统授权页；授权回执由 MediaProjectionRequestActivity 送回服务。
                            AudioLinkService.setCaptureSource(this, CaptureSourceKind.SystemLoopback)
                            startActivity(Intent(this, MediaProjectionRequestActivity::class.java))
                        }
                    }
                },
                // 界面里换了主题 → 系统栏图标的深浅要跟着换（Activity 不重建，onCreate 不会再跑）。
                onThemeModeChanged = { mode -> applySystemBars(resolveDark(mode)) },
            )
        }
    }
}
