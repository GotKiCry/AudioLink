package com.gotkicry.audiolink.ui.theme

import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.graphics.Color

/**
 * 调色板 —— **Fluent 2（Windows 11 口径）**，与 `desktop/src/index.css` 的令牌同值。
 *
 * 权威：`DESIGN.md` §Colors（语义令牌表）+ §Migration & Gaps 的
 * 「M3 角色 → 基准令牌（实施对照）」逐角色映射表。两端共享同一套状态语义与色值：
 * 同一盏灯，两种外壳。上一轮的自研暖调（On-Air Console：铜金 / 橙红 / 暖黑）已退役。
 *
 * 纪律：
 *  - 深色底不用纯黑（`window #202020` 档的中性灰），浅色底不用纯白（`surface #FBFBFB`）；
 *  - 语义色只表达状态，不做装饰、不做分类；
 *  - 这里只放色值；语义映射（哪个角色配哪个色）在 [Theme.kt]。组件一律消费
 *    `MaterialTheme.colorScheme` / `MaterialTheme.statusColors`，不许再写十六进制。
 */

// ── 深色（默认；夜间听音是主场景） ──────────────────────────────────
private val DarkBackground = Color(0xFF1A1A1A) // sunken
private val DarkSurface = Color(0xFF202020) // window
private val DarkSurfaceLowest = Color(0xFF1A1A1A) // sunken
private val DarkSurfaceLow = Color(0xFF202020) // window
private val DarkSurfaceContainer = Color(0xFF2B2B2B) // surface
private val DarkSurfaceHigh = Color(0xFF323232) // surface-2
private val DarkSurfaceHighest = Color(0xFF3D3D3D) // Fluent grey-24
private val DarkOnSurface = Color(0xFFFFFFFF) // text
private val DarkOnSurfaceVariant = Color(0xFFC5C5C5) // text-2
private val DarkOutline = Color(0xFF616161) // Fluent grey-38
private val DarkOutlineVariant = Color(0xFF3D3D3D) // Fluent grey-24
private val DarkPrimary = Color(0xFF4CC2FF) // accent
private val DarkOnPrimary = Color(0xFF003A5C) // accent-on
private val DarkPrimaryContainer = Color(0xFF0F548C) // brandWeb[60]
// brandWeb[110] 原值 #62ABF5 在 primaryContainer(#0F548C) 上只有 3.25:1 —— 而这一对
// 是配对卡与省电白名单卡的正文（见 DESIGN.md §Colors「配对法则（血泪）」）。
// 按实际配对取 brandWeb 浅档 #9CD3FF → 4.94:1，过正文门槛。
private val DarkOnPrimaryContainer = Color(0xFF9CD3FF)
private val DarkSecondary = Color(0xFFC5C5C5) // text-2
private val DarkOnSecondary = Color(0xFF003A5C) // accent-on
private val DarkSecondaryContainer = Color(0xFF323232) // surface-2
private val DarkOnSecondaryContainer = Color(0xFFFFFFFF) // text
private val DarkTertiary = Color(0xFF479EF5) // brandWeb[100]
private val DarkOnTertiary = Color(0xFF003A5C) // accent-on
private val DarkTertiaryContainer = Color(0xFF0C3B5E) // brandWeb[40]
private val DarkOnTertiaryContainer = Color(0xFF77B7F7) // brandWeb[120]
private val DarkError = Color(0xFFFF99A4) // danger（= 状态色 live）
private val DarkOnError = Color(0xFF442726) // critical-bg
private val DarkErrorContainer = Color(0xFF442726) // critical-bg
private val DarkOnErrorContainer = Color(0xFFFF99A4) // danger
private val DarkInverseSurface = Color(0xFFFFFFFF) // text
private val DarkInverseOnSurface = Color(0xFF202020) // window
private val DarkInversePrimary = Color(0xFF0F6CBD) // accent（另一主题）

// ── 浅色（同一台机器的日间版本，不是深色的反转） ────────────────────
private val LightBackground = Color(0xFFF3F3F3) // window
private val LightSurface = Color(0xFFFBFBFB) // surface
private val LightSurfaceLowest = Color(0xFFEDEDED) // sunken
private val LightSurfaceLow = Color(0xFFF9F9F9) // chrome
private val LightSurfaceContainer = Color(0xFFFBFBFB) // surface
private val LightSurfaceHigh = Color(0xFFF5F5F5) // surface-2
private val LightSurfaceHighest = Color(0xFFEDEDED) // Fluent grey-92
private val LightOnSurface = Color(0xFF1B1B1B) // text
private val LightOnSurfaceVariant = Color(0xFF616161) // text-2
private val LightOutline = Color(0xFF8A8A8A) // text-3
private val LightOutlineVariant = Color(0xFFD1D1D1) // Fluent grey-82
private val LightPrimary = Color(0xFF0F6CBD) // accent
private val LightOnPrimary = Color(0xFFFFFFFF) // accent-on
private val LightPrimaryContainer = Color(0xFFDCE9F7) // 派生（浅色无官方值）
private val LightOnPrimaryContainer = Color(0xFF0F548C) // brandWeb[80]
private val LightSecondary = Color(0xFF616161) // text-2
private val LightOnSecondary = Color(0xFFFFFFFF) // accent-on
private val LightSecondaryContainer = Color(0xFFF5F5F5) // surface-2
private val LightOnSecondaryContainer = Color(0xFF1B1B1B) // text
private val LightTertiary = Color(0xFF115EA3) // brandWeb[70]
private val LightOnTertiary = Color(0xFFFFFFFF) // accent-on
private val LightTertiaryContainer = Color(0xFFDCE9F7) // 派生
private val LightOnTertiaryContainer = Color(0xFF0C3B5E) // brandWeb[40]
private val LightError = Color(0xFFC42B1C) // danger（= 状态色 live）
private val LightOnError = Color(0xFFFFFFFF) // accent-on
private val LightErrorContainer = Color(0xFFFDE7E9) // critical-bg
private val LightOnErrorContainer = Color(0xFFC42B1C) // danger
private val LightInverseSurface = Color(0xFF202020) // window
private val LightInverseOnSurface = Color(0xFFFFFFFF) // text
private val LightInversePrimary = Color(0xFF4CC2FF) // accent（另一主题）

/**
 * 深色方案。**每一个角色都显式写出** —— 漏掉的角色会落到 M3 baseline 的紫色，
 * 那种紫色不属于 Fluent 的世界。
 */
internal val AudioLinkDarkScheme = darkColorScheme(
    primary = DarkPrimary,
    onPrimary = DarkOnPrimary,
    primaryContainer = DarkPrimaryContainer,
    onPrimaryContainer = DarkOnPrimaryContainer,
    inversePrimary = DarkInversePrimary,
    // secondary 是「弱化一档的文字/容器」：与 onSurfaceVariant / surface-2 同值，
    // 对应 Fluent 的 text-2 与 surface-2 两档。
    secondary = DarkSecondary,
    onSecondary = DarkOnSecondary,
    secondaryContainer = DarkSecondaryContainer,
    onSecondaryContainer = DarkOnSecondaryContainer,
    tertiary = DarkTertiary,
    onTertiary = DarkOnTertiary,
    tertiaryContainer = DarkTertiaryContainer,
    onTertiaryContainer = DarkOnTertiaryContainer,
    // background 与 surfaceContainer 差一档（#1A1A1A → #2B2B2B），卡片边界靠这个差
    // + 1px 描边表达，不靠投影 —— 与桌面端的层级口径同一套。
    background = DarkBackground,
    onBackground = DarkOnSurface,
    surface = DarkSurface,
    onSurface = DarkOnSurface,
    surfaceVariant = DarkSurfaceHighest,
    onSurfaceVariant = DarkOnSurfaceVariant,
    surfaceTint = DarkPrimary,
    surfaceBright = DarkSurfaceHighest,
    surfaceDim = DarkSurfaceLowest,
    inverseSurface = DarkInverseSurface,
    inverseOnSurface = DarkInverseOnSurface,
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

/** 浅色方案（同一台机器的日间版本）。同样全角色显式写出。 */
internal val AudioLinkLightScheme = lightColorScheme(
    primary = LightPrimary,
    onPrimary = LightOnPrimary,
    primaryContainer = LightPrimaryContainer,
    onPrimaryContainer = LightOnPrimaryContainer,
    inversePrimary = LightInversePrimary,
    secondary = LightSecondary,
    onSecondary = LightOnSecondary,
    secondaryContainer = LightSecondaryContainer,
    onSecondaryContainer = LightOnSecondaryContainer,
    tertiary = LightTertiary,
    onTertiary = LightOnTertiary,
    tertiaryContainer = LightTertiaryContainer,
    onTertiaryContainer = LightOnTertiaryContainer,
    background = LightBackground,
    onBackground = LightOnSurface,
    surface = LightSurface,
    onSurface = LightOnSurface,
    surfaceVariant = LightSurfaceHigh,
    onSurfaceVariant = LightOnSurfaceVariant,
    surfaceTint = LightPrimary,
    surfaceBright = LightSurfaceHighest,
    surfaceDim = LightSurfaceLowest,
    inverseSurface = LightInverseSurface,
    inverseOnSurface = LightInverseOnSurface,
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
