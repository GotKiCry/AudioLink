package com.gotkicry.audiolink

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import com.gotkicry.audiolink.ui.AudioLinkTheme
import com.gotkicry.audiolink.ui.HomeScreenPlaceholder

/**
 * 单页信息架构（docs/08-ui-spec.md §3.1）：
 * 顶部「本机状态卡片」→下方「附近节点列表」→底部「本机发送源切换」。
 *
 * 注意：Activity 只负责 UI。前台服务（AudioLinkService）由用户操作显式启动，
 * 不存在「必须先打开 App 才能工作」的隐式依赖。
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            AudioLinkTheme {
                // TODO(M1)：HomeScreen(state = viewModel.state.collectAsStateWithLifecycle())
                HomeScreenPlaceholder()
            }
        }
    }
}
