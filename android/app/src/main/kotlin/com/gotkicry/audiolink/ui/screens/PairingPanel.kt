package com.gotkicry.audiolink.ui.screens

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.ui.components.PanelCard
import com.gotkicry.audiolink.ui.i18n.LocalStrings

/**
 * 配对 PIN 面板（协议 §5）。
 *
 * 立场与旧版一致：**原值直出**，外壳不补零、不截断、不分组"美化" —— 内核返回什么就显示什么，
 * 免得读的人和内核里的真值不是同一个数。
 *
 * 自适应：字号用 `56.dp.toSp()` 而不是 `56.sp`。**这是刻意的**：
 * 56 sp 在系统字体 1.3× 下会变成 72.8 sp，六位数字加字距直接顶出卡片（旧版真机裁切的原因）。
 * 换成"固定的物理字号"后，无论 fontScale 多大，PIN 都是 56 dp 高、永远放得下 ——
 * 而 56 dp 本身就远大于任何正文，可读性不依赖放大。
 *
 * 外壳走 [PanelCard]：卡片形状（8 dp）与内边距（16 dp）在那一处定义，
 * 这里只给"这一张卡特有的两件衣服"（容器色 + 描边色）。
 */
@Composable
fun PairingPanel(
    pin: String,
    stale: Boolean,
    note: String?,
    modifier: Modifier = Modifier,
) {
    val strings = LocalStrings.current
    val density = LocalDensity.current
    val pinStyle = TextStyle(
        fontSize = with(density) { 56.dp.toSp() },
        letterSpacing = with(density) { 8.dp.toSp() },
        lineHeight = with(density) { 64.dp.toSp() },
        fontWeight = FontWeight.Bold,
        fontFamily = FontFamily.Monospace,
    )
    // PIN 未过期时正文用 onPrimaryContainer（已按「配对法则」重取 4.94:1）；过期后整卡降为中性。
    val ink = if (stale) {
        MaterialTheme.colorScheme.onSurfaceVariant
    } else {
        MaterialTheme.colorScheme.onPrimaryContainer
    }

    PanelCard(
        modifier = modifier,
        containerColor = if (stale) {
            MaterialTheme.colorScheme.surfaceContainerHigh
        } else {
            MaterialTheme.colorScheme.primaryContainer
        },
        borderColor = if (stale) {
            MaterialTheme.colorScheme.outlineVariant
        } else {
            MaterialTheme.colorScheme.primary
        },
    ) {
        Text(
            text = if (stale) strings.pairingTitleStale else strings.pairingTitle,
            style = MaterialTheme.typography.titleMedium,
            color = ink,
        )
        Text(
            text = pin,
            style = pinStyle,
            color = ink,
            maxLines = 1,
            softWrap = false,
            modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
        )
        Text(
            text = if (stale) strings.pairingStaleNote else strings.pairingNote,
            style = MaterialTheme.typography.bodySmall,
            color = ink,
        )
        if (note != null) {
            Text(
                text = note,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )
        }
    }
}
