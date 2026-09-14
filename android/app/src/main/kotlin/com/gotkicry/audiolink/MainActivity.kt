package com.gotkicry.audiolink

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.gotkicry.audiolink.service.AudioLinkService
import com.gotkicry.audiolink.ui.AudioLinkTheme
import com.gotkicry.audiolink.ui.PlaybackScreen

/**
 * 单页信息架构（docs/08-ui-spec.md §3.1）：M1 只落地「本机状态」这一半 ——
 * 服务启停 + 播放可观测项（低延迟是否生效 / 采样率 / 声道 / 缓冲帧数 / 欠载）。
 * 节点列表与发送源切换属 M2+，等内核接上后再填。
 *
 * 注意：Activity 只负责 UI。前台服务（AudioLinkService）由用户操作显式启动，
 * 不存在「必须先打开 App 才能工作」的隐式依赖；状态经 `AudioLinkService.state` 收流，
 * 所以"服务先起、UI 后开"也能立刻显示正确状态。
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            AudioLinkTheme {
                val state by AudioLinkService.state.collectAsStateWithLifecycle()

                // Android 13+ 通知权限：被拒**不影响**播放（前台服务照样能起来），
                // 只是通知不可见 —— 与 ADR-012 的取舍一致，所以拒绝时不做任何降级动作。
                // 回调结果刻意不处理：拒绝通知权限不影响播放（前台服务照常启动），只是通知不可见，
                // 这与 ADR-012 的取舍一致，所以这里没有任何降级动作。
                val notificationPermissionLauncher = rememberLauncherForActivityResult(
                    contract = ActivityResultContracts.RequestPermission(),
                ) { _: Boolean -> }

                PlaybackScreen(
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
                )
            }
        }
    }
}
