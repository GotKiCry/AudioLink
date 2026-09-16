package com.gotkicry.audiolink.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.compliance.NoticesLoader

/**
 * 开源许可页（M5 合规）：把打进 APK 的第三方声明读出来给用户看。
 *
 * 与桌面端「关于」面板同一条规则：**找不到时说清怎么生成**，而不是显示空白页。
 * 内容随包分发（assets），所以离线也能看。
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LicensesScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    // 读一次就够：它在 APK 里是静态资源，不会中途变化。
    val state = remember {
        val text = runCatching {
            context.assets.open(NoticesLoader.ASSET_NAME).bufferedReader().use { it.readText() }
        }.getOrNull()
        NoticesLoader.fromText(text)
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("开源许可") },
                navigationIcon = { TextButton(onClick = onBack) { Text("返回") } },
            )
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(16.dp)
                .verticalScroll(rememberScrollState()),
        ) {
            Text(
                text = if (state.available) {
                    "随包分发 · ${state.bytes / 1024} KB"
                } else {
                    "未找到声明文件"
                },
                style = MaterialTheme.typography.labelMedium,
            )
            Spacer(modifier = Modifier.height(8.dp))
            Text(text = state.text, style = MaterialTheme.typography.bodySmall)
        }
    }
}
