package com.gotkicry.audiolink.ui.components

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.RowScope
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardColors
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CardElevation
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.ui.theme.fluentColors
import com.gotkicry.audiolink.ui.theme.fluentEdges

/**
 * 控件形状的统一出口 —— **M3 默认形状的显式覆盖点，只有这一处**。
 *
 * 为什么必须有这一层（DESIGN.md §Shapes「Android 落点」的原话）：
 * M3 的 `Button` / `FilledTonalButton` / `ElevatedButton` 默认
 * `ButtonDefaults.shape = CircleShape` —— **整条药丸**，而且**根本不读**
 * `MaterialTheme.shapes` 的档位。也就是说把 `AudioLinkShapes.small` 钉成 4dp
 * 对按钮毫无影响，药丸照旧。这不是"没配"，是 M3 的默认值压在上面。
 *
 * 基准语言（Fluent 2 / Windows 11）里 pill 是**白名单**：标签、开关轨道、滑块轨道、
 * 头像 —— 按钮不在内。所以每个按钮都必须显式吃 `shapes.small` = 4dp。
 *
 * 覆盖方式：**在这里覆盖一次**，调用点写 `AlButton(...)` 而不是 `Button(shape = …)`。
 * 13 个调用点各贴一遍 `shape = MaterialTheme.shapes.small` 是同一个参数抄 13 份 ——
 * 下次改档位就会漏掉几个，那种漏法在界面上是"这一颗还是药丸"的随机感。
 *
 * 参数只透传**被真正用到**的那些，其余交给 M3 默认值：
 * 少一个转发参数就少一处版本漂移（例如 `contentPadding` 的默认值在 M3 各版本里改过，
 * 转发等于把这个默认值冻在 2026-09 这一天）。
 */

/**
 * 主按钮的禁用态口径（DESIGN.md §Components「按钮」表：accent 型 disabled = **填充 30%
 * 不透明 + 文字 `text-disabled`**）。
 *
 * 为什么必须覆盖 —— 这条曾经是「屏幕底部一条浅灰横带」的真因（2026-09-18 真机取证）：
 * M3 的 `ButtonDefaults.buttonColors()` 把禁用容器定义成 `onSurface` 的 **12% 不透明**，
 * 深色下叠在卡片 `surfaceContainer(#2B2B2B)` 上得到 **#444444** —— 比卡片亮 1.55:1，
 * 比页面背景 `#1A1A1A` 亮得多。任何一个被滚出屏幕的禁用主按钮，都会在屏幕边缘留下
 * 一条明显的亮带（真机上 uiautomator 实测：`class="android.widget.Button"
 * enabled="false"`，bounds `[112,2763][1128,2772]`，即屏幕底部 9 px）。
 *
 * 按 30% accent 填充后：深色下 = `#35586B`（实测截图 `#35596B`，与卡片 `#2B2B2B` 约 1.88:1），
 * 浅色下 = `#B4D0E8`。**要如实知道：这条带子并没有消失** —— 它本来就只是「一个被滚动容器
 * 裁切的禁用按钮的顶边」，不是可修的故障；改口径只是把它的**语义**从「像系统漏光的中性灰」
 * 换成「一个禁用的主按钮」。要让它彻底不出现，只能改布局（首屏底部留白）或让禁用主按钮不
 * 填充 —— 两者都偏离本文基准，故不做（曾误记为 `#353535`/「横带消失」，2026-09-18 实测更正）。
 */
@Composable
fun alButtonColors(): ButtonColors = ButtonDefaults.buttonColors(
    // 禁用态必须**一眼看出不可点**（真机评审 P1）：原来用 primary@30% 的浅蓝，在真机上被当成
    // "可以点的次要按钮"。改成中性面色 + 更弱的字色：没有主色就不再暗示"这是主行动"。
    disabledContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
    disabledContentColor = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.36f), // Fluent text-disabled
)

/** M3 [Button] 的形状对齐版：默认 `shapes.small`(4dp)，**不是** M3 的整条药丸。 */
@Composable
fun AlButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = MaterialTheme.shapes.small,
    colors: ButtonColors = alButtonColors(),
    content: @Composable RowScope.() -> Unit,
) {
    Button(
        onClick = onClick,
        modifier = modifier,
        enabled = enabled,
        shape = shape,
        colors = colors,
        content = content,
    )
}

/** Fluent standard 次按钮：中性填充与细描边，沿用既有调用名称。 */
@Composable
fun AlOutlinedButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = MaterialTheme.shapes.small,
    colors: ButtonColors = ButtonDefaults.buttonColors(
        containerColor = MaterialTheme.fluentColors.controlFill,
        contentColor = MaterialTheme.colorScheme.onSurface,
        disabledContainerColor = MaterialTheme.fluentColors.controlFill,
        disabledContentColor = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.36f),
    ),
    content: @Composable RowScope.() -> Unit,
) {
    Button(
        onClick = onClick,
        modifier = modifier,
        border = BorderStroke(0.5.dp, MaterialTheme.fluentColors.stroke),
        enabled = enabled,
        shape = shape,
        colors = colors,
        content = content,
    )
}

/**
 * M3 [TextButton] 的形状对齐版。
 *
 * 文字按钮同样默认整条药丸：它的按下涟漪与焦点框都按 `shape` 裁剪，
 * 不覆盖的话，一颗"看起来无背景"的文字按钮在按下瞬间会露出药丸形涟漪。
 */
@Composable
fun AlTextButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = MaterialTheme.shapes.small,
    colors: ButtonColors = ButtonDefaults.textButtonColors(),
    content: @Composable RowScope.() -> Unit,
) {
    TextButton(
        onClick = onClick,
        modifier = modifier,
        enabled = enabled,
        shape = shape,
        colors = colors,
        content = content,
    )
}

/**
 * M3 [Card] 的形状对齐版：默认 `shapes.large`(8dp)，elevation 默认 0（层次靠描边，不靠投影）。
 *
 * 已有 [PanelCard] 覆盖"标准卡"（16dp 内边距 + 8dp 行距）；这个封装给**布局自定义**的卡片
 * ——[ExpandablePanel] 要自己排头部、[NoticePanel] 要自己控分隔，它们套不进 PanelCard 的
 * 固定内边距，但仍然必须吃同一档圆角与同一条"不用投影"的规矩。
 */
@Composable
fun AlCard(
    modifier: Modifier = Modifier,
    shape: Shape = MaterialTheme.shapes.large,
    colors: CardColors = CardDefaults.cardColors(),
    elevation: CardElevation = CardDefaults.cardElevation(defaultElevation = 0.dp),
    border: BorderStroke? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    Card(
        modifier = modifier.fluentEdges(),
        shape = shape,
        colors = colors,
        elevation = elevation,
        border = border,
        content = content,
    )
}
