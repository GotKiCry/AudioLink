package com.gotkicry.audiolink.service

import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.Manifest
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioTrack
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
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
import com.gotkicry.audiolink.capture.CaptureController
import com.gotkicry.audiolink.capture.CaptureSourceKind
import com.gotkicry.audiolink.capture.CaptureState
import com.gotkicry.audiolink.capture.MediaProjectionRequestActivity
import com.gotkicry.audiolink.core.EngineStartConfig
import com.gotkicry.audiolink.core.FfiException
import com.gotkicry.audiolink.core.LocalStatus
import com.gotkicry.audiolink.core.connect
import com.gotkicry.audiolink.core.displayedPin
import com.gotkicry.audiolink.core.engineStart
import com.gotkicry.audiolink.core.engineStop
import com.gotkicry.audiolink.core.peers
import com.gotkicry.audiolink.core.startSend
import com.gotkicry.audiolink.core.stopSend
import com.gotkicry.audiolink.core.submitPin
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

        /** UI → 服务：切换发送源（带 [EXTRA_CAPTURE_SOURCE]；缺省 = 关闭）。 */
        const val ACTION_SET_CAPTURE_SOURCE = "com.gotkicry.audiolink.action.SET_CAPTURE_SOURCE"

        /** 发送源的枚举名（`CaptureSourceKind.name`）；不传 = 关闭。 */
        const val EXTRA_CAPTURE_SOURCE = "capture_source"

        // ---- 发送方向（FR-17 手工 IP 连接 + §8 推流）：与采集同一套「Intent 请求式」入口 ----

        /** UI → 服务：连接目标地址（本机作为**发起方**）。 */
        const val ACTION_CONNECT = "com.gotkicry.audiolink.action.CONNECT"

        /** 目标地址（`192.168.1.5` 或 `192.168.1.5:58290`）。 */
        const val EXTRA_TARGET_ADDR = "target_addr"

        /** UI → 服务：提交对端屏幕上显示的 6 位 PIN（`connect` 返回 `1002` 之后）。 */
        const val ACTION_SUBMIT_PIN = "com.gotkicry.audiolink.action.SUBMIT_PIN"

        /** 用户输入的 6 位 PIN。 */
        const val EXTRA_PIN = "pin"

        /** 配对 PIN 位数（协议 §5：6 位数字）。**只用于本地格式校验**，权威在服务端 `PinGate`。 */
        private const val PIN_LENGTH = 6

        /** 「已停止发送」的固定说法：与「已断开」严格区分（`stopSend` 只停流、保留连接）。 */
        private const val STOPPED_SENDING_NOTE = "已停止发送（连接保持）"

        /** UI → 服务：开始向当前对端推流。 */
        const val ACTION_START_SEND = "com.gotkicry.audiolink.action.START_SEND"

        /** UI → 服务：停止推流（**保留连接与信任** —— 内核 `Engine::stop_send` 的语义）。 */
        const val ACTION_STOP_SEND = "com.gotkicry.audiolink.action.STOP_SEND"

        /**
         * UI 入口：连接一台电脑（发送方向）。
         *
         * 为什么这一族走 Intent 而不是 task-8 那种 `@Volatile` 标量：连接/推流是**离散动作**
         * （不是每拍下发的连续意图），必须在服务自己的线程与生命周期里执行 —— 同 [setCaptureSource]。
         */
        fun connect(context: Context, addr: String) {
            context.startService(
                Intent(context, AudioLinkService::class.java)
                    .setAction(ACTION_CONNECT)
                    .putExtra(EXTRA_TARGET_ADDR, addr),
            )
        }

        /** UI 入口：提交对端屏幕上显示的 6 位 PIN（`1002 NOT_PAIRED` 之后继续**同一条**连接）。 */
        fun submitPin(context: Context, pin: String) {
            context.startService(
                Intent(context, AudioLinkService::class.java)
                    .setAction(ACTION_SUBMIT_PIN)
                    .putExtra(EXTRA_PIN, pin),
            )
        }

        /** UI 入口：开始向当前对端推流。 */
        fun startSend(context: Context) {
            context.startService(
                Intent(context, AudioLinkService::class.java).setAction(ACTION_START_SEND),
            )
        }

        /** UI 入口：停止推流（连接保持不变）。 */
        fun stopSend(context: Context) {
            context.startService(
                Intent(context, AudioLinkService::class.java).setAction(ACTION_STOP_SEND),
            )
        }

        /**
         * UI → 服务：请求切换发送源（FR-38 的「关闭 / 系统内录 / 麦克风」）；`null` = 关闭。
         *
         * 为什么是「请求式」（UI 只登记意图，服务在 500 ms 拍点上消费）而不是 UI 直接调服务方法：
         * 与 task-8 的 `requestedQueueTargetFrames` 同源 —— Activity 与服务生命周期互相独立，
         * UI 不该被逼着绑定服务；而真正要动的三件事（引擎启动参数、前台服务类型位、采集设备）
         * 都必须在服务自己的线程与状态里完成。
         */
        fun requestCaptureSource(source: CaptureSourceKind?) {
            requestedCaptureSource = source
            captureSourceRequestPending = true
        }

        /** UI 入口：请求切换发送源（`null` = 关闭）。**走 Intent**，不依赖任何刷新循环。 */
        fun setCaptureSource(context: Context, source: CaptureSourceKind?) {
            val intent = Intent(context, AudioLinkService::class.java)
                .setAction(ACTION_SET_CAPTURE_SOURCE)
                .putExtra(EXTRA_CAPTURE_SOURCE, source?.name)
            context.startService(intent)
        }

        /** UI 请求的发送源；配合 [captureSourceRequestPending] 使用（同一拍内读，避免读到半程状态）。 */
        @Volatile
        private var requestedCaptureSource: CaptureSourceKind? = null

        /** 是否有待消费的发送源请求。 */
        @Volatile
        private var captureSourceRequestPending: Boolean = false

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

    // ---- 发送采集（FR-06/07）：采集出口与它的状态 ----

    /**
     * 采集控制器（实例级，与 [pcmFeed] / [playoutRing] 同域）。
     *
     * 它持有采集环与 `PcmPull` 出口：**引擎拿到的就是 `captureController.pull`**。
     * 生命周期与引擎一致 —— 服务起来时建、服务销毁前 [CaptureController.close]（见 [onDestroy]）。
     */
    private val captureController = CaptureController()

    /**
     * 当前发送源；`null` = 关闭。
     *
     * 只在主线程读写（[onStartCommand] 与 [refreshState] 都在主线程），所以不加锁。
     * 「关闭」是**一等状态**：此时引擎的 `capture` 传 `null`（能力位只有 CAN_RECEIVE），
     * 绝不能为了「把链路接通」而无条件打开采集。
     */
    private var captureSelection: CaptureSourceKind? = null

    /**
     * 已拿到的内录投影（授权回执到达后保存）。
     *
     * 为什么要存下来：Android 14+ **每次会话都要重新授权**，所以「授权成功」与「真的开始采集」
     * 可能不在同一时刻（用户可能先授权、后选源）；投影必须由服务持有并在销毁时 stop 掉
     * （不 stop 会让系统持续显示投屏并占用投影配额）。
     */
    private var pendingProjection: MediaProjection? = null

    /**
     * 内录授权是否已到手（**用户点过「允许」**）。
     *
     * 它是 mediaProjection 类型位**唯一**的开关，理由是 Android 16 的硬校验（2026-09-18 真机崩过）：
     * `startForeground(type=mediaProjection)` 要求 `project_media` AppOp，而该 AppOp 只在用户在
     * MediaProjection 授权页点「允许」**之后**才由系统授予。授权之前带上这个类型位 →
     * `SecurityException: Starting FGS with type mediaProjection ...` → **整个服务进程崩溃**，
     * 真机症状是「选了系统内录，服务就没了」。
     */
    private var loopbackAuthorized: Boolean = false

    /**
     * 一次性采集提示（拒绝授权 / 取投影失败）：留给用户看，直到下一次源切换。
     *
     * 与「常规采集状态」分开的原因：常规状态由 [CaptureWiring.captureNote] 每拍重算（发送源关闭时为 null），
     * 若把拒绝提示也交给它算，用户会在下一拍就看不到反馈 —— 那正是「静默吞掉」。
     */
    private var captureNotice: String? = null

    /** 最近一次播放状态文案（通知正文的前半段）；由 [startPlayback] 写入。 */
    private var playbackNote: String = ""

    /** 最近一次写进通知的正文：只在变化时 notify（避免每 500 ms 一次无意义刷新）。 */
    private var lastNotificationText: String = ""

    // ---- 发送方向（本机 → 对端）----

    /** 发送方向的 UI 状态（连接 + 推流）。主线程读写（[onStartCommand] 与 [refreshState] 同线程）。 */
    private var senderState = SenderUiState()

    /**
     * 本机的发送**意图**：最后一次 `startSend()` 成功、且还没 `stopSend()`。
     *
     * 为什么要单独记：内核没有「本机是否在推流」的直接查询（`peers()` 只给会话状态），
     * 而「连着但暂时没数据」（采集等授权）时发帧数同样是 0 —— 用遥测反推会把它显示成
     * 「没在发送」，与用户刚点的动作不符（口径见 [SenderStateMapper.isSending]）。
     */
    private var localSendStarted = false

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
                        // §13：把「这台设备具备哪些采集能力」告诉内核，由它带进握手协商 ——
                        // 不声明的话，内录/麦克风已经可用而对端永远不知道（见 [declaredCapabilities]）。
                        capabilities = declaredCapabilities(),
                    ),
                    playout = pcmFeed,
                    capture = captureArgument(),
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

            MediaProjectionRequestActivity.ACTION_CAPTURE_GRANTED -> {
                // 内录授权成功。**必须先在前台服务里**（Android 14+ 要求取投影时已处于
                // mediaProjection 类型的前台服务中），所以这里先幂等地刷一次前台类型位 ——
                // 若服务本就在跑，重复 startForeground 是幂等更新。
                startForegroundWithTypes()
                // `when (intent?.action)` 不建立 `intent != null` 的智能转换，这里显式 let。
                intent?.let { handleCaptureGranted(it) }
                return START_STICKY
            }

            MediaProjectionRequestActivity.ACTION_CAPTURE_DENIED -> {
                startForegroundWithTypes()
                handleCaptureDenied()
                return START_STICKY
            }

            ACTION_SET_CAPTURE_SOURCE -> {
                // 为什么这条走 Intent 而不是只在 [refreshState] 里消费标量：
                // 播放刷新循环只在**播放器存在**时续期（见 [refreshTask]），
                // 于是「没在播放时切换发送源」会卡住 —— 而那恰恰是内录最常见的用法（先把声音推出去）。
                val raw = intent?.getStringExtra(EXTRA_CAPTURE_SOURCE)
                val requested = raw?.let { name ->
                    CaptureSourceKind.entries.firstOrNull { it.name == name }
                }
                // 顺序（2026-09-18 真机崩溃后定的）：**先把新选择落下去，再刷前台**。
                // 反过来会出现「用户从系统内录切到麦克风，这一拍却仍带着旧的 mediaProjection 类型位
                // 去 startForeground」—— 而那个位此刻已不该出现（投影尚未授权 / 已失效），
                // 系统直接 SecurityException 把服务进程带走。`applyCaptureSource` 末尾自己会刷一次前台。
                applyCaptureSource(requested)
                startForegroundWithTypes()
                refreshState()
                return START_STICKY
            }

            ACTION_CONNECT -> {
                startForegroundWithTypes()
                val addr = intent?.getStringExtra(EXTRA_TARGET_ADDR).orEmpty()
                connectSender(addr)
                return START_STICKY
            }

            ACTION_SUBMIT_PIN -> {
                startForegroundWithTypes()
                val pin = intent?.getStringExtra(EXTRA_PIN).orEmpty()
                submitSenderPin(pin)
                return START_STICKY
            }

            ACTION_START_SEND -> {
                startForegroundWithTypes()
                startSender()
                return START_STICKY
            }

            ACTION_STOP_SEND -> {
                startForegroundWithTypes()
                stopSender()
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
    private fun startForegroundWithTypes(text: String = composeNotificationText()) {
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
        // **顺序不变量：采集必须在引擎停止之前关掉。**
        //
        // 为什么：内核采集线程正阻塞在 `PcmPull.readPcm` 上（有界阻塞，最长约 100 ms 醒一次），
        // 而引擎停止要等这个线程退出 —— 先 close 采集环能立刻唤醒它并让它返回空，引擎才停得干净。
        // 顺序反了不只是「白等一轮」：旧引擎还会在一个已关闭的出口上取数，而新服务实例随后会建
        // **新的**采集环 —— 两个实例指向不同出口，现场无法解释（`CaptureController` 的 KDoc 同款不变量）。
        captureController.close()
        // 投影必须显式 stop：不 stop 会让系统持续显示「正在投屏」，并一直占用投影配额。
        try {
            pendingProjection?.stop()
        } catch (_: Throwable) {
            // 停止路径：投影可能已被系统回收，stop 抛异常没有可做的补救。
        }
        pendingProjection = null
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
     * - `capture`：**按当前发送源决定**（见 [captureArgument]）。「关闭」时传 `null` ——
     *   这是合法状态：能力位只有 `CAN_RECEIVE`，UI 据此把发送入口置灰，而不是假装能发
     *   （能力位由内核如实给出，Kotlin 侧不美化）；「内录 / 麦克风」时传 [captureController]
     *   的 `PcmPull` 出口，采集线程由它托管。
     *   ⚠️ `capture` 是**引擎启动参数**（内核 `engine_bridge` 在 start 时读），运行期换不了 ——
     *   所以「关闭 ⇄ 非关闭」的切换必须重启引擎（判定见 [CaptureWiring.requiresEngineRestart]），
     *   而「内录 ⇄ 麦克风」不需要（两者共用同一个出口）。
     */
    /**
     * 引擎启动时该传给 `capture` 的东西：发送源关闭 → `null`（**合法状态**），否则给采集出口。
     *
     * 返回类型刻意不写出来：它是 FFI 生成物里的 `PcmPull`，而调用点只关心「有 / 无」。
     */
    private fun captureArgument() =
        if (CaptureWiring.engineCaptureEnabled(captureSelection)) captureController.pull else null

    /**
     * 本机要向对端声明的**平台能力位**（§13）：内录按 API 门槛、麦克风按 manifest 声明。
     *
     * 与发送源**无关**：能力位说的是「这台设备能做什么」，不是「此刻正在用什么」——
     * 发送源关着也照旧声明（口径与 `CAPTURE`/`PLAYOUT` 一致，理由见
     * [CaptureWiring.declaredCapabilities]）。
     */
    private fun declaredCapabilities(): UInt = CaptureWiring.declaredCapabilities(
        sdkInt = Build.VERSION.SDK_INT,
        declaresRecordAudio = declaresRecordAudio(),
    )

    /**
     * 应用在 manifest 里**声明**了 `RECORD_AUDIO` 吗？
     *
     * 查的是 manifest 声明，不是运行时授权：能力位描述的是「能做到」，
     * 而「用户此刻给不给权限」是采集启动时的事（拒绝授权有自己的用户可见反馈）。
     * 两者混为一谈会让能力位随用户点击而变，那不是它的语义。
     */
    @Suppress("DEPRECATION")
    private fun declaresRecordAudio(): Boolean =
        packageManager
            .getPackageInfo(packageName, PackageManager.GET_PERMISSIONS)
            .requestedPermissions
            ?.contains(Manifest.permission.RECORD_AUDIO) == true

    /**
     * 应用发送源选择（主线程，在 [refreshState] 的拍点上调用）。
     *
     * 三态要做三件事：
     * 1. **跨态切换要重启引擎**（`capture` 是启动参数）：只有「关闭 ⇄ 非关闭」需要；
     *    内录 ⇄ 麦克风 共用同一个出口 —— 重启会掐断正在播的接收流，能不做就不做；
     * 2. **采集源要跟着换**：内录需要已授权的投影（没有就等回执，这不是错误）；麦克风直接开；
     * 3. **前台类型位要跟着换**（Android 14+ 的硬要求，见 [foregroundServiceTypes]）。
     *
     * 这里**不**再调 [refreshState]：调用方（[refreshState] / 回执处理）本来就会走完这一拍，
     * 再调一次只会让同一拍刷两遍。
     */
    private fun applyCaptureSource(next: CaptureSourceKind?) {
        val previous = captureSelection
        captureSelection = next
        // 用户重新选了源：上一次的「拒绝授权」提示到此为止（它已完成告知的职责）。
        captureNotice = null

        if (CaptureWiring.requiresEngineRestart(previous, next) && engineStatus != null) {
            // 引擎在跑且「要不要采集」变了 → 必须重启才能换掉 capture 参数；
            // 走现成的 stopEngine/startEngine（各自带配对快照清理与发布权限检查）。
            stopEngine()
            startEngine()
        }

        // 推流中把采集源切到「关闭」：**先停流、再停采集**。
        // 顺序反了会出现「采集已停、流还开着」的空窗 —— 对端会一直等一个永远不会到的流
        // （它那边的播放环会一路欠载）。
        // 注意：`stopSend()` 只停流、**保留连接与信任**（内核 `Engine::stop_send` 的语义），
        // 所以文案是「已停止发送」而不是「已断开」。
        if (SenderStateMapper.shouldStopSendOnCaptureChange(senderState.sending, next)) {
            localSendStarted = false
            senderState = senderState.copy(sending = false, note = STOPPED_SENDING_NOTE)
            engineScope.launch {
                withContext(Dispatchers.IO) {
                    try {
                        stopSend()
                    } catch (e: CancellationException) {
                        throw e
                    } catch (t: Throwable) {
                        // 停流失败不阻断停采集：用户的意图是「别发了」，采集该停还是得停。
                        senderState = senderState.copy(
                            error = SenderStateMapper.noteForSendFailure(ffiCodeOf(t), ffiContextOf(t)),
                        )
                    }
                }
                captureController.stop()
                refreshState()
            }
            updateForegroundTypes()
            return
        }

        when (next) {
            null -> {
                // 关掉发送源 = 这条内录会话结束：类型位也必须跟着撤（否则会一直以 mediaProjection 名义挂着，
                // 而投影其实已经不需要了）。
                loopbackAuthorized = false
                captureController.stop()
            }

            CaptureSourceKind.Microphone -> {
                // 切到麦克风 = 内录这条会话结束：把授权标志与旧投影一起收干净。
                // 不收的后果（Android 14+ 每会话都要重新授权）：日后切回内录会**立刻**带着
                // mediaProjection 类型位去 startForeground、并使用一个已经失效的投影 ——
                // 要么再崩一次，要么静默抓不到声音（`pendingProjection` 非空但已停）。
                loopbackAuthorized = false
                pendingProjection?.stop()
                pendingProjection = null
                captureController.startMicrophone()
            }

            CaptureSourceKind.SystemLoopback -> {
                val projection = pendingProjection
                if (projection != null) {
                    captureController.startLoopback(projection)
                }
                // 没有投影 = 还没授权：保持等待（回执到了会启动），这里不该报错。
            }
        }
        updateForegroundTypes()
    }

    /** 按当前发送源刷新前台通知与类型位（Android 14+ 要求「使用中的类型」出现在类型位里）。 */
    private fun updateForegroundTypes() {
        if (destroyed) return
        startForegroundWithTypes()
    }

    /** 通知正文 = 播放状态（前段）+ 采集状态（后段）；发送源关闭且无提示时只有前段。 */
    private fun composeNotificationText(): String {
        val base = playbackNote.ifEmpty { getString(R.string.notif_starting) }
        val note = captureNotice
            ?: CaptureWiring.captureNote(captureSelection, captureController.snapshot())
        return if (note.isNullOrEmpty()) base else "$base · $note"
    }

    /** 内录授权成功：取投影，并（若当前发送源就是内录）启动采集。 */
    private fun handleCaptureGranted(intent: Intent) {
        val resultCode = intent.getIntExtra(
            MediaProjectionRequestActivity.EXTRA_RESULT_CODE,
            Activity.RESULT_CANCELED,
        )
        val data: Intent? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            intent.getParcelableExtra(MediaProjectionRequestActivity.EXTRA_RESULT_DATA, Intent::class.java)
        } else {
            @Suppress("DEPRECATION")
            val legacy: Intent? = intent.getParcelableExtra(MediaProjectionRequestActivity.EXTRA_RESULT_DATA)
            legacy
        }
        if (data == null) {
            // 回执缺数据（Activity 侧已判过一次，这里是服务侧兜底）：必须让用户看见。
            captureNotice = "系统内录：授权回执缺少数据，无法取得投影"
            lastError = captureNotice
            refreshState()
            return
        }
        // 顺序不能反（Android 14+ 的硬要求；2026-09-18 在 Android 16 上按错误顺序崩过一次）：
        //   ① 回执到达 ⇒ 用户点过「允许」⇒ project_media AppOp 已授予 ⇒ **现在**才允许进 mediaProjection 前台；
        //   ② 进入该类型前台**之后**才允许 getMediaProjection()（14+ 的校验就在这一步）；
        //   ③ 拿到投影再启动采集。
        loopbackAuthorized = true
        startForegroundWithTypes()
        val manager = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        val projection = try {
            manager.getMediaProjection(resultCode, data)
        } catch (t: Throwable) {
            // 授权回执本身有问题（数据被系统回收、state 已失效）：必须让用户看见，不能静默。
            captureNotice = "系统内录：取得投影失败（${t.javaClass.simpleName}: ${t.message}）"
            lastError = captureNotice
            refreshState()
            return
        }
        if (projection == null) {
            captureNotice = "系统内录：系统没有给出投影（授权可能已过期）"
            lastError = captureNotice
            refreshState()
            return
        }
        pendingProjection = projection
        if (captureSelection == CaptureSourceKind.SystemLoopback) {
            captureController.startLoopback(projection)
        }
        updateForegroundTypes()
        refreshState()
    }

    /**
     * 内录授权被拒：**保持关闭**并给出用户可见反馈。
     *
     * 两条刻意的选择：
     * 1. **不静默吞**：写进 [captureNotice]（随通知正文与 UI 状态出网），并同时记进 [lastError]；
     * 2. **不偷偷换成麦克风**：换源必须是用户的动作（FR-38 的三态由用户选），
     *    替用户换一个正在录音的源比「什么都没发生」更糟。
     */
    private fun handleCaptureDenied() {
        pendingProjection = null
        loopbackAuthorized = false
        captureNotice = "系统内录：未获授权，发送源保持关闭"
        lastError = captureNotice
        applyCaptureSource(null)
        refreshState()
    }

    // ---------------------------------------------------------------------------
    // 发送方向（本机 → 对端）：连接与推流
    // ---------------------------------------------------------------------------

    /** 连接一台电脑（本机作为**发起方**）。 */
    private fun connectSender(addr: String) {
        senderState = senderState.copy(
            targetAddr = addr,
            connecting = true,
            note = null,
            error = null,
        )
        val generation = engineLifecycle.generation
        engineScope.launch {
            val outcome = withContext(Dispatchers.IO) {
                try {
                    Result.success(connect(addr))
                } catch (e: CancellationException) {
                    throw e
                } catch (t: Throwable) {
                    Result.failure(t)
                }
            }
            // 早退也必须**收敛 connecting**：这条早退的含义是「引擎代次变了」= 本次连接请求已经作废
            // （切发送源、启停服务都会 ++generation，见 EngineLifecycle）。不收敛的话 `connecting` 永远是
            // true，而界面上输入框受 `enabled = !sender.connecting` 控制、按钮恒显示「连接中…」，
            // 用户从此既改不了地址也点不动按钮 —— 只能重启服务（2026-09-18 真机定位：握手超时后再操作即锁死）。
            if (destroyed) return@launch
            if (!engineLifecycle.isCurrent(generation)) {
                senderState = senderState.copy(connecting = false)
                refreshState()
                return@launch
            }
            outcome.fold(
                onSuccess = { peer ->
                    senderState = senderState.copy(
                        connecting = false,
                        peerIdShort = peer.idShort,
                        peerState = peer.state,
                        peerStateLabel = PairingStateMapper.stateLabel(peer.state),
                        awaitingPin = false,
                        note = null,
                        error = null,
                    )
                },
                onFailure = { error -> applyConnectFailure(error) },
            )
            refreshState()
        }
    }

    /**
     * 连接失败的处理：`1002 NOT_PAIRED` 走**提示**（引导输入 PIN），其余走错误。
     *
     * `1002` 不是失败：FFI 的 `connect` 文档写明「QUIC 握手与会话已经在，UI 提示用户输入 PIN
     * 后调用 `submit_pin` 即可继续**同一条**连接」，所以它与真正的失败必须分开显示 ——
     * 混在一起会让用户以为连接坏了，去反复重连（那反而会丢掉已完成的握手）。
     */
    private fun applyConnectFailure(error: Throwable) {
        val code = ffiCodeOf(error)
        val detail = ffiContextOf(error)
        senderState = if (SenderStateMapper.connectFailureIsNotice(code)) {
            senderState.copy(
                connecting = false,
                awaitingPin = true,
                note = SenderStateMapper.noteForConnectFailure(code, detail),
                error = null,
            )
        } else {
            senderState.copy(
                connecting = false,
                awaitingPin = false,
                note = null,
                error = SenderStateMapper.noteForConnectFailure(code, detail),
            )
        }
    }

    /** 提交对端屏幕上显示的 6 位 PIN（`1002` 之后继续同一条连接）。 */
    private fun submitSenderPin(pin: String) {
        if (pin.length != PIN_LENGTH) {
            senderState = senderState.copy(
                error = "配对码应该是 $PIN_LENGTH 位数字",
                note = null,
            )
            refreshState()
            return
        }
        val generation = engineLifecycle.generation
        engineScope.launch {
            val outcome = withContext(Dispatchers.IO) {
                try {
                    submitPin(pin)
                    Result.success(Unit)
                } catch (e: CancellationException) {
                    throw e
                } catch (t: Throwable) {
                    Result.failure(t)
                }
            }
            if (destroyed || !engineLifecycle.isCurrent(generation)) return@launch
            outcome.fold(
                onSuccess = {
                    senderState = senderState.copy(awaitingPin = false, note = null, error = null)
                },
                onFailure = { error ->
                    senderState = senderState.copy(
                        note = null,
                        error = SenderStateMapper.noteForSendFailure(ffiCodeOf(error), ffiContextOf(error)),
                    )
                },
            )
            refreshState()
        }
    }

    /** 开始推流。 */
    private fun startSender() {
        // 单对端语义的兜底：`startSend()` **不带 peer 参数**，内核按 `current_peer()`（优先 streaming、
        // 否则第一个）选对端 —— 本机若同时还有一条入站会话，就可能发到错误的设备上。
        // 宁可让用户看到一句人话，也不静默发错（见 [SenderStateMapper.sessionGate]）。
        val gate = SenderStateMapper.sessionGate(senderState.peerIdShort, pairingState.peers)
        if (gate != SendGate.Allowed) {
            senderState = senderState.copy(error = SenderStateMapper.gateNote(gate), note = null)
            refreshState()
            return
        }
        val generation = engineLifecycle.generation
        engineScope.launch {
            val outcome = withContext(Dispatchers.IO) {
                try {
                    startSend()
                    Result.success(Unit)
                } catch (e: CancellationException) {
                    throw e
                } catch (t: Throwable) {
                    Result.failure(t)
                }
            }
            if (destroyed || !engineLifecycle.isCurrent(generation)) return@launch
            outcome.fold(
                onSuccess = {
                    localSendStarted = true
                    senderState = senderState.copy(sending = true, note = null, error = null)
                },
                onFailure = { error ->
                    localSendStarted = false
                    senderState = senderState.copy(
                        sending = false,
                        error = SenderStateMapper.noteForSendFailure(ffiCodeOf(error), ffiContextOf(error)),
                    )
                },
            )
            refreshState()
        }
    }

    /**
     * 停止推流。**只停流、不断连**。
     *
     * 依据：内核 `Engine::stop_send` 的文档原文是「关闭与对端的流（**保留连接与信任**）」，
     * 它只发一条 `CLOSE_STREAM` 控制帧并停采集（`SessionCommand::CloseStream` 分支），
     * 会话本身留在表里 —— 所以文案是「已停止发送」而不是「已断开」，用户再点一次就能继续。
     */
    private fun stopSender(note: String = STOPPED_SENDING_NOTE) {
        localSendStarted = false
        senderState = senderState.copy(sending = false, note = note, error = null)
        val generation = engineLifecycle.generation
        engineScope.launch {
            val outcome = withContext(Dispatchers.IO) {
                try {
                    stopSend()
                    Result.success(Unit)
                } catch (e: CancellationException) {
                    throw e
                } catch (t: Throwable) {
                    Result.failure(t)
                }
            }
            if (destroyed || !engineLifecycle.isCurrent(generation)) return@launch
            outcome.onFailure { error ->
                senderState = senderState.copy(
                    error = SenderStateMapper.noteForSendFailure(ffiCodeOf(error), ffiContextOf(error)),
                )
            }
            refreshState()
        }
    }

    /**
     * 会话状态同步：**内核是唯一权威**。会话从表里消失（断开 / 被对端关掉）就把发送状态归零。
     *
     * 空表要当成「还没拉到」而不是「已断开」：刚 `connect()` 完的那一小段里 `peers()` 可能还是空的，
     * 那会儿把状态清零会让用户看到一条假断开。
     */
    private fun syncSenderWithPeers() {
        val expected = senderState.peerIdShort
        if (expected == null) {
            // 还没有登记会话：**PIN 配对成功后的登记就发生在这里**。
            // 为什么必须补这一段（2026-09-18 真机定位的真缺陷）：需要 PIN 时 `connect()` 返回的是
            // `1002 NOT_PAIRED`（不是成功），所以 `peerIdShort` 一直是 null；而旧版这里
            // `?: return` 直接放行 ⇒ 会话**永远登记不进来**：界面停在「未连接」，`startSender()` 的
            // `sessionGate` 又按 `PeerMismatch` 把「开始发送」禁用 —— 而桌面端此刻明明已经显示
            // 「已配对（白名单命中）」且对端数 = 1，链路是通的。
            // 只登记**唯一的那一个**对端：单对端 UI 语义（契约 §8），多个会话时不猜（宁可让用户重新连接，
            // 也不能把声音发到错误的设备上）。
            if (senderState.awaitingPin || senderState.error != null) return
            val only = pairingState.peers.singleOrNull() ?: return
            senderState = senderState.copy(
                peerIdShort = only.idShort,
                peerState = only.state,
                peerStateLabel = only.stateLabel,
                note = null,
            )
            return
        }
        if (pairingState.peers.isEmpty()) return
        val peer = pairingState.peers.firstOrNull { it.idShort.equals(expected, ignoreCase = true) }
        senderState = if (peer == null) {
            localSendStarted = false
            senderState.copy(
                peerIdShort = null,
                peerState = "",
                peerStateLabel = "",
                sending = false,
                awaitingPin = false,
                note = "与电脑的连接已断开",
            )
        } else {
            senderState.copy(peerState = peer.state, peerStateLabel = peer.stateLabel)
        }
    }

    /** FFI 失败的错误码（`docs/03-protocol.md` §11 的数值）；非 FFI 异常返回 0。 */
    private fun ffiCodeOf(error: Throwable): Int = when (error) {
        is FfiException.Failure -> error.code.toInt()
        else -> 0
    }

    /** FFI 失败的机械上下文；非 FFI 异常退回 `message`。 */
    private fun ffiContextOf(error: Throwable): String? = when (error) {
        is FfiException.Failure -> error.context
        else -> error.message
    }

    private fun startEngine() {
        engineLifecycle.start()
        // 引擎起来后立刻刷一次：把「引擎启动前登记的」发送源请求落地（[refreshState] 会消费标量请求），
        // 也让 UI 尽快看到 `engineRunning = true`（不必等下一拍）。
        mainHandler.post { refreshState() }
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
    private fun foregroundServiceTypes(): Int {
        var types = when {
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.R ->
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK or
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE

            Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q ->
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK

            else -> 0
        }
        // 采集用的类型位：Android 14+ 要求「正在使用的类型」必须出现在 startForeground 的类型位里，
        // 否则系统按「未声明用途」处置（麦克风被静音、内录拿不到投影）。
        // 版本门槛：microphone 自 API 30 起有该位，mediaProjection 自 API 29 起有该位。
        when (CaptureWiring.extraForegroundServiceType(captureSelection)) {
            CaptureServiceType.None -> Unit

            // 麦克风：与内录同一条纪律 —— 只有**运行时权限真的在手里**才带这个位。
            // Android 14+ 的 microphone 前台服务除 manifest 权限外还要求 RECORD_AUDIO 已授予；
            // 用户撤权（或「仅这一次」失效）后仍以该位 startForeground → SecurityException 带走服务进程。
            CaptureServiceType.Microphone -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R &&
                checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
            ) {
                types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
            }

            // 内录：**只有授权到手之后**才带这个类型位（理由见 [loopbackAuthorized]）；
            // 授权之前带上它，`startForeground` 会直接抛 SecurityException 并把服务进程带走。
            CaptureServiceType.MediaProjection -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
                loopbackAuthorized
            ) {
                types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION
            }
        }
        return types
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
            playbackNote = text
            getSystemService(NotificationManager::class.java).notify(
                NOTIFICATION_ID,
                buildNotification(composeNotificationText()),
            )
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

        // 消费 UI 的发送源请求（请求式入口，见 companion 的 requestCaptureSource）。
        if (captureSourceRequestPending) {
            captureSourceRequestPending = false
            applyCaptureSource(requestedCaptureSource)
        }

        val snapshot = player?.stats()
        val captureSnapshot = captureController.snapshot()
        val captureRing = captureController.ringStats()
        val captureNote = captureNotice
            ?: CaptureWiring.captureNote(captureSelection, captureSnapshot)
        val engine = engineStatus
        // 发送方向：会话状态以 pairingState.peers 为权威（同一拍里刚刷新，见 syncSenderWithPeers）。
        syncSenderWithPeers()
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

            // ---- 发送采集（FR-06/07）----
            captureSelection = captureSelection,
            captureState = captureSnapshot.state,
            captureAttempts = captureSnapshot.attempts,
            captureNote = captureNote,
            captureRingOverflowFrames = captureRing.overflowFrames,
            captureRingAvailableFrames = captureRing.availableFrames,

            // ---- 发送方向（本机 → 对端）----
            sender = senderState.copy(
                sending = SenderStateMapper.isSending(localSendStarted, senderState),
            ),
        )

        // 采集状态变化要反映到通知正文：否则用户只看到「正在播放」，采集失败/被拒是**静默**的。
        // 只在文本变化时 notify（每 500 ms 无意义刷新会被系统降频，也浪费一次跨进程调用）。
        val text = composeNotificationText()
        if (text != lastNotificationText) {
            lastNotificationText = text
            try {
                getSystemService(NotificationManager::class.java)
                    .notify(NOTIFICATION_ID, buildNotification(text))
            } catch (_: Throwable) {
                // 通知失败不该影响音频路径（用户关掉通知权限时系统本就会丢弃）。
            }
        }
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
                            // 遥测：首屏「延迟 / 码率 / 丢包」三个读数的唯一来源（字段说明见 PeerUi）。
                            // UInt -> Long：内核给的是无符号，转宽比截断安全。
                            e2eLatencyUs = peer.telemetry.e2eLatencyUs.toLong(),
                            bitrateBps = peer.telemetry.bitrateBps.toLong(),
                            lossPct = peer.telemetry.lossPct,
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
