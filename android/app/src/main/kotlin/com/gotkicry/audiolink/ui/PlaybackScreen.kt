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
import androidx.compose.material3.ButtonDefaults
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.gotkicry.audiolink.audio.LowLatencyPlayer
import com.gotkicry.audiolink.diagnostics.ProtocolSelfTest
import com.gotkicry.audiolink.diagnostics.SelfTestResult
import com.gotkicry.audiolink.service.AudioLinkService
import com.gotkicry.audiolink.service.PeerUi
import com.gotkicry.audiolink.service.PlaybackUiState
import java.util.Locale
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
            // 有 PIN 时它排在最前：此刻用户唯一该做的动作就是把这 6 位数字打进电脑端。
            // 排在服务/自检卡片后面会让"现在要配对了"被埋进滚动区 —— 而配对是个有时限的窗口
            // （§5：PIN 60 s 有效），错过就得让对端重连。
            state.pairingPin?.let { pin ->
                PairingCard(pin = pin, stale = state.pinIsStale, note = state.pairingNote)
            }
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
            if (state.peers.isNotEmpty()) {
                PeersCard(peers = state.peers)
            }
            LowLatencyCard(state = state)
            StatsCard(state = state)
            RingCard(state = state)
            QueueLeverCard(state = state)

            // 取配对状态失败（不是"没有配对"）：有 PIN 时它已经在 PIN 卡里显示了，这里只在没有 PIN 时兜底，
            // 免得同一句话出现两次。
            if (state.pairingNote != null && state.pairingPin == null) {
                Card(
                    colors = CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.errorContainer,
                    ),
                ) {
                    Column(Modifier.fillMaxWidth().padding(16.dp)) {
                        Text("配对状态读取失败", fontWeight = FontWeight.Bold)
                        Text(state.pairingNote, style = MaterialTheme.typography.bodySmall)
                    }
                }
            }

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

/**
 * 配对 PIN 卡（`docs/03-protocol.md` §5）。
 *
 * 展示立场与其他卡片一致：**原值直出**。PIN 由内核的 `PinGate` 生成，外壳**不做任何加工**
 * （不补零、不截断、不分组"美化"）—— 内核返回什么就显示什么，免得读的人和内核里的真值不是同一个数。
 *
 * 字号 56sp + 等宽 + 字距 8sp：这一屏的用途是"人眼读 6 位数字再打到电脑上"，
 * 任何"看起来精致但读起来费劲"的排版都是负分。背景用 `primaryContainer` 与观测面板区分开
 * —— 其他卡片都是"看"，这张是"要做动作"。
 */
@Composable
private fun PairingCard(pin: String, stale: Boolean, note: String?) {
    // 失效态换个底色：不是"藏起来"，而是"别照着它输"。
    val container = if (stale) {
        MaterialTheme.colorScheme.surfaceVariant
    } else {
        MaterialTheme.colorScheme.primaryContainer
    }
    Card(colors = CardDefaults.cardColors(containerColor = container)) {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                if (stale) "配对请求（已失效）" else "配对请求：在电脑端输入这 6 位数字",
                style = MaterialTheme.typography.titleMedium,
                color = if (stale) MaterialTheme.colorScheme.onSurfaceVariant else Color.Unspecified,
            )
            Text(
                pin,
                fontSize = 56.sp,
                fontWeight = FontWeight.Bold,
                fontFamily = FontFamily.Monospace,
                letterSpacing = 8.sp,
                color = if (stale) {
                    MaterialTheme.colorScheme.onSurfaceVariant
                } else {
                    MaterialTheme.colorScheme.onPrimaryContainer
                },
            )
            Text(
                if (stale) {
                    "⚠ 当前没有等待配对的对端 —— 这个 PIN 来自上一次连接，内核侧的 PinGate 已经随连接销毁，" +
                        "照着输必然失败。请让对方重新连接，本卡会自动换成新的 PIN。"
                } else {
                    "PIN 由内核生成且限时有效（§5：60 s 过期、失败多次短时锁定）。配对成功后本卡自动消失；" +
                        "对端主动断开则 PIN 作废，需重新连接。"
                },
                style = MaterialTheme.typography.bodySmall,
            )
            note?.let {
                Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}

/**
 * 对端列表（M1 接收端最需要的一眼信息）。
 *
 * 为什么**同时**显示自报名与短指纹：名字是对端自己写的（内核文档明说"不参与信任判定"），
 * 光看名字判断不了"连上的是不是我要的那台机器"；短指纹来自证书 SHA-256，才是信任库里的锚点。
 * `trusted` 一列直接读内核的信任位 —— 验收时判断"配对是否真的落盘"就看它，
 * 外壳不自己推断（不因为"连上了"就显示已信任）。
 */
@Composable
private fun PeersCard(peers: List<PeerUi>) {
    Card {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("对端（${peers.size}）", style = MaterialTheme.typography.titleSmall)
            peers.forEach { peer ->
                Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(peer.name, fontWeight = FontWeight.Bold)
                    KeyValue("短指纹", peer.idShort)
                    KeyValue("地址", peer.addr)
                    KeyValue("状态", peer.stateLabel)
                    KeyValue(
                        "信任",
                        if (peer.trusted) "✔ 已信任（信任库已落盘）" else "✘ 未信任（还需 PIN 配对）",
                    )
                }
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
            KeyValue("溢出丢弃帧（累计）", state.ringOverflowFrames.toString())
            KeyValue("溢出丢弃帧（区间）", "+${state.ringOverflowSinceReset}")
            KeyValue("读空次数（累计）", state.ringUnderruns.toString())
            KeyValue("读空次数（区间）", "+${state.ringUnderrunsSinceReset}")
            KeyValue("区间时长", "${state.ringWindowSeconds} s")
            Button(onClick = { AudioLinkService.resetRingWindow() }) { Text("区间清零") }
            Text(
                "区间增量 = 自上次清零以来的增量（前后各读一次相减）—— 用它区分「历史上丢过一大段」与「现在正在持续丢」；" +
                    "累计值不受清零影响（口径不许为了好看而改）。",
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

/**
 * 输出队列水位控制卡（task-8 的延迟杠杆）。
 *
 * 两条路各自独立、都在运行时切：
 *  · 路② [QUEUE_TARGET_CHOICES]：控**注水深度** —— 队列里只保持目标帧数，多了不写；
 *  · 路① 容量收缩：`setBufferSizeInFrames` 直接把**容量上限**压下去（丢了 FAST 就点「恢复容量」回滚）。
 *
 * 为什么做成可切的档位而不是写死一个值：低延迟与抗抖动是**取舍**，取舍必须在真机上量出来。
 * 同一条连接里前后各切一段，比「改一次、装一次、配一次对」干净得多 —— 后者会引入环境漂移。
 */
@Composable
private fun QueueLeverCard(state: PlaybackUiState) {
    Card {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("输出队列水位（task-8 延迟杠杆）", style = MaterialTheme.typography.titleSmall)
            KeyValue("设备容量", "${state.bufferCapacityFrames} 帧（${framesToMs(state.bufferCapacityFrames)}）")
            KeyValue("设备队列水位", "${state.queuedFrames} 帧（${framesToMs(state.queuedFrames)}）")
            KeyValue(
                "队列目标",
                if (state.queueTargetFrames <= 0) {
                    "满灌（A/B 对照侧）"
                } else {
                    "${state.queueTargetFrames} 帧（${framesToMs(state.queueTargetFrames)}）"
                },
            )
            KeyValue(
                "容量收缩结果",
                if (state.shrinkGrantedFrames < 0) {
                    "未执行"
                } else {
                    "${state.shrinkGrantedFrames} 帧（${framesToMs(state.shrinkGrantedFrames)}）"
                },
            )
            Text("路② 队列目标（控注水深度，下一拍生效）", style = MaterialTheme.typography.bodySmall)
            LeverRow(
                options = QUEUE_TARGET_CHOICES,
                selected = state.queueTargetFrames,
                onPick = { AudioLinkService.setQueueTargetFrames(it) },
            )
            Text(
                "路① 容量收缩（play() 后 setBufferSizeInFrames；丢了 FAST 就点「恢复容量」回滚）",
                style = MaterialTheme.typography.bodySmall,
            )
            LeverRow(
                options = listOf(
                    "20ms" to 960,
                    "30ms" to 1_440,
                    "40ms" to 1_920,
                    "恢复容量" to state.bufferCapacityFrames,
                ),
                // 收缩是**一次性动作**，没有「当前值」可高亮 —— 结果看上面那行「容量收缩结果」。
                selected = -1,
                onPick = { AudioLinkService.requestBufferShrink(it) },
            )
            Text(
                "容量是「最多能压多少」，水位是「现在压了多少」—— 低延迟模式生效 ≠ 缓冲小：" +
                    "真机上容量 3844 帧（80.1 ms）而队列被灌满，flinger 才会报 Latency=101 ms。",
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

/** 队列目标档位（帧 @48 kHz）：0 = 满灌（对照），其余 = 20/30/40/60 ms。 */
private val QUEUE_TARGET_CHOICES = listOf(
    "满灌" to 0,
    "20ms" to 960,
    "30ms" to 1_440,
    "40ms" to 1_920,
    "60ms" to 2_880,
)

/** 一排骨牌档位按钮；[selected] 命中的那个打勾（-1 = 没有可高亮的当前值）。 */
@Composable
private fun LeverRow(options: List<Pair<String, Int>>, selected: Int, onPick: (Int) -> Unit) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        options.forEach { (label, value) ->
            val active = value == selected
            Button(
                onClick = { onPick(value) },
                colors = if (active) {
                    ButtonDefaults.buttonColors()
                } else {
                    ButtonDefaults.outlinedButtonColors()
                },
            ) {
                Text(if (active) "✔ $label" else label, style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}

/** 帧数 → 毫秒文本。全链路固定 48 kHz，所以这是**精确换算**，不是估算。 */
private fun framesToMs(frames: Int): String =
    if (frames <= 0) "0.0 ms" else String.format(Locale.US, "%.1f ms", frames * 1000.0 / 48_000.0)

/** 一行「标签 : 值」。数值用等宽字体，避免刷新时数字宽度跳动。 */
@Composable
private fun KeyValue(label: String, value: String) {
    Row(Modifier.fillMaxWidth()) {
        Text(label, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        Text(value, style = MaterialTheme.typography.bodyMedium, fontFamily = FontFamily.Monospace)
    }
}
