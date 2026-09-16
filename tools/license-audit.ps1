#!/usr/bin/env pwsh
# M5 合规：依赖许可审计（Rust + 桌面前端）。
#
# 为什么分成两半：「采集」要会调 cargo 与 pnpm（环境知识，放本脚本）；
# 「判定与渲染」要有单测（代码，放 audiolink-tools::license）。
#
# 退出码：0 = 没有禁止许可；1 = 有（发布前必须处理）；其它 = 用法/解析错误。
[CmdletBinding()]
param(
    # 前端依赖没装（冷 checkout / 只想看 Rust 侧）时跳过 pnpm。
    [switch]$SkipNpm,
    # 报告落盘位置（相对仓库根）。
    [string]$Report = "docs/compliance/license-report.md",
    # 同时生成第三方组件声明（清单 + 去重后的许可全文）。发布前才需要，平时不生成。
    [switch]$Notices,
    # 声明落盘位置（相对仓库根）。
    [string]$NoticesPath = "docs/compliance/THIRD-PARTY-NOTICES.md"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$tmp = Join-Path $root "target/license-audit"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

$cargoJson = Join-Path $tmp "cargo-metadata.json"
Write-Host "license-audit: 采集 cargo metadata ..."
& cargo metadata --format-version 1 --all-features 2>$null | Set-Content -Path $cargoJson -Encoding utf8
if ($LASTEXITCODE -ne 0) { throw "cargo metadata 失败（exit $LASTEXITCODE）" }

$npmJson = $null
if (-not $SkipNpm) {
    $desktop = Join-Path $root "desktop"
    if (Test-Path (Join-Path $desktop "node_modules")) {
        Write-Host "license-audit: 采集 pnpm licenses ..."
        $npmJson = Join-Path $tmp "pnpm-licenses.json"
        Push-Location $desktop
        & pnpm licenses list --json 2>$null | Set-Content -Path $npmJson -Encoding utf8
        $npmCode = $LASTEXITCODE
        Pop-Location
        if ($npmCode -ne 0) {
            Write-Host "license-audit: pnpm licenses 不可用（exit $npmCode），前端侧跳过"
            $npmJson = $null
        }
    }
    else {
        Write-Host "license-audit: desktop/node_modules 不存在，前端侧跳过（先 pnpm install）"
    }
}

$cargoArgs = @(
    "run", "-q", "-p", "audiolink-tools", "--bin", "license-audit", "--",
    "--cargo", $cargoJson,
    "--write", (Join-Path $root $Report)
)
if ($npmJson) { $cargoArgs += @("--npm", $npmJson) }
if ($Notices) { $cargoArgs += @("--notices", (Join-Path $root $NoticesPath)) }

Push-Location $root
& cargo @cargoArgs
$auditCode = $LASTEXITCODE
Pop-Location

if ($auditCode -eq 1) {
    Write-Host "license-audit: 发现禁止许可 —— 发布前必须处理（报告：$Report）"
    exit 1
}
if ($auditCode -ne 0) { throw "license-audit 失败（exit $auditCode）" }
Write-Host "license-audit: OK（报告：$Report）"
