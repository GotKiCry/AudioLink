package com.gotkicry.audiolink.ui.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/**
 * 排版：**M3 type scale + 系统 Roboto**（不引入新字体 —— APK 体积敏感）。
 *
 * 丝印感（广播面板上那种压印大写标签）不靠字体，靠三件事：
 * 大写化 + `letterSpacing 0.11–0.14em` + labelSmall/labelMedium 角色。
 * 读数/指纹/地址/延迟一律等宽 + `tnum`，这样每 500 ms 刷新时数字不会左右跳。
 */
internal val AudioLinkTypography = Typography()

/** 排版常量：只在组件层消费，别在屏幕里随手写 sp。 */
object AudioLinkType {

    /** 丝印小标签（卡片标题、分区标签）。11 sp / 字距 1.4 sp ≈ 0.13 em。 */
    val silk = TextStyle(
        fontSize = 11.sp,
        lineHeight = 14.sp,
        letterSpacing = 1.4.sp,
        fontWeight = FontWeight.Medium,
    )

    /** 大号丝印标签（卡片主标题）。 */
    val silkLarge = TextStyle(
        fontSize = 12.sp,
        lineHeight = 16.sp,
        letterSpacing = 1.5.sp,
        fontWeight = FontWeight.SemiBold,
    )

    /** 读数（等宽 + 定宽数字）。 */
    val readout = TextStyle(
        fontSize = 14.sp,
        lineHeight = 20.sp,
        letterSpacing = 0.sp,
        fontFamily = FontFamily.Monospace,
        fontFeatureSettings = "tnum",
        fontWeight = FontWeight.Medium,
    )

    /** 关键读数（端到端延迟这类一眼要看到的数字）。 */
    val readoutLarge = TextStyle(
        fontSize = 18.sp,
        lineHeight = 24.sp,
        letterSpacing = 0.sp,
        fontFamily = FontFamily.Monospace,
        fontFeatureSettings = "tnum",
        fontWeight = FontWeight.Medium,
    )

    /** 状态大字（「接收中 / 未在接收」）。 */
    val stateHeadline = TextStyle(
        fontSize = 24.sp,
        lineHeight = 30.sp,
        letterSpacing = 0.sp,
        fontWeight = FontWeight.Medium,
    )
}
