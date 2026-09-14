package com.gotkicry.audiolink.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.audio.LowLatencyPlayer
import com.gotkicry.audiolink.diagnostics.ProtocolSelfTest
import com.gotkicry.audiolink.diagnostics.SelfTestResult
import com.gotkicry.audiolink.service.PlaybackUiState
import kotlinx.coroutines.launch

/**
 * M1 最小页面：服务启停 + 播放状态。
 *
 * 规格来源：`docs/11-m1-contract.md` §7 验收第 3 条（服务启停 + 状态显示：低延迟是否生效、
 * 采样率、声道数、缓冲帧数、欠载次数）。
 *
 * 设计立场：这是**观测面板**而不是"好看的首屏"。`docs/08-ui-spec.md` 的卡片化信息架构
 * 留在 M5 做；这里每个数字都直接对应一个验收口径，宁可朴素也不要"看起来很行"的假象 ——
 * 低延迟没生效就显示"未生效"并把 `getPerformanceMode()` 的原值一并摆出来。
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PlaybackScreen(
    state: PlaybackUiState,
    onTogglePlayback: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    // 自检状态刻意放在 UI 本地：它**不依赖服务**（验的是协议层），
    // 所以服务没启动、没连接、甚至没开网都能点 —— 这正是真机排障第一步需要的性质。
    var selfTestResult by remember { mutableStateOf<SelfTestResult?>(null) }
    var selfTestRunning by remember { mutableStateOf(false) }
    val uiScope = rememberCoroutineScope()

    Scaffold(
        modifier = modifier,
        topBar = { TopAppBar(title = { Text("AudioLink") }) },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(16.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            ServiceCard(state = state, onTogglePlayback = onTogglePlayback)
            SelfTestCard(
                result = selfTestResult,
                running = selfTestRunning,
                onRun = {
                    if (!selfTestRunning) {
                        selfTestRunning = true
                        uiScope.launch {
                            try {
                                selfTestResult = ProtocolSelfTest.run()
                            } finally {
                                selfTestRunning = false
                            }
                        }
                    }
                },
            )
            EngineCard(state = state)
            LowLatencyCard(state = state)
            StatsCard(state = state)
            RingCard(state = state)

            state.lastError?.let { message ->
                Card(
                    colors = CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.errorContainer,
                    ),
                ) {
                    Column(Modifier.fillMaxWidth().padding(16.dp)) {
                        Text("错误", fontWeight = FontWeight.Bold)
                        Text(message, style = MaterialTheme.typography.bodySmall)
                    }
                }
            }

            Text(
                "M1 说明：引擎随服务启停，本机角色是「接收端」（可接收、不可发送）。播放环里的 PCM 来自内核解码 ——" +
                    "PC 端尚未连接/推流时，供给欠载会持续增长、输出为静音，属预期；" +
                    "低延迟是否生效与缓冲帧数不受此影响，可直接读。",
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

@Composable
private fun ServiceCard(state: PlaybackUiState, onTogglePlayback: (Boolean) -> Unit) {
    Card {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                if (state.serviceRunning) "前台服务：运行中" else "前台服务：已停止",
                style = MaterialTheme.typography.titleMedium,
            )
            Text(
                if (state.playing) "播放：进行中" else "播放：已停止",
                color = MaterialTheme.colorScheme.primary,
            )
            Button(onClick = { onTogglePlayback(!state.serviceRunning) }) {
                Text(if (state.serviceRunning) "停止服务" else "启动并开始接收")
            }
        }
    }
}

@Composable
private fun SelfTestCard(result: SelfTestResult?, running: Boolean, onRun: () -> Unit) {
    Card {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text("内核自检（协议 golden vectors）", style = MaterialTheme.typography.titleSmall)
            when (result) {
                null -> Text(
                    "尚未自检。自检验的是协议编解码层，不依赖引擎与连接 —— 服务未启动也能跑。",
                    style = MaterialTheme.typography.bodySmall,
                )

                is SelfTestResult.Passed -> Text(
                    "✔ ${result.summary}",
                    color = MaterialTheme.colorScheme.primary,
                    style = MaterialTheme.typography.bodySmall,
                )

                is SelfTestResult.Rejected -> {
                    Text(
                        "✘ 内核拒绝",
                        color = MaterialTheme.colorScheme.error,
                        fontWeight = FontWeight.Bold,
                    )
                    KeyValue("错误码", result.code.toString())
                    KeyValue("短名", result.shortName)
                    KeyValue("细节", result.context)
                }

                is SelfTestResult.Unavailable -> {
                    Text(
                        "✘ 自检未能运行",
                        color = MaterialTheme.colorScheme.error,
                        fontWeight = FontWeight.Bold,
                    )
                    Text(result.reason, style = MaterialTheme.typography.bodySmall)
                }
            }
            Button(onClick = onRun, enabled = !running) {
                Text(if (running) "自检中…" else "运行自检")
            }
        }
    }
}

@Composable
private fun EngineCard(state: PlaybackUiState) {
    Card {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text("内核引擎（接收端）", style = MaterialTheme.typography.titleSmall)
            Text(
                if (state.engineRunning) "● 运行中（已监听 QUIC 端口）" else "○ 未启动",
                color = if (state.engineRunning) {
                    MaterialTheme.colorScheme.primary
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
                fontWeight = FontWeight.Bold,
            )
            if (state.engineRunning) {
                KeyValue("本机名称", state.engineName)
                KeyValue("指纹短码", state.engineIdShort)
                KeyValue("监听地址", state.engineAddr)
            }
            KeyValue(
                "能力",
                // 能力位由内核如实给出，Kotlin 侧不美化：M1 只有接收（没有 capture 工厂 → 不可发送）。
                if (state.engineCanSend) "✔ 接收 / ✔ 发送" else "✔ 接收 / ✘ 发送（M1 未实现）",
            )
            state.engineError?.let { message ->
                Text(
                    message,
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }
    }
}

@Composable
private fun LowLatencyCard(state: PlaybackUiState) {
    Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant)) {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text("低延迟", style = MaterialTheme.typography.titleSmall)
            Text(
                if (state.lowLatency) "✔ 已生效（PERFORMANCE_MODE_LOW_LATENCY）" else "✘ 未生效",
                color = if (state.lowLatency) {
                    MaterialTheme.colorScheme.primary
                } else {
                    MaterialTheme.colorScheme.error
                },
                fontWeight = FontWeight.Bold,
            )
            KeyValue("getPerformanceMode()", LowLatencyPlayer.performanceModeName(state.performanceMode))
            KeyValue("采样率", "${state.sampleRate} Hz")
            KeyValue("声道数", state.channelCount.toString())
            KeyValue("输出缓冲（请求）", "${state.requestedBufferFrames} 帧")
            KeyValue("输出缓冲（实际生效）", "${state.actualBufferFrames} 帧")
        }
    }
}

@Composable
private fun StatsCard(state: PlaybackUiState) {
    Card {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text("统计", style = MaterialTheme.typography.titleSmall)
            KeyValue("欠载（系统口径）", state.trackUnderruns.toString())
            KeyValue("欠载（供给口径）", state.sourceUnderruns.toString())
            KeyValue("已写入音频帧", state.framesWritten.toString())
            KeyValue("静音填充帧", state.silenceFrames.toString())
            KeyValue("丢弃帧", state.framesDropped.toString())
            KeyValue(
                "写入错误次数",
                if (state.writeErrors == 0) {
                    "0"
                } else {
                    "${state.writeErrors}（最近错误码 ${state.lastWriteError}）"
                },
            )
        }
    }
}

@Composable
private fun RingCard(state: PlaybackUiState) {
    Card {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text("播放环（内核 PCM 落地缓冲）", style = MaterialTheme.typography.titleSmall)
            KeyValue("水位", "${state.ringBufferedFrames} / ${state.ringCapacityFrames} 帧")
            KeyValue("溢出丢弃帧", state.ringOverflowFrames.toString())
            KeyValue("读空次数", state.ringUnderruns.toString())
        }
    }
}

/** 一行「标签 : 值」。数值用等宽字体，避免刷新时数字宽度跳动。 */
@Composable
private fun KeyValue(label: String, value: String) {
    Row(Modifier.fillMaxWidth()) {
        Text(label, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        Text(value, style = MaterialTheme.typography.bodyMedium, fontFamily = FontFamily.Monospace)
    }
}
