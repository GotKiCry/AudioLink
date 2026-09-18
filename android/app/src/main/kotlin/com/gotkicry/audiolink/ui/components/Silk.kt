package com.gotkicry.audiolink.ui.components

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.ui.theme.AudioLinkType
import java.util.Locale

/**
 * 丝印标签：面板上那种压印的大写小字。
 *
 * 丝印感来自 **大写化 + 字距 0.13 em + labelSmall 角色**，不来自字体（不引入新字体，APK 体积敏感）。
 * 中文没有大小写，所以中文界面下它只表现为"小号、宽字距、颜色更弱"的分区标签 —— 这正是想要的。
 */
@Composable
fun SilkLabel(
    text: String,
    modifier: Modifier = Modifier,
    color: Color = MaterialTheme.colorScheme.onSurfaceVariant,
) {
    Text(
        text = text.uppercase(Locale.ROOT),
        modifier = modifier,
        style = AudioLinkType.silk,
        color = color,
        maxLines = 1,
    )
}

/** 分区标题（丝印大字 + heading 语义，供 TalkBack 按标题跳转）。 */
@Composable
fun DeckHeader(
    text: String,
    modifier: Modifier = Modifier,
    trailing: (@Composable () -> Unit)? = null,
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = text.uppercase(Locale.ROOT),
            style = AudioLinkType.silkLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.semantics { heading() },
        )
        if (trailing != null) {
            Spacer(modifier = Modifier.weight(1f))
            trailing()
        }
    }
}

/**
 * 面板卡：**阳极氧化铝面板**。
 *
 * 层次靠 1 px 边框 + 面色差表达，**不用投影**（elevation 恒为 0）—— 投影是"浮在页面上"，
 * 而这里要的是"嵌在机箱里"。圆角取 [MaterialTheme.shapes.large]（8 dp）—— 卡片档，
 * 与 DESIGN.md §Shapes 的 `large` 档一致（M3 默认 12 dp 太大）。
 */
@Composable
fun PanelCard(
    modifier: Modifier = Modifier,
    containerColor: Color = MaterialTheme.colorScheme.surfaceContainer,
    borderColor: Color = MaterialTheme.colorScheme.outlineVariant,
    content: @Composable ColumnScope.() -> Unit,
) {
    Card(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.large,
        colors = CardDefaults.cardColors(containerColor = containerColor),
        elevation = CardDefaults.cardElevation(defaultElevation = 0.dp),
        border = BorderStroke(1.dp, borderColor),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
            content = content,
        )
    }
}

/** 面板内的子分区标题（诊断区里分隔「引擎 / 低延迟 / 统计 …」）。 */
@Composable
fun DiagSection(title: String, modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.semantics { heading() },
        )
        content()
    }
}
