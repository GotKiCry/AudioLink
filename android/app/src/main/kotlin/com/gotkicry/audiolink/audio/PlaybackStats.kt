package com.gotkicry.audiolink.audio

/**
 * 播放器启动结果 —— **形状冻结**（`docs/11-m1-contract.md` §7），字段名/顺序不得改动。
 *
 * @property lowLatency 低延迟是否**真的生效**（= [performanceMode] == `PERFORMANCE_MODE_LOW_LATENCY`）。
 *   注意：请求低延迟 ≠ 拿到低延迟，厂商/设备可以拒绝（返回 `PERFORMANCE_MODE_NONE`），
 *   所以这个字段只能由 `start()` 之后实测 `getPerformanceMode()` 得到，不能由调用参数推断。
 * @property actualBufferFrames 实际生效的输出缓冲帧数（`AudioTrack.getBufferSizeInFrames()`），
 *   可能与请求值不同：设备会把过小的请求抬到 `getMinBufferSize()` 之上。
 * @property performanceMode 原始 `getPerformanceMode()` 返回值（不要在这里转成字符串，UI 层再转）。
 */
data class PlaybackReport(
    val lowLatency: Boolean,
    val actualBufferFrames: Int,
    val sampleRate: Int,
    val channelCount: Int,
    val performanceMode: Int,
)

/**
 * 播放统计快照（口径见 [PlaybackCounters] 的类注释）。
 *
 * 契约 `docs/11-m1-contract.md` §7 要求至少包含 `underruns / framesWritten / writeErrors /
 * actualBufferFrames / performanceMode`；这里额外带上供给侧欠载、静音帧、丢弃帧与参数回显，
 * 目的是让 UI 与日志**不需要再问播放器第二个问题**就能解释清楚"现在是什么状态"。
 *
 * @property underruns 系统口径：`AudioTrack.getUnderrunCount()` —— "输出缓冲被写空"的次数。
 *   它与 [sourceUnderruns] 是两件事：前者说明**播放线程/系统调度**没喂上，后者说明**数据源**没数据。
 *   补静音策略正常工作时它应当长期为 0，一旦增长就是调度问题的直接证据。
 * @property sourceUnderruns 供给侧口径：一次拉取拿不满 = 一次欠载。
 * @property framesWritten 写入输出设备的**有效音频帧**（不含静音填充）。
 * @property silenceFrames 欠载时为保持输出流连续而补写的静音帧；设备实收 = framesWritten + silenceFrames。
 * @property framesDropped 推模式下被丢弃的帧（拉模式永不丢弃，见 [PlayoutLoop]）。
 * @property lastWriteError 最近一次输出设备错误码（[PlaybackCounters.ERROR_NONE] = 无错误）。
 */
data class PlaybackStats(
    val running: Boolean,
    val underruns: Int,
    val sourceUnderruns: Long,
    val framesWritten: Long,
    val silenceFrames: Long,
    val framesDropped: Long,
    val writeErrors: Int,
    val lastWriteError: Int,
    val actualBufferFrames: Int,
    val requestedBufferFrames: Int,
    val performanceMode: Int,
    val sampleRate: Int,
    val channelCount: Int,

    // ---- task-8：队列水位控制（延迟杠杆）----
    /**
     * 设备队列**当前水位**（帧）—— 输出设备里还压着没播完的帧数。
     *
     * 与 [actualBufferFrames] 的区别是这一条最要紧：后者是**容量**（"最多能压多少"），
     * 前者是**水位**（"现在压了多少"）。真机上的延迟地板由水位决定，不是容量
     * （实测容量 3844 帧 / flinger Latency 101 ms，正是因为此前每拍都把它灌满）。
     */
    val queuedFrames: Int,
    /** 当前队列目标深度（帧）；`0` = 未设目标（旧行为：尽力写满，仅用于 A/B 的对照侧）。 */
    val queueTargetFrames: Int,
    /** 设备**容量**上限（`getBufferCapacityInFrames()`）—— 队列目标不可能超过它。 */
    val bufferCapacityFrames: Int,
    /**
     * 最近一次「容量收缩」（路①）执行后设备给回的实际缓冲帧数；
     * `-1` = 尚未执行过收缩。非负时与 [actualBufferFrames] 应当一致。
     */
    val shrinkGrantedFrames: Int,
)
