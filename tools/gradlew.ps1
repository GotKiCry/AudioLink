# AudioLink —— Android Gradle 包装脚本（Windows 本地开发用）
#
# 为什么需要它：
#   Gradle 9 的 **launcher JVM** 必须是 JDK 17+。本机全局 JAVA_HOME 可能指向 JDK 11，
#   此时 `.\gradlew.bat` 会直接启动失败。而把 JDK 路径写进 android/gradle.properties
#   又会让 CI（Ubuntu）因路径不存在而挂掉 —— 所以改由本脚本在运行时校验/注入。
#
# 用法：
#   pwsh tools\gradlew.ps1 assembleDebug
#   pwsh tools\gradlew.ps1 -JavaHome 'C:\path\to\jdk-17' assembleRelease
#   pwsh tools\gradlew.ps1 --version

param(
    [string]$JavaHome = $env:JAVA_HOME,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$GradleArgs
)

$ErrorActionPreference = "Stop"

$repoRoot   = Split-Path -Parent $PSScriptRoot
$androidDir = Join-Path $repoRoot "android"

function Get-JavaMajor([string]$javaHomePath) {
    if ([string]::IsNullOrWhiteSpace($javaHomePath)) { return 0 }
    $exe = Join-Path $javaHomePath "bin\java.exe"
    if (-not (Test-Path $exe)) { return 0 }
    $line = (& $exe -version 2>&1 | Select-Object -First 1)
    if ($line -match '"(\d+)') { return [int]$Matches[1] }
    return 0
}

$major = Get-JavaMajor $JavaHome
if ($major -lt 17) {
    $hint = @"
Gradle 9 需要 JDK 17+，但检测到的 JAVA_HOME 不满足要求。
  当前 JAVA_HOME : '$JavaHome'   (检测到主版本: $major)

任选其一：
  1) 传入参数：pwsh tools\gradlew.ps1 -JavaHome 'C:\Users\<你>\scoop\apps\corretto17-jdk\current' assembleDebug
  2) 设置用户环境变量： [Environment]::SetEnvironmentVariable('JAVA_HOME','<JDK17 路径>','User')
  3) 把 JDK 17 放到 PATH 最前面
"@
    throw $hint
}

$env:JAVA_HOME = $JavaHome
$env:ANDROID_HOME = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA "Android\Sdk" }
$env:ANDROID_SDK_ROOT = $env:ANDROID_HOME

Write-Host "JAVA_HOME    = $JavaHome (JDK $major)" -ForegroundColor DarkGray
Write-Host "ANDROID_HOME = $env:ANDROID_HOME" -ForegroundColor DarkGray

if (-not $GradleArgs -or $GradleArgs.Count -eq 0) { $GradleArgs = @("tasks") }

Push-Location $androidDir
try {
    & .\gradlew.bat @GradleArgs
    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
