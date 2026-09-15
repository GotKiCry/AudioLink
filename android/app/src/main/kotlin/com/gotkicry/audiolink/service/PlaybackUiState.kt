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
    /**
     * 环统计的**区间增量**（自上次「区间清零」以来的增量）。
     *
     * 为什么必须有这一对：累计值（[ringOverflowFrames] / [ringUnderruns]）分不清
     * "历史上丢过一大段"与"现在正在持续丢"。真机上看到 `溢出丢弃帧 4 332 000` 时，
     * 光凭累计值无法判断要不要动刀 —— 区间增量一读就知道。
     */
    val ringOverflowSinceReset: Long = 0,
    val ringUnderrunsSinceReset: Long = 0,
    /** 区间起点到现在的时长（秒）；区间刚开始时为 0。 */
    val ringWindowSeconds: Long = 0,

    // ---- 输出设备队列水位（task-8 延迟杠杆）----
    /** 设备队列**当前水位**（帧）—— 与 [actualBufferFrames]（容量）是两件事。 */
    val queuedFrames: Int = 0,
    /** 当前队列目标深度（帧）；0 = 不设目标（旧的"尽力写满"）。 */
    val queueTargetFrames: Int = 0,
    /** 设备容量上限（帧）。 */
    val bufferCapacityFrames: Int = 0,
    /** 最近一次容量收缩后设备给回的实际帧数；-1 = 未执行过。 */
    val shrinkGrantedFrames: Int = -1,

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

    // ---- 配对（§5）----
    /**
     * 当前要展示给用户的 6 位配对 PIN（内核 `displayedPin()` 原值，未加工）。
     *
     * `null` = 当前没有配对在进行 —— UI 据此**不占位**，而不是显示一个空框。
     * 「有 PIN」这个状态只可能来自内核的 `EngineEvent::DisplayPin`，Kotlin 侧不自己造 PIN。
     */
    val pairingPin: String? = null,
    /** 已连接对端（内核 `peers()`，映射成纯 Kotlin 模型；见 [PeerUi]）。 */
    val peers: List<PeerUi> = emptyList(),
    /** 取 PIN / 对端列表失败时的人话原因；`null` = 正常。 */
    val pairingNote: String? = null,
) {
    /**
     * [pairingPin] 是否已经没有对应的活跃配对会话 —— 口径与理由见
     * [PairingUiState.pinIsStale]（真机实测：对端断开后 FFI 仍会返回旧 PIN）。
     */
    val pinIsStale: Boolean get() = PairingStateMapper.pinIsStale(pairingPin, peers)
}
