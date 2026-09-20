package com.gotkicry.audiolink.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.ui.i18n.LocalStrings
import com.gotkicry.audiolink.ui.theme.AudioLinkDarkScheme
import com.gotkicry.audiolink.ui.theme.AudioLinkLightScheme
import com.gotkicry.audiolink.ui.theme.ThemeMode

/** 外观选项直接预览本产品的色板，缩略图无交互、无伪造业务数据。 */
@Composable
fun AppearancePicker(selected: ThemeMode, onSelect: (ThemeMode) -> Unit) {
    val strings = LocalStrings.current
    Row(Modifier.fillMaxWidth().selectableGroup(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        ThemeMode.entries.forEach { mode ->
            val checked = selected == mode
            val label = when (mode) {
                ThemeMode.System -> strings.themeSystem
                ThemeMode.Light -> strings.themeLight
                ThemeMode.Dark -> strings.themeDark
            }
            Column(
                Modifier.weight(1f).clip(MaterialTheme.shapes.small)
                    .selectable(checked, role = Role.RadioButton, onClick = { onSelect(mode) })
                    .padding(4.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Box(
                    Modifier.fillMaxWidth().clip(MaterialTheme.shapes.small)
                        .border(if (checked) 2.dp else 0.5.dp,
                            if (checked) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outlineVariant,
                            MaterialTheme.shapes.small),
                ) {
                    Canvas(Modifier.fillMaxWidth().height(72.dp).padding(4.dp)) {
                        when (mode) {
                            ThemeMode.Light -> windowPreview(AudioLinkLightScheme)
                            ThemeMode.Dark -> windowPreview(AudioLinkDarkScheme)
                            ThemeMode.System -> {
                                clipRect(right = size.width / 2) { windowPreview(AudioLinkLightScheme) }
                                clipRect(left = size.width / 2) { windowPreview(AudioLinkDarkScheme) }
                            }
                        }
                    }
                }
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    if (checked) Icon(AppIcons.Check, null, Modifier.size(14.dp), tint = MaterialTheme.colorScheme.primary)
                    Text(label, style = MaterialTheme.typography.bodySmall)
                }
            }
        }
    }
}

private fun DrawScope.windowPreview(scheme: ColorScheme) {
    val w = size.width
    val h = size.height
    val radius = CornerRadius(2.dp.toPx())
    drawRect(scheme.background)
    drawRect(scheme.surfaceContainerLow, size = Size(w, h * 0.2f))
    drawRoundRect(scheme.onSurfaceVariant, Offset(w * 0.09f, h * 0.085f), Size(w * 0.22f, h * 0.035f), radius)
    drawRoundRect(scheme.surfaceContainer, Offset(w * 0.09f, h * 0.3f), Size(w * 0.82f, h * 0.6f), radius)
    drawRoundRect(scheme.onSurfaceVariant, Offset(w * 0.16f, h * 0.4f), Size(w * 0.25f, h * 0.035f), radius)
    drawRoundRect(scheme.surfaceContainerLowest, Offset(w * 0.16f, h * 0.5f), Size(w * 0.68f, h * 0.13f), radius)
    drawRoundRect(scheme.primary, Offset(w * 0.56f, h * 0.71f), Size(w * 0.28f, h * 0.1f), radius)
}
