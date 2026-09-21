# AudioLink —— 用 cargo-ndk 把 Rust 内核交叉编译成 Android 动态库
#
# 用法：
#   pwsh .\scripts\build-rust.ps1                       # release，双 ABI（**默认**）
#   pwsh .\scripts\build-rust.ps1 -Debug                # dev profile（只给要调试内核的人）
#   pwsh .\scripts\build-rust.ps1 -Abi arm64-v8a        # 只编 arm64
#
# 为什么默认是 release（2026-09-14 踩过的坑）：
#   dev profile 带 debuginfo，本 crate 的产物是 **94 MB / 75 MB**（双 ABI），进 APK 完全不可接受；
#   而 jniLibs 里的东西会被 `assembleDebug` **原样打包**，所以「随手跑一次脚本」就能把 APK 撑到离谱。
#   release 是 3.7 MB / 2.5 MB —— 相差 25 倍。需要内核级调试（native 断点、符号化堆栈）时才用 -Debug，
#   并且要清楚那份产物**不能拿去发布**。
#
# 前置：
#   * rustup target add aarch64-linux-android armv7-linux-androideabi
#   * cargo install cargo-ndk --locked
#   * $env:ANDROID_HOME / $env:NDK_HOME 指向本机 SDK 与 NDK（见 docs/06-dev-environment.md）

param(
    [string]$Abi = "arm64-v8a,armeabi-v7a",
    [switch]$Debug,
    [switch]$SkipUniffi
)

$ErrorActionPreference = "Stop"

$androidDir = Split-Path -Parent $PSScriptRoot
$repoRoot   = Split-Path -Parent $androidDir
$jniLibs    = Join-Path $androidDir "app\src\main\jniLibs"
# 注意 1：Cargo 的开发 profile 名为 `dev`（`debug` 是保留名，会被直接拒绝）。
# 注意 2：变量不要叫 $profile —— PowerShell 变量名不区分大小写，会与自动变量 $PROFILE 撞名。
# 注意 3：开关叫 `-Debug` 而不是 `-Release`，是为了让**默认值落在安全的那一侧**（见文件头说明）。
$cargoProfile = if ($Debug) { "dev" } else { "release" }
if ($Debug) {
    Write-Warning "正在构建 dev profile 的 .so（带 debuginfo，约 94 MB / 75 MB）——仅用于内核调试，**不要**拿去发布或提交 APK。"
}

# ---- 解析 Android SDK / NDK：环境变量 → android/local.properties → 默认路径 ----

<#
把 local.properties 里的路径值还原成 PowerShell 能用的路径。

为什么需要它：`.properties` 是 Java 格式，**路径里的冒号与反斜杠必须转义**
（Android Studio 生成的就是 `sdk.dir=C\:\\Users\\...\\Sdk`；lint 的 PropertyEscape 也会要求
`C\:/Users/...` 这种写法）。而手工写的往往是未转义的 `C:/Users/...`。
两种写法都必须认，否则会得到一个离谱的报错：`\` 被当成普通字符后路径变成 `C/:/Users/...`，
PowerShell 会报「找不到驱动器。名为"C/"的驱动器不存在」，而真正原因藏在转义规则里。
#>
function ConvertFrom-PropertiesPath([string]$value) {
    $v = $value.Trim()
    # 先还原 `\:` → `:`（否则后面把 `\` 归一成 `/` 时会破坏盘符），再还原 `\\` → `\`。
    $v = $v.Replace('\:', ':').Replace('\\', '\').Replace('\/', '/')
    return $v.Replace('\', '/')
}

function Resolve-AndroidSdk {
    if ($env:ANDROID_HOME)     { return $env:ANDROID_HOME }
    if ($env:ANDROID_SDK_ROOT) { return $env:ANDROID_SDK_ROOT }
    $lp = Join-Path $androidDir "local.properties"
    if (Test-Path $lp) {
        $line = Get-Content $lp | Where-Object { $_ -match '^\s*sdk\.dir\s*=' } | Select-Object -First 1
        if ($line) { return (ConvertFrom-PropertiesPath (($line -replace '^\s*sdk\.dir\s*=\s*', '').Trim())) }
    }
    $guess = Join-Path $env:LOCALAPPDATA "Android\Sdk"
    if (Test-Path $guess) { return $guess }
    throw "无法定位 Android SDK：请设置 ANDROID_HOME，或创建 android/local.properties（见 docs/06-dev-environment.md §3）"
}

function Resolve-Ndk([string]$sdk) {
    if ($env:NDK_HOME)         { return $env:NDK_HOME }
    if ($env:ANDROID_NDK_HOME) { return $env:ANDROID_NDK_HOME }
    $ndkRoot = Join-Path $sdk "ndk"
    if (Test-Path $ndkRoot) {
        $ver = Get-ChildItem $ndkRoot -Directory | Sort-Object Name -Descending | Select-Object -First 1
        if ($ver) { return $ver.FullName }
    }
    return $null
}

$sdkPath = Resolve-AndroidSdk
$ndkPath = Resolve-Ndk $sdkPath
$env:ANDROID_HOME = $sdkPath
$env:ANDROID_SDK_ROOT = $sdkPath
if ($ndkPath) { $env:NDK_HOME = $ndkPath; $env:ANDROID_NDK_HOME = $ndkPath }

Write-Host "SDK : $sdkPath" -ForegroundColor DarkGray
if ($ndkPath) { Write-Host "NDK : $ndkPath" -ForegroundColor DarkGray }
else { Write-Warning "未找到 NDK 目录，cargo-ndk 可能失败（可用 -NdkPath 手工指定）" }

$targets = @($Abi -split "," | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if ($targets.Count -eq 0) { throw "未指定任何 ABI" }

# 清掉不属于本次 targets 的旧 ABI 目录。
# 为什么必须清：`abiFilters` 会把它们挡在包外，但残留的 .so 会**骗人** ——
# 目录里躺着 armeabi-v7a/，看起来像"这个 ABI 还在支持"，而包里其实早没有它了。
if (Test-Path $jniLibs) {
    Get-ChildItem $jniLibs -Directory |
        Where-Object { $targets -notcontains $_.Name } |
        ForEach-Object {
            Write-Host "==> 清理不再构建的 ABI 产物：$($_.Name)" -ForegroundColor DarkYellow
            Remove-Item $_.FullName -Recurse -Force
        }
}

Write-Host "==> 交叉编译 Rust 内核 ($cargoProfile)：$($targets -join ', ')" -ForegroundColor Cyan
# 与 app 的 minSdk=26 对齐；发现层 getifaddrs 从 API 24 起可用。
# cargo-ndk 默认 API 21 会让可执行探针链接失败，也无法正确校验库的系统符号版本。
$ndkArgs = @("ndk", "--platform", "26", "-o", $jniLibs)
foreach ($t in $targets) { $ndkArgs += @("-t", $t) }
$ndkArgs += @("build", "-p", "audiolink-ffi", "--profile", $cargoProfile)

Push-Location $repoRoot
try {
    & cargo @ndkArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo ndk 失败（exit $LASTEXITCODE）" }
}
finally {
    Pop-Location
}

if (-not $SkipUniffi) {
    # 绑定是**入库交付物**（`core/crates/audiolink-ffi/bindings/kotlin`），与 .so 同源同版本。
    # 重新生成必须在**宿主平台**上做（要 x86_64-pc-windows-msvc 的 cdylib 做元数据来源；
    # Android .so 不能喂给 bindgen），所以它不属于本脚本 —— 命令见
    # `core/crates/audiolink-ffi/bindings/README.md`：
    #   cargo build -p audiolink-ffi --features bindgen
    #   cargo run   -p audiolink-ffi --features bindgen --bin uniffi-bindgen -- generate ...
    # 本脚本只负责重编 .so；ffi 侧另有护栏测试，会在"源码改了却没重新生成"时把 cargo test 打红。
    Write-Host "==> Kotlin 绑定：不在此生成（需宿主 cdylib，见 core/crates/audiolink-ffi/bindings/README.md）" -ForegroundColor DarkGray
}

Write-Host "==> 产物：" -ForegroundColor Green
foreach ($t in $targets) {
    Get-ChildItem (Join-Path $jniLibs $t) -Filter *.so -ErrorAction SilentlyContinue |
        ForEach-Object { "    {0}  ({1:N1} MB)" -f $_.FullName.Substring($repoRoot.Length + 1), ($_.Length / 1MB) }
}
