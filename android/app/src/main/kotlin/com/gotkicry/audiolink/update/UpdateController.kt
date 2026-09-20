package com.gotkicry.audiolink.update

import android.content.Context
import android.os.Build
import com.gotkicry.audiolink.BuildConfig
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import java.io.File
import java.io.IOException
import java.net.SocketTimeoutException
import java.net.UnknownHostException

/** 「软件更新」面板的状态机。 */
sealed interface UpdateState {

    /** 还没查过。 */
    data object Idle : UpdateState

    data object Checking : UpdateState

    /** 已是最新（把当前版本显示出来，让用户确认自己在哪个版本上）。 */
    data class UpToDate(val currentVersion: String) : UpdateState

    /** 有新版，且已经选好了与本机 ABI 匹配的包。 */
    data class Available(
        val currentVersion: String,
        val info: ReleaseInfo,
        val asset: ReleaseAsset,
    ) : UpdateState

    /** [percent] 为 -1 表示服务端没给长度（不定进度）。 */
    data class Downloading(val version: String, val percent: Int) : UpdateState

    data class ReadyToInstall(val version: String, val apk: File) : UpdateState

    /** 缺「安装未知应用」授权：授权页已拉起，用户回来后点「继续安装」即可。 */
    data class NeedsPermission(val version: String, val apk: File) : UpdateState

    data class Failed(val message: String) : UpdateState
}

/**
 * 应用内更新的控制器（Android 侧）。
 *
 * 与桌面端同一条立场（见 `desktop/src/i18n.ts` 的 `upd.hint`）：**更新只在用户点击时检查**，
 * 不在后台自动下载或安装 —— 正在推流/录音时被静默重启是不可接受的。
 *
 * 安全模型（三层，缺一不可）：
 *  1. 版本与下载地址来自**公开仓库的 GitHub Releases**（无需凭据，也无法被单方面篡改）；
 *  2. 下载完成后**比对 APK 的签名证书指纹**（[ApkSignature]）——不是我们自己签的包直接删掉；
 *  3. 交给系统安装器后，Android 还会再校一次「签名与已装版本一致」。
 */
class UpdateController(context: Context) {

    private val appContext = context.applicationContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private val mutableState = MutableStateFlow<UpdateState>(UpdateState.Idle)
    val state: StateFlow<UpdateState> = mutableState.asStateFlow()

    /** 本机当前版本（`build.gradle.kts` 的 `versionName` → `BuildConfig`）。 */
    val currentVersion: String = BuildConfig.VERSION_NAME

    /** 本机支持的 ABI（系统已按优先级排序）。 */
    val supportedAbis: List<String> = Build.SUPPORTED_ABIS.toList()

    /** 用户点「检查更新」。 */
    fun check() {
        if (mutableState.value is UpdateState.Checking) return
        mutableState.value = UpdateState.Checking
        scope.launch {
            try {
                val info = GitHubReleasesClient.fetchLatest()
                if (VersionCompare.compare(info.version, currentVersion) <= 0) {
                    mutableState.value = UpdateState.UpToDate(currentVersion)
                    return@launch
                }
                val asset = ReleaseSelection.apkForAbis(info.assets, supportedAbis)
                mutableState.value = if (asset == null) {
                    UpdateState.Failed("这次发布没有适配本机的安装包（ABI: " + supportedAbis.joinToString("/") + "）")
                } else {
                    UpdateState.Available(currentVersion, info, asset)
                }
            } catch (error: Throwable) {
                mutableState.value = UpdateState.Failed(describe(error))
            }
        }
    }

    /** 用户点「下载并安装」：下载 → 验签 → 落到可安装态（真正的安装由 [install] 触发）。 */
    fun download() {
        val available = mutableState.value as? UpdateState.Available ?: return
        val version = available.info.version
        mutableState.value = UpdateState.Downloading(version, -1)
        scope.launch {
            try {
                val directory = File(appContext.filesDir, "updates")
                val target = ApkDownload.targetFile(directory, version, available.asset.name)
                ApkDownload.download(available.asset.downloadUrl, target) { percent ->
                    mutableState.value = UpdateState.Downloading(version, percent)
                }

                // debug 包是用 debug keystore 签的，与 release 证书天然不同 —— 此时只提示、不删除，
                // 免得开发机上反复下载。release 包则必须过这一关。
                if (!BuildConfig.DEBUG && !ApkSignature.isSignedByReleaseKey(appContext, target)) {
                    target.delete()
                    mutableState.value = UpdateState.Failed(
                        "下载到的安装包签名与本应用不一致，已删除（可能被替换过）",
                    )
                    return@launch
                }
                mutableState.value = UpdateState.ReadyToInstall(version, target)
            } catch (error: Throwable) {
                mutableState.value = UpdateState.Failed(describe(error))
            }
        }
    }

    /** 用户点「安装」：必要时先要「安装未知应用」授权，然后交给系统安装器。 */
    fun install() {
        val pending = mutableState.value
        val apk = when (pending) {
            is UpdateState.ReadyToInstall -> pending.apk to pending.version
            is UpdateState.NeedsPermission -> pending.apk to pending.version
            else -> return
        }
        if (!ApkInstall.canRequestInstall(appContext)) {
            mutableState.value = UpdateState.NeedsPermission(apk.second, apk.first)
            ApkInstall.openInstallPermissionSettings(appContext)
            return
        }
        try {
            ApkInstall.launchInstaller(appContext, apk.first)
        } catch (error: Throwable) {
            mutableState.value = UpdateState.Failed("无法启动安装器：" + (error.message ?: error.javaClass.simpleName))
        }
    }

    /** 界面离开时取消在飞的请求（下载中断留下的只有 `.part`，不会被误当安装包）。 */
    fun dispose() {
        scope.cancel()
    }

    private fun describe(error: Throwable): String = when (error) {
        is UnknownHostException -> "连不上 GitHub：检查网络后重试"
        is SocketTimeoutException -> "连接 GitHub 超时：网络被限制时可能需要代理"
        is IOException -> "更新失败：" + (error.message ?: "网络错误")
        else -> "更新失败：" + (error.message ?: error.javaClass.simpleName)
    }
}
