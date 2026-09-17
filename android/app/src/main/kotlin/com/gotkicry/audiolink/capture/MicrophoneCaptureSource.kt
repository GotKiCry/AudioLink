package com.gotkicry.audiolink.capture

import android.media.AudioRecord
import android.media.MediaRecorder

/**
 * 麦克风采集源（FR-07：「手机麦克风当无线麦克风」）。
 *
 * ## 为什么默认用 `VOICE_RECOGNITION` 而不是 `MIC`
 * `MIC` 会挂上平台的 AGC / 降噪 / 回声消除，那些处理**都是带延迟的自适应滤波**：
 * 对一个「低延迟直推」的产品来说，它们既改变音色又增加端到端延迟，而且行为随设备而异。
 * `VOICE_RECOGNITION` 的语义正是「不要为我做语音美化、给我原始信号」，与 `AudioLink` 的目标一致。
 * 需要复现某台设备上 `MIC` 的听感时，用构造参数 `audioSource` 传 `MediaRecorder.AudioSource.MIC`。
 *
 * ## 仍需真机验证（本机无法验证，不要当成已验证）
 * * 真实采样：48 kHz 是否被设备静默改成 44.1 kHz —— **由内核的「零重采样」硬闸兜底**，
 *   本模块不做重采样，格式不符时 `AudioRecord` 会直接初始化失败（报 UnsupportedFormat）；
 * * 权限弹窗流程与「拒绝后再次请求」的实际表现；
 * * MIUI 等定制系统上的后台限制（前台服务类型见 manifest）。
 */
class MicrophoneCaptureSource(
    override val format: CaptureFormat = CaptureFormat.DEFAULT,
    private val audioSource: Int = MediaRecorder.AudioSource.VOICE_RECOGNITION,
) : AudioCaptureSource {

    private var record: AudioRecord? = null

    override fun start() {
        val bufferBytes = AudioRecordSupport.minBufferBytes(format)
        val created = try {
            AudioRecord(
                audioSource,
                format.sampleRateHz,
                AudioRecordSupport.channelMaskOf(format.channelCount),
                AudioRecordSupport.encodingOf(format.encoding),
                bufferBytes,
            )
        } catch (denied: SecurityException) {
            // 显式接住「没有录音授权」这条路径。为什么写成独立分支而不是靠下面的
            // `catch (Throwable)` 兜：权限请求的运行时流程在上层（见 [CaptureController.startMicrophone]
            // 的调用方），这里只做**收敛**；而 lint 的 MissingPermission 只认
            // `checkSelfPermission` 或**显式**的 `catch (SecurityException)` —— 宽 catch 在它眼里
            // 等于「没处理」。写出来既让结论可见，也不必用 @SuppressLint 把 lint 关掉。
            // 真正抛 SecurityException 的地方其实是 startRecording()（见 [AudioRecordSupport.startOrFail]）。
            throw CaptureFailure.fromException(denied)
        } catch (error: Throwable) {
            throw CaptureFailure.fromException(error)
        }
        record = AudioRecordSupport.startOrFail(created, "麦克风")
    }

    override fun read(buffer: CaptureBuffer): Int {
        val active = record ?: return 0
        return AudioRecordSupport.readBlocking(active, buffer)
    }

    override fun stop() {
        val active = record
        record = null
        AudioRecordSupport.releaseQuietly(active)
    }
}
