package com.gotkicry.audiolink.update

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/** GitHub Release 里的一个可下载资产。 */
data class ReleaseAsset(
    val name: String,
    val downloadUrl: String,
    val sizeBytes: Long,
)

/** 从 GitHub Releases API 的 `releases/latest` 解析出的「最新正式版」。 */
data class ReleaseInfo(
    /** 规范化后的版本号（去掉 tag 的 `v` 前缀与预发布后缀），例如 `0.1.1`。 */
    val version: String,
    val notes: String,
    val publishedAt: String?,
    val assets: List<ReleaseAsset>,
)

/**
 * 版本比较：与桌面端 `tauri-plugin-updater` 同一判据（**远端 > 本地** 才提示更新）。
 *
 * 口径（与 `docs/42-m5-release-pipeline.md` §12.5 记的边界一致）：
 *  * 只比数字段，逐段比较、缺段按 0 补齐（`0.2` 与 `0.2.0` 相等）；
 *  * 预发布后缀（`-beta`）**不参与比较** —— 插件侧的 semver 才是权威；Android 侧目前
 *    只发布正式版，这个简化是可接受的，且**会在遇到预发布时偏保守**（当作正式版比较）。
 */
object VersionCompare {

    /** 去掉 `v`/`V` 前缀与 `-pre`/`+build` 后缀：`v0.1.1-beta` → `0.1.1`。 */
    fun normalize(raw: String): String =
        raw.trim()
            .removePrefix("v")
            .removePrefix("V")
            .substringBefore('-')
            .substringBefore('+')

    /** a > b → 正；a == b → 0；a < b → 负。非法段按 0 处理（不抛异常：版本号来自网络）。 */
    fun compare(a: String, b: String): Int {
        val left = segments(a)
        val right = segments(b)
        for (index in 0 until maxOf(left.size, right.size)) {
            val x = left.getOrElse(index) { 0 }
            val y = right.getOrElse(index) { 0 }
            if (x != y) return x.compareTo(y)
        }
        return 0
    }

    private fun segments(version: String): List<Int> =
        normalize(version).split('.').map { segment ->
            segment.takeWhile { it.isDigit() }.toIntOrNull() ?: 0
        }
}

/** 从一次 Release 的资产列表里挑出与本机 ABI 匹配的那一个。 */
object ReleaseSelection {

    /** CI 产出的 APK 命名（见 `release.yml` 的「分 ABI 打包」）。 */
    fun apkNameFor(abi: String): String = "app-" + abi + "-release.apk"

    /**
     * 按 [abis] 的**优先级顺序**（`Build.SUPPORTED_ABIS` 已排序）找第一个存在的包。
     * 找不到返回 null —— 调用方据此给出「这次发布没有适配你设备的包」，而不是装错 ABI 的包。
     */
    fun apkForAbis(assets: List<ReleaseAsset>, abis: List<String>): ReleaseAsset? {
        for (abi in abis) {
            val wanted = apkNameFor(abi)
            assets.firstOrNull { it.name == wanted }?.let { return it }
        }
        return null
    }
}

/** GitHub Releases API 的最小解析器（只取用得到的字段，忽略其余）。 */
object ReleaseParser {

    /**
     * 解析 `GET /repos/{owner}/{repo}/releases/latest` 的响应体。
     *
     * 为什么手写解析而不是 `@Serializable`：本模块的 Kotlin 序列化插件没有打开
     * （`build.gradle.kts` 只挂了 compose 插件），而 `kotlinx-serialization-json` 的
     * JsonElement API 不需要代码生成 —— 为一次 API 调用引入编译期插件不划算。
     *
     * 缺字段一律**显式失败**（throw）：GitHub 换字段名时要在这一步响，而不是静默当成「已是最新」。
     */
    fun parse(body: String): ReleaseInfo {
        val root = Json.parseToJsonElement(body).jsonObject
        val tag = root.stringOrThrow("tag_name")
        val assets = root["assets"]?.jsonArray.orEmpty().map { element ->
            val asset = element.jsonObject
            ReleaseAsset(
                name = asset.stringOrThrow("name"),
                downloadUrl = asset.stringOrThrow("browser_download_url"),
                sizeBytes = asset["size"]?.jsonPrimitive?.contentOrNull?.toLongOrNull() ?: 0L,
            )
        }
        return ReleaseInfo(
            version = VersionCompare.normalize(tag),
            notes = root["body"]?.jsonPrimitive?.contentOrNull.orEmpty(),
            publishedAt = root["published_at"]?.jsonPrimitive?.contentOrNull,
            assets = assets,
        )
    }

    private fun JsonObject.stringOrThrow(key: String): String =
        this[key]?.jsonPrimitive?.contentOrNull
            ?: throw IllegalStateException("Release 响应缺少字段 " + key)
}
