package com.gotkicry.audiolink.ui.components

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathBuilder
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.unit.dp

/**
 * 图标集中处 —— **整个 UI 层只有这一个文件知道图标从哪来**。
 *
 * 三条纪律：
 * 1. **不再用 Unicode 字符当图标**（旧版的 ✔ ✘ ● ○ ⚠ 在等宽或大字体下会变成奇怪的字形，
 *    TalkBack 还会把它念出来）；
 * 2. **不引入新依赖**：`material-icons-core` 在当前 Compose BOM 里已经不是 material3 的传递依赖
 *    （实测 `androidx.compose.material.icons` 解析不到），而 `material-icons-extended` 是几万个图标 ——
 *    APK 体积不可接受。所以这里**全部自绘**；
 * 3. 自绘的形状就是这台机器的语言：**方头、等宽线、无衬线**，与面板上的丝印同一套笔触。
 *
 * 笔画图（stroke）与实心图（fill）混用是刻意的：警示/勾叉是"丝印线条"，播放/停止是"键帽上的实心符号"。
 */
object AppIcons {

    /** 勾：两条斜线，方头。 */
    val Check: ImageVector = strokeIcon("Check") {
        moveTo(4.5f, 12.6f)
        lineTo(9.8f, 17.9f)
        lineTo(19.5f, 6.6f)
    }

    /** 叉：两条对角线。 */
    val Close: ImageVector = strokeIcon("Close") {
        moveTo(5.5f, 5.5f)
        lineTo(18.5f, 18.5f)
        moveTo(18.5f, 5.5f)
        lineTo(5.5f, 18.5f)
    }

    /** 信息：圈 + 竖杠 + 点。 */
    val Info: ImageVector = ImageVector.Builder(
        name = "Info",
        defaultWidth = 24.dp,
        defaultHeight = 24.dp,
        viewportWidth = 24f,
        viewportHeight = 24f,
    ).apply {
        path(
            stroke = SolidColor(Color.White),
            strokeLineWidth = 2.2f,
            strokeLineCap = StrokeCap.Square,
            pathBuilder = {
                moveTo(12f, 4f)
                arcToRelative(8f, 8f, 0f, false, true, 0f, 16f)
                arcToRelative(8f, 8f, 0f, false, true, 0f, -16f)
                close()
            },
        )
        path(fill = SolidColor(Color.White)) {
            moveTo(11.1f, 10.8f)
            lineTo(12.9f, 10.8f)
            lineTo(12.9f, 17f)
            lineTo(11.1f, 17f)
            close()
            moveTo(11.1f, 6.8f)
            lineTo(12.9f, 6.8f)
            lineTo(12.9f, 8.8f)
            lineTo(11.1f, 8.8f)
            close()
        }
    }.build()

    /** 警示：三角轮廓 + 感叹号。 */
    val Warning: ImageVector = ImageVector.Builder(
        name = "Warning",
        defaultWidth = 24.dp,
        defaultHeight = 24.dp,
        viewportWidth = 24f,
        viewportHeight = 24f,
    ).apply {
        path(
            stroke = SolidColor(Color.White),
            strokeLineWidth = 2f,
            strokeLineJoin = StrokeJoin.Round,
            pathBuilder = {
                moveTo(12f, 4.2f)
                lineTo(21.4f, 20f)
                lineTo(2.6f, 20f)
                close()
            },
        )
        path(fill = SolidColor(Color.White)) {
            moveTo(11.1f, 9.6f)
            lineTo(12.9f, 9.6f)
            lineTo(12.9f, 15.2f)
            lineTo(11.1f, 15.2f)
            close()
            moveTo(11.1f, 16.6f)
            lineTo(12.9f, 16.6f)
            lineTo(12.9f, 18.2f)
            lineTo(11.1f, 18.2f)
            close()
        }
    }.build()

    /** 播放：实心三角。 */
    val Play: ImageVector = ImageVector.Builder(
        name = "Play",
        defaultWidth = 24.dp,
        defaultHeight = 24.dp,
        viewportWidth = 24f,
        viewportHeight = 24f,
    ).apply {
        path(fill = SolidColor(Color.White)) {
            moveTo(7f, 4.5f)
            lineTo(20f, 12f)
            lineTo(7f, 19.5f)
            close()
        }
    }.build()

    /** 停止：实心方块。 */
    val Stop: ImageVector = ImageVector.Builder(
        name = "Stop",
        defaultWidth = 24.dp,
        defaultHeight = 24.dp,
        viewportWidth = 24f,
        viewportHeight = 24f,
    ).apply {
        path(fill = SolidColor(Color.White)) {
            moveTo(6f, 6f)
            lineTo(18f, 6f)
            lineTo(18f, 18f)
            lineTo(6f, 18f)
            close()
        }
    }.build()

    /** 展开指示：向下折线。 */
    val ChevronDown: ImageVector = strokeIcon("ChevronDown", strokeWidth = 2.4f) {
        moveTo(6f, 9.5f)
        lineTo(12f, 15.5f)
        lineTo(18f, 9.5f)
    }

    /** 收起指示：向上折线。 */
    val ChevronUp: ImageVector = strokeIcon("ChevronUp", strokeWidth = 2.4f) {
        moveTo(6f, 14.5f)
        lineTo(12f, 8.5f)
        lineTo(18f, 14.5f)
    }
}

/**
 * 自绘一个**线条图标**（丝印笔触：方头、方接）。
 *
 * 颜色一律给白色：`Icon` 会用它自己的 tint 盖上去（ColorFilter 作用在整个绘制上），
 * 所以这里的白色只是一个"会被替换的占位"。
 */
private fun strokeIcon(
    name: String,
    strokeWidth: Float = 2.4f,
    pathBuilder: PathBuilder.() -> Unit,
): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 24f,
    viewportHeight = 24f,
).apply {
    path(
        stroke = SolidColor(Color.White),
        strokeLineWidth = strokeWidth,
        strokeLineCap = StrokeCap.Square,
        strokeLineJoin = StrokeJoin.Round,
        pathBuilder = pathBuilder,
    )
}.build()
