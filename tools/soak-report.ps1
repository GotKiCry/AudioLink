#Requires -Version 7.0
<#
.SYNOPSIS
  汇总 soak-runner 的长跑报告，输出判定摘要。
.DESCRIPTION
  读取 soak-runner 写出的 JSON 报告（planned_seconds / summary / violations / coarse），输出：
  判决、粗采样次数、违规种类聚合（次数、首次与末次秒数、样例说明）、终值关键指标。
  退出码：0 = ok，1 = failed 或存在违规，2 = 报告缺失或不可解析。
.PARAMETER Path
  报告路径（相对仓库根或绝对），默认 target/evidence/soak/soak-8h-netem.json。
.PARAMETER Json
  输出机器可读 JSON 摘要，便于 CI 或看板回填。
#>
[CmdletBinding()]
param(
  [string]$Path = 'target/evidence/soak/soak-8h-netem.json',
  [switch]$Json
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot

if (-not [System.IO.Path]::IsPathRooted($Path)) {
  $Path = Join-Path $repoRoot $Path
}

if (-not (Test-Path -LiteralPath $Path)) {
  [Console]::Error.WriteLine("找不到报告：$Path")
  exit 2
}

try {
  $report = Get-Content -Raw -LiteralPath $Path | ConvertFrom-Json
}
catch {
  [Console]::Error.WriteLine("报告不是合法 JSON：" + $_.Exception.Message)
  exit 2
}

$summary = $report.summary
if ($null -eq $summary) {
  [Console]::Error.WriteLine("报告缺少 summary 字段：$Path")
  exit 2
}

$violations = @($report.violations)
$coarse = @($report.coarse)
$final = $summary.final_stats

$groups = @(
  $violations |
    Group-Object -Property kind |
    Sort-Object -Property Count -Descending |
    ForEach-Object {
      $secs = @($_.Group | ForEach-Object { [int]$_.at_secs })
      [pscustomobject]@{
        kind       = $_.Name
        count      = $_.Count
        first_secs = ($secs | Measure-Object -Minimum).Minimum
        last_secs  = ($secs | Measure-Object -Maximum).Maximum
        sample     = [string]($_.Group | Select-Object -First 1).detail
      }
    }
)

$span = 'none'
if ($violations.Count -gt 0) {
  $all = @($violations | ForEach-Object { [int]$_.at_secs })
  $span = "$(($all | Measure-Object -Minimum).Minimum)s..$(($all | Measure-Object -Maximum).Maximum)s"
}

$verdict = [string]$summary.verdict
$result = [pscustomobject]@{
  path                 = $Path
  tool                 = [string]$report.tool
  verdict              = $verdict
  planned_seconds      = [int]$report.planned_seconds
  frame_ms             = [int]$report.frame_ms
  expected_bitrate_bps = [int]$report.expected_bitrate_bps
  coarse_buckets       = $coarse.Count
  coarse_samples       = [int]$summary.samples
  violation_count      = [int]$summary.violations
  violations_total     = if ($null -ne $summary.violations_total) { [int]$summary.violations_total } else { [int]$summary.violations + [int]$summary.dropped_violations }
  dropped_violations   = [int]$summary.dropped_violations
  by_kind              = $summary.violations_by_kind
  last_violation_at_secs = $summary.last_violation_at_secs
  last_violation_kind  = $summary.last_violation_kind
  violation_span       = $span
  kinds                = $groups
  final                = $final
}

if ($Json) {
  $result | ConvertTo-Json -Depth 6
}
else {
  Write-Host "报告：$Path"
  Write-Host ("工具 {0} / 计划 {1}s / 帧长 {2}ms / 目标码率 {3} bps" -f $result.tool, $result.planned_seconds, $result.frame_ms, $result.expected_bitrate_bps)
  Write-Host ("判决 {0} / 粗采样桶 {1} 个（{2} 次采样）/ 违规 {3} 次 / 丢弃溢出 {4} 次" -f $verdict, $result.coarse_buckets, $result.coarse_samples, $result.violation_count, $result.dropped_violations)

  if ($groups.Count -eq 0) {
    Write-Host '违规种类：无'
  }
  else {
    Write-Host ("违规种类（共 {0} 类，发生区间 {1}）：" -f $groups.Count, $span)
    foreach ($g in $groups) {
      Write-Host ("  {0} x{1}  {2}s..{3}s" -f $g.kind, $g.count, $g.first_secs, $g.last_secs)
      Write-Host ("    样例：{0}" -f $g.sample)
    }
  }

  if ($null -ne $summary.violations_by_kind) {
    $rows = @($summary.violations_by_kind.PSObject.Properties | Where-Object { $_.Value -gt 0 } | ForEach-Object { $_.Name + "x" + $_.Value })
    Write-Host ("异常总数 {0} 条（留存 {1}，未留存 {2}；每类留存上限 {3}）" -f $result.violations_total, $result.violation_count, $result.dropped_violations, $summary.violation_limit_per_kind)
    if ($rows.Count -gt 0) { Write-Host ("  按类：" + ($rows -join ", ")) }
    if ($null -ne $summary.last_violation_at_secs) {
      Write-Host ("  最后一次异常：t={0}s [{1}]" -f $summary.last_violation_at_secs, $summary.last_violation_kind)
    }
  }

  Write-Host ''
  Write-Host '终值指标：'
  Write-Host ("  码率 {0} bps / 缓冲 {1} us / 时钟偏差 {2} us / 漂移 {3} ppm" -f $final.bitrate_bps, $final.buffer_level_us, $final.clock_offset_us, $final.drift_ppm)
  Write-Host ("  抖动 {0} us (p95 {1} us) / RTT {2} us / 端到端 {3} us" -f $final.jitter_us, $final.jitter_p95_us, $final.rtt_us, $final.e2e_latency_us)
  Write-Host ("  丢包率 x100 {0} / 迟到丢弃 {1} / 欠载 {2} / NACK {3} / PLC {4} / 流 {5}" -f $final.loss_pct_x100, $final.late_drops, $final.underruns, $final.nack_count, $final.plc_count, $final.stream_id)

  Write-Host ''
  if ($verdict -eq 'ok' -and $result.violation_count -eq 0) {
    Write-Host ("判定：通过（计划时长 {0}s 内无违规采样）" -f $result.planned_seconds)
  }
  else {
    Write-Host ("判定：不通过（{0} 次违规，见上表）" -f $result.violation_count)
  }
}

if ($verdict -eq 'ok' -and $result.violation_count -eq 0) { exit 0 }
exit 1
