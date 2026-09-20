package com.gotkicry.audiolink.ui.components

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.runtime.getValue
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import com.gotkicry.audiolink.ui.theme.LocalReducedMotion
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import com.gotkicry.audiolink.ui.i18n.LocalStrings

/** 同一套宽度与安全区域，手机、横屏和平板都不把表单拉满屏幕。 */
@Composable
fun ConsoleContent(
    padding: PaddingValues,
    scrollState: ScrollState,
    content: @Composable ColumnScope.() -> Unit,
) {
    Box(
        modifier = Modifier.fillMaxSize().padding(padding).consumeWindowInsets(padding).imePadding(),
        contentAlignment = Alignment.TopCenter,
    ) {
        Column(
            modifier = Modifier.widthIn(max = 680.dp).fillMaxSize()
                .verticalScroll(scrollState).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
            content = content,
        )
    }
}

/** 同级顶部导航。短指示线表达选择，主色块留给连接操作。 */
@Composable
fun ConsoleRoleSwitch(sending: Boolean, onSelect: (Boolean) -> Unit) {
    val strings = LocalStrings.current
    val reduced = LocalReducedMotion.current
    val indicator by animateFloatAsState(
        targetValue = if (sending) 0.75f else 0.25f,
        animationSpec = tween(if (reduced) 0 else 200),
        label = "consoleSelection",
    )
    val accent = MaterialTheme.colorScheme.primary
    Row(
        modifier = Modifier.fillMaxWidth().selectableGroup().drawWithContent {
            drawContent()
            val center = size.width * indicator
            val half = 12.dp.toPx()
            drawLine(accent, Offset(center - half, size.height - 2.dp.toPx()),
                Offset(center + half, size.height - 2.dp.toPx()), 3.dp.toPx(), StrokeCap.Round)
        },
        horizontalArrangement = Arrangement.spacedBy(0.dp),
    ) {
        listOf(false, true).forEach { isSend ->
            val selected = sending == isSend
            val ink = if (selected) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant
            Row(
                modifier = Modifier.weight(1f).clip(MaterialTheme.shapes.small)
                    .selectable(selected = selected, role = Role.Tab, onClick = { onSelect(isSend) })
                    .heightIn(min = 52.dp).padding(horizontal = 8.dp, vertical = 12.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(if (isSend) AppIcons.Broadcast else AppIcons.Headphones, null, Modifier.size(20.dp), tint = ink)
                Text(if (isSend) strings.sendTab else strings.receiveTab,
                    style = if (selected) MaterialTheme.typography.titleMedium else MaterialTheme.typography.bodyMedium, color = ink)
            }
        }
    }
}

@Composable
fun ConsolePageHeading(title: String, description: String? = null) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(top = 4.dp, bottom = 8.dp)) {
        Text(title, style = MaterialTheme.typography.headlineLarge, modifier = Modifier.semantics { heading() })
        if (description != null) Text(description, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}
