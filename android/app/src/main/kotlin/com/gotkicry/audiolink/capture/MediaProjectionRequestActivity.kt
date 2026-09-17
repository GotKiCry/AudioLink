package com.gotkicry.audiolink.capture

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.media.projection.MediaProjectionManager
import android.os.Bundle

/**
 * 系统内录（AudioPlaybackCapture）授权中转页（FR-06）。
 *
 * 流程：用户选择「系统内录」→ 本页展示说明 → 拉起 MediaProjection 系统授权 → **把结果回执给服务**，
 * 由服务创建 `MediaProjection` 并交给 [CaptureController.startLoopback]。
 *
 * ## 为什么本页**不**创建 `MediaProjection`（本轮改动的要点）
 * 1. 旧版本在这里 `getMediaProjection(...)` 之后立刻 `stop()`（带着 `TODO(M1)`）—— 那等于**拿到授权就毁掉它**，
 *    服务侧再想用只会拿到一个已经停止的投影；
 * 2. Android 14+ 要求「应用已在前台服务（类型含 `mediaProjection`）中」才能取得投影，
 *    而**前台服务的生命周期在服务手里**，不在这个 Activity 手里；
 * 3. 每次会话都要重新授权是系统行为，所以「谁持有投影」必须只有一个答案 —— 服务。
 *
 * ## 服务侧接线（**service/ 不在本模块写作用域内，需另行接线**）
 * 在 `AudioLinkService.onStartCommand` 里处理下面两个 action：
 * ```kotlin
 * when (intent?.action) {
 *     ACTION_CAPTURE_GRANTED -> {
 *         val code = intent.getIntExtra(EXTRA_RESULT_CODE, Activity.RESULT_CANCELED)
 *         val data = intent.getParcelableExtra<Intent>(EXTRA_RESULT_DATA)
 *         val projection = (getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager)
 *             .getMediaProjection(code, data)
 *         capture.startLoopback(projection)
 *     }
 *     ACTION_CAPTURE_DENIED -> capture.stop()
 * }
 * ```
 * 前提：调用前服务必须已经 `startForeground(...)` 且类型含 `mediaProjection`（见 manifest）。
 *
 * ## 用户可见的规则（UI 规格已写，实现时不得删）
 * * 需 **Android 10（API 29）+**，低版本入口置灰；
 * * 目标应用可声明 `allowAudioPlaybackCapture=false`，DRM 内容永远不可捕获 → UI 需给出解释；
 * * Android 14+ 每次会话都需重新授权（系统行为，无法绕过）。
 */
class MediaProjectionRequestActivity : Activity() {

    companion object {
        /** 授权结果码（服务侧读）。 */
        const val EXTRA_RESULT_CODE = "result_code"

        /** 授权结果数据（服务侧读）。 */
        const val EXTRA_RESULT_DATA = "result_data"

        /** 用户同意授权。 */
        const val ACTION_CAPTURE_GRANTED = "com.gotkicry.audiolink.action.CAPTURE_GRANTED"

        /** 用户拒绝授权（或系统直接取消）。 */
        const val ACTION_CAPTURE_DENIED = "com.gotkicry.audiolink.action.CAPTURE_DENIED"

        /**
         * 目标服务的类名。
         *
         * 用字符串而不是 `import`：`capture/` 不该依赖 `service/` 的编译期符号
         * （方向是服务编排采集，不是采集认识服务）。
         */
        private const val SERVICE_CLASS = "com.gotkicry.audiolink.service.AudioLinkService"

        private const val REQUEST_CODE = 0x2001
    }

    private lateinit var manager: MediaProjectionManager

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        manager = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        startActivityForResult(manager.createScreenCaptureIntent(), REQUEST_CODE)
    }

    @Deprecated("MediaProjection 授权结果仍走 onActivityResult；待接入 ActivityResult APIs")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQUEST_CODE) return

        val granted = resultCode == RESULT_OK && data != null
        val reply = Intent(if (granted) ACTION_CAPTURE_GRANTED else ACTION_CAPTURE_DENIED)
            .setClassName(packageName, SERVICE_CLASS)
        if (granted) {
            // 只转发**结果**：投影由服务在「已进入 mediaProjection 前台服务」之后创建（见类注释第 2 条）。
            reply.putExtra(EXTRA_RESULT_CODE, resultCode)
            reply.putExtra(EXTRA_RESULT_DATA, data)
        }
        try {
            startService(reply)
        } catch (error: IllegalStateException) {
            // 服务没起来（或已被系统回收）：授权结果无处安放。这里没有可做的补救，
            // 由用户重新点一次「系统内录」——所以只结束本页，不吞掉其它异常。
        }
        finish()
    }
}
