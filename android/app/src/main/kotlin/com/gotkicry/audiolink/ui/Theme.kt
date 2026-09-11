package com.gotkicry.audiolink.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

/** AudioLink 主题（配色与桌面端统一，见 docs/08-ui-spec.md §1） */
private val Indigo = Color(0xFF4F46E5)
private val IndigoLight = Color(0xFF818CF8)

private val LightColors = lightColorScheme(primary = Indigo)
private val DarkColors = darkColorScheme(primary = IndigoLight)

@Composable
fun AudioLinkTheme(
    darkTheme: Boolean = androidx.compose.foundation.isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    // TODO(M5)：接入主题设置（跟随系统 / 手动），并支持动态取色（Android 12+）
    MaterialTheme(colorScheme = if (darkTheme) DarkColors else LightColors, content = content)
}

/**
 * 首页骨架（规格：docs/08-ui-spec.md §3.1）
 * 顶部状态卡片 + 节点列表；数据接入点在 M1（由 AudioLinkService 经 StateFlow 提供）。
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreenPlaceholder() {
    var gain by remember { mutableFloatStateOf(70f) }

    Scaffold(topBar = { TopAppBar(title = { Text("AudioLink") }) }) { padding ->
        LazyColumn(
            modifier = Modifier.fillMaxSize().padding(padding).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item {
                Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant)) {
                    Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("本机「我的手机」 · fp:00000000", style = MaterialTheme.typography.titleMedium)
                        Text("● 空闲（未连接）", color = MaterialTheme.colorScheme.primary)
                        Text("端到端 — ms · — kbps · 缓冲 — ms", style = MaterialTheme.typography.bodySmall)
                        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                            Text("音量")
                            Slider(value = gain, onValueChange = { gain = it }, modifier = Modifier.weight(1f))
                            Text("${gain.toInt()}")
                        }
                    }
                }
            }
            item { Text("附近节点", style = MaterialTheme.typography.titleSmall) }
            items(2) { index ->
                Card {
                    Column(Modifier.fillMaxWidth().padding(16.dp)) {
                        Text(if (index == 0) "💻 书房 PC · 已配对" else "📱 卧室手机 · 未配对")
                        Text("同一网络 · 可接收", style = MaterialTheme.typography.bodySmall)
                    }
                }
            }
            item {
                Text(
                    "底部发送源：关闭 / 系统内录（Android 10+）/ 麦克风",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }
    }
}
