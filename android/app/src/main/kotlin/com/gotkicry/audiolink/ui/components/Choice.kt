package com.gotkicry.audiolink.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp

/** 一个可选项。 */
data class Choice<T>(val value: T, val label: String)

/**
 * 单选组（M3 [FilterChip] + [FlowRow]）。
 *
 * 为什么不是 M3 SegmentedButton：它是**一行不换行**的控件，[options] 里的中文标签在 360 dp
 * 宽的手机上会被挤成竖排断字（这正是要修的硬伤之一）。FilterChip 放进 [FlowRow] 后
 * 空间不够就自动折行，宽度也跟着内容走。
 *
 * 无障碍：容器 [selectableGroup] 让 TalkBack 把这一组当成"一组选项"读；
 * 每个 chip 由 M3 提供 selected 语义，这里再把 role 明确成 [Role.RadioButton]（单选，不是多选）。
 * 触控目标统一抬到 **≥48 dp**（M3 默认 32 dp，手指点不准）。
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun <T> ChoiceGroup(
    options: List<Choice<T>>,
    selected: T?,
    onSelect: (T) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    FlowRow(
        modifier = modifier
            .fillMaxWidth()
            .selectableGroup(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        options.forEach { option ->
            val isSelected = option.value == selected
            FilterChip(
                shape = MaterialTheme.shapes.small,
                selected = isSelected,
                onClick = { onSelect(option.value) },
                enabled = enabled,
                label = {
                    Text(
                        text = option.label,
                        style = MaterialTheme.typography.labelLarge,
                        maxLines = 1,
                    )
                },
                leadingIcon = if (isSelected) {
                    {
                        Icon(
                            imageVector = AppIcons.Check,
                            contentDescription = null,
                            modifier = Modifier.size(18.dp),
                        )
                    }
                } else {
                    null
                },
                modifier = Modifier
                    .heightIn(min = 48.dp)
                    .semantics { role = Role.RadioButton },
            )
        }
    }
}
