# Android 应用内更新（GitHub Releases）

> 日期 **2026-09-20**。桌面端的自动更新在 `docs/42` §12 已有完整记录；这一份只讲 **Android 侧**
> 新增的更新链路 —— 它与桌面端共用同一个分发渠道（GitHub Releases），但**安全模型与实现完全不同**。

## 0. 一句话

手机上的 AudioLink 现在能自己检查新版本、下载、并把安装包交给系统安装器；**检查由用户点击触发**，
下载完成后会**先校验签名证书**再交出去。

## 1. 为什么不照搬桌面端那套

| | 桌面（Tauri updater） | Android |
|---|---|---|
| 「有没有新版」从哪来 | `releases/latest/download/latest.json`（CI 生成的清单） | `api.github.com/repos/.../releases/latest`（Release 元数据） |
| 怎么保证包没被换过 | **minisign 签名**（私钥在 CI，公钥编进客户端） | **APK 签名证书指纹**（比对内置 SHA-256）+ 系统安装器的签名一致性校验 |
| 谁来装 | 安装器进程（应用 `exit(0)` 后接管） | 系统安装器 Activity（`ACTION_VIEW` + FileProvider） |
| 额外权限 | 无 | `REQUEST_INSTALL_PACKAGES`（按应用授权，需用户手动允许） |

**为什么 Android 不用 minisign**：APK 自带签名，系统安装时本来就会校验「签名与已装版本一致」——
这是 Android 的信任模型；为一个已有保障再移植一套签名库不划算。但系统那道防线**只在覆盖安装时成立**，
所以我们在交给安装器**之前**又自己比了一次证书指纹（§3）。

## 2. 触发方式：只在用户点击时检查

与桌面端同一立场（`desktop/src/i18n.ts` 的 `upd.hint`）：**不后台轮询、不静默下载、不静默安装** ——
正在推流或录音时被静默重启是不可接受的。

入口：控制台顶栏 →「更新」→「检查更新」。

## 3. 安全模型（三层）

1. **来源**：公开仓库的 `releases/latest` —— 只认**正式发布**的版本（草稿与 prerelease 都不算）。
   这与桌面端 `latest.json` 是同一条规则，所以两端「有没有新版」的答案天然一致；
2. **验签**：下载完成后读 APK 的签名证书 SHA-256，与 `ApkSignature.RELEASE_CERT_SHA256` 比对，
   不一致就**删除文件**并报错（`update/ApkSignature.kt`）；
3. **系统层**：交给安装器后，Android 再校一次「签名与已安装版本一致」—— 只有同一把 keystore 签的包
   才能覆盖安装。

⚠️ **换 keystore 时必须同步改 `ApkSignature.RELEASE_CERT_SHA256`**，否则 release 版会拒绝自己的更新。
取值方式：`apksigner verify --print-certs app-arm64-v8a-release.apk`。见 `docs/42-m5-release-pipeline.md` §14.1。

## 4. 选哪个包：按 ABI

CI 产出两个 APK（`app-arm64-v8a-release.apk` / `app-armeabi-v7a-release.apk`，见 `release.yml` 的分 ABI 打包）。
客户端按 `Build.SUPPORTED_ABIS` 的**优先级顺序**找第一个存在的包；全都没有时**明确报错**
（「这次发布没有适配本机的安装包」），绝不装错 ABI —— 那是装上去就崩的故障。

## 5. versionCode 规则（覆盖安装的硬要求）

`versionCode` 从 `versionName` 派生：`major*10000 + minor*100 + patch`
（0.1.0 → 100，0.1.1 → 101，0.2.0 → 200，1.0.0 → 10000）。

**为什么必须动它**：原来写死 `versionCode = 1` —— 那样 0.1.1 的包与 0.1.0 的 code 相同，
系统判为「同版本/降级」直接拒绝，**应用内更新永远装不上**。

派生而不是手写第二个数字：版本只有**一个**真相（`tools/check-version.ps1` 校验的三处里就有 versionName 这处）。

## 6. 未做与边界

| 项 | 说明 |
|---|---|
| debug 包 | 可以检查更新，但**不能应用内安装**（applicationId 带 `.debug` 后缀、签名也不同）；界面会明说 |
| 静默安装 | 系统不允许，也不打算要 |
| Play 上架后 | 若改走 Play 分发，更新应由 Play 负责；本机制适用于「GitHub Release 直装」这条路（当前形态） |
| 增量更新 | 不做。全量 APK 只有 6 MB 量级，差分不值得 |
| 自动重试 | 不做。失败就报错，用户再点一次 —— 与桌面端一致的保守选择 |

## 7. 复现与证据

```powershell
# 编译 + 单测（13 项纯逻辑用例：版本比较 / ABI 选包 / GitHub API 解析）
pwsh tools/gradlew.ps1 -JavaHome '<JDK 17+>' :app:testDebugUnitTest

# 出签名包（需 android/keystore.properties）
pwsh tools/gradlew.ps1 -JavaHome '<JDK 17+>' :app:assembleRelease
$env:LOCALAPPDATA\Android\Sdk\build-tools\<ver>\aapt2.exe dump badging `
    android/app/build/outputs/apk/release/app-arm64-v8a-release.apk | Select-String "package:"
#   → versionCode=100 versionName=0.1.0
```

真机验证入口：装 release 包 → 顶栏「更新」→「检查更新」。
**注意**：仓库当前**还没有正式 Release**（只有草稿），此时应显示「还没有正式发布的版本」—— 那是正确行为。

## 8. 代码落点

| 文件 | 职责 |
|---|---|
| `android/app/src/main/kotlin/.../update/ReleaseInfo.kt` | 数据模型 + 版本比较 + ABI 选包 + API 解析（**纯逻辑，有单测**） |
| `.../update/GitHubReleasesClient.kt` | 一次 GET（`HttpURLConnection`，不引 okhttp） |
| `.../update/ApkDownload.kt` | 下载到 `filesDir/updates`（先写 `.part`，完整才改名） |
| `.../update/ApkSignature.kt` | 签名证书指纹校验 |
| `.../update/ApkInstall.kt` | FileProvider + 系统安装器 + 「安装未知应用」授权引导 |
| `.../update/UpdateController.kt` | 状态机（Idle/Checking/UpToDate/Available/Downloading/Ready/NeedsPermission/Failed） |
| `.../ui/screens/UpdateScreen.kt` | 界面（二级目的地，返回键回控制台） |
