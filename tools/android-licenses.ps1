#!/usr/bin/env pwsh
# M5 合规：采集 Android（Gradle/Maven）依赖的许可，产出供 license-audit 判定的 JSON。
#
# 为什么不用第三方 Gradle 许可插件：那要往构建里再塞一个供应链依赖（还要联网）。
# 这里只用两样**本地已有**的东西：`gradlew :app:dependencies` 的依赖树 + Gradle 缓存里 POM 的
# `<licenses>` 节点。POM 写的是自然语言许可名（`Apache License, Version 2.0`），
# 规范化成 SPDX 与判定都在 `audiolink-tools::license` 里做（可离线单测）。
[CmdletBinding()]
param(
    # 依赖树报告（先跑：pwsh tools/gradlew.ps1 -JavaHome <JDK17> :app:dependencies --configuration releaseRuntimeClasspath）
    [string]$Report = "target/evidence/compliance/android-dependencies.txt",
    # Gradle 缓存（modules-2）。
    [string]$Cache = ".gradle-home/caches/modules-2/files-2.1",
    # 输出 JSON。
    [string]$Out = "target/evidence/compliance/android-licenses.json"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

# 相对路径按仓库根解析；**绝对路径原样使用**。
# 为什么需要它：PowerShell 的 Join-Path 在第二段是绝对路径时**不会**丢弃第一段，
# 而是直接拼接（与 .NET 的 Path.Combine 行为不同）—— 于是 CI 里传绝对路径会被
# 拼成 `/repo//home/runner/.gradle/...` 这种不存在的路径，再误报成「缓存不存在」。
function Resolve-RepoPath {
    param([Parameter(Mandatory)][string]$Path)
    if ([System.IO.Path]::IsPathRooted($Path)) { return $Path }
    return (Join-Path $root $Path)
}

$reportPath = Resolve-RepoPath $Report
$cachePath = Resolve-RepoPath $Cache
$outPath = Resolve-RepoPath $Out

# 报错一律打印**实际检查的路径**（而不是原始参数）：上一版的报错信息打印 $Cache，
# 结果把「拼接出错」伪装成「缓存不存在」，在 CI 上白烧了一轮。
if (-not (Test-Path $reportPath)) { throw "依赖树报告不存在：$reportPath（先用 gradlew 生成）" }
if (-not (Test-Path $cachePath)) { throw "Gradle 缓存不存在：$cachePath" }

# ---------- 1. 从依赖树抽出坐标 ----------
$coords = [ordered]@{}
foreach ($line in Get-Content $reportPath) {
    if ($line -notmatch "---\s") { continue }
    $rest = ($line -split "---\s+", 2)[1]
    if (-not $rest) { continue }
    # `(c)` = 版本约束（constraint），不引入实际依赖，跳过。
    if ($rest -match "\(c\)\s*$") { continue }
    if ($rest -match "^([\w\.\-]+):([\w\.\-]+):([\w\.\-]+)(?:\s*->\s*([\w\.\-]+))?") {
        $group = $Matches[1]
        $artifact = $Matches[2]
        # `->` 表示版本被提升：取提升后的版本（那才是真正进包的那个）。
        $version = if ($Matches[4]) { $Matches[4] } else { $Matches[3] }
        $coords["$group`:$artifact"] = $version
    }
}
Write-Host "android-licenses: 依赖树里解析出 $($coords.Count) 个坐标"

# ---------- 2. 从缓存里的 POM 读许可 ----------
$rows = @()
$missing = 0
foreach ($key in $coords.Keys) {
    $group, $artifact = $key -split ":", 2
    $version = $coords[$key]
    $dir = Join-Path $cachePath "$group/$artifact/$version"
    $license = $null
    if (Test-Path $dir) {
        $pom = Get-ChildItem $dir -Recurse -Filter "*.pom" -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($pom) {
            try {
                [xml]$xml = Get-Content $pom.FullName -Raw
                $names = @($xml.project.licenses.license | ForEach-Object { $_.name } | Where-Object { $_ })
                if ($names.Count -gt 0) { $license = ($names -join " OR ") }
            }
            catch { $license = $null }
        }
    }
    if (-not $license) {
        $missing += 1
        $license = "(未声明)"
    }
    $rows += [ordered]@{ name = $key; version = $version; license = $license }
}

New-Item -ItemType Directory -Force -Path (Split-Path -Parent $outPath) | Out-Null
($rows | ConvertTo-Json -Depth 3) | Set-Content -Path $outPath -Encoding utf8NoBOM
Write-Host "android-licenses: 写入 $Out（$($rows.Count) 条，其中 $missing 条没有读到许可）"
