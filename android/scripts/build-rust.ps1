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
$profile    = if ($Release) { "release" } else { "debug" }

function Assert-Env([string]$Name) {
    if (-not (Get-Item "Env:$Name" -ErrorAction SilentlyContinue)) {
        throw "环境变量 $Name 未设置。参见 docs/06-dev-environment.md §2"
    }
}

Assert-Env "ANDROID_HOME"
if (-not $env:NDK_HOME) { Write-Warning "NDK_HOME 未设置，cargo-ndk 将尝试自行探测" }

$targets = @($Abi -split "," | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if ($targets.Count -eq 0) { throw "未指定任何 ABI" }

Write-Host "==> 交叉编译 Rust 内核 ($profile)：$($targets -join ', ')" -ForegroundColor Cyan
$ndkArgs = @("ndk", "-o", $jniLibs)
foreach ($t in $targets) { $ndkArgs += @("-t", $t) }
$ndkArgs += @("build", "-p", "audiolink-ffi", "--profile", $profile)

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
