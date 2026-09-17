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
 * §13 能力协商的位常量（与 `audiolink-types` 的 `Capabilities` 逐位一致）。
 *
 * 为什么在这里复制而不是从 FFI 绑定里拿：生成的 Kotlin 只导出函数与记录类型，**不导出位常量**；
 * 而这一层是纯逻辑，多开一条 FFI 面只为两个常数不划算。漂移风险由 Rust 侧守着 ——
 * `core/crates/audiolink-ffi/tests/capability_declaration.rs` 断言的是「声明的位真的到了对端」，
 * 位值写错会让那条测试直接红。
 */
internal object CapabilityBits {
    // 写成字面量而不是 `1u shl 4`：Kotlin 的 `const val` 只接受编译期常量，而 UInt 的 shl 是
    // 函数调用 —— 实测直接编译失败（"Const 'val' initializer must be a constant value"）。
    /** 支持系统内录（MediaProjection + AudioPlaybackCapture）；= 1 shl 4 = 16。 */
    const val SYSTEM_LOOPBACK: UInt = 16u

    /** 支持麦克风采集；= 1 shl 5 = 32。 */
    const val MICROPHONE: UInt = 32u
}

/** 系统内录的硬 API 门槛：MediaProjection 与 AudioPlaybackCapture 都是 API 29（Android 10）起。 */
internal const val SYSTEM_LOOPBACK_MIN_SDK: Int = 29

/**
 * 采集 → 服务的**纯逻辑**：三态发送源（关闭 / 内录 / 麦克风）该怎么落到引擎参数与前台上。
 *
 * 全部是纯函数（不碰 Android、不碰 FFI、不碰时间），因此可以在 JVM 单测里逐条钉死；
 * 服务侧只负责「按判定结果去调 `engineStart` / `startForeground` / `CaptureController`」。
 */
internal object CaptureWiring {

    /**
     * 引擎启动时要向对端声明的**平台能力位**（§13）。
     *
     * 语义是「**这台设备能**做什么」，**不是**「此刻正在用什么」—— 与 `CAPTURE` / `PLAYOUT` 同口径
     * （纯接收端「不需要」CAPTURE，但它并不是「不能」采集）。所以这里**不看**当前发送源：
     * 发送源关着也照旧声明，对端据此知道「这台设备具备内录 / 麦克风」，
     * 至于这一次会不会真的用上，由后续的授权流程与用户选择决定 ——
     * 就像「我能拍照」不等于「我正在拍」。
     *
     * 两个位都按**平台真实能力**给，不无条件声明：
     * * `SYSTEM_LOOPBACK` 仅当 [sdkInt] ≥ [SYSTEM_LOOPBACK_MIN_SDK]；
     * * `MICROPHONE` 仅当应用在 manifest 里声明了 `RECORD_AUDIO`（[declaresRecordAudio]）。
     *
     * 参数注入而不是直接读 `Build`：这一层整体是纯逻辑，才能在 JVM 单测里逐条钉死（见文件头）。
     */
    fun declaredCapabilities(sdkInt: Int, declaresRecordAudio: Boolean): UInt {
        var bits = 0u
        if (sdkInt >= SYSTEM_LOOPBACK_MIN_SDK) bits = bits or CapabilityBits.SYSTEM_LOOPBACK
        if (declaresRecordAudio) bits = bits or CapabilityBits.MICROPHONE
        return bits
    }

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
