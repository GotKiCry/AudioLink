# AudioLink —— 用 cargo-ndk 把 Rust 内核交叉编译成 Android 动态库
#
# 用法：
#   pwsh .\scripts\build-rust.ps1                       # debug，双 ABI
#   pwsh .\scripts\build-rust.ps1 -Release              # release（发布用）
#   pwsh .\scripts\build-rust.ps1 -Abi arm64-v8a        # 只编 arm64
#
# 前置：
#   * rustup target add aarch64-linux-android armv7-linux-androideabi
#   * cargo install cargo-ndk --locked
#   * $env:ANDROID_HOME / $env:NDK_HOME 指向本机 SDK 与 NDK（见 docs/06-dev-environment.md）

param(
    [string]$Abi = "arm64-v8a,armeabi-v7a",
    [switch]$Release,
    [switch]$SkipUniffi
)

$ErrorActionPreference = "Stop"

$androidDir = Split-Path -Parent $PSScriptRoot
$repoRoot   = Split-Path -Parent $androidDir
$jniLibs    = Join-Path $androidDir "app\src\main\jniLibs"
# 注意 1：Cargo 的开发 profile 名为 `dev`（`debug` 是保留名，会被直接拒绝）。
# 注意 2：变量不要叫 $profile —— PowerShell 变量名不区分大小写，会与自动变量 $PROFILE 撞名。
$cargoProfile = if ($Release) { "release" } else { "dev" }

# ---- 解析 Android SDK / NDK：环境变量 → android/local.properties → 默认路径 ----
function Resolve-AndroidSdk {
    if ($env:ANDROID_HOME)     { return $env:ANDROID_HOME }
    if ($env:ANDROID_SDK_ROOT) { return $env:ANDROID_SDK_ROOT }
    $lp = Join-Path $androidDir "local.properties"
    if (Test-Path $lp) {
        $line = Get-Content $lp | Where-Object { $_ -match '^\s*sdk\.dir\s*=' } | Select-Object -First 1
        if ($line) { return (($line -replace '^\s*sdk\.dir\s*=\s*', '').Replace('\', '/').Trim()) }
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

Write-Host "==> 交叉编译 Rust 内核 ($cargoProfile)：$($targets -join ', ')" -ForegroundColor Cyan
$ndkArgs = @("ndk", "-o", $jniLibs)
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
    Write-Host "==> 生成 Kotlin 绑定（UniFFI）" -ForegroundColor Cyan
    # TODO(M0)：内核 UDL/派生稳定后启用：
    #   cargo run -p audiolink-ffi --bin uniffi-bindgen generate `
    #     --library "$jniLibs\arm64-v8a\libaudiolink_ffi.so" `
    #     --language kotlin --out-dir android/app/src/main/kotlin
    Write-Host "    （当前为占位：UniFFI 绑定生成待 M0 内核接口定型后启用）" -ForegroundColor DarkGray
}

Write-Host "==> 产物：" -ForegroundColor Green
foreach ($t in $targets) {
    Get-ChildItem (Join-Path $jniLibs $t) -Filter *.so -ErrorAction SilentlyContinue |
        ForEach-Object { "    {0}  ({1:N1} MB)" -f $_.FullName.Substring($repoRoot.Length + 1), ($_.Length / 1MB) }
}
