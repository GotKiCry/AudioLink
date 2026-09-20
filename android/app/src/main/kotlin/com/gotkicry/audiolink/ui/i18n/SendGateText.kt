package com.gotkicry.audiolink.ui.i18n

import com.gotkicry.audiolink.service.SendGate
import com.gotkicry.audiolink.service.SenderStateMapper

/**
 * 动作门禁 → 当前语言的禁用原因（[SendGate.Allowed] 没有原因，返回 null）。
 *
 * 为什么 UI 层再写一份（service 层的 `SenderStateMapper.gateNote` 已经有一份中文）：
 * 与 [peerStateText] 同一条理由 —— 那一份是中文常量且被 JVM 单测逐字钉住，语言跟随系统时
 * 英文界面会被它拖回中文；而且它的措辞把主机默认写成了「电脑」（主机也可能是另一部手机），
 * 与冻结术语冲突。**判据（哪一个门禁）完全不变，只是换语言与换说法** ——
 * 能不能连、能不能推仍然只由 [SenderStateMapper] 判定，这里不新造任何一个条件。
 *
 * 做成纯函数而不是 `@Composable`：这些字符串值得被单测钉住（每条门禁都必须有文案，
 * 一条都不许漏、一条都不许再出现「电脑」），而 `@Composable` 在 JVM 单测里跑不起来。
 */
internal fun sendGateText(strings: UiStrings, gate: SendGate): String? = when (gate) {
    SendGate.Allowed -> null
    SendGate.EngineDown -> strings.gateEngineDown
    SendGate.Connecting -> strings.gateConnecting
    SendGate.AddressEmpty -> strings.gateAddressEmpty
    SendGate.AddressInvalid ->
        String.format(strings.gateAddressInvalidFormat, SenderStateMapper.DEFAULT_PORT_HINT)
    SendGate.NoSession -> strings.gateNoSession
    SendGate.AwaitingPin -> strings.gateAwaitingPin
    SendGate.AlreadySending -> strings.gateAlreadySending
    SendGate.NotSending -> strings.gateNotSending
    SendGate.CaptureOff -> strings.gateCaptureOff
    SendGate.CaptureStopping -> strings.gateCaptureStopping
    SendGate.PeerMismatch -> strings.gatePeerMismatch
}
