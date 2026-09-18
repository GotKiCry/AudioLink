package com.gotkicry.audiolink.ui.theme

import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.graphics.Color

/**
 * 调色板（On-Air Console）—— **与桌面端 `desktop/src/index.css` 同一套物理灯色**。
 *
 * 世界血统：广播播出调音台 / 家用 Hi-Fi 前级。深色（黑面板）是默认，浅色是同一台机器的
 * 银面版本（silver-face receiver），**不是把深色反转**。
 *
 * 纪律：这里只放色值；语义映射（哪个角色配哪个色）在 [Theme.kt]。组件一律消费
 * `MaterialTheme.colorScheme` / `MaterialTheme.statusColors`，不许再写十六进制。
 */

// ── 深色（黑面板；默认，因为主要场景是夜里听音） ──────────────────────
private val DarkSurface = Color(0xFF1A1614)
private val DarkSurfaceLowest = Color(0xFF100D0C)
private val DarkSurfaceLow = Color(0xFF1A1614)
private val DarkSurfaceContainer = Color(0xFF241E1B)
private val DarkSurfaceHigh = Color(0xFF2A2320)
private val DarkSurfaceHighest = Color(0xFF322A26)
private val DarkOnSurface = Color(0xFFE8DCCF)
private val DarkOnSurfaceVariant = Color(0xFFBCAC9D)
private val DarkOutline = Color(0xFF4E4139)
private val DarkOutlineVariant = Color(0xFF3A302B)
private val DarkPrimary = Color(0xFFC9A227)
private val DarkOnPrimary = Color(0xFF17130F)
private val DarkPrimaryContainer = Color(0xFF3A2E0E)
private val DarkOnPrimaryContainer = Color(0xFFE5C158)
private val DarkTertiary = Color(0xFFE8542F)
private val DarkOnTertiary = Color(0xFF17130F)
private val DarkTertiaryContainer = Color(0xFF4A1B10)
private val DarkOnTertiaryContainer = Color(0xFFFF7A56)
private val DarkError = Color(0xFFE8542F)
private val DarkOnError = Color(0xFF17130F)
private val DarkErrorContainer = Color(0xFF4A1B10)
private val DarkOnErrorContainer = Color(0xFFFF7A56)

// ── 浅色（银面机：同一台机器的日间版本） ──────────────────────────────
private val LightSurface = Color(0xFFD5CFC5)
private val LightSurfaceLowest = Color(0xFFC6C0B6)
private val LightSurfaceLow = Color(0xFFD5CFC5)
private val LightSurfaceContainer = Color(0xFFE4DFD6)
private val LightSurfaceHigh = Color(0xFFEFEAE2)
private val LightSurfaceHighest = Color(0xFFF5F1EA)
private val LightOnSurface = Color(0xFF1F1A17)
private val LightOnSurfaceVariant = Color(0xFF4A4038)
private val LightOutline = Color(0xFF8E857A)
private val LightOutlineVariant = Color(0xFFAFA79B)
private val LightPrimary = Color(0xFF8A6D12)
private val LightOnPrimary = Color(0xFFFBF8F2)
private val LightPrimaryContainer = Color(0xFFE8D9A8)
private val LightOnPrimaryContainer = Color(0xFF3A2E0E)
private val LightTertiary = Color(0xFFC8441F)
private val LightOnTertiary = Color(0xFFFBF8F2)
/** 派生：M3 需要该角色，取自同一色相（浅橙底 + 深红字），不引入新色相。 */
private val LightTertiaryContainer = Color(0xFFF4DBD0)
private val LightOnTertiaryContainer = Color(0xFF4A1108)
private val LightError = Color(0xFFB33A18)
private val LightOnError = Color(0xFFFBF8F2)
/** 派生：同上，浅红底 + 深红字。 */
private val LightErrorContainer = Color(0xFFF5D9D1)
private val LightOnErrorContainer = Color(0xFF8A2A0E)

/**
 * 深色方案。**每一个角色都显式写出** —— 漏掉的角色会落到 M3 baseline 的紫色，
 * 那种紫色在这台机器上不属于任何东西。
 */
internal val AudioLinkDarkScheme = darkColorScheme(
    primary = DarkPrimary,
    onPrimary = DarkOnPrimary,
    primaryContainer = DarkPrimaryContainer,
    onPrimaryContainer = DarkOnPrimaryContainer,
    inversePrimary = LightPrimary,
    // secondary：金属灰（弱化的丝印），用于低强调的容器与控件。
    secondary = DarkOnSurfaceVariant,
    onSecondary = DarkOnPrimary,
    secondaryContainer = DarkSurfaceHigh,
    onSecondaryContainer = DarkOnSurface,
    tertiary = DarkTertiary,
    onTertiary = DarkOnTertiary,
    tertiaryContainer = DarkTertiaryContainer,
    onTertiaryContainer = DarkOnTertiaryContainer,
    // background = 机箱底盘（chassis）。面板（surfaceContainer）比它亮一档，
    // 卡片边界靠这个差 + 1 px 边框表达，不靠投影 —— 与桌面端 label/bench 的分层同一套。
    background = DarkSurfaceLowest,
    onBackground = DarkOnSurface,
    surface = DarkSurface,
    onSurface = DarkOnSurface,
    surfaceVariant = DarkSurfaceHighest,
    onSurfaceVariant = DarkOnSurfaceVariant,
    surfaceTint = DarkPrimary,
    surfaceBright = DarkSurfaceHighest,
    surfaceDim = DarkSurfaceLowest,
    inverseSurface = DarkOnSurface,
    inverseOnSurface = DarkSurface,
    error = DarkError,
    onError = DarkOnError,
    errorContainer = DarkErrorContainer,
    onErrorContainer = DarkOnErrorContainer,
    outline = DarkOutline,
    outlineVariant = DarkOutlineVariant,
    scrim = Color(0xFF000000),
    surfaceContainerLowest = DarkSurfaceLowest,
    surfaceContainerLow = DarkSurfaceLow,
    surfaceContainer = DarkSurfaceContainer,
    surfaceContainerHigh = DarkSurfaceHigh,
    surfaceContainerHighest = DarkSurfaceHighest,
)

/** 浅色方案（银面机）。同样全角色显式写出。 */
internal val AudioLinkLightScheme = lightColorScheme(
    primary = LightPrimary,
    onPrimary = LightOnPrimary,
    primaryContainer = LightPrimaryContainer,
    onPrimaryContainer = LightOnPrimaryContainer,
    inversePrimary = DarkPrimary,
    secondary = LightOnSurfaceVariant,
    onSecondary = LightOnPrimary,
    secondaryContainer = LightSurfaceHigh,
    onSecondaryContainer = LightOnSurface,
    tertiary = LightTertiary,
    onTertiary = LightOnTertiary,
    tertiaryContainer = LightTertiaryContainer,
    onTertiaryContainer = LightOnTertiaryContainer,
    background = LightSurfaceLowest,
    onBackground = LightOnSurface,
    surface = LightSurface,
    onSurface = LightOnSurface,
    surfaceVariant = LightSurfaceHighest,
    onSurfaceVariant = LightOnSurfaceVariant,
    surfaceTint = LightPrimary,
    surfaceBright = LightSurfaceHighest,
    surfaceDim = LightSurfaceLowest,
    inverseSurface = LightOnSurface,
    inverseOnSurface = LightSurface,
    error = LightError,
    onError = LightOnError,
    errorContainer = LightErrorContainer,
    onErrorContainer = LightOnErrorContainer,
    outline = LightOutline,
    outlineVariant = LightOutlineVariant,
    scrim = Color(0xFF000000),
    surfaceContainerLowest = LightSurfaceLowest,
    surfaceContainerLow = LightSurfaceLow,
    surfaceContainer = LightSurfaceContainer,
    surfaceContainerHigh = LightSurfaceHigh,
    surfaceContainerHighest = LightSurfaceHighest,
)
