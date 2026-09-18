package com.gotkicry.audiolink.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.ui.theme.LocalReducedMotion
import com.gotkicry.audiolink.ui.theme.statusColors

/** 四种灯态 —— 与桌面端的 `--t-lamp-*` 一一对应。 */
enum class LampTone { On, Warn, Live, Idle }

/** 灯/填充色。 */
@Composable
fun LampTone.lampColor(): Color = when (this) {
    LampTone.On -> MaterialTheme.statusColors.on
    LampTone.Warn -> MaterialTheme.statusColors.warn
    LampTone.Live -> MaterialTheme.statusColors.live
    LampTone.Idle -> MaterialTheme.statusColors.idle
}

/** 同灯态下**文字**用的色（对比度 ≥4.5:1；灯色本身不够黑底白字也读不清）。 */
@Composable
fun LampTone.inkColor(): Color = when (this) {
    LampTone.On -> MaterialTheme.statusColors.onInk
    LampTone.Warn -> MaterialTheme.statusColors.warnInk
    LampTone.Live -> MaterialTheme.statusColors.liveInk
    LampTone.Idle -> MaterialTheme.statusColors.idleInk
}

/**
 * 钨丝指示灯的圆点。
 *
 * **全屏唯一允许的持续动画是它的呼吸**（≤1 Hz）：[breathing] = true 时做 1.6 s 周期的明暗循环，
 * 用来表达"正在通流"。系统关掉动画（`ANIMATOR_DURATION_SCALE = 0`）时它会**真的静止**。
 *
 * 控件本身 **decorative**（无 contentDescription）：状态永远由旁边的图标 + 文字承载，
 * 灯只是余光可捕捉的那一层。这样"状态不靠颜色单独传达"在任何情况下都成立。
 */
@Composable
fun StatusLamp(
    tone: LampTone,
    modifier: Modifier = Modifier,
    breathing: Boolean = false,
    diameter: Dp = 12.dp,
) {
    val reducedMotion = LocalReducedMotion.current
    val color = tone.lampColor()
    val alpha = if (breathing && !reducedMotion) {
        val transition = rememberInfiniteTransition(label = "statusLamp")
        val pulse by transition.animateFloat(
            initialValue = 0.35f,
            targetValue = 1f,
            animationSpec = infiniteRepeatable(
                animation = tween(durationMillis = 1600, easing = LinearEasing),
                repeatMode = RepeatMode.Reverse,
            ),
            label = "statusLampAlpha",
        )
        pulse
    } else {
        1f
    }
    Box(
        modifier = modifier.size(diameter + 8.dp),
        contentAlignment = Alignment.Center,
    ) {
        // 光晕：灯的"周围有点亮"那一圈
        Box(
            Modifier
                .size(diameter + 8.dp)
                .clip(CircleShape)
                .background(color.copy(alpha = 0.16f * alpha)),
        )
        Box(
            Modifier
                .size(diameter)
                .clip(CircleShape)
                .background(color.copy(alpha = alpha)),
        )
    }
}

/**
 * **三通道状态**：灯 + 图标 + 文字。任何一条通道单独失效（色盲、灰度打印、TalkBack）时，
 * 状态仍然成立 —— 这是硬门槛，不是加分项。
 */
@Composable
fun StatusBadge(
    tone: LampTone,
    icon: ImageVector,
    text: String,
    modifier: Modifier = Modifier,
    breathing: Boolean = false,
    textStyle: TextStyle = MaterialTheme.typography.bodyMedium,
) {
    Row(
        modifier = modifier,
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        StatusLamp(tone = tone, breathing = breathing)
        Icon(
            imageVector = icon,
            contentDescription = null, // decorative：语义由文字承载
            tint = tone.inkColor(),
            modifier = Modifier.size(18.dp),
        )
        Text(text = text, style = textStyle, color = tone.inkColor())
    }
}
