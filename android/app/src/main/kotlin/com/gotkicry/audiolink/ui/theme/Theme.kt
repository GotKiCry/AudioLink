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
 * 每一档给两个色：`*` 是灯/填充色，`*Ink` 是**文字**色。基准口径下两者同值
 * （见 `DESIGN.md` §Migration & Gaps 的 `StatusColors` 段），与桌面端
 * `desktop/src/index.css` 的 `--t-ok` / `--t-warn` / `--t-danger` 同一组值 ——
 * 两端说的是同一盏灯。
 *
 * **语义红线**：`live` 是「ON AIR / 失败断开」的红，与桌面
 * `--color-lamp-live: var(--t-danger)` 同义，永远不要改成绿色（绿只属于 `on`）。
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

// 深色（默认）：ok / warn / danger / idle —— 取基准令牌原值，Ink 同值。
// 在 surfaceContainer(#2B2B2B) 上的对比度：on 6.98 · warn 10.73 · live 6.97 · idle 5.12。
private val DarkStatusColors = StatusColors(
    on = Color(0xFF6CCB5F), // ok
    onInk = Color(0xFF6CCB5F),
    warn = Color(0xFFFCE100), // warn
    warnInk = Color(0xFFFCE100),
    live = Color(0xFFFF99A4), // danger
    liveInk = Color(0xFFFF99A4),
    idle = Color(0xFF9A9BA3), // idle
    idleInk = Color(0xFF9A9BA3),
)

// 浅色：同一台机器的日间版本，不是深色的反转。
// 在 surfaceContainer(#FBFBFB) 上的对比度：on 5.26 · warn 5.07 · live 5.47 · idle 4.90。
// idle 原值 #82838C 只有 3.64:1 —— DESIGN.md §Colors 已把它判废（"浅色旧值 #82838C 只有
// 3.64:1，已废"），新值 #6E6E73 与 text-3 同源，两端一起动。
private val LightStatusColors = StatusColors(
    on = Color(0xFF0F7B0F), // ok
    onInk = Color(0xFF0F7B0F),
    warn = Color(0xFF9D5D00), // warn
    warnInk = Color(0xFF9D5D00),
    live = Color(0xFFC42B1C), // danger
    liveInk = Color(0xFFC42B1C),
    idle = Color(0xFF6E6E73), // idle / text-3 同源
    idleInk = Color(0xFF6E6E73),
)

/** 当前状态色；组件通过 [MaterialTheme.statusColors] 读，不直接碰这个。 */
val LocalStatusColors = staticCompositionLocalOf { DarkStatusColors }

/**
 * 文字色 —— M3 的 `colorScheme` 里**没有**三级文字这个角色。
 *
 * 为什么不复用 `outline`：`outline` 在 M3 里是**描边角色**（卡片边、分隔线、控件轮廓），
 * 语义与"文字"无关。DESIGN.md 的映射表虽然把 `outline` 的「来源」标成 grey-38 / text-3，
 * 但它给的值是描边灰（深 `#616161` / 浅 `#8A8A8A`），**不是** text-3 的
 * （深 `#9A9A9A` / 浅 `#6E6E73`）。为了凑 text-3 去改 `outline`，会把所有描边一起带偏 ——
 * 所以这里照 [StatusColors] 的先例另起一个语义层。
 *
 * 承载者盘点（改前实测于 `android/.../ui/`）：指纹、地址、状态、信任这些三级字段
 * 走的是 `KeyValueRow`，用的角色是 `onSurfaceVariant` —— 那是 **text-2** 的值
 * （深 `#C5C5C5` / 浅 `#616161`）。也就是说 Android 侧原本**没有** text-3 的承载者，
 * 三级文字被 text-2 顶替。本次把 [TextColors.text3] 接到 `KeyValueRow` 的字段名上。
 *
 * 对比度（DESIGN.md 定死）：深 `#9A9A9A` 在 surfaceContainer(#2B2B2B) 上 5.03:1；
 * 浅 `#6E6E73` 在 #FBFBFB 上 4.90:1 —— 两端都过正文门槛。
 */
@Immutable
data class TextColors(
    /** 三级文字：指纹、地址、表头、时间戳这类辅助字段。 */
    val text3: Color,
)

private val DarkTextColors = TextColors(text3 = Color(0xFF9A9A9A)) // text-3（深）
private val LightTextColors = TextColors(text3 = Color(0xFF6E6E73)) // text-3（浅）

/** 当前文字色；组件通过 [MaterialTheme.textColors] 读。 */
val LocalTextColors = staticCompositionLocalOf { DarkTextColors }

/**
 * 控件色 —— 滑块/进度条的**轨道底座**与**描边环**。
 *
 * 为什么不复用 `outline`：DESIGN.md §Colors「状态灯底座」实测过 —— `outline` 的
 * `#616161` 在深色 `#2B2B2B` 上只有 **2.29:1**，低于非文字 UI 的 3:1 门槛。
 * 官方对应令牌是 Fluent `ControlStrongFillDefault`：深 `rgba(255,255,255,.544)`
 * （在 `#2B2B2B` 上 5.29:1）、浅 `rgba(0,0,0,.446)`（在 `#FBFBFB` 上 3.29:1）。
 *
 * [subtleStroke] 是 Fluent `ControlStrokeColorDefault`（与 `line-strong` 同值），
 * 用作滑块拇指的那一圈 1px 内环。
 */
@Immutable
data class ControlColors(
    /** Fluent `ControlStrongFillDefault`：滑块/进度轨道未填充部分。 */
    val strongFill: Color,
    /** Fluent `ControlStrokeColorDefault`：控件 1px 内环。 */
    val subtleStroke: Color,
)

// 深色：白 54.4% / 白 14%
private val DarkControlColors = ControlColors(
    strongFill = Color.White.copy(alpha = 0.544f),
    subtleStroke = Color.White.copy(alpha = 0.14f),
)

// 浅色：黑 44.6% / 黑 14%
private val LightControlColors = ControlColors(
    strongFill = Color.Black.copy(alpha = 0.446f),
    subtleStroke = Color.Black.copy(alpha = 0.14f),
)

/** 当前控件色；组件通过 [MaterialTheme.controlColors] 读。 */
val LocalControlColors = staticCompositionLocalOf { DarkControlColors }

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
 * AudioLink 主题（Fluent 2 / Windows 11 口径）。
 *
 * 结构是 Material 3 的（color roles / type scale / shape / motion），
 * 底下的令牌与状态语义是 Fluent 2 的：与 `desktop/src/index.css` 同值，
 * 权威在 `DESIGN.md`。组件不许绕过这里的角色自己配色。
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
        LocalTextColors provides if (dark) DarkTextColors else LightTextColors,
        LocalControlColors provides if (dark) DarkControlColors else LightControlColors,
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

/** `MaterialTheme.textColors` —— 文字色的读法（text-3 这类 M3 没有的角色）。 */
val MaterialTheme.textColors: TextColors
    @Composable
    @ReadOnlyComposable
    get() = LocalTextColors.current

/** `MaterialTheme.controlColors` —— 控件色的读法（轨道底座 / 描边环）。 */
val MaterialTheme.controlColors: ControlColors
    @Composable
    @ReadOnlyComposable
    get() = LocalControlColors.current
