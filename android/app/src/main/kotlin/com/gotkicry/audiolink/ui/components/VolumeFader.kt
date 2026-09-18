package com.gotkicry.audiolink.ui.components

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioManager
import android.os.Build
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlin.math.roundToInt

/**
 * 系统音量变化的广播 action。
 *
 * 为什么写字符串字面量而不是 `AudioManager.VOLUME_CHANGED_ACTION`：后者在 SDK 里**不是公开 API**
 * （被 @hide），Kotlin 侧引用不到；而这条系统广播的 action 字符串本身是稳定契约。
 * 它只用于"用户按了音量键 → 推子跟着动"，且注册失败会被静默降级（见下），不影响主路径。
 */
private const val ACTION_VOLUME_CHANGED = "android.media.VOLUME_CHANGED_ACTION"

/**
 * 媒体音量控制器 —— **听筒端三件事里的第三件**（开始接收 / 停止接收 / 调音量）。
 *
 * 为什么调的是**系统媒体音量**（`STREAM_MUSIC`）而不是播放器内部增益：
 * 1. 本机播放走 `USAGE_MEDIA`，它本来就受这一条音量控制 —— 推子和音量键是**同一个真值**，
 *    用户按音量键、推子必须跟着动（否则界面上显示的就不是实际音量，那是在骗人）；
 * 2. 这是平台惯例：Android 上"调音量"就是调流音量，不引入 App 私有音量层；
 * 3. 它在 UI 层闭环，**不需要动 service 与音频链路**（服务层的契约本次刻意不动）。
 *
 * 值得记下的代价：改变的是整机媒体音量（会影响其它正在放音频的应用）。这是系统流音量的固有语义，
 * 不是本实现的选择。
 */
@Stable
class MediaVolumeController internal constructor(
    private val audioManager: AudioManager?,
    maxLevel: Int,
    initialLevel: Int,
) {
    /** 0..[maxLevel] 的档位（与 `getStreamVolume` 同一口径）。 */
    var level by mutableIntStateOf(initialLevel)
        private set

    val max: Int = if (maxLevel > 0) maxLevel else 1

    val fraction: Float get() = (level.toFloat() / max).coerceIn(0f, 1f)

    /** 百分比读数（0–100）。 */
    val percent: Int get() = (fraction * 100f).roundToInt()

    fun setFraction(value: Float) {
        val target = (value.coerceIn(0f, 1f) * max).roundToInt().coerceIn(0, max)
        level = target
        runCatching {
            audioManager?.setStreamVolume(AudioManager.STREAM_MUSIC, target, 0)
        }
    }

    internal fun syncFromSystem() {
        val current = runCatching {
            audioManager?.getStreamVolume(AudioManager.STREAM_MUSIC)
        }.getOrNull() ?: return
        if (current != level) level = current
    }
}

/**
 * 拿到媒体音量控制器：`ON_RESUME` 与系统 `VOLUME_CHANGED_ACTION` 都会重读，
 * 所以用户按音量键后推子会跟着走。
 */
@Composable
fun rememberMediaVolumeController(): MediaVolumeController {
    val context = LocalContext.current
    val audioManager = remember(context) { context.getSystemService(AudioManager::class.java) }
    val maxLevel = remember(audioManager) {
        runCatching { audioManager?.getStreamMaxVolume(AudioManager.STREAM_MUSIC) }.getOrNull() ?: 1
    }
    val initial = remember(audioManager) {
        runCatching { audioManager?.getStreamVolume(AudioManager.STREAM_MUSIC) }.getOrNull() ?: 0
    }
    val controller = remember(audioManager, maxLevel, initial) {
        MediaVolumeController(audioManager, maxLevel, initial)
    }

    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner, controller) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) controller.syncFromSystem()
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    DisposableEffect(context, controller) {
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(receiverContext: Context?, intent: Intent?) {
                if (intent?.action == ACTION_VOLUME_CHANGED) controller.syncFromSystem()
            }
        }
        // 注册失败（个别 ROM 会拒绝非导出 receiver）不是错误：退化成"只在回到前台时同步"。
        val registered = runCatching {
            val filter = IntentFilter(ACTION_VOLUME_CHANGED)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                context.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
            } else {
                @Suppress("UnspecifiedRegisterReceiverFlag")
                context.registerReceiver(receiver, filter)
            }
            true
        }.getOrDefault(false)
        onDispose {
            if (registered) runCatching { context.unregisterReceiver(receiver) }
        }
    }
    return controller
}

/**
 * 大号音量推子。
 *
 * 触控目标 **≥48 dp**：M3 的 Slider thumb 自带 `minimumInteractiveComponentSize`（48 dp 命中区），
 * 这里再把容器高度钉在 48 dp 以上 —— 拇指按下去是一次命中，不是一场精确手术。
 */
@Composable
fun VolumeFader(
    controller: MediaVolumeController,
    contentDescription: String,
    modifier: Modifier = Modifier,
) {
    Slider(
        value = controller.fraction,
        onValueChange = { controller.setFraction(it) },
        modifier = modifier
            .fillMaxWidth()
            .heightIn(min = 48.dp)
            .semantics { this.contentDescription = contentDescription },
        colors = SliderDefaults.colors(
            thumbColor = MaterialTheme.colorScheme.primary,
            activeTrackColor = MaterialTheme.colorScheme.primary,
            inactiveTrackColor = MaterialTheme.colorScheme.outline,
        ),
    )
}
