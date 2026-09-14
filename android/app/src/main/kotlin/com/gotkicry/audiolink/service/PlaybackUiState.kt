package com.gotkicry.audiolink.service

import com.gotkicry.audiolink.audio.LowLatencyPlayer

/**
 * UI 观测用的播放状态快照（不可变值对象）。
 *
 * 为什么不让 UI 直接读 `LowLatencyPlayer`：`AudioTrack` 的一切读数都必须在播放线程里采
 * （见 [com.gotkicry.audiolink.audio.LowLatencyPlayer] 的线程亲和纪律），UI 只能看快照。
 * 服务每 500 ms 刷一次（对齐 `docs/02-architecture.md` §4 的"按 500 ms 批量推给 UI"）。
 *
 * 字段分三组，对应"现在什么状态 / 播得怎么样 / 数据从哪来"三个问题。
 */
data class PlaybackUiState(
    // ---- 服务与播放器 ----
    val serviceRunning: Boolean = false,
    val playing: Boolean = false,
    /** 低延迟**实测**是否生效（= [performanceMode] == `PERFORMANCE_MODE_LOW_LATENCY`）。 */
    val lowLatency: Boolean = false,
    val performanceMode: Int = LowLatencyPlayer.PERFORMANCE_MODE_UNKNOWN,
    val sampleRate: Int = 0,
    val channelCount: Int = 0,
    /** 请求的输出缓冲帧数（构造参数）。 */
    val requestedBufferFrames: Int = 0,
    /** 实际生效的输出缓冲帧数（`getBufferSizeInFrames()`）；设备可能把请求值抬到下限之上。 */
    val actualBufferFrames: Int = 0,

    // ---- 统计（口径见 PlaybackCounters 的类注释）----
    /** 系统口径欠载：`AudioTrack.getUnderrunCount()`，正常应长期为 0。 */
    val trackUnderruns: Int = 0,
    /** 供给侧欠载：播放环一次拉取拿不满 = 一次。 */
    val sourceUnderruns: Long = 0,
    val framesWritten: Long = 0,
    val silenceFrames: Long = 0,
    val framesDropped: Long = 0,
    val writeErrors: Int = 0,
    val lastWriteError: Int = 0,

    // ---- 播放环（内核 PCM 的落地缓冲）----
    val ringBufferedFrames: Int = 0,
    val ringCapacityFrames: Int = 0,
    val ringUnderruns: Long = 0,
    val ringOverflowFrames: Long = 0,

    /** 最近一次错误（启动失败 / 播放线程异常）。 */
    val lastError: String? = null,

    // ---- 内核引擎（Rust，经 FFI）----
    /**
     * 引擎是否已启动（`engineStart` 成功）。
     *
     * 生命周期口径（Lead 裁决）：**引擎跟随服务** —— 服务启 → 引擎启（开始监听 QUIC 端口），
     * 服务停 → 引擎停。用户的显式动作是"启动服务"，"可被连接"是这个动作的自然结果。
     */
    val engineRunning: Boolean = false,
    /** 本机节点名（来自内核 `LocalStatus`）。 */
    val engineName: String = "",
    /** 本机指纹短码（展示用）。 */
    val engineIdShort: String = "",
    /** 本机 QUIC 监听地址。 */
    val engineAddr: String = "",
    /** 内核是否具备接收能力（M1 Android 恒为 true）。 */
    val engineCanReceive: Boolean = false,
    /** 内核是否具备发送能力（M1 恒为 false：没提供 capture 工厂，能力位如实体现，不假装能发）。 */
    val engineCanSend: Boolean = false,
    /** 引擎侧错误（启动失败 / 停止失败 / `.so` 加载失败）。与播放错误分开显示，便于定位。 */
    val engineError: String? = null,
)
