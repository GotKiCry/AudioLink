package com.gotkicry.audiolink.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.unit.dp

/** DESIGN.md 的表面与控件 alias。与语义状态色分开，避免拿成功/警告色装饰控件。 */
@Immutable
data class FluentColors(
    val stroke: Color,
    val highlight: Color,
    val lowerEdge: Color,
    val controlFill: Color,
    val inputFill: Color,
    val inputActive: Color,
)

val MaterialTheme.fluentColors: FluentColors
    @Composable get() {
        val colors = colorScheme
        val dark = colors.background.luminance() < 0.5f
        return FluentColors(
            stroke = colors.onSurface.copy(alpha = if (dark) 0.08f else 0.06f),
            highlight = Color.White.copy(alpha = if (dark) 0.10f else 0.65f),
            lowerEdge = Color.Black.copy(alpha = if (dark) 0.34f else 0.08f),
            // WinUI ControlFillColorDefault：深白 5.9%、浅白 70.2%。
            controlFill = Color.White.copy(alpha = if (dark) 0.059f else 0.702f),
            inputFill = colors.surfaceContainerLowest,
            inputActive = colors.surface,
        )
    }

/** 小圆角表面的内边高光，使用真实色阶而非实时玻璃；无持续 GPU 动画。 */
@Composable
fun Modifier.fluentEdges(inset: Float = 8f): Modifier {
    val colors = MaterialTheme.fluentColors
    return drawWithContent {
        drawContent()
        val edge = 0.5.dp.toPx()
        val start = inset.dp.toPx().coerceAtMost(size.width / 2)
        drawLine(colors.highlight, Offset(start, edge / 2), Offset(size.width - start, edge / 2), edge)
        drawLine(colors.lowerEdge, Offset(start, size.height - edge / 2), Offset(size.width - start, size.height - edge / 2), edge)
    }
}
