# AudioLink —— 版本一致性校验（单一来源：根 Cargo.toml 的 workspace.package.version）
#
# 旧版教训：csproj / build.gradle / git tag 三处人工同步，必然漂移。
# 本脚本在 CI 与本地发布前运行，任一不一致即失败。

param(
    [switch]$Quiet
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

function Get-VersionFromCargo {
    $cargo = Get-Content (Join-Path $root "Cargo.toml") -Raw
    if ($cargo -match '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') { return $Matches[1] }
    throw "无法从根 Cargo.toml 读取 workspace.package.version"
}

function Get-VersionFromTauri {
    $conf = Get-Content (Join-Path $root "desktop/src-tauri/tauri.conf.json") -Raw | ConvertFrom-Json
    return $conf.version
}

function Get-VersionFromGradle {
    $gradle = Get-Content (Join-Path $root "android/app/build.gradle.kts") -Raw
    if ($gradle -match 'versionName\s*=\s*"([^"]+)"') { return $Matches[1] }
    throw "无法从 app/build.gradle.kts 读取 versionName"
}

$cargo   = Get-VersionFromCargo
$tauri   = Get-VersionFromTauri
$gradle  = Get-VersionFromGradle

$rows = @(
    [pscustomobject]@{ Source = "Cargo.toml (workspace)"; Version = $cargo;  Ok = $true }
    [pscustomobject]@{ Source = "tauri.conf.json";         Version = $tauri;  Ok = ($tauri  -eq $cargo) }
    [pscustomobject]@{ Source = "build.gradle.kts";        Version = $gradle; Ok = ($gradle -eq $cargo) }
)

if (-not $Quiet) { $rows | Format-Table -AutoSize }

if ($rows.Ok -contains $false) {
    Write-Error "版本不一致：以 $cargo 为单一来源，请修正上表标红的文件。"
    exit 1
}

Write-Host "版本一致：$cargo" -ForegroundColor Green
