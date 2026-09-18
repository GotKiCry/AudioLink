package com.gotkicry.audiolink.ui.theme

import android.content.Context

/**
 * 主题偏好的落地（跟随系统 / 浅 / 深）。
 *
 * **刻意用 SharedPreferences 而不是 DataStore**：这是 UI 层的单个枚举，读写都在主线程一次完成，
 * 引入 Flow/协程只是为了让一行配置异步化。服务层的偏好（省电白名单的"不再提示"）有它自己的
 * 存储，两者互不干扰 —— UI 偏好不该影响音频链路。
 */
object ThemePreference {

    private const val FILE = "audiolink.ui"
    private const val KEY_MODE = "theme_mode"

    fun read(context: Context): ThemeMode {
        val raw = runCatching {
            context.getSharedPreferences(FILE, Context.MODE_PRIVATE).getString(KEY_MODE, null)
        }.getOrNull()
        return ThemeMode.entries.firstOrNull { it.name == raw } ?: ThemeMode.System
    }

    fun write(context: Context, mode: ThemeMode) {
        runCatching {
            context.getSharedPreferences(FILE, Context.MODE_PRIVATE)
                .edit()
                .putString(KEY_MODE, mode.name)
                .apply()
        }
    }
}
