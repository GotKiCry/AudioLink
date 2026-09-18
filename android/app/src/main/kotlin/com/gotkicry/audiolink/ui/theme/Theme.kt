package com.gotkicry.audiolink.ui.theme

import android.content.Context
import android.provider.Settings
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

/**
 * 状态色 —— M3 **没有** success/warning 角色，所以这一组是语义扩展，
 * 用 CompositionLocal 提供（不是往 colorScheme 里塞含义不明的角色）。
 *
 * 每一档给两个色：`*` 是灯/填充色，`*Ink` 是**文字**色（保证 4.5:1 对比度）。
 * 与桌面端的 `--t-lamp-*` / `--t-ink-*` 同一组值 —— 两端说的是同一盏灯。
 */
@Immutable
data class StatusColors(
    /** 通流中（推流中 / 接收中）。 */
    val on: Color,
    val onInk: Color,
    /** 降级 / 重连 / 需要留意。 */
    val warn: Color,
    val warnInk: Color,
    /** ON AIR：正在推流，或失败/断开。 */
    val live: Color,
    val liveInk: Color,
    /** 空闲：什么都没发生。 */
    val idle: Color,
    val idleInk: Color,
)

private val DarkStatusColors = StatusColors(
    on = Color(0xFF3D8B5F),
    onInk = Color(0xFF74D3A0),
    warn = Color(0xFFC9A227),
    warnInk = Color(0xFFE5C158),
    live = Color(0xFFE8542F),
    liveInk = Color(0xFFFF7A56),
    idle = Color(0xFF5A5049),
    idleInk = Color(0xFF9B8B7D),
)

private val LightStatusColors = StatusColors(
    on = Color(0xFF2F7A50),
    onInk = Color(0xFF246B45),
    warn = Color(0xFFA8841A),
    warnInk = Color(0xFF7E6410),
    live = Color(0xFFC8441F),
    liveInk = Color(0xFFB33A18),
    idle = Color(0xFFA79E93),
    idleInk = Color(0xFF6E635A),
)

/** 当前状态色；组件通过 [MaterialTheme.statusColors] 读，不直接碰这个。 */
val LocalStatusColors = staticCompositionLocalOf { DarkStatusColors }

/**
 * 系统是否要求「移除动画」（开发者选项 / 无障碍里的动画缩放 = 0）。
 *
 * Compose 动画本身会跟随 animator duration scale，这里额外读一次是为了**明确**：
 * 状态灯的呼吸是唯一允许的持续动画，用户关掉动画时它必须真的停住，而不是"几乎不动"。
 */
val LocalReducedMotion = staticCompositionLocalOf { false }

/** 主题模式：跟随系统 / 强制浅色 / 强制深色。 */
enum class ThemeMode { System, Light, Dark }

/**
 * AudioLink 主题（On-Air Console）。
 *
 * 结构是 Material 3 的（color roles / type scale / shape / motion），
 * 气质是这台机器的：方角、丝印大写、钨丝灯。组件不许绕过这里的角色自己配色。
 */
@Composable
fun AudioLinkTheme(
    mode: ThemeMode = ThemeMode.System,
    content: @Composable () -> Unit,
) {
    val dark = when (mode) {
        ThemeMode.System -> isSystemInDarkTheme()
        ThemeMode.Light -> false
        ThemeMode.Dark -> true
    }
    val context = LocalContext.current
    // 读一次系统动画缩放（Settings.Global.ANIMATOR_DURATION_SCALE；0 = 关闭动画）。
    val reducedMotion = rememberReducedMotion(context)
    CompositionLocalProvider(
        LocalStatusColors provides if (dark) DarkStatusColors else LightStatusColors,
        LocalReducedMotion provides reducedMotion,
    ) {
        MaterialTheme(
            colorScheme = if (dark) AudioLinkDarkScheme else AudioLinkLightScheme,
            typography = AudioLinkTypography,
            shapes = AudioLinkShapes,
            content = content,
        )
    }
}

/** 系统动画缩放 = 0 时返回 true（读不到就当没关：宁可多一次呼吸，也不要假装读到了）。 */
@Composable
private fun rememberReducedMotion(context: Context): Boolean =
    remember(context) {
        runCatching {
            Settings.Global.getFloat(
                context.contentResolver,
                Settings.Global.ANIMATOR_DURATION_SCALE,
                1f,
            ) == 0f
        }.getOrDefault(false)
    }

/** `MaterialTheme.statusColors` —— 状态色的读法（与 colorScheme 同一层级）。 */
val MaterialTheme.statusColors: StatusColors
    @Composable
    @ReadOnlyComposable
    get() = LocalStatusColors.current
