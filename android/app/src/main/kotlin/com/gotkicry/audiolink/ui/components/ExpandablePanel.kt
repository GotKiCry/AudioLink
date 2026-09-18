package com.gotkicry.audiolink.ui.components

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp

/**
 * 可折叠面板 —— 诊断区的容器（产品原则：**诊断归诊断**，内核真值默认折叠）。
 *
 * 折叠动效只用 M3 标准动效（淡入淡出 + 容器变换），200 ms；展开状态由调用方持有
 * （屏幕级 `rememberSaveable`），所以转屏、切深浅色都不会丢掉用户的选择。
 */
@Composable
fun ExpandablePanel(
    title: String,
    summary: String?,
    expanded: Boolean,
    onToggle: () -> Unit,
    expandedStateText: String,
    collapsedStateText: String,
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    AlCard(
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
    ) {
        Column(modifier = Modifier.fillMaxWidth()) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 48.dp)
                    .clickable(onClick = onToggle)
                    .semantics {
                        stateDescription = if (expanded) expandedStateText else collapsedStateText
                    }
                    .padding(horizontal = 16.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = title,
                        style = MaterialTheme.typography.titleSmall,
                        color = MaterialTheme.colorScheme.onSurface,
                        modifier = Modifier.semantics { heading() },
                    )
                    if (summary != null) {
                        Text(
                            text = summary,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                Icon(
                    imageVector = if (expanded) AppIcons.ChevronUp else AppIcons.ChevronDown,
                    contentDescription = null, // decorative：展开状态由 stateDescription 承载
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(24.dp),
                )
            }
            AnimatedVisibility(
                visible = expanded,
                enter = expandVertically(animationSpec = tween(200)) + fadeIn(animationSpec = tween(200)),
                exit = shrinkVertically(animationSpec = tween(200)) + fadeOut(animationSpec = tween(200)),
            ) {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                    content()
                }
            }
        }
    }
}

/** 错误/提示条：**问题 + 恢复路径**，颜色只是第三通道（前面还有图标与文字）。 */
@Composable
fun NoticePanel(
    title: String,
    message: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    tone: LampTone,
    modifier: Modifier = Modifier,
    detail: String? = null,
    detailLabel: String? = null,
) {
    AlCard(
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.errorContainer),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.error),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            StatusBadge(
                tone = tone,
                icon = icon,
                text = title,
                textStyle = MaterialTheme.typography.titleSmall,
            )
            Text(
                text = message,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onErrorContainer,
            )
            if (detail != null && detailLabel != null) {
                Spacer(modifier = Modifier.size(4.dp))
                SilkBare(detailLabel)
                Text(
                    text = detail,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onErrorContainer,
                )
            }
        }
    }
}

/** 没有卡片外壳的丝印标签（用于已经在卡片里的场合）。 */
@Composable
private fun SilkBare(text: String) {
    Text(
        text = text.uppercase(java.util.Locale.ROOT),
        style = com.gotkicry.audiolink.ui.theme.AudioLinkType.silk,
        color = MaterialTheme.colorScheme.onErrorContainer,
    )
}
