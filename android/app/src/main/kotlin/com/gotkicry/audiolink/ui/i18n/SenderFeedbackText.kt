package com.gotkicry.audiolink.ui.i18n

import com.gotkicry.audiolink.service.SenderStateMapper
import com.gotkicry.audiolink.service.SenderUiState

/**
 * 会话反馈的**归属**：拆卡之后，"提示与错误"必须各归各的卡，既不重复也不遗漏。
 *
 * 为什么需要一层显式的归属判定：[SenderUiState] 的 `note` / `error` 是**一条**通道，
 * 里面混着两个角色的消息（连接失败、输码提示、推流失败、停流提示、会话状态）。拆成
 * 「接收端入口卡」与「主机卡」之后，如果两张卡都显示它，用户会看到同一句话出现两遍；
 * 只放其中一张，另一张的关键反馈就会人间蒸发。
 *
 * 判据只用**已经存在的事实字段**，不去猜字符串：
 * * 没有会话（[SenderUiState.hasSession]）时的错误，只可能来自这一次连接尝试 —— 归接收端入口卡；
 * * 有会话之后才出现的错误，必然是推流 / 停流 / 门禁拦截造成的 —— 归主机卡。
 *
 * 做成纯函数是为了能单测（Compose 在 JVM 单测里跑不起来）：错一条规则就是"某类错误不显示"，
 * 而这种缺陷在界面上是**静默**的，只有测试盯得住。
 */
internal fun connectPathError(sender: SenderUiState): String? =
    if (sender.hasSession) null else sender.error

/** 主机卡的错误：有会话之后的失败（推流 / 停流 / 会话门禁拦截）。 */
internal fun hostPathError(sender: SenderUiState): String? =
    if (sender.hasSession) sender.error else null

/**
 * 接收端入口卡的提示：输码（本机要提交主机屏幕上的码）与断开（那条会话没了）。
 *
 * 断开提示放这一张卡：会话是接收端入口建立起来的，断开在这里说最自然；主机卡那边
 * 状态徽章本来就会从「发送中」退回「未连接」，不会漏消息。
 */
internal fun connectPathNote(strings: UiStrings, sender: SenderUiState): String? = when {
    sender.awaitingPin -> strings.senderAwaitingPinNote
    sender.sessionDropped -> strings.senderSessionDropped
    else -> null
}

/**
 * 连接入口卡上要出的门禁提示（**最多一条**，顺序即优先级）。
 *
 * 为什么把候选集中到一处：地址为空（或格式不对）时，[SenderStateMapper.addressGate] 与
 * [SenderStateMapper.canConnect] 返回的是**同一个** SendGate —— 输入框下渲染一次、按钮下再渲染
 * 一次，界面上就是同一句「先填主机地址（例如 192.168.1.5）」印了两遍（真机评审的收口项）。
 *
 * 收成一个列表、由界面只取第一条，重复就从结构上不可能再发生；而"最多一条"这条不变量
 * 也终于能被 JVM 单测穷举（Compose 在单测里跑不起来）。
 *
 * 优先级：连接门禁在前（它贴近按钮，还能表达"正在连接"这种状态），地址格式兜底。
 * **非空即返回**，而不是两条并列 —— 并列不仅会重复，还会让用户同时读两件要处理的事。
 */
internal fun connectEntryGateHints(
    strings: UiStrings,
    sender: SenderUiState,
    addr: String,
): List<String> {
    sendGateText(strings, SenderStateMapper.canConnect(sender, addr))?.let { return listOf(it) }
    sendGateText(strings, SenderStateMapper.addressGate(addr))?.let { return listOf(it) }
    return emptyList()
}

/**
 * 主机卡的提示：其余 note（推流相关）。上面两类已经归了接收端入口卡，这里不重复。
 *
 * 不需要 `strings`：这一格的文案就是 service 给的原文（本层只决定"归哪张卡"）。
 */
internal fun hostPathNote(sender: SenderUiState): String? = when {
    sender.awaitingPin || sender.sessionDropped -> null
    else -> sender.note
}
