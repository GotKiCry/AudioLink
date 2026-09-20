package com.gotkicry.audiolink.update

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import java.io.File
import java.security.MessageDigest

/**
 * 下载包的**签名校验**：这一层是「自愿更新」唯一能被欺骗的地方，所以单独成对象、单独测。
 *
 * 威胁模型：用户点了「下载并安装」，我们从一个公开 URL 拉了一个 APK。如果这个 APK 不是我们自己
 * 签的，装上就等于把设备交出去。系统安装器**会**拒绝「签名与已装版本不一致」的包 —— 但那只在
 * **覆盖安装**时成立；诱导用户先卸载再装，那道防线就没了。所以这里在**交给安装器之前**先自己
 * 校一遍证书指纹，把「装错包」变成「这一步就报错」。
 *
 * 为什么不用 minisign（桌面端那套）：Android 上没有现成实现，移植一套签名库为这一件事不值得；
 * 而 APK 自带的签名 + 指纹比对，在 Android 的信任模型里本来就是标准做法。
 */
object ApkSignature {

    /**
     * 现役 release keystore 的证书 SHA-256（小写、无冒号，与 `apksigner` 的输出同格式）。
     *
     * ⚠️ **换 keystore 时必须同步改这里**，否则 release 版会拒绝安装自己的更新。
     * 取值方式：`apksigner verify --print-certs app-arm64-v8a-release.apk`。
     * 见 `docs/42-m5-release-pipeline.md` §14.1 与 `~/.audiolink/README.md`。
     */
    const val RELEASE_CERT_SHA256 =
        "593b7766db0956fe073c251df36c81915a253a6a7c08173a40e6a36fea349c3c"

    /** 读一个 **APK 文件**（不是已安装的包）的签名证书指纹；读不到返回 null。 */
    @Suppress("DEPRECATION")
    fun certificateSha256(context: Context, apk: File): String? {
        val manager = context.packageManager
        val info = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            manager.getPackageArchiveInfo(apk.absolutePath, PackageManager.GET_SIGNING_CERTIFICATES)
        } else {
            manager.getPackageArchiveInfo(apk.absolutePath, PackageManager.GET_SIGNATURES)
        } ?: return null

        val signers = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            info.signingInfo?.apkContentsSigners
        } else {
            info.signatures
        } ?: return null
        val first = signers.firstOrNull() ?: return null

        val digest = MessageDigest.getInstance("SHA-256").digest(first.toByteArray())
        val hex = StringBuilder(digest.size * 2)
        for (byte in digest) {
            val value = byte.toInt() and 0xFF
            if (value < 0x10) hex.append('0')
            hex.append(value.toString(16))
        }
        return hex.toString()
    }

    /** 这个 APK 是不是由现役 release 证书签的。 */
    fun isSignedByReleaseKey(context: Context, apk: File): Boolean =
        certificateSha256(context, apk)?.equals(RELEASE_CERT_SHA256, ignoreCase = true) == true
}

/** 把 APK 交给系统安装器（含「安装未知应用」授权引导）。 */
object ApkInstall {

    /** FileProvider authority —— 必须与 `AndroidManifest.xml` 里的声明一字不差。 */
    fun authority(context: Context): String = context.packageName + ".fileprovider"

    /** 是否已允许本应用安装未知来源的应用（API 26+ 起是**按应用**授权的）。 */
    fun canRequestInstall(context: Context): Boolean =
        context.packageManager.canRequestPackageInstalls()

    /** 跳到「安装未知应用」的授权页。用户授权后返回本应用即可继续安装。 */
    fun openInstallPermissionSettings(context: Context) {
        val intent = Intent(
            Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
            Uri.parse("package:" + context.packageName),
        ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(intent)
    }

    /**
     * 拉起系统安装器。
     *
     * 用 `content://`（FileProvider）而不是 `file://`：API 24+ 上后者会直接
     * 抛 `FileUriExposedException`。安装结果**不回调本进程** —— 安装成功时本进程会被杀掉，
     * 所以 UI 只承诺「已交给安装器」，不承诺「装完了」。
     */
    fun launchInstaller(context: Context, apk: File) {
        val uri = FileProvider.getUriForFile(context, authority(context), apk)
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, "application/vnd.android.package-archive")
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        context.startActivity(intent)
    }
}
