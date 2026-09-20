package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.BuildConfig
import com.gotkicry.audiolink.ui.components.AlTextButton
import com.gotkicry.audiolink.ui.components.SilkLabel
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import com.gotkicry.audiolink.ui.i18n.UiStrings
import com.gotkicry.audiolink.update.UpdateController
import com.gotkicry.audiolink.update.UpdateState

/**
 * 软件更新页（M5 · 与桌面端同一个立场）：
 *
 *  * **只在用户点击时检查**，不后台轮询、不静默下载；
 *  * 版本与下载地址来自公开仓库的 GitHub Releases（`releases/latest`，只认正式版）；
 *  * 下载完**先验签名证书**再交给系统安装器（见 `update/ApkSignature.kt`）。
 *
 * 与 [LicensesScreen] 同构：二级目的地、返回键回控制台、滚动位置由上层持有。
 *
 * [controller] 由 [AudioLinkRoot] 持有（不是本页 remember）：下载中途切走再回来，进度还在。
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun UpdateScreen(
    controller: UpdateController,
    onBack: () -> Unit,
    scrollState: ScrollState,
    modifier: Modifier = Modifier,
) {
    val strings = LocalStrings.current
    val state by controller.state.collectAsState()

    Scaffold(
        modifier = modifier,
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                title = { Text(strings.updateTitle) },
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
            SilkLabel(strings.updateHint)
            Spacer(modifier = Modifier.height(12.dp))
            Text(
                text = String.format(strings.updateCurrentFormat, controller.currentVersion),
                style = MaterialTheme.typography.bodyMedium,
            )
            if (BuildConfig.DEBUG) {
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = strings.updateDevBuildNote,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Spacer(modifier = Modifier.height(16.dp))
            UpdateBody(state = state, strings = strings, controller = controller)
            Spacer(modifier = Modifier.height(24.dp))
        }
    }
}

@Composable
private fun UpdateBody(
    state: UpdateState,
    strings: UiStrings,
    controller: UpdateController,
) {
    when (state) {
        is UpdateState.Idle -> {
            PrimaryAction(strings.updateCheck) { controller.check() }
        }

        is UpdateState.Checking -> {
            Text(strings.updateChecking, style = MaterialTheme.typography.bodyMedium)
        }

        is UpdateState.UpToDate -> {
            Text(strings.updateUpToDate, style = MaterialTheme.typography.bodyMedium)
            Spacer(modifier = Modifier.height(12.dp))
            PrimaryAction(strings.updateCheck) { controller.check() }
        }

        is UpdateState.Available -> {
            Text(
                text = String.format(
                    strings.updateAvailableFormat,
                    state.info.version,
                    state.currentVersion,
                ),
                style = MaterialTheme.typography.bodyLarge,
            )
            state.info.publishedAt?.let { published ->
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    text = String.format(strings.updatePublishedFormat, published.take(10)),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (state.info.notes.isNotBlank()) {
                Spacer(modifier = Modifier.height(12.dp))
                SilkLabel(strings.updateNotes)
                Spacer(modifier = Modifier.height(4.dp))
                Text(text = state.info.notes, style = MaterialTheme.typography.bodySmall)
            }
            Spacer(modifier = Modifier.height(16.dp))
            PrimaryAction(String.format(strings.updateDownloadFormat, state.info.version)) {
                controller.download()
            }
        }

        is UpdateState.Downloading -> {
            val label = if (state.percent >= 0) {
                String.format(strings.updateDownloadingFormat, state.percent)
            } else {
                // 服务端没给长度时不猜进度 —— 只说实话。
                strings.updateDownloadingUnknown
            }
            Text(label, style = MaterialTheme.typography.bodyMedium)
        }

        is UpdateState.ReadyToInstall -> {
            Text(
                text = String.format(strings.updateReadyFormat, state.version),
                style = MaterialTheme.typography.bodyMedium,
            )
            Spacer(modifier = Modifier.height(12.dp))
            PrimaryAction(strings.updateInstall) { controller.install() }
        }

        is UpdateState.NeedsPermission -> {
            Text(strings.updatePermissionNote, style = MaterialTheme.typography.bodyMedium)
            Spacer(modifier = Modifier.height(12.dp))
            PrimaryAction(strings.updateContinueInstall) { controller.install() }
        }

        is UpdateState.Failed -> {
            Text(
                text = state.message,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.error,
            )
            Spacer(modifier = Modifier.height(12.dp))
            PrimaryAction(strings.updateCheck) { controller.check() }
        }
    }
}

/** 主操作行：高度按 48 dp 的下限给（触控目标），与仓库其它按钮一致。 */
@Composable
private fun PrimaryAction(label: String, onClick: () -> Unit) {
    AlTextButton(onClick = onClick, modifier = Modifier.heightIn(min = 48.dp)) {
        Text(label)
    }
}
