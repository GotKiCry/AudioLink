package com.gotkicry.audiolink.service

import com.gotkicry.audiolink.capture.CaptureSourceKind
import com.gotkicry.audiolink.capture.CaptureState
import com.gotkicry.audiolink.capture.CaptureStateSnapshot

/**
 * 采集 → 服务接线所需的**额外**前台服务类型。
 *
 * 为什么用自家的枚举而不是直接拿 `ServiceInfo.FOREGROUND_SERVICE_TYPE_*`：
 * Android 常量在 JVM 单测里是 stub（取值 0），而「哪个源该配哪个类型位」这条映射
 * 恰恰是接线里最容易写错、又最该被钉住的地方（写错的表现是：采集在后台被系统掐掉）。
 * 所以映射本身做成纯逻辑，由服务侧再翻成 `ServiceInfo` 常量。
 */
internal enum class CaptureServiceType {
    /** 不需要额外的类型位（发送源关闭）。 */
    None,

    /** 麦克风采集：`FOREGROUND_SERVICE_TYPE_MICROPHONE`（API 30+）。 */
    Microphone,

    /** 系统内录：`FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION`（API 29+）。 */
    MediaProjection,
}

/**
 * 采集 → 服务的**纯逻辑**：三态发送源（关闭 / 内录 / 麦克风）该怎么落到引擎参数与前台上。
 *
 * 全部是纯函数（不碰 Android、不碰 FFI、不碰时间），因此可以在 JVM 单测里逐条钉死；
 * 服务侧只负责「按判定结果去调 `engineStart` / `startForeground` / `CaptureController`」。
 */
internal object CaptureWiring {

    /**
     * 引擎启动时**是否**要把采集出口（`PcmPull`）交给内核。
     *
     * `null`（发送源 = 关闭）时必须是 `false` —— 引擎的 `capture = null` 本身就是一个合法状态
     * （能力位只有 `CAN_RECEIVE`，UI 据此把发送入口置灰），不该为了「接通链路」而无条件打开采集。
     */
    fun engineCaptureEnabled(selection: CaptureSourceKind?): Boolean = selection != null

    /**
     * 切换发送源**是否要重启引擎**。
     *
     * 依据：`capture` 是引擎的**启动参数**（内核侧 `engine_config.capture`，见
     * `core/crates/audiolink-ffi/src/engine_bridge.rs`），运行期换不了 ——
     * 所以只有「关闭 ⇄ 非关闭」这一对跨态切换需要重启；
     * 内录 ⇄ 麦克风之间**不需要**（两者共用同一个 `PcmPull` 出口，换的只是采集源），
     * 这一点很值钱：重启引擎会掐断正在播的接收流，能不做就不做。
     */
    fun requiresEngineRestart(previous: CaptureSourceKind?, next: CaptureSourceKind?): Boolean =
        engineCaptureEnabled(previous) != engineCaptureEnabled(next)

    /** 该选择除了播放/连接之外，还需要哪个前台服务类型位。 */
    fun extraForegroundServiceType(selection: CaptureSourceKind?): CaptureServiceType = when (selection) {
        null -> CaptureServiceType.None
        CaptureSourceKind.Microphone -> CaptureServiceType.Microphone
        CaptureSourceKind.SystemLoopback -> CaptureServiceType.MediaProjection
    }

    /** 发送源的中文名（通知与面板用同一份，避免两处各写一遍）。 */
    fun sourceLabel(selection: CaptureSourceKind): String = when (selection) {
        CaptureSourceKind.Microphone -> "麦克风"
        CaptureSourceKind.SystemLoopback -> "系统内录"
    }

    /**
     * 采集状态 → **给用户看的一句话**（通知正文 / 面板）。
     *
     * 返回 `null` = 没有值得说的：发送源关闭是**正常态**，不该在通知里永久占一行。
     * 失败态必须说清两件不同的事：**要不要用户动手**（去授权/换源）还是**我们自己会重试**
     * —— 这两件事在界面上是两种完全不同的提示（`docs/08-ui-spec.md` §4 的文案规范）。
     */
    fun captureNote(selection: CaptureSourceKind?, snapshot: CaptureStateSnapshot?): String? {
        if (selection == null) return null
        val label = sourceLabel(selection)
        if (snapshot == null) return "发送源：$label（等待启动）"
        return when (snapshot.state) {
            CaptureState.Idle -> "发送源：$label（已停止）"
            CaptureState.AwaitingPermission -> "$label：等待授权"
            CaptureState.Starting -> "$label：正在打开采集设备"
            CaptureState.Running -> "$label：正在采集"
            CaptureState.Stopping -> "$label：正在停止"
            CaptureState.Failed -> {
                val failure = snapshot.failure
                val reason = failure?.message ?: "采集失败"
                if (failure?.needsUserAction == true) {
                    "$label：$reason（需要你处理）"
                } else {
                    "$label：$reason（第 ${snapshot.attempts} 次失败，稍后自动重试）"
                }
            }
        }
    }
}
