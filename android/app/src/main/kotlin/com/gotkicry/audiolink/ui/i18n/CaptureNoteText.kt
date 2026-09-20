package com.gotkicry.audiolink.ui.i18n

import com.gotkicry.audiolink.capture.CaptureSourceKind
import com.gotkicry.audiolink.capture.CaptureState
import com.gotkicry.audiolink.service.SenderStateMapper

/**
 * 采集尚未就绪时的补充提示 → 当前语言（没有发送源、或采集已经跑起来时为 null）。
 *
 * 与 [sendGateText] 同一套做法、同一条理由：service 层的 [SenderStateMapper.captureNote] 是中文常量
 * 且被 JVM 单测逐字钉住，语言跟随系统时英文界面会被它拖回中文。**判定口径一个字都没改**
 * （哪些状态要提示、哪些不要），只是把"怎么说"搬到这一层。
 *
 * 做成纯函数而不是 `@Composable`：理由同 [sendGateText]（要能被 JVM 单测穷举）。
 */
internal fun captureNoteText(
    strings: UiStrings,
    captureSelection: CaptureSourceKind?,
    captureState: CaptureState,
): String? {
    if (captureSelection == null) return null
    return when (captureState) {
        CaptureState.Running -> null
        CaptureState.AwaitingPermission, CaptureState.Starting, CaptureState.Idle ->
            strings.captureNoteWaiting
        // 该状态本身会被 canStartSend 拦成禁用，不需要补充提示。
        CaptureState.Stopping -> null
        CaptureState.Failed -> strings.captureNoteFailed
    }
}
