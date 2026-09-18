package com.gotkicry.audiolink.ui.theme

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Shapes
import androidx.compose.ui.unit.dp

/**
 * 形状：**阳极氧化铝的方角**。圆角一律极小 —— M3 默认那套药丸形状（12/16/28 dp）
 * 属于另一种产品语言，这里全部替换。
 *
 * extraSmall 2dp · small 3dp · medium 4dp · large 6dp。
 * 「够圆到不像直角，方到不像卡片」就是这把尺子的全部意图。
 */
internal val AudioLinkShapes = Shapes(
    extraSmall = RoundedCornerShape(2.dp),
    small = RoundedCornerShape(3.dp),
    medium = RoundedCornerShape(4.dp),
    large = RoundedCornerShape(6.dp),
    extraLarge = RoundedCornerShape(6.dp),
)
