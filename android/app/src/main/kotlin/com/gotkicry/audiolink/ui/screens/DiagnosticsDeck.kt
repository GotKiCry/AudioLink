package com.gotkicry.audiolink.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.audio.LowLatencyPlayer
import com.gotkicry.audiolink.diagnostics.ProtocolSelfTest
import com.gotkicry.audiolink.diagnostics.SelfTestResult
import com.gotkicry.audiolink.service.AudioLinkService
import com.gotkicry.audiolink.service.PlaybackUiState
import com.gotkicry.audiolink.ui.components.AppIcons
import com.gotkicry.audiolink.ui.components.Choice
import com.gotkicry.audiolink.ui.components.ChoiceGroup
import com.gotkicry.audiolink.ui.components.DiagSection
import com.gotkicry.audiolink.ui.components.ExpandablePanel
import com.gotkicry.audiolink.ui.components.KeyValueRow
import com.gotkicry.audiolink.ui.components.LampTone
import com.gotkicry.audiolink.ui.components.SilkLabel
import com.gotkicry.audiolink.ui.components.StatusBadge
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import java.util.Locale
import kotlinx.coroutines.launch

/**
 * 诊断区 —— **内核真值默认折叠**（产品原则 5）。
 *
 * 原来平铺在首屏的六张卡（自检 / 引擎 / 低延迟实测 / 播放统计 / 播放环 / 输出队列水位）
 * **一个都没删**，只是收进一个折叠面板：它们服务工程师，不参与首屏叙事。
 *
 * 自检状态刻意放在**这一层**而不是展开内容里：折叠时内容会被移出 composition，
 * 放里面会让"折叠一次就把跑完的自检结果丢掉"，而自检是要用户读的。
 */
@Composable
fun DiagnosticsDeck(
    state: PlaybackUiState,
    expanded: Boolean,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val strings = LocalStrings.current
    // 自检不依赖服务（验的是协议层），所以它能独立跑、独立留结果。
    var selfTestResult by remember { mutableStateOf<SelfTestResult?>(null) }
    var selfTestRunning by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    ExpandablePanel(
        title = strings.deckDiagnostics,
        summary = strings.diagnosticsSummary,
        expanded = expanded,
        onToggle = onToggle,
        expandedStateText = strings.semanticsExpanded,
        collapsedStateText = strings.semanticsCollapsed,
        modifier = modifier,
    ) {
        // ── 自检 ───────────────────────────────────────────────────
        DiagSection(title = strings.diagSelfTest) {
            when (val result = selfTestResult) {
                null -> Text(
                    text = strings.diagSelfTestIdle,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )

                is SelfTestResult.Passed -> StatusBadge(
                    tone = LampTone.On,
                    icon = AppIcons.Check,
                    text = "${strings.diagSelfTestPassed} · ${result.summary}",
                    textStyle = MaterialTheme.typography.bodySmall,
                )

                is SelfTestResult.Rejected -> {
                    StatusBadge(
                        tone = LampTone.Live,
                        icon = AppIcons.Close,
                        text = strings.diagSelfTestRejected,
                        textStyle = MaterialTheme.typography.bodySmall,
                    )
                    KeyValueRow(strings.diagSelfTestCode, result.code.toString())
                    KeyValueRow(strings.diagSelfTestShortName, result.shortName)
                    KeyValueRow(strings.diagSelfTestDetail, result.context)
                }

                is SelfTestResult.Unavailable -> {
                    StatusBadge(
                        tone = LampTone.Live,
                        icon = AppIcons.Warning,
                        text = strings.diagSelfTestUnavailable,
                        textStyle = MaterialTheme.typography.bodySmall,
                    )
                    Text(
                        text = result.reason,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Button(
                onClick = {
                    if (!selfTestRunning) {
                        selfTestRunning = true
                        scope.launch {
                            try {
                                selfTestResult = ProtocolSelfTest.run()
                            } finally {
                                selfTestRunning = false
                            }
                        }
                    }
                },
                enabled = !selfTestRunning,
                modifier = Modifier.heightIn(min = 48.dp),
            ) {
                Icon(AppIcons.Play, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(modifier = Modifier.size(8.dp))
                Text(if (selfTestRunning) strings.actionSelfTesting else strings.actionRunSelfTest)
            }
            if (selfTestRunning) {
                LinearProgressIndicator(
                    modifier = Modifier
                        .fillMaxWidth()
                        .semantics { liveRegion = LiveRegionMode.Polite },
                )
            }
        }

        // ── 引擎 ───────────────────────────────────────────────────
        DiagSection(title = strings.diagEngine) {
            StatusBadge(
                tone = if (state.engineRunning) LampTone.On else LampTone.Idle,
                icon = if (state.engineRunning) AppIcons.Check else AppIcons.Info,
                text = if (state.engineRunning) strings.diagEngineRunning else strings.diagEngineStopped,
                textStyle = MaterialTheme.typography.bodyMedium,
            )
            if (state.engineRunning) {
                KeyValueRow(strings.diagLocalName, state.engineName)
                KeyValueRow(strings.diagLocalId, state.engineIdShort)
                KeyValueRow(strings.diagListenAddr, state.engineAddr)
            }
            KeyValueRow(
                label = strings.diagCaps,
                value = if (state.engineCanSend) strings.diagCapsBoth else strings.diagCapsReceiveOnly,
            )
            state.engineError?.let { message ->
                Text(
                    text = message,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
            }
        }

        // ── 低延迟（实测） ──────────────────────────────────────────
        DiagSection(title = strings.diagLowLatency) {
            StatusBadge(
                tone = if (state.lowLatency) LampTone.On else LampTone.Live,
                icon = if (state.lowLatency) AppIcons.Check else AppIcons.Close,
                text = if (state.lowLatency) strings.diagLowLatencyOn else strings.diagLowLatencyOff,
                textStyle = MaterialTheme.typography.bodyMedium,
            )
            KeyValueRow(strings.diagPerformanceMode, LowLatencyPlayer.performanceModeName(state.performanceMode))
            KeyValueRow(strings.diagSampleRate, "${state.sampleRate} Hz")
            KeyValueRow(strings.diagChannels, state.channelCount.toString())
            KeyValueRow(strings.diagBufferRequested, "${state.requestedBufferFrames} ${strings.diagFramesUnit}")
            KeyValueRow(strings.diagBufferActual, "${state.actualBufferFrames} ${strings.diagFramesUnit}")
        }

        // ── 统计 ───────────────────────────────────────────────────
        DiagSection(title = strings.diagCounters) {
            KeyValueRow(strings.diagUnderrunsSystem, state.trackUnderruns.toString())
            KeyValueRow(strings.diagUnderrunsSource, state.sourceUnderruns.toString())
            KeyValueRow(strings.diagFramesWritten, state.framesWritten.toString())
            KeyValueRow(strings.diagSilenceFrames, state.silenceFrames.toString())
            KeyValueRow(strings.diagFramesDropped, state.framesDropped.toString())
            KeyValueRow(
                label = strings.diagWriteErrors,
                value = if (state.writeErrors == 0) {
                    "0"
                } else {
                    String.format(
                        strings.diagWriteErrorsDetail,
                        state.writeErrors,
                        state.lastWriteError,
                    )
                },
            )
        }

        // ── 播放环 ─────────────────────────────────────────────────
        DiagSection(title = strings.diagRing) {
            KeyValueRow(
                strings.diagRingLevel,
                "${state.ringBufferedFrames} / ${state.ringCapacityFrames} ${strings.diagFramesUnit}",
            )
            KeyValueRow(strings.diagRingOverflowTotal, state.ringOverflowFrames.toString())
            KeyValueRow(strings.diagRingOverflowWindow, "+${state.ringOverflowSinceReset}")
            KeyValueRow(strings.diagRingUnderrunTotal, state.ringUnderruns.toString())
            KeyValueRow(strings.diagRingUnderrunWindow, "+${state.ringUnderrunsSinceReset}")
            KeyValueRow(strings.diagRingWindowSeconds, "${state.ringWindowSeconds} s")
            Button(
                onClick = { AudioLinkService.resetRingWindow() },
                modifier = Modifier.heightIn(min = 48.dp),
            ) {
                Text(strings.diagRingReset)
            }
            Text(
                text = strings.diagRingNote,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        // ── 输出队列水位（延迟杠杆） ────────────────────────────────
        DiagSection(title = strings.diagQueue) {
            KeyValueRow(
                strings.diagQueueCapacity,
                "${state.bufferCapacityFrames} ${strings.diagFramesUnit}（${framesToMs(state.bufferCapacityFrames)}）",
            )
            KeyValueRow(
                strings.diagQueueLevel,
                "${state.queuedFrames} ${strings.diagFramesUnit}（${framesToMs(state.queuedFrames)}）",
            )
            KeyValueRow(
                label = strings.diagQueueTarget,
                value = if (state.queueTargetFrames <= 0) {
                    strings.leverFull
                } else {
                    "${state.queueTargetFrames} ${strings.diagFramesUnit}（${framesToMs(state.queueTargetFrames)}）"
                },
            )
            KeyValueRow(
                label = strings.diagQueueShrink,
                value = if (state.shrinkGrantedFrames < 0) {
                    strings.diagQueueShrinkNone
                } else {
                    "${state.shrinkGrantedFrames} ${strings.diagFramesUnit}（${framesToMs(state.shrinkGrantedFrames)}）"
                },
            )
            SilkLabel(strings.diagQueueTargetLabel)
            ChoiceGroup(
                options = listOf(
                    Choice(0, strings.leverFull),
                    Choice(960, "20ms"),
                    Choice(1_440, "30ms"),
                    Choice(1_920, "40ms"),
                    Choice(2_880, "60ms"),
                ),
                selected = state.queueTargetFrames,
                onSelect = { AudioLinkService.setQueueTargetFrames(it) },
            )
            SilkLabel(strings.diagQueueShrinkLabel)
            ChoiceGroup(
                options = listOf(
                    Choice(960, "20ms"),
                    Choice(1_440, "30ms"),
                    Choice(1_920, "40ms"),
                    Choice(state.bufferCapacityFrames, strings.leverRestore),
                ),
                // 收缩是**一次性动作**，没有「当前值」可高亮 —— 结果看上面那行「容量收缩结果」。
                selected = null,
                onSelect = { AudioLinkService.requestBufferShrink(it) },
            )
            Text(
                text = strings.diagQueueNote,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        // 最近一次错误的**内核原文**：首屏只讲人话与恢复路径，原文在这里（产品原则 3 与 5）。
        if (state.lastError != null) {
            DiagSection(title = strings.errorKernelDetail) {
                Text(
                    text = state.lastError,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = strings.diagExplanation,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/** 帧数 → 毫秒文本。全链路固定 48 kHz，所以这是**精确换算**，不是估算。 */
private fun framesToMs(frames: Int): String =
    if (frames <= 0) "0.0 ms" else String.format(Locale.US, "%.1f ms", frames * 1000.0 / 48_000.0)
