#!/usr/bin/env pwsh
# M5：桌面安装包构建（带更新签名；没有私钥时**降级并说清楚**）。
#
# 为什么单独成脚本：在 GitHub Actions 的 `run: |` 里做「条件 + 覆盖配置」要过
# PowerShell → pnpm → tauri 三层传参，内联 JSON 的引号会被吞掉
# （2026-09-16 CI 实测：`key must be a string at line 1 column 2`）。逻辑放进脚本里只有一层，
# 而且本地能跑、能验证。
[CmdletBinding()]
param(
    # 打包目标（tauri 的 --bundles）。
    [string]$Bundles = "nsis"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$desktop = Join-Path $root "desktop"

$signed = -not [string]::IsNullOrEmpty($env:TAURI_SIGNING_PRIVATE_KEY)
Push-Location $desktop
try {
    if ($signed) {
        Write-Host "tauri-build: 带更新签名（TAURI_SIGNING_PRIVATE_KEY 已设置）"
        & pnpm tauri build --bundles $Bundles
    }
    else {
        Write-Host "::warning::未配置 TAURI_SIGNING_PRIVATE_KEY：本次产物不带更新签名（不能用于自动更新）"
        $override = Join-Path ([IO.Path]::GetTempPath()) "audiolink-no-updater.json"
        '{"bundle":{"createUpdaterArtifacts":false}}' | Set-Content -Path $override -Encoding utf8NoBOM
        & pnpm tauri build --bundles $Bundles --config $override
    }
    if ($LASTEXITCODE -ne 0) { throw "tauri build 失败（exit $LASTEXITCODE）" }
}
finally {
    Pop-Location
}

$dir = Join-Path $root "target/x86_64-pc-windows-msvc/release/bundle/$Bundles"
if (Test-Path $dir) {
    Write-Host "tauri-build: 产物目录 $dir"
    Get-ChildItem $dir | ForEach-Object { Write-Host ("  {0}  {1:N1} KB" -f $_.Name, ($_.Length / 1KB)) }
}
