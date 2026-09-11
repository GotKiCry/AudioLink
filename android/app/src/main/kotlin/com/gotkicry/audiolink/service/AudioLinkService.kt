package com.gotkicry.audiolink.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import com.gotkicry.audiolink.MainActivity

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
 * TODO(M1)：接入 Rust 内核（经 FFI 启动 QUIC 端点、会话与播放调度），
 *          并用 Kotlin 侧 AudioTrack（PERFORMANCE_MODE_LOW_LATENCY + WRITE_NON_BLOCKING）消费 PCM。
 */
class AudioLinkService : Service() {

    companion object {
        const val CHANNEL_PLAYBACK = "playback"
        const val NOTIFICATION_ID = 0x1001
        const val ACTION_STOP = "com.gotkicry.audiolink.action.STOP"
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopSelf()
            return START_NOT_STICKY
        }
        startForeground(NOTIFICATION_ID, buildNotification("已就绪"), foregroundServiceTypes())
        // TODO(M1)：启动内核引擎与会话状态流
        return START_STICKY
    }

    override fun onDestroy() {
        // TODO(M1)：优雅关闭所有会话（发送 BYE）并释放 AudioTrack / 采集源
        super.onDestroy()
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

    private fun createChannel() {
        val channel = NotificationChannel(
            CHANNEL_PLAYBACK,
            getString(com.gotkicry.audiolink.R.string.notif_channel_playback),
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
            .setContentTitle(getString(com.gotkicry.audiolink.R.string.app_name))
            .setContentText(text)
            .setSmallIcon(com.gotkicry.audiolink.R.mipmap.ic_launcher)
            .setContentIntent(open)
            // 用 Notification.Action.Builder 而非已废弃的 addAction(int, CharSequence, PendingIntent)
            .addAction(
                Notification.Action.Builder(
                    android.graphics.drawable.Icon.createWithResource(
                        this,
                        com.gotkicry.audiolink.R.mipmap.ic_launcher,
                    ),
                    getString(com.gotkicry.audiolink.R.string.action_stop),
                    stop,
                ).build(),
            )
            .setOngoing(true)
            .build()
    }
}
