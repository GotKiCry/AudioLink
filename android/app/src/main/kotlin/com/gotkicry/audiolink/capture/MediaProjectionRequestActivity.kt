package com.gotkicry.audiolink.capture

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Bundle

/**
 * 系统内录（AudioPlaybackCapture）授权中转页（FR-06）。
 *
 * 流程：用户选择「系统内录」→ 本页展示说明 → 拉起 MediaProjection 系统授权 → 回传结果给
 * [com.gotkicry.audiolink.service.AudioLinkService] 启动 AudioPlaybackCapture。
 *
 * 注意（已在 UI 规格中说明，必须遵守）：
 * * 需 **Android 10（API 29）+**；低版本入口置灰；
 * * 目标应用可声明 `allowAudioPlaybackCapture=false`，DRM 内容永远不可捕获 → UI 需给出解释；
 * * Android 14+ 每次会话都需重新授权（系统行为，无法绕过）。
 */
class MediaProjectionRequestActivity : Activity() {

    companion object {
        const val EXTRA_RESULT_CODE = "result_code"
        const val EXTRA_RESULT_DATA = "result_data"
        private const val REQUEST_CODE = 0x2001
    }

    private lateinit var manager: MediaProjectionManager

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        manager = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        startActivityForResult(manager.createScreenCaptureIntent(), REQUEST_CODE)
    }

    @Deprecated("MediaProjection 授权结果仍走 onActivityResult；待 M1 接入 ActivityResult APIs")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQUEST_CODE) return
        if (resultCode == RESULT_OK && data != null) {
            val projection: MediaProjection? = manager.getMediaProjection(resultCode, data)
            // TODO(M1)：把 projection 交给服务，构建 AudioPlaybackCaptureConfiguration (USAGE_MEDIA/GAME)
            projection?.stop()
        }
        finish()
    }
}
