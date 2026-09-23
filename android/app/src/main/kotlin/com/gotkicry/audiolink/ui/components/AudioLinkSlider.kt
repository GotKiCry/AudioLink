package com.gotkicry.audiolink.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.ui.theme.controlColors

/**
 * 全 App 唯一的滑块（Fluent 2 几何）。
 *
 * **为什么只剩这一个滑块**：总音量归系统。本机播放走 `USAGE_MEDIA`，它本来就被媒体流音量
 * 控制，而那条音量的入口是**手机音量键** —— App 再摆一个"总音量"推子就是维护第二个真值：
 * 两处都能让声音变小，用户只会猜"该拖哪一个"。所以界面上的音量只剩一处：设备通道里每台
 * 主机**这一路**的本地增益（0–200%，只影响本机混音，一个字节都不出网，见 DeviceDeck）。
 *
 * 形状与配色按 **Fluent 2** 而不是 M3 默认：
 * * **拇指是圆点**（16 dp + 1 px 内环 + 1 px 外描边）。M3 新版 `Slider` 的拇指是一根**竖条**
 *   （pill），那是 M3 的语言；Fluent 基准里滑块拇指就是一个圆点。这里用 `thumb` 参数
 *   自己画，把它换回来。
 * * **轨道 4 dp**（`track` 参数自绘），pill 形；已填充段 accent，未填充段用 Fluent
 *   `ControlStrongFillDefault`（**不再**是 `outline` —— 后者在深色 `#2B2B2B` 上只有
 *   2.29:1，低于非文字 UI 的 3:1 门槛，见 DESIGN.md §Colors「状态灯底座」）。
 *
 * 触控目标 **≥48 dp**：M3 的 Slider 自带 `minimumInteractiveComponentSize`（48 dp 命中区），
 * 这里再把容器高度钉在 48 dp 以上 —— 拇指按下去是一次命中，不是一场精确手术。
 * 视觉上轨道只有 4 dp、拇指 16 dp，**命中区仍然是 48 dp**，没有为了好看把目标做小。
 *
 * `@OptIn(ExperimentalMaterial3Api::class)` 是必须的：带 `thumb`/`track` 两个 lambda 的
 * `Slider` 重载在 material3 1.4.0 上仍是实验 API。这里认下这个 opt-in，是因为**不用它
 * 就没有别的办法换掉拇指形状** —— M3 的 `SliderDefaults.Thumb` 是竖条，而基准要圆点。
 * 用到的只有这两个 lambda 参数，`Slider` 的其它行为（手势、涟漪、无障碍语义、48 dp 命中区）
 * 全部走稳定路径。
 */

// ── Fluent 滑块几何（DESIGN.md §Components「滑块」）──────────────────────
/** 轨道：pill 形，**4 dp 高**（M3 默认是 16 dp 的粗轨）。 */
private val TrackHeight = 4.dp

/** 拇指本体：**16 dp 圆点**。 */
private val ThumbDiameter = 16.dp

/** 拇指含 1 px 外描边的外径。 */
private val ThumbOuterDiameter = 18.dp

/** 禁用态的降权系数：与 M3 的 disabled 内容色（38%）对齐，别自己发明一套。 */
private const val DisabledAlpha = 0.38f

/**
 * 唯一的滑块：设备通道里每一路本机增益用它（0–200%）。
 *
 * 几何与配色见文件头。`enabled` 由调用方决定 —— 没有可作用的对象时明确灰掉，
 * 比"能拖但毫无作用"更诚实。
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AudioLinkSlider(
    value: Float,
    onValueChange: (Float) -> Unit,
    contentDescription: String,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    val controls = MaterialTheme.controlColors
    val accent = MaterialTheme.colorScheme.primary
    val fraction = value.coerceIn(0f, 1f)

    Slider(
        value = fraction,
        onValueChange = onValueChange,
        enabled = enabled,
        modifier = modifier
            .fillMaxWidth()
            .heightIn(min = 48.dp)
            .semantics { this.contentDescription = contentDescription },
        // colors 仍显式给全：自绘的 thumb/track 不吃它，但 Slider 内部（涟漪、禁用态、
        // 无障碍节点的取色）仍然读它，留着才不会有 M3 默认的紫色冒出来。
        colors = SliderDefaults.colors(
            thumbColor = accent,
            activeTrackColor = accent,
            inactiveTrackColor = controls.strongFill,
        ),
        thumb = { FluentThumb(enabled = enabled) },
        track = { FluentTrack(fraction = fraction, enabled = enabled) },
    )
}

/** Fluent 拇指：16 dp 圆点 + 1 px 内环（`ControlStrokeColorDefault`）+ 1 px 外描边（`ControlStrongStroke`）。 */
@Composable
private fun FluentThumb(enabled: Boolean) {
    val controls = MaterialTheme.controlColors
    // 禁用时整颗拇指降权成中性灰：自绘的 thumb **不吃** Slider 的 colors，所以这里必须自己认 enabled ——
    // 否则"拖不动却长得像能拖"照样会骗到用户（真机评审 P1）。
    val ink = MaterialTheme.colorScheme.onSurface.copy(alpha = DisabledAlpha)
    val accent = if (enabled) MaterialTheme.colorScheme.primary else ink
    val outer = if (enabled) controls.strongFill else ink
    Box(
        modifier = Modifier
            .size(ThumbOuterDiameter)
            .clip(CircleShape)
            .background(outer),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            modifier = Modifier
                .size(ThumbDiameter)
                .clip(CircleShape)
                .background(accent)
                .border(1.dp, controls.subtleStroke, CircleShape),
        )
    }
}

/**
 * Fluent 轨道：4 dp 高的 pill；已填充段 accent，未填充段 `ControlStrongFillDefault`。
 *
 * `wrapContentHeight(Alignment.CenterVertically)` 是必需的：M3 给 track 的容器比 4 dp 高，
 * 不加这一句轨道会贴在容器顶部（视觉上推子偏上）。
 *
 * 进度直接用外部传进来的 [fraction]，**不读** `SliderState` —— 少一个版本相关的 API 依赖，
 * 也让这里画的和拇指所在的位置来自同一个真值（调用方传进 [AudioLinkSlider] 的 `value`）。
 */
@Composable
private fun FluentTrack(fraction: Float, enabled: Boolean) {
    val controls = MaterialTheme.controlColors
    // 同上：自绘的 track 也不吃 Slider 的 colors，禁用态要在这里自己画出来。
    val ink = MaterialTheme.colorScheme.onSurface.copy(alpha = DisabledAlpha)
    val fill = if (enabled) controls.strongFill else ink.copy(alpha = DisabledAlpha / 2f)
    val progress = if (enabled) MaterialTheme.colorScheme.primary else ink
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .wrapContentHeight(Alignment.CenterVertically)
            .height(TrackHeight)
            .clip(CircleShape)
            .background(fill),
    ) {
        Box(
            modifier = Modifier
                .fillMaxWidth(fraction.coerceIn(0f, 1f))
                .fillMaxHeight()
                .clip(CircleShape)
                .background(progress),
        )
    }
}
