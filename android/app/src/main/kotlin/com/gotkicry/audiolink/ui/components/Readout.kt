package com.gotkicry.audiolink.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.ui.theme.AudioLinkType
import com.gotkicry.audiolink.ui.theme.textColors

/**
 * 一栏读数：上面丝印标签、下面等宽数字。
 *
 * 等宽 + 定宽数字（`tnum`）不是为了好看 —— 它保证每 500 ms 刷新时数字不会左右跳动，
 * 也让"延迟从 84 变成 184"这种变化一眼可见。
 */
@Composable
fun Readout(
    label: String,
    value: String,
    modifier: Modifier = Modifier,
    valueStyle: TextStyle = AudioLinkType.readout,
    valueColor: Color = MaterialTheme.colorScheme.onSurface,
) {
    // 间距只用 4/8/12/16/24 这一套栅格（2 dp 那种"野生值"在 560 dpi 的真机上等于 0）。
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        SilkLabel(label)
        Text(
            text = value,
            style = valueStyle,
            color = valueColor,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/**
 * 一排读数（延迟 / 码率 / 丢包）。
 *
 * 用 [FlowRow] 而不是三等分的 Row：窄屏 + 大字体（1.3×）下它会**自动换行**，
 * 而不是把 "1.44 Mbps" 挤成竖排断字或裁掉。每栏给一个最小宽度，
 * 数字长度变化时也不会互相推挤。
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ReadoutFlow(
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    FlowRow(
        modifier = modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        content()
    }
}

/**
 * 诊断区里的一行「标签 : 值」。值用等宽、右对齐 —— 竖着扫下来是一列整齐的数字。
 *
 * 标签（指纹 / 地址 / 状态 / 信任 这类字段名）用 `textColors.text3`：
 * DESIGN.md 把 text-3 定给「指纹、时间戳」这类三级文字，而 M3 的 `colorScheme` **没有**
 * 这个角色 —— 原本这里用的是 `onSurfaceVariant`（那是 text-2 的值）。详见 [TextColors]。
 */
@Composable
fun KeyValueRow(
    label: String,
    value: String,
    modifier: Modifier = Modifier,
    valueColor: Color = MaterialTheme.colorScheme.onSurface,
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        verticalAlignment = Alignment.Top,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.textColors.text3,
            // 标签列给得比数值列宽：像 getPerformanceMode() 这种技术标识在中文字体下很占地方，
            // 给窄了会被迫按字符断行（真机上看到过 "getPerformanceM / ode()" 那种难看的样子）。
            modifier = Modifier.weight(1.4f),
        )
        Text(
            text = value,
            style = AudioLinkType.readout,
            color = valueColor,
            textAlign = TextAlign.End,
            modifier = Modifier.weight(1f),
        )
    }
}
