package com.gotkicry.audiolink.update

import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL

/**
 * GitHub Releases 的只读客户端（未认证：仓库是公开的）。
 *
 * 为什么用 `HttpURLConnection` 而不是引入 okhttp：`build.gradle.kts` 里有一条明写的纪律
 * ——「不引入 okhttp / ExoPlayer / AndroidAsync」。本功能只需要一次 GET + 一次文件下载，
 * 平台自带的实现足够，为它引一个 HTTP 栈会让 APK 多几百 KB 和一份需要跟着升级的供应链。
 *
 * 端点语义：`/releases/latest` 只返回**最新的正式版**（draft 与 prerelease 都不算）——
 * 这与桌面端 updater 的 `releases/latest/download/latest.json` 是同一条规则，
 * 所以两端「有没有新版本」的答案天然一致。
 */
object GitHubReleasesClient {

    const val OWNER = "GotKiCry"
    const val REPO = "AudioLink"

    const val LATEST_API_URL = "https://api.github.com/repos/" + OWNER + "/" + REPO + "/releases/latest"

    private const val CONNECT_TIMEOUT_MS = 10_000
    private const val READ_TIMEOUT_MS = 20_000

    /** 拉取最新正式版的元数据。任何失败都抛 [IOException]，由上层翻译成人话。 */
    fun fetchLatest(): ReleaseInfo {
        val connection = (URL(LATEST_API_URL).openConnection() as HttpURLConnection).apply {
            requestMethod = "GET"
            connectTimeout = CONNECT_TIMEOUT_MS
            readTimeout = READ_TIMEOUT_MS
            // GitHub API 要求带 Accept 与 User-Agent（不带给 403）。
            setRequestProperty("Accept", "application/vnd.github+json")
            setRequestProperty("X-GitHub-Api-Version", "2022-11-28")
            setRequestProperty("User-Agent", "AudioLink-Android")
            instanceFollowRedirects = true
        }
        try {
            val status = connection.responseCode
            if (status != HttpURLConnection.HTTP_OK) {
                // 404 的常见原因是「还没有任何正式 Release」（全是草稿）—— 这句话直接写给用户看。
                val hint = if (status == HttpURLConnection.HTTP_NOT_FOUND) {
                    "（还没有正式发布的版本）"
                } else {
                    ""
                }
                throw IOException("GitHub 返回 $status$hint")
            }
            val body = connection.inputStream.bufferedReader().use { it.readText() }
            return ReleaseParser.parse(body)
        } finally {
            connection.disconnect()
        }
    }
}
