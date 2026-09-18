package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
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
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.gotkicry.audiolink.service.PowerWhitelistMapper
import com.gotkicry.audiolink.service.PowerWhitelistProbe
import com.gotkicry.audiolink.service.PowerWhitelistVariant
import com.gotkicry.audiolink.ui.components.PanelCard
import com.gotkicry.audiolink.ui.components.SilkLabel

/**
 * 省电白名单引导（FR-37）。
 *
 * **为什么这块钱值**：长连接、采集、播放全活在前台服务里，而系统的电池优化会回收后台的前台服务 ——
 * 用户把手机放一边听音频，可能过一阵就被系统优化掉，声音断了而界面上什么都看不出来。
 * 这张卡是用户能自己解决这件事的**唯一入口**。
 *
 * 分层（与旧版一致）：本文件**只渲染** —— 状态、文案、动作链、以及"要不要弹引导"全部来自
 * [PowerWhitelistMapper]（纯逻辑，有 JVM 单测）；系统读数与 Intent 由 [PowerWhitelistProbe] 提供。
 * 本次重做只换外壳（面板 / 48 dp / 丝印标题），一条判断都没有搬进来。
 */
@Composable
fun PowerWhitelistDeck(modifier: Modifier = Modifier) {
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
        PowerWhitelistVariant.Ok, PowerWhitelistVariant.Unsupported -> Column(
            modifier = modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            SilkLabel(state.statusLabel)
        }

        PowerWhitelistVariant.Quiet -> PanelCard(modifier = modifier) {
            Text(
                text = state.statusLabel,
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.semantics { heading() },
            )
            Text(state.detail, style = MaterialTheme.typography.bodySmall)
            state.runtimeNote?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
            state.actionLabel?.let { label ->
                TextButton(
                    onClick = openSettings,
                    modifier = Modifier.heightIn(min = 48.dp),
                ) { Text(label) }
            }
            OpenFailureNote(openFailed)
        }

        PowerWhitelistVariant.Action -> PanelCard(
            modifier = modifier,
            containerColor = MaterialTheme.colorScheme.primaryContainer,
            borderColor = MaterialTheme.colorScheme.primary,
        ) {
            state.prompt?.let { prompt ->
                Text(
                    text = prompt.title,
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                    modifier = Modifier.semantics { heading() },
                )
                Text(
                    text = state.detail,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                )
                state.runtimeNote?.let {
                    Text(
                        text = it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onPrimaryContainer,
                    )
                }
                Button(
                    onClick = openSettings,
                    modifier = Modifier.heightIn(min = 48.dp),
                ) { Text(prompt.actionLabel) }
                prompt.dismissLabel?.let { label ->
                    TextButton(
                        onClick = dismissPrompt,
                        modifier = Modifier.heightIn(min = 48.dp),
                    ) { Text(label) }
                }
            }
            OpenFailureNote(openFailed)
        }
    }
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
        text = PowerWhitelistMapper.OPEN_FAILED_NOTE,
        color = MaterialTheme.colorScheme.error,
        style = MaterialTheme.typography.bodySmall,
    )
}
