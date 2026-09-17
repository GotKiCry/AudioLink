#!/usr/bin/env pwsh
# M5 合规：依赖许可审计（Rust + 桌面前端 + Android）。
#
# 为什么分成两半：「采集」要会调 cargo 与 pnpm（环境知识，放本脚本）；
# 「判定与渲染」要有单测（代码，放 audiolink-tools::license）。
#
# 退出码：0 = 没有禁止许可；1 = 有（发布前必须处理）；其它 = 用法/解析错误；
#          生成声明（-Notices）时缺 Android 依赖清单 = 失败（见下）。
#
# ## -SkipAndroid：逃生门，不是消音器
#
# Android（Gradle/Maven）依赖由 tools/android-licenses.ps1 采集（它需要 gradlew 的依赖树报告
# 与 Gradle 缓存）。采集缺席时，本脚本**按产物分档**处理：
#   * 报告（默认，落 docs/compliance/license-report.md）：**宽松** —— 报告里会写明
#     「本次不含 Android 一栏」，退出码仍是 0（CI 里没有 Gradle，不该因此变红）。产物自证。
#   * 声明（-Notices，会随安装包 / APK 发给用户）：**严格** —— 缺清单直接失败，不降级。
#     声明是发出去的合规产物，「看起来完整」比「少一栏」更坏：流水线全绿，用户拿到的是残缺的。
#
# 于是 -SkipAndroid 只该用于两类场景：① 内部 / 临时草稿，且这份声明不会发给用户；
# ② 手里确实没有 Gradle 环境，只要一份不含 Android 的草稿。
# **发布产物不得用它** —— 用它等于主动发一份缺一整栏的声明。
# 与 -Notices 同用时，它还会**跳过**把声明同步进 android assets（那条路径随 APK 分发）。
[CmdletBinding()]
param(
    # 前端依赖没装（冷 checkout / 只想看 Rust 侧）时跳过 pnpm。
    [switch]$SkipNpm,
    # 报告落盘位置（相对仓库根）。
    [string]$Report = "docs/compliance/license-report.md",
    # 同时生成第三方组件声明（清单 + 去重后的许可全文）。发布前才需要，平时不生成。
    [switch]$Notices,
    # 声明落盘位置（相对仓库根）。
    [string]$NoticesPath = "docs/compliance/THIRD-PARTY-NOTICES.md",
    # 明确声明「本次产物不含 Android 一栏」（逃生门；语义与适用场景见文件头）。
    # 不给它时：报告路径宽松（缺清单会在报告里写明），-Notices 路径严格（缺清单即失败）。
    [switch]$SkipAndroid
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
$androidRel = "target/evidence/compliance/android-licenses.json"
$androidJson = Join-Path $root $androidRel
if ($SkipAndroid) {
    Write-Host "license-audit: 按 -SkipAndroid 跳过 Android —— 本次产物不含 Android 一栏（见文件头：这是逃生门，不该用于发布产物）"
}
elseif (Test-Path $androidJson) {
    $cargoArgs += @("--android", $androidJson)
}
elseif ($Notices) {
    # 声明随包发出去：缺一栏必须**当场失败**，而不是降级成一份看起来完整的声明
    # （流水线全绿的静默缺陷 —— 第 110 轮 task-36 的范围外发现）。
    throw "生成第三方声明需要 Android 依赖清单，但 $androidRel 不存在。先跑：pwsh tools/android-licenses.ps1（需要 gradlew 的依赖树报告与 Gradle 缓存）。若确实要出一份不含 Android 的草稿，用 -SkipAndroid 显式跳过（它不会同步到 android assets；语义见文件头）。"
}
else {
    # 报告路径：缺清单不失败，但报告里会写明「本次不含 Android 一栏」（产物自证，见文件头）。
    Write-Host "license-audit: 未找到 Android 依赖清单，本次跳过 Android（先跑 tools/android-licenses.ps1）；报告里会写明「不含 Android 一栏」"
}

if ($npmJson) { $cargoArgs += @("--npm", $npmJson) }
if ($Notices) {
    $noticesFull = Join-Path $root $NoticesPath
    $cargoArgs += @("--notices", $noticesFull)
    # 顺带同步一份到 Android assets：那边的「开源许可」页从 assets 读（随 APK 分发，离线可看）。
    # 放在这一步是为了**只有一个生成源** —— 手工复制迟早会漂移。
    if ($SkipAndroid) {
        # 逃生门必须收口在这里：那份声明不含 Android 一栏，而 assets 会随 APK 发出去 ——
        # 同步它等于把不完整的声明打进包里（正是本脚本要防的事）。
        Write-Warning "license-audit: -SkipAndroid 下**不同步**声明到 android assets（该声明不含 Android 一栏，不能随 APK 分发）"
        Write-Warning "license-audit: 注意 $Report 也会被写成不含 Android 一栏的版本 —— 别把它提交进仓库（正式报告请在清单齐备时重新生成）"
    }
    else {
        $assetsDir = Join-Path $root "android/app/src/main/assets"
        New-Item -ItemType Directory -Force -Path $assetsDir | Out-Null
        Copy-Item -Path $noticesFull -Destination (Join-Path $assetsDir "THIRD-PARTY-NOTICES.md") -Force
        Write-Host "license-audit: 已同步声明到 android/app/src/main/assets/"
    }
}

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
