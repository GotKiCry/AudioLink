package com.gotkicry.audiolink.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

/** AudioLink 主题（配色与桌面端统一，见 docs/08-ui-spec.md §1） */
private val Indigo = Color(0xFF4F46E5)
private val IndigoLight = Color(0xFF818CF8)

private val LightColors = lightColorScheme(primary = Indigo)
private val DarkColors = darkColorScheme(primary = IndigoLight)

@Composable
fun AudioLinkTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    // TODO(M5)：接入主题设置（跟随系统 / 手动），并支持动态取色（Android 12+）
    MaterialTheme(colorScheme = if (darkTheme) DarkColors else LightColors, content = content)
}
