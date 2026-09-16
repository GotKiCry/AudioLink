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
import android.os.SystemClock
import com.gotkicry.audiolink.MainActivity
import com.gotkicry.audiolink.R
import com.gotkicry.audiolink.audio.FfiPcmFeed
import com.gotkicry.audiolink.audio.LowLatencyPlayer
import com.gotkicry.audiolink.audio.PcmRingBuffer
import com.gotkicry.audiolink.core.EngineStartConfig
import com.gotkicry.audiolink.core.LocalStatus
import com.gotkicry.audiolink.core.displayedPin
import com.gotkicry.audiolink.core.engineStart
import com.gotkicry.audiolink.core.engineStop
import com.gotkicry.audiolink.core.peers
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

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

        // 进程级队列不随 Service 销毁而取消；旧 stop 完成后才允许新实例 start。
        private val engineOperations = EngineOperationQueue(
            CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate),
        )

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

        // ---- task-8：延迟杠杆的 UI 入口（请求式：UI 只登记意图，由服务下发给播放器）----

        /**
         * UI 请求的队列目标深度（帧）。服务每拍把它下发给播放器。
         *
         * 为什么走"请求 + 每拍下发"而不是让 UI 直接持有播放器：AudioTrack 的亲和性纪律要求
         * 所有设备调用都发生在播放线程上，UI 线程只允许改一个标量意图。
         */
        @Volatile
        private var requestedQueueTargetFrames: Int = LowLatencyPlayer.DEFAULT_QUEUE_TARGET_FRAMES

        /** UI 请求的一次性容量收缩（帧）；0 = 无请求。消费后清零。 */
        @Volatile
        private var requestedShrinkFrames: Int = 0

        /** UI 请求：把环统计的「区间增量」基线挪到当前值。 */
        @Volatile
        private var ringWindowResetRequested: Boolean = false

        /** UI → 播放器：设置输出设备队列目标深度（帧）。0 = 满灌（A/B 对照侧）。 */
        fun setQueueTargetFrames(frames: Int) {
            requestedQueueTargetFrames = frames
        }

        /** UI → 播放器：请求把输出缓冲**容量**收缩到 [frames] 帧（路①；由播放线程执行）。 */
        fun requestBufferShrink(frames: Int) {
            requestedShrinkFrames = frames
        }

        /** UI → 服务：把「溢出/读空」的区间增量基线挪到当前值（累计值不受影响）。 */
        fun resetRingWindow() {
            ringWindowResetRequested = true
        }
    }

    private var player: LowLatencyPlayer? = null

    /** 内核 PCM 的落地环：FFI 回调写它，播放线程读它。 */
    private val playoutRing = PcmRingBuffer(PLAYOUT_RING_FRAMES, CHANNEL_COUNT)

    /** FFI 接缝：内核 `PcmFeed.feedPcm` 的 Kotlin 实现（把 PCM 写进 [playoutRing]）。 */
    private val pcmFeed = FfiPcmFeed(playoutRing)

    /** 本实例的配对查询域；状态只在主线程发布，JNI 查询在 IO 上执行。 */
    private val engineScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var destroyed = false
    private var engineStatus: LocalStatus? = null
    private var engineError: String? = null
    private val engineLifecycle = EngineLifecycle(
        queue = engineOperations,
        startEngine = {
            withContext(Dispatchers.IO) {
                engineStart(
                    EngineStartConfig(
                        nodeName = Build.MODEL.orEmpty().ifBlank { "Android" },
                        dataDir = filesDir.absolutePath,
                        listenPort = 0u,
                    ),
                    playout = pcmFeed,
                    capture = null,
                )
            }
        },
        stopEngine = { withContext(Dispatchers.IO) { engineStop() } },
        publish = { status, error ->
            engineStatus = status
            engineError = error
            refreshState()
        },
    )

    private val mainHandler = Handler(Looper.getMainLooper())

    private var lastError: String? = null

    /**
     * 配对状态（PIN + 对端列表）的快照。
     *
     * FFI 查询在 IO 线程运行；回到主线程后核对引擎代次，再与 UI 串行发布。
     * 停止、重启或销毁后，不接受旧查询返回的 PIN。
     */
    private var pairingState: PairingUiState = PairingUiState()

    /** 正在跑的那次配对轮询；用它做"同一时刻只有一次在飞"的闸门（见 [refreshPairingAsync]）。 */
    private var pairingJob: Job? = null

    // ---- task-8：环统计的「区间增量」基线（累计值照旧保留）----
    private var ringOverflowBaseline: Long = 0
    private var ringUnderrunsBaseline: Long = 0

    /** 区间起点（uptimeMillis）；0 = 尚未开始计时。 */
    private var ringWindowStartUptimeMs: Long = 0

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
        // 区间口径要在**服务创建时**就起算：环计数从这个实例的 0 开始，
        // 若时长留在"未开始"状态，UI 会出现"区间时长 0 s 而增量 4000 万帧"这种自相矛盾的读数。
        ringWindowStartUptimeMs = SystemClock.uptimeMillis()
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
        // 必须先失效发布权限：音频/引擎收尾可能晚于新的 Service 实例。
        destroyed = true
        engineLifecycle.close()
        engineScope.cancel()
        stopPlayback()
        pairingJob?.cancel()
        pairingJob = null
        pairingState = PairingUiState()
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
        engineLifecycle.start()
    }

    /** 立即清除旧快照；底层停止进入进程级队列，不随服务销毁而取消。 */
    private fun stopEngine() {
        pairingJob?.cancel()
        pairingJob = null
        pairingState = PairingUiState()
        engineLifecycle.stop()
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
        if (destroyed) return
        val snapshot = player?.stats()
        val engine = engineStatus
        val pairing = pairingState

        // task-8：把 UI 的请求下发给播放器（设备调用本身仍在播放线程里做）。
        val active = player
        if (active != null) {
            active.setQueueTargetFrames(requestedQueueTargetFrames)
            val shrink = requestedShrinkFrames
            if (shrink > 0) {
                requestedShrinkFrames = 0
                active.requestBufferShrink(shrink)
            }
        }

        // task-8：「区间增量」的基线。清零只挪基线，**不动内核计数** ——
        // 累计值是一个跨会话的事实，改它就是改口径（纪律：不许为了好看换口径）。
        if (ringWindowResetRequested) {
            ringWindowResetRequested = false
            ringOverflowBaseline = playoutRing.overflowFrames
            ringUnderrunsBaseline = playoutRing.underrunCount
            ringWindowStartUptimeMs = SystemClock.uptimeMillis()
        }
        val windowSeconds = if (ringWindowStartUptimeMs == 0L) {
            0L
        } else {
            (SystemClock.uptimeMillis() - ringWindowStartUptimeMs) / 1_000L
        }

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
            ringOverflowSinceReset = (playoutRing.overflowFrames - ringOverflowBaseline)
                .coerceAtLeast(0L),
            ringUnderrunsSinceReset = (playoutRing.underrunCount - ringUnderrunsBaseline)
                .coerceAtLeast(0L),
            ringWindowSeconds = windowSeconds,
            queuedFrames = snapshot?.queuedFrames ?: 0,
            queueTargetFrames = snapshot?.queueTargetFrames ?: 0,
            bufferCapacityFrames = snapshot?.bufferCapacityFrames ?: 0,
            shrinkGrantedFrames = snapshot?.shrinkGrantedFrames
                ?: LowLatencyPlayer.SHRINK_NOT_ATTEMPTED,
            lastError = lastError ?: player?.lastFailureReason,
            engineRunning = engine != null,
            engineName = engine?.name.orEmpty(),
            engineIdShort = engine?.idShort.orEmpty(),
            engineAddr = engine?.addr.orEmpty(),
            engineCanReceive = engine?.canReceive == true,
            engineCanSend = engine?.canSend == true,
            engineError = engineError,
            pairingPin = pairing.pin,
            peers = pairing.peers,
            pairingNote = pairing.note,
        )
        // 配对面板是"按需拉"的：主线程只读快照，真正的 FFI 调用丢到 IO（见 refreshPairingAsync）。
        refreshPairingAsync()
    }

    /**
     * 拉一次配对状态（内核 `displayedPin()` + `peers()`），映射后写进 [pairingState]。
     *
     * 三个刻意的设计：
     * 1. **在 IO 线程调 FFI**：`displayedPin()` / `peers()` 是跨 JNA 的同步调用，每 500 ms 在主线程走一次
     *    是拿 UI 流畅度换便利；这里只让主线程读 `@Volatile` 快照。
     * 2. **catch Throwable（不是 Exception）**：`.so` 缺失/ABI 不匹配时 JNA 抛 `UnsatisfiedLinkError`（Error），
     *    漏掉它会让**服务进程崩掉** —— 而正确行为是"服务活着，把原因显示出来"。
     * 3. **一次只飞一个**：引擎里 `peers()` 要拿会话表，UI 刷新是 500 ms 一跳，不设闸门会在引擎卡顿时堆积调用；
     *    用 [pairingJob] 做闸门，慢的时候自然是"降频"，不会排队。
     *
     * 引擎没起来时**不动 FFI**：那会儿没有会话也没有 PIN，调用只会抛 `NOT_STARTED`，
     * 把"引擎没启动"渲染成"配对出错"是纯噪声。
     */
    private fun refreshPairingAsync() {
        if (destroyed || pairingJob?.isActive == true) return
        if (engineStatus == null) {
            pairingState = PairingUiState()
            return
        }
        val generation = engineLifecycle.generation
        pairingJob = engineScope.launch {
            val result = withContext(Dispatchers.IO) {
                var pin: String? = null
                var note: String? = null
                try {
                    pin = displayedPin()
                } catch (e: CancellationException) {
                    throw e
                } catch (t: Throwable) {
                    note = "读取配对 PIN 失败：${t.javaClass.simpleName}: ${t.message}"
                }

                var snapshots: List<PeerSnapshot> = emptyList()
                try {
                    snapshots = peers().map { peer ->
                        PeerSnapshot(
                            idShort = peer.idShort,
                            name = peer.name,
                            addr = peer.addr,
                            state = peer.state,
                            trusted = peer.trusted,
                        )
                    }
                } catch (e: CancellationException) {
                    throw e
                } catch (t: Throwable) {
                    val reason = "读取对端列表失败：${t.javaClass.simpleName}: ${t.message}"
                    note = if (note == null) reason else "$note；$reason"
                }
                PairingStateMapper.map(pin = pin, snapshots = snapshots, error = note)
            }
            // 此检查与发布都在主线程，停止不能插入两者之间。
            if (engineStatus == null || !engineLifecycle.isCurrent(generation)) return@launch
            pairingState = result
            refreshState()
        }
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
