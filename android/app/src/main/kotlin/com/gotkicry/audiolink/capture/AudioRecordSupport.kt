package com.gotkicry.audiolink.capture

import android.media.AudioFormat
import android.media.AudioRecord

/**
 * `AudioRecord` 的共用胶水（麦克风与内录两条路都要用）。
 *
 * 抽在这里的理由与 `audio/` 侧一致：**把「必须有的一套检查」收敛到一处**，
 * 免得两条采集路径各自漏掉一个 `recordingState` 检查 —— 那会变成「看起来启动成功、其实一个样本都不来」
 * 的静默故障。
 *
 * 这些函数都**不**在 JVM 单测里被调用（`AudioRecord` 在 JVM 上只有 stub，一调就抛），
 * 所以它们是「必须真机才能验证」的部分，见各函数的注释。
 */
internal object AudioRecordSupport {

    /** 每帧的字节数（协议一帧 20 ms；`bytesPerSample` 随编码而变）。 */
    fun bytesPerFrame(format: CaptureFormat): Int {
        val bytesPerSample = when (format.encoding) {
            PcmEncoding.F32 -> 4
            PcmEncoding.S16 -> 2
        }
        return FRAMES_PER_PROTOCOL_FRAME * format.channelCount * bytesPerSample
    }

    /** 声道数 → `AudioFormat.CHANNEL_IN_*`（本模块只支持 1 与 2）。 */
    fun channelMaskOf(channelCount: Int): Int = when (channelCount) {
        1 -> AudioFormat.CHANNEL_IN_MONO
        2 -> AudioFormat.CHANNEL_IN_STEREO
        else -> throw CaptureFailure(
            CaptureErrorKind.UnsupportedFormat,
            "只支持 1 或 2 声道采集：$channelCount",
        )
    }

    /** 本模块的编码 → `AudioFormat.ENCODING_*`。 */
    fun encodingOf(encoding: PcmEncoding): Int = when (encoding) {
        PcmEncoding.F32 -> AudioFormat.ENCODING_PCM_FLOAT
        PcmEncoding.S16 -> AudioFormat.ENCODING_PCM_16BIT
    }

    /** 带宽余量：按 4 个协议帧（80 ms）申请，避免设备以「最小缓冲」跑（那会频繁溢出）。 */
    const val BUFFER_FRAMES = 4

    internal const val FRAMES_PER_PROTOCOL_FRAME = 20

    /**
     * 要求的最小缓冲字节数；`<= 0` 一律当「不支持这个格式」处理。
     *
     * 真机相关：`getMinBufferSize` 的返回值随设备/采样率/声道而变（有时返回 `ERROR_BAD_VALUE`），
     * 这里把它当成**明确的失败**而不是「用 0 试试看」。
     */
    fun minBufferBytes(format: CaptureFormat): Int {
        val min = AudioRecord.getMinBufferSize(
            format.sampleRateHz,
            channelMaskOf(format.channelCount),
            encodingOf(format.encoding),
        )
        if (min <= 0) {
            throw CaptureFailure(
                CaptureErrorKind.UnsupportedFormat,
                "设备不支持 ${format.sampleRateHz} Hz / ${format.channelCount}ch / ${format.encoding}（getMinBufferSize=$min）",
            )
        }
        return maxOf(min, bytesPerFrame(format) * BUFFER_FRAMES)
    }

    /**
     * 把「构造出来的 `AudioRecord`」推进到**确实在录**的状态；任一步不成立就释放并抛 [CaptureFailure]。
     *
     * 真机相关：`AudioRecord` 的构造**不会**因为设备被占用而抛异常 —— 它只是 `state != STATE_INITIALIZED`。
     * 漏掉这个检查就会出现「引擎在推流、对端一片安静」，而且日志里什么都没有。
     */
    fun startOrFail(record: AudioRecord, what: String): AudioRecord {
        if (record.state != AudioRecord.STATE_INITIALIZED) {
            releaseQuietly(record)
            throw CaptureFailure(
                CaptureErrorKind.DeviceBusy,
                "$what 初始化失败（设备被占用或参数被拒）",
            )
        }
        try {
            record.startRecording()
        } catch (denied: SecurityException) {
            // **这里才是没有 RECORD_AUDIO 时真正抛出的地方** —— AudioRecord 的构造不校验权限
            // （lint 报的两处构造点保守地标了 @RequiresPermission，我们按同一形状对齐表达）。
            // 与那两处一致：显式接住 → 收敛成 CaptureFailure(PermissionDenied) → 界面提示去授权，
            // 且不自动重试（见 CaptureErrorKind.needsUserAction / isRetryable）。
            releaseQuietly(record)
            throw CaptureFailure.fromException(denied)
        } catch (error: Throwable) {
            releaseQuietly(record)
            throw CaptureFailure.fromException(error)
        }
        if (record.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
            releaseQuietly(record)
            throw CaptureFailure(CaptureErrorKind.DeviceBusy, "$what 没有进入录音状态")
        }
        return record
    }

    /** 阻塞读一块（返回样本数；负值由调用方按错误处理）。真机相关。 */
    fun readBlocking(record: AudioRecord, buffer: CaptureBuffer): Int = try {
        when (buffer.encoding) {
            PcmEncoding.F32 -> record.read(buffer.floats, 0, buffer.floats.size, AudioRecord.READ_BLOCKING)
            PcmEncoding.S16 -> record.read(buffer.shorts, 0, buffer.shorts.size, AudioRecord.READ_BLOCKING)
        }
    } catch (error: Throwable) {
        throw CaptureFailure.fromException(error)
    }

    /** 停止并释放；停止路径上**吞掉**异常（幂等，且不该让收尾把别的东西带崩）。 */
    fun releaseQuietly(record: AudioRecord?) {
        if (record == null) return
        try {
            if (record.recordingState == AudioRecord.RECORDSTATE_RECORDING) record.stop()
        } catch (_: Throwable) {
            // 收尾路径：设备可能已经被系统回收，stop 抛异常没有可做的补救。
        }
        try {
            record.release()
        } catch (_: Throwable) {
            // 同上。
        }
    }
}
