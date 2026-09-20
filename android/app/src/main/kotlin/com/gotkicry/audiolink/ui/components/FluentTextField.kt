package com.gotkicry.audiolink.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.text.style.TextOverflow
import com.gotkicry.audiolink.ui.theme.controlColors
import com.gotkicry.audiolink.ui.theme.fluentColors

/** 外置 label、下沉槽、强调底边；输入法、选择与读屏仍由原生文本编辑控件承载。 */
@Composable
fun FluentTextField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    placeholder: String = "",
    enabled: Boolean = true,
    isError: Boolean = false,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
    keyboardActions: KeyboardActions = KeyboardActions.Default,
) {
    val interaction = remember { MutableInteractionSource() }
    val focused by interaction.collectIsFocusedAsState()
    val fluent = MaterialTheme.fluentColors
    val colors = MaterialTheme.colorScheme
    val bottomColor = when {
        isError -> colors.error
        focused -> colors.primary
        else -> MaterialTheme.controlColors.strongFill
    }
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = colors.onSurface)
        BasicTextField(
            value = value,
            onValueChange = onValueChange,
            enabled = enabled,
            singleLine = true,
            textStyle = MaterialTheme.typography.bodyLarge.copy(color = colors.onSurface),
            keyboardOptions = keyboardOptions,
            keyboardActions = keyboardActions,
            interactionSource = interaction,
            cursorBrush = SolidColor(colors.primary),
            modifier = Modifier.fillMaxWidth().semantics { contentDescription = label }
                .clip(MaterialTheme.shapes.small)
                .background(if (focused) fluent.inputActive else fluent.inputFill)
                .border(0.5.dp, fluent.stroke, MaterialTheme.shapes.small)
                .drawWithContent {
                    drawContent()
                    val stroke = if (focused || isError) 2.dp.toPx() else 1.dp.toPx()
                    drawLine(bottomColor, Offset(0f, size.height - stroke / 2), Offset(size.width, size.height - stroke / 2), stroke)
                },
            decorationBox = { inner ->
                Box(Modifier.heightIn(min = 52.dp).padding(horizontal = 12.dp, vertical = 12.dp), contentAlignment = Alignment.CenterStart) {
                    if (value.isEmpty()) Text(placeholder, style = MaterialTheme.typography.bodyLarge,
                        color = colors.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    inner()
                }
            },
        )
    }
}
