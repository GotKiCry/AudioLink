#!/usr/bin/env pwsh
# M5：从签名产物生成 Tauri 更新清单 latest.json。
#
# 为什么需要它：tauri build 只产出「安装包 + .sig」，而 updater 要的是一个 JSON 清单
# （版本 / 平台 / 下载 URL / 签名）；缺这一步，自动更新永远是「配置在、功能不可用」。
#
# 与 tauri.conf.json 的 updater.endpoints 对应：清单最终要发布到
# https://github.com/<repo>/releases/latest/download/latest.json
[CmdletBinding()]
param(
    # 打包产物目录（tauri build --bundles nsis 的输出）。
    [string]$BundleDir = "target/x86_64-pc-windows-msvc/release/bundle/nsis",
    # 仓库（用于拼下载 URL）。
    [string]$Repo = "GotKiCry/AudioLink",
    # 清单落盘位置。
    [string]$Out = "target/evidence/release/latest.json",
    # 发布说明（写进清单；空字符串也合法）。
    [string]$Notes = ""
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

# 版本从 tauri.conf.json 取 —— 单一来源，不在这里另写一份（tools/check-version.ps1 已保证三处一致）。
$confPath = Join-Path $root "desktop/src-tauri/tauri.conf.json"
$version = (Get-Content $confPath -Raw | ConvertFrom-Json).version

$dir = Join-Path $root $BundleDir
if (-not (Test-Path $dir)) { throw "产物目录不存在：$BundleDir（先跑 pnpm tauri:build）" }

$installer = Get-ChildItem -Path $dir -File | Where-Object { $_.Name -like "*-setup.exe" } | Select-Object -First 1
if (-not $installer) { throw "没找到安装包（*-setup.exe）：$dir" }
$sigPath = "$($installer.FullName).sig"
if (-not (Test-Path $sigPath)) {
    throw "没找到签名文件 $($installer.Name).sig —— 打包时是不是漏了 TAURI_SIGNING_PRIVATE_KEY？没有签名的更新包会被客户端拒绝。"
}

$signature = (Get-Content $sigPath -Raw).Trim()
$platform = "windows-x86_64"
$url = "https://github.com/$Repo/releases/latest/download/$($installer.Name)"

$manifest = [ordered]@{
    version   = $version
    notes     = $Notes
    pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    platforms = [ordered]@{
        $platform = [ordered]@{
            signature = $signature
            url       = $url
        }
    }
}

$outPath = Join-Path $root $Out
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $outPath) | Out-Null
($manifest | ConvertTo-Json -Depth 5) | Set-Content -Path $outPath -Encoding utf8

Write-Host "latest.json: $Out"
Write-Host "  version   = $version"
Write-Host "  installer = $($installer.Name)（$([math]::Round($installer.Length/1KB,1)) KB）"
Write-Host "  signature = $($signature.Substring(0, [Math]::Min(40, $signature.Length)))…"
Write-Host "  url       = $url"
