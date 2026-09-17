package com.gotkicry.audiolink.capture

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.projection.MediaProjection
import android.os.Build

/**
 * 系统内录采集源（FR-06）：MediaProjection 授权 + AudioPlaybackCapture + AudioRecord。
 *
 * 三条前提由调用方保证（见 [CaptureController]）：
 * 1. **API 29+**（Android 10）—— 低于此版本的入口必须在上层置灰并说明原因（FR-06 的 UI 要求）；
 * 2. **Android 14+ 必须是前台服务、且类型含 `mediaProjection`**，否则系统拒绝给出投影；
 * 3. **每次会话都要重新授权**（系统行为，无法绕过）→ 由 `PermissionRevoked` 这条正常路径承接。
 *
 * ## 抓不到的声音（不是缺陷，是系统规则，UI 必须能解释）
 * * 目标应用声明了 `allowAudioPlaybackCapture=false`；
 * * DRM 保护的音视频内容（永远不可捕获）；
 * * Android 13+ 起，电话、闹钟等系统用法不在可捕获集合内。
 *
 * 本类只负责「把这些规则之内的声音读出来」；解释与兜底（改用麦克风）是界面的责任。
 */
class SystemLoopbackCaptureSource(
    private val projection: MediaProjection,
    override val format: CaptureFormat = CaptureFormat.DEFAULT,
    private val usages: List<Int> = DEFAULT_USAGES,
) : AudioCaptureSource {

    private var record: AudioRecord? = null

    override fun start() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            throw CaptureFailure(
                CaptureErrorKind.NoCaptureSource,
                "系统内录需要 Android 10（API 29）及以上，本机 API ${Build.VERSION.SDK_INT}",
            )
        }

        val bufferBytes = AudioRecordSupport.minBufferBytes(format)
        val captureConfig = AudioPlaybackCaptureConfiguration.Builder(projection)
            .apply { usages.forEach { usage -> addMatchingUsage(usage) } }
            .build()

        val created = try {
            AudioRecord.Builder()
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setEncoding(AudioRecordSupport.encodingOf(format.encoding))
                        .setSampleRate(format.sampleRateHz)
                        .setChannelMask(AudioRecordSupport.channelMaskOf(format.channelCount))
                        .build(),
                )
                .setBufferSizeInBytes(bufferBytes)
                .setAudioPlaybackCaptureConfig(captureConfig)
                .build()
        } catch (denied: SecurityException) {
            // 与麦克风那条路同一个形状：内录同样要 RECORD_AUDIO（外加 MediaProjection 授权，
            // 后者由上层负责）。显式 catch 让「权限被拒」在代码里可见，且不必 @SuppressLint
            // —— 收敛目标仍是 CaptureFailure(PermissionDenied)，见 [AudioRecordSupport.startOrFail]。
            throw CaptureFailure.fromException(denied)
        } catch (error: Throwable) {
            throw CaptureFailure.fromException(error)
        }
        record = AudioRecordSupport.startOrFail(created, "系统内录")
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

    companion object {
        /**
         * 默认捕获的用法集合：媒体与游戏（这是「把手机正在放的声音推给 PC」的主场景），
         * 外加 `USAGE_UNKNOWN` —— 不少应用的音频落在它上面，漏掉会表现成「某些 App 抓不到」。
         */
        val DEFAULT_USAGES: List<Int> = listOf(
            AudioAttributes.USAGE_MEDIA,
            AudioAttributes.USAGE_GAME,
            AudioAttributes.USAGE_UNKNOWN,
        )
    }
}
