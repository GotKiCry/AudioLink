package com.gotkicry.audiolink.update

import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL

/** APK 下载：先写 `.part`，完整落盘后再改名 —— 半个包绝不会被当成可安装的文件。 */
object ApkDownload {

    private const val CONNECT_TIMEOUT_MS = 15_000
    private const val READ_TIMEOUT_MS = 60_000

    /** 下载目录：应用私有、无需任何存储权限。 */
    fun targetFile(dir: File, version: String, assetName: String): File = File(dir, version + "-" + assetName)

    /**
     * 下载 [url] 到 [target]。
     *
     * [onProgress] 收到 0–100；服务端没给 Content-Length 时回调 -1（**不猜**，由 UI 显示不定进度）。
     * 返回落盘后的文件；任何失败抛 [IOException]。
     */
    fun download(url: String, target: File, onProgress: (Int) -> Unit = {}): File {
        val part = File(target.parentFile, target.name + ".part")
        part.parentFile?.mkdirs()
        part.delete()

        val connection = (URL(url).openConnection() as HttpURLConnection).apply {
            requestMethod = "GET"
            connectTimeout = CONNECT_TIMEOUT_MS
            readTimeout = READ_TIMEOUT_MS
            // GitHub 的资产直链会 302 到 objects.githubusercontent.com。
            instanceFollowRedirects = true
            setRequestProperty("Accept", "application/octet-stream")
            setRequestProperty("User-Agent", "AudioLink-Android")
        }
        try {
            val status = connection.responseCode
            if (status != HttpURLConnection.HTTP_OK) throw IOException("下载失败：HTTP $status")

            val total = connection.contentLengthLong
            var written = 0L
            connection.inputStream.use { input ->
                part.outputStream().use { output ->
                    val buffer = ByteArray(64 * 1024)
                    while (true) {
                        val read = input.read(buffer)
                        if (read <= 0) break
                        output.write(buffer, 0, read)
                        written += read
                        onProgress(if (total > 0) ((written * 100) / total).toInt() else -1)
                    }
                }
            }
            if (written == 0L) throw IOException("下载到的内容是空的")
            // 完整落盘后才改名：中断留下的只会是 .part。
            target.delete()
            if (!part.renameTo(target)) throw IOException("无法保存到 " + target.name)
            return target
        } finally {
            connection.disconnect()
        }
    }
}
