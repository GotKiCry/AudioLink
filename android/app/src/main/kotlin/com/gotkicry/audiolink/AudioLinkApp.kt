package com.gotkicry.audiolink

import android.app.Application

/**
 * AudioLink 应用入口。
 *
 * 纪律（对应旧版最大教训「功能正确性依赖 Activity 被打开过」）：
 * 这里只做进程级初始化，**不承载业务状态**；服务与 UI 各自持有依赖，互不依赖对方启动顺序。
 */
class AudioLinkApp : Application() {
    override fun onCreate() {
        super.onCreate()
        // TODO(M1)：初始化日志、身份（自签证书）与信任库
        // TODO(M1)：加载 Rust 内核（System.loadLibrary("audiolink_ffi") 由 FFI 层完成）
    }
}
