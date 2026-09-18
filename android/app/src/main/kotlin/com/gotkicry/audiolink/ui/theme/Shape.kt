package com.gotkicry.audiolink.ui.theme

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Shapes
import androidx.compose.ui.unit.dp

/**
 * 形状 —— 档位与 `DESIGN.md` §Shapes 的「Android 落点」表**逐档对齐**，不许更大。
 *
 * | 档位 | 值 | 用途 |
 * |---|---|---|
 * | extraSmall | 2dp | 小元素（≤32px：色块、分段选中项、滑块拇指） |
 * | small      | 4dp | 控件（按钮、输入框、下拉） |
 * | medium     | 4dp | 分段外框 / 次级容器 |
 * | large      | 8dp | 卡片 / 浮层 / 对话框 |
 * | extraLarge | 8dp | 大片容器（**上限就是 8dp**） |
 *
 * 基准语言是 Fluent 2（Windows 11 口径）：4/8 px 的小圆角，方而不锐。
 * M3 默认那套（12/16/28 dp）属于另一种产品语言，已被本表整体替换。
 *
 * ⚠️ **只改这里不够**：M3 的 `Button` 家族默认 `ButtonDefaults.shape = CircleShape`，
 * `AlertDialog` 默认 `extraLarge` —— 它们**不读** `MaterialTheme.shapes` 档位。
 * 组件默认形状的覆盖点集中在 `ui/components/AlControls.kt`（`AlButton` 家族）与
 * `PanelCard` / `ExpandablePanel` / `NoticePanel`，不要在调用点重复写 shape 参数。
 */
internal val AudioLinkShapes = Shapes(
    extraSmall = RoundedCornerShape(2.dp),
    small = RoundedCornerShape(4.dp),
    medium = RoundedCornerShape(4.dp),
    large = RoundedCornerShape(8.dp),
    extraLarge = RoundedCornerShape(8.dp),
)
