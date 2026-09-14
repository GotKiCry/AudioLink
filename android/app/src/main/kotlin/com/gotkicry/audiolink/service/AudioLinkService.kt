package com.gotkicry.audiolink.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioTrack
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import com.gotkicry.audiolink.MainActivity
import com.gotkicry.audiolink.R
import com.gotkicry.audiolink.audio.FfiPcmFeed
import com.gotkicry.audiolink.audio.LowLatencyPlayer
import com.gotkicry.audiolink.audio.PcmRingBuffer
import com.gotkicry.audiolink.core.EngineStartConfig
import com.gotkicry.audiolink.core.LocalStatus
import com.gotkicry.audiolink.core.engineStart
import com.gotkicry.audiolink.core.engineStop
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * AudioLink 核心前台服务：接收播放 + 发送采集。
 *
 * 设计要点（对应 docs/04-tech-stack.md ADR-012 与旧版教训）：
 * 1. **启动即进入前台**（`onStartCommand` 内立刻 `startForeground`），不等到开始播放；
 * 2. 前台服务类型 `mediaPlayback|connectedDevice`（两者均无时长上限）；
 * 3. **接收/发送循环不得挂在 WorkManager / JobScheduler**（Android 16 起 FGS 内后台作业同样受配额限制）；
 * 4. 通知动作使用**显式 Intent**（旧版 PendingIntent 指向 BroadcastReceiver 导致点击崩溃）；
 * 5. **不做开机自启**：Android 15+ 禁止由 BOOT_COMPLETED 启动本类型前台服务。
 *
 * 数据流（M1）：
 * ```
 * Rust 引擎 ──PcmFeed.feedPcm──► FfiPcmFeed ──► PcmRingBuffer ──► LowLatencyPlayer（自带播放线程）──► AudioTrack
 * ```
 * 播放线程由 [LowLatencyPlayer] 在 Kotlin 侧自管（`docs/02-architecture.md` §1 第 3 条），
 * 服务既不碰 `AudioTrack`，也不托管音频线程；状态经 [state] 以 500 ms 节奏推给 UI。
 */
class AudioLinkService : Service() {

    companion object {
        const val CHANNEL_PLAYBACK = "playback"
        const val NOTIFICATION_ID = 0x1001
        const val ACTION_STOP = "com.gotkicry.audiolink.action.STOP"
        const val ACTION_START_PLAYBACK = "com.gotkicry.audiolink.action.START_PLAYBACK"
        const val ACTION_STOP_PLAYBACK = "com.gotkicry.audiolink.action.STOP_PLAYBACK"

        /** 播放环深度：60 ms @48 kHz = 2880 帧（契约 `docs/11-m1-contract.md` §5 的 `buffer_ms` 默认值）。 */
        const val PLAYOUT_RING_FRAMES = 2_880

        /**
         * AudioTrack 输出缓冲：20 ms @48 kHz = 960 帧。
         *
         * 与上面的播放环是**两层**：环是"内核 PCM 的落地缓冲"（吃网络抖动），
         * 这里是"设备侧输出缓冲"（吃调度抖动）。两层都过大会把端到端延迟抬起来 ——
         * M1 的 P50 ≤ 110 ms 目标里，这两层合计只允许占几十毫秒。
         * 实际生效值以 [PlaybackUiState.actualBufferFrames] 为准（设备可能抬高）。
         */
        const val TRACK_BUFFER_FRAMES = 960

        const val SAMPLE_RATE = LowLatencyPlayer.SAMPLE_RATE_48K
        const val CHANNEL_COUNT = LowLatencyPlayer.CHANNEL_COUNT_STEREO

        /** UI 刷新节奏，对齐架构 §4 的 500 ms 批量推送。 */
        private const val UI_REFRESH_MS = 500L

        /**
         * 服务与 UI 之间的状态通道。
         *
         * 为什么放 `companion`（进程级）而不是靠 bind：Activity 与服务生命周期互相独立
         * （`AudioLinkApp` 的纪律：不依赖"Activity 被打开过"），UI 不该被逼着绑定服务才能看状态；
         * 而进程被杀时两者一起消失，进程级单例不会造成跨进程的陈旧状态。
         */
        private val _state = MutableStateFlow(PlaybackUiState())
        val state: StateFlow<PlaybackUiState> = _state.asStateFlow()

        /** UI 入口：启动（或复用）前台服务并开始接收播放。 */
        fun start(context: Context) {
            context.startForegroundService(
                Intent(context, AudioLinkService::class.java).setAction(ACTION_START_PLAYBACK),
            )
        }

        /** UI 入口：停止服务（连带停止引擎、停止播放、释放 AudioTrack）。 */
        fun stop(context: Context) {
            context.stopService(Intent(context, AudioLinkService::class.java))
        }
    }

    private var player: LowLatencyPlayer? = null

    /** 内核 PCM 的落地环：FFI 回调写它，播放线程读它。 */
    private val playoutRing = PcmRingBuffer(PLAYOUT_RING_FRAMES, CHANNEL_COUNT)

    /** FFI 接缝：内核 `PcmFeed.feedPcm` 的 Kotlin 实现（把 PCM 写进 [playoutRing]）。 */
    private val pcmFeed = FfiPcmFeed(playoutRing)

    /**
     * 引擎协程域：`engineStart` / `engineStop` 都是 suspend，**绝不能**挂在主线程上跑。
     * 引擎真正的活儿在 Rust 侧自建的 tokio 运行时上，这里只负责发起与收尾。
     */
    private val engineScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private var engineJob: Job? = null

    @Volatile
    private var engineStatus: LocalStatus? = null

    @Volatile
    private var engineError: String? = null

    private val mainHandler = Handler(Looper.getMainLooper())

    private var lastError: String? = null

    private val refreshTask = object : Runnable {
        override fun run() {
            refreshState()
            if (player != null) {
                mainHandler.postDelayed(this, UI_REFRESH_MS)
            }
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
        refreshState()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stopPlayback()
                stopEngine()
                stopSelf()
                return START_NOT_STICKY
            }

            ACTION_STOP_PLAYBACK -> {
                // 停止**接收**：播放器与引擎一起停（不听了就没必要继续占着网络与 CPU），
                // 但服务保持常驻前台（通知还在，用户可再次启动）。
                stopPlayback()
                stopEngine()
                return START_STICKY
            }
        }
        startForegroundWithTypes(getString(R.string.notif_starting))
        startPlayback()
        startEngine()
        return START_STICKY
    }

    /**
     * 进入前台（按 API 等级选择 startForeground 重载）。
     *
     * **为什么还要在这里分级**：带 `foregroundServiceType` 的三参 `startForeground` 是
     * **API 29** 才有的重载，而本模块 minSdk 是 26 —— 在 API 26–28 上直接调三参重载会抛
     * `NoSuchMethodError`，服务起来就崩。所以调用点必须与 [foregroundServiceTypes] 一样
     * 按版本精确分级，而不是"反正给了类型位"。
     */
    private fun startForegroundWithTypes(text: String) {
        val notification = buildNotification(text)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification, foregroundServiceTypes())
        } else {
            // API 26–28：没有类型位，manifest 里的 foregroundServiceType 在这两个版本上会被系统忽略。
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    override fun onDestroy() {
        stopPlayback()
        // 引擎停止是 suspend：交给 IO 协程收尾，**不阻塞主线程**（onDestroy 跑在 UI 线程上）。
        stopEngine()
        // 服务已不在：UI 必须看到"停止"而不是最后一次的快照。
        _state.value = PlaybackUiState()
        super.onDestroy()
    }

    /**
     * 手动 PCM 接缝（无装箱版）：直接写交错 `FloatArray` 到播放环。
     *
     * 内核的正式路径走 [pcmFeed]（`FfiPcmFeed`，实现 UniFFI 的 `PcmFeed`，入参是 `List<Float>`；
     * 装箱代价见该类注释）。本方法保留给非 FFI 的直推场景（本地自测注入 PCM，
     * 或未来 PCM 通道换成 UDL `bytes` 零拷贝后复用同一入口）。
     *
     * 返回实际写入帧数；环满时按 drop-oldest 丢**最旧**帧（理由见 [PcmRingBuffer] 的类注释：
     * 实时音频的坏数据是延迟累积，宁可断一下也不能让延迟越积越大）。
     */
    fun feedPcm(samples: FloatArray, frames: Int): Int = playoutRing.write(samples, frames)

    /**
     * 启动内核引擎（接收端）。
     *
     * **生命周期口径（Lead 裁决）：引擎跟随服务** —— 服务启 → 引擎启（开始监听 QUIC 端口），
     * 服务停 → 引擎停。用户的显式动作是"启动服务"，"可被连接"是这个动作的自然结果；
     * 反过来（等用户点连接才监听）会让 PC 端的连接尝试直接扑空。
     *
     * 参数口径：
     * - `listenPort = 0` = 用内核默认端口（`audiolink-types::DEFAULT_QUIC_PORT = 58290`）——
     *   刻意**不在 Kotlin 侧硬编码端口号**，避免与内核漂移；
     * - `dataDir = filesDir`：证书 / 信任库落在应用私有目录（ADR-011）；
     * - `capture = null`：M1 Android 不做发送方向 → 能力位只有 `CAN_RECEIVE`，
     *   UI 据此把"发送"置灰，而不是假装能发（能力位由内核如实给出，Kotlin 侧不美化）。
     */
    private fun startEngine() {
        if (engineJob != null) return
        engineJob = engineScope.launch {
            try {
                val status = engineStart(
                    EngineStartConfig(
                        nodeName = Build.MODEL.orEmpty().ifBlank { "Android" },
                        dataDir = filesDir.absolutePath,
                        listenPort = 0u,
                    ),
                    playout = pcmFeed,
                    capture = null,
                )
                engineStatus = status
                engineError = null
            } catch (e: Throwable) {
                // 必须接住 Throwable：`.so` 缺失或 ABI/版本不匹配时 JNA 抛的是
                // UnsatisfiedLinkError（Error，不是 Exception）。漏掉它会让整个服务进程崩掉，
                // 而正确行为是"服务活着 + 把原因如实显示出来"。
                engineError = "引擎启动失败：${e.javaClass.simpleName}: ${e.message}"
                engineStatus = null
                engineJob = null
            }
            refreshState()
        }
    }

    /** 停止引擎（幂等）。`engineStop()` 是 suspend，且内核侧"未启动时返回 Ok"，所以这里无需判空。 */
    private fun stopEngine() {
        if (engineJob == null && engineStatus == null) return
        engineJob = null
        engineStatus = null
        engineScope.launch {
            try {
                engineStop()
            } catch (e: Throwable) {
                engineError = "引擎停止失败：${e.javaClass.simpleName}: ${e.message}"
            }
            refreshState()
        }
    }

    /**
     * 前台服务类型：
     * * `mediaPlayback` 自 API 29 起可用；
     * * `connectedDevice` 自 **API 30** 起才有该类型位 —— 在 API 29 上提交未知位属于未定义行为，
     *   因此这里按版本精确分级（而不是简单地 `>= Q` 一刀切）。
     */
    private fun foregroundServiceTypes(): Int = when {
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.R ->
            ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK or
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE

        Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q ->
            ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK

        else -> 0
    }

    /** 启动播放器并接上播放环；失败**不抛**（服务要活着，错误进状态给 UI 看）。 */
    private fun startPlayback() {
        if (player != null) return
        lastError = null

        val candidate = LowLatencyPlayer(
            sampleRate = SAMPLE_RATE,
            channelCount = CHANNEL_COUNT,
            bufferFrames = TRACK_BUFFER_FRAMES,
        )
        // 唯一的 PCM 接缝：谁往环里写，播放线程就从这里读。
        candidate.setSource(playoutRing)

        try {
            val report = candidate.start()
            player = candidate
            val text = if (report.lowLatency) {
                getString(R.string.notif_playing_low_latency)
            } else {
                getString(
                    R.string.notif_playing_fallback,
                    LowLatencyPlayer.performanceModeName(report.performanceMode),
                )
            }
            getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, buildNotification(text))
        } catch (e: IllegalStateException) {
            lastError = "播放启动失败：${e.message}"
            candidate.stop()
        }

        refreshState()
        mainHandler.removeCallbacks(refreshTask)
        mainHandler.postDelayed(refreshTask, UI_REFRESH_MS)
    }

    /** 停止播放器并清空播放环（停止后环里的 PCM 已过期，留着只会在下次播放先播旧音频）。 */
    private fun stopPlayback() {
        mainHandler.removeCallbacks(refreshTask)
        player?.stop()
        player = null
        playoutRing.clear()
        refreshState()
    }

    /** 采一份快照推给 UI。设备读数都来自播放器内部的播放线程快照，主线程不碰 `AudioTrack`。 */
    private fun refreshState() {
        val snapshot = player?.stats()
        val engine = engineStatus
        _state.value = PlaybackUiState(
            serviceRunning = true,
            playing = snapshot?.running ?: false,
            lowLatency = snapshot?.performanceMode == AudioTrack.PERFORMANCE_MODE_LOW_LATENCY,
            performanceMode = snapshot?.performanceMode ?: LowLatencyPlayer.PERFORMANCE_MODE_UNKNOWN,
            sampleRate = snapshot?.sampleRate ?: SAMPLE_RATE,
            channelCount = snapshot?.channelCount ?: CHANNEL_COUNT,
            requestedBufferFrames = snapshot?.requestedBufferFrames ?: TRACK_BUFFER_FRAMES,
            actualBufferFrames = snapshot?.actualBufferFrames ?: 0,
            trackUnderruns = snapshot?.underruns ?: 0,
            sourceUnderruns = snapshot?.sourceUnderruns ?: 0,
            framesWritten = snapshot?.framesWritten ?: 0,
            silenceFrames = snapshot?.silenceFrames ?: 0,
            framesDropped = snapshot?.framesDropped ?: 0,
            writeErrors = snapshot?.writeErrors ?: 0,
            lastWriteError = snapshot?.lastWriteError ?: 0,
            ringBufferedFrames = playoutRing.sizeFrames,
            ringCapacityFrames = playoutRing.capacityFrames,
            ringUnderruns = playoutRing.underrunCount,
            ringOverflowFrames = playoutRing.overflowFrames,
            lastError = lastError ?: player?.lastFailureReason,
            engineRunning = engine != null,
            engineName = engine?.name.orEmpty(),
            engineIdShort = engine?.idShort.orEmpty(),
            engineAddr = engine?.addr.orEmpty(),
            engineCanReceive = engine?.canReceive == true,
            engineCanSend = engine?.canSend == true,
            engineError = engineError,
        )
    }

    private fun createChannel() {
        val channel = NotificationChannel(
            CHANNEL_PLAYBACK,
            getString(R.string.notif_channel_playback),
            NotificationManager.IMPORTANCE_LOW,
        ).apply { setShowBadge(false) }
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun buildNotification(text: String): Notification {
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val stop = PendingIntent.getService(
            this,
            1,
            Intent(this, AudioLinkService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return Notification.Builder(this, CHANNEL_PLAYBACK)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentIntent(open)
            // 用 Notification.Action.Builder 而非已废弃的 addAction(int, CharSequence, PendingIntent)
            .addAction(
                Notification.Action.Builder(
                    android.graphics.drawable.Icon.createWithResource(this, R.mipmap.ic_launcher),
                    getString(R.string.action_stop),
                    stop,
                ).build(),
            )
            .setOngoing(true)
            .build()
    }
}
