package com.gotkicry.audiolink.ui.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/**
 * DESIGN.md 字阶 + Android 系统字体。读数使用等宽数字，状态刷新时不跳动。
 */
private val caption = TextStyle(fontSize = 12.sp, lineHeight = 16.sp, letterSpacing = 0.sp)
private val body = TextStyle(fontSize = 14.sp, lineHeight = 20.sp, letterSpacing = 0.sp)
private val subtitle = TextStyle(
    fontSize = 20.sp, lineHeight = 28.sp, letterSpacing = 0.sp, fontWeight = FontWeight.SemiBold,
)
private val title = TextStyle(
    fontSize = 28.sp, lineHeight = 36.sp, letterSpacing = 0.sp, fontWeight = FontWeight.SemiBold,
)

// DESIGN.md 的四个字阶；显式覆盖 M3，避免输入框和按钮回落到另一套字号。
internal val AudioLinkTypography = Typography(
    displayLarge = title, displayMedium = title, displaySmall = title,
    headlineLarge = title, headlineMedium = subtitle, headlineSmall = subtitle,
    titleLarge = subtitle,
    titleMedium = body.copy(fontWeight = FontWeight.SemiBold),
    titleSmall = body.copy(fontWeight = FontWeight.SemiBold),
    bodyLarge = body, bodyMedium = body, bodySmall = caption,
    labelLarge = body.copy(fontWeight = FontWeight.SemiBold),
    labelMedium = caption, labelSmall = caption,
)

/** 排版常量：只在组件层消费，别在屏幕里随手写 sp。 */
object AudioLinkType {

    /** Caption 辅助标签；名称兼容既有调用点。 */
    val silk = TextStyle(
        fontSize = 12.sp,
        lineHeight = 16.sp,
        letterSpacing = 0.sp,
        fontWeight = FontWeight.Normal,
    )

    /** Body 分区标题，600 字重。 */
    val silkLarge = TextStyle(
        fontSize = 14.sp,
        lineHeight = 20.sp,
        letterSpacing = 0.sp,
        fontWeight = FontWeight.SemiBold,
    )

    /** 读数（等宽 + 定宽数字）。 */
    val readout = TextStyle(
        fontSize = 12.sp,
        lineHeight = 16.sp,
        letterSpacing = 0.sp,
        fontFamily = FontFamily.Monospace,
        fontFeatureSettings = "tnum",
        fontWeight = FontWeight.Normal,
    )

    /** 关键读数（端到端延迟这类一眼要看到的数字）。 */
    val readoutLarge = TextStyle(
        fontSize = 20.sp,
        lineHeight = 28.sp,
        letterSpacing = 0.sp,
        fontFamily = FontFamily.Monospace,
        fontFeatureSettings = "tnum",
        fontWeight = FontWeight.SemiBold,
    )

    /** 状态大字（「接收中 / 未在接收」）。 */
    val stateHeadline = TextStyle(
        fontSize = 20.sp,
        lineHeight = 28.sp,
        letterSpacing = 0.sp,
        fontWeight = FontWeight.SemiBold,
    )
}
