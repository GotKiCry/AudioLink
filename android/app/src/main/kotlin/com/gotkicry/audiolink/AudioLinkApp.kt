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
        // 这里**不做**两件看起来该在这里做的事，它们都已在别处落地：
        // ① 身份（自签证书）与信任库：由内核在 `engineStart(dataDir = filesDir)` 时建立（ADR-011），
        //    与 Application.onCreate 是否跑过无关；
        // ② 加载 Rust 内核：`System.loadLibrary("audiolink_ffi")` 由 FFI 生成的绑定层完成。
    }
}
