package com.gotkicry.audiolink.compliance

/**
 * 第三方组件声明的加载结果（M5 合规：Android 侧的投放）。
 *
 * 为什么单独成类型而不是直接返回字符串：**找不到时必须说清怎么生成**，而不是给一个空白页 ——
 * 空白页只会让人以为应用坏了。这条与桌面端的「关于」面板保持一致（docs/41 §7）。
 */
data class NoticesState(
    val available: Boolean,
    val bytes: Int,
    val text: String,
)

object NoticesLoader {
    /** 打进 APK 的声明文件名（由 tools/license-audit.ps1 -Notices 同步到 assets）。 */
    const val ASSET_NAME = "THIRD-PARTY-NOTICES.md"

    const val MISSING_HINT =
        "第三方组件声明尚未生成。在仓库根执行：pwsh tools/license-audit.ps1 -Notices"

    /**
     * 纯逻辑：把「读到的内容」折成界面要的形状。
     *
     * 刻意做成纯函数：读 assets 需要 Context（真机/仪器测试才能覆盖），
     * 而真正需要钉住的是**「找不到怎么办」这条规则** —— 它可以在纯 JVM 上单测。
     */
    fun fromText(text: String?): NoticesState {
        if (text.isNullOrEmpty()) {
            return NoticesState(available = false, bytes = 0, text = MISSING_HINT)
        }
        return NoticesState(
            available = true,
            bytes = text.toByteArray(Charsets.UTF_8).size,
            text = text,
        )
    }
}
