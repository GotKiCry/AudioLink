package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.compliance.NoticesLoader
import com.gotkicry.audiolink.ui.components.AlTextButton
import com.gotkicry.audiolink.ui.components.SilkLabel
import com.gotkicry.audiolink.ui.i18n.LocalStrings

/**
 * 开源许可页（M5 合规）：把打进 APK 的第三方声明读出来给用户看。
 *
 * 与桌面端「关于」面板同一条规则：**找不到时说清怎么生成**，而不是显示空白页。
 * 内容随包分发（assets），所以离线也能看。
 *
 * 本次修的是**返回键陷阱**：旧版用一个布尔把整屏换掉、既没有 BackHandler 也不是二级页，
 * 按系统返回会**直接退出 App**。现在它由 [AudioLinkRoot] 作为真正的二级目的地管理：
 * 返回键回列表、并且**离开再回来时滚动位置还在**（[scrollState] 由上层持有，不随页面销毁）。
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LicensesScreen(
    onBack: () -> Unit,
    scrollState: ScrollState,
    modifier: Modifier = Modifier,
) {
    val strings = LocalStrings.current
    val context = LocalContext.current
    // 读一次就够：它在 APK 里是静态资源，不会中途变化。
    val state = remember {
        val text = runCatching {
            context.assets.open(NoticesLoader.ASSET_NAME).bufferedReader().use { it.readText() }
        }.getOrNull()
        NoticesLoader.fromText(text)
    }

    Scaffold(
        modifier = modifier,
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                title = { Text(strings.licensesTitle) },
                navigationIcon = {
                    AlTextButton(
                        onClick = onBack,
                        modifier = Modifier.heightIn(min = 48.dp),
                    ) { Text(strings.actionBack) }
                },
            )
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(horizontal = 16.dp, vertical = 8.dp)
                .verticalScroll(scrollState),
        ) {
            SilkLabel(
                text = if (state.available) {
                    String.format(strings.licensesBundledFormat, state.bytes / 1024)
                } else {
                    strings.licensesMissing
                },
            )
            Spacer(modifier = Modifier.height(8.dp))
            Text(text = state.text, style = MaterialTheme.typography.bodySmall)
            Spacer(modifier = Modifier.height(24.dp))
        }
    }
}
