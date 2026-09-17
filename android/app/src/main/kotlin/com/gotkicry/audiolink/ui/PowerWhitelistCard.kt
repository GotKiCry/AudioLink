package com.gotkicry.audiolink.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.gotkicry.audiolink.service.PowerWhitelistMapper
import com.gotkicry.audiolink.service.PowerWhitelistProbe
import com.gotkicry.audiolink.service.PowerWhitelistVariant

/**
 * 省电白名单引导（FR-37 / docs/05 M5 交付物 2）。
 *
 * **为什么这块钱值**：长连接、采集、播放全活在前台服务里，而系统的电池优化会回收后台的前台服务 ——
 * 用户把手机放一边听音频，可能过一阵就被系统优化掉，声音断了而界面上什么都看不出来。
 * 这张卡是用户能自己解决这件事的**唯一入口**。
 *
 * 分层（与 LicensesScreen / PlaybackScreen 的既有立场一致）：
 * 本文件**只渲染**——状态、文案、动作链、以及"要不要弹引导"全部来自
 * [com.gotkicry.audiolink.service.PowerWhitelistMapper]（纯逻辑，有 JVM 单测）；
 * 系统读数与 Intent 由 [com.gotkicry.audiolink.service.PowerWhitelistProbe] 提供。
 * 这里没有任何一处判断能影响用户看到什么。
 *
 * 刻意**不**做成弹窗/通知：白名单是"建议"而不是阻断式前置条件，本应用没加也能跑
 * （只是后台更容易被打断）。用户点过"不再提示"之后本卡退化成一行状态 + 一个入口，不再占注意力。
 */
@Composable
fun PowerWhitelistCard(modifier: Modifier = Modifier) {
    val context = LocalContext.current
    var dismissed by remember { mutableStateOf(PowerWhitelistProbe.isPromptDismissed(context)) }
    var openFailed by remember { mutableStateOf(false) }
    // 从系统设置页返回时重新读一次系统读数：用户很可能刚在那里把本应用加进白名单，
    // 回来必须看到状态变了（不缓存，见 PowerWhitelistProbe.snapshot 的说明）。
    var resumeToken by remember { mutableIntStateOf(0) }
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                resumeToken++
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    val state = remember(dismissed, resumeToken) {
        PowerWhitelistMapper.map(PowerWhitelistProbe.snapshot(context))
    }

    /** 依次试设置页；一个都拉不起来就说话（由 OPEN_FAILED_NOTE 给出手动路径）。 */
    val openSettings: () -> Unit = {
        openFailed = !PowerWhitelistProbe.openSettings(context, state.actions)
    }

    val dismissPrompt: () -> Unit = {
        PowerWhitelistProbe.markPromptDismissed(context)
        dismissed = true
        openFailed = false
    }

    when (state.variant) {
        // 已加入 / 本机不支持：只留一行状态。用户排查"后台声音断了"时第一眼能看到结论。
        PowerWhitelistVariant.Ok, PowerWhitelistVariant.Unsupported -> Text(
            text = state.statusLabel,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = modifier,
        )

        PowerWhitelistVariant.Quiet -> Card(modifier = modifier) {
            Column(
                Modifier.fillMaxWidth().padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                WhitelistHeadline(state.statusLabel)
                Text(state.detail, style = MaterialTheme.typography.bodySmall)
                state.runtimeNote?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
                state.actionLabel?.let { label ->
                    TextButton(onClick = openSettings) { Text(label) }
                }
                OpenFailureNote(openFailed)
            }
        }

        PowerWhitelistVariant.Action -> Card(
            modifier = modifier,
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.primaryContainer,
            ),
        ) {
            Column(
                Modifier.fillMaxWidth().padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                state.prompt?.let { prompt ->
                    WhitelistHeadline(prompt.title)
                    Text(state.detail, style = MaterialTheme.typography.bodySmall)
                    state.runtimeNote?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
                    Button(onClick = openSettings) { Text(prompt.actionLabel) }
                    prompt.dismissLabel?.let { label ->
                        TextButton(onClick = dismissPrompt) { Text(label) }
                    }
                }
                OpenFailureNote(openFailed)
            }
        }
    }
}

/** 卡片标题（带状态行的小字，任何档位下都要能一眼看到"到底加没加"）。 */
@Composable
private fun WhitelistHeadline(title: String) {
    Text(title, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.Bold)
}

/**
 * 跳转失败时的那句话。
 *
 * 必须显示：厂商 ROM 上没有白名单列表页是常见情况，静默失败会让用户以为"点了没反应"，
 * 而问题其实在系统那边（文案与手动路径都在 service 层的常量里）。
 */
@Composable
private fun OpenFailureNote(failed: Boolean) {
    if (!failed) {
        return
    }
    Text(
        PowerWhitelistMapper.OPEN_FAILED_NOTE,
        color = MaterialTheme.colorScheme.error,
        style = MaterialTheme.typography.bodySmall,
    )
}
