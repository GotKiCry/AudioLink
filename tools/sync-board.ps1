<#
.SYNOPSIS
    把 AudioLink 的里程碑 / 任务同步到 GitHub Projects 看板（幂等：重复运行只补齐差异）。

.DESCRIPTION
    内容来源：`docs/05-roadmap.md`（里程碑与退出条件）、`docs/10-handoff.md`（当前进度）。
    **文档仍是唯一事实来源**，本脚本里的任务表只是它的机械映射 —— 改任务请先改文档，再同步这里。

    前置条件（一次性，交互式）：
        gh auth refresh -s project
    token 缺 `project` scope 时脚本会直接给出这条提示并退出。

.EXAMPLE
    pwsh tools/sync-board.ps1 -DryRun      # 只打印将要发生的变更
    pwsh tools/sync-board.ps1              # 真正同步（创建看板 / 字段 / 条目，并回填状态）
#>
[CmdletBinding()]
param(
    [string]$Owner = 'GotKiCry',
    [string]$Repo = 'GotKiCry/AudioLink',
    [string]$ProjectTitle = 'AudioLink 任务看板',
    [switch]$DryRun,
    [switch]$NoLink
)

$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# 任务表（docs/05-roadmap.md 的机械映射：M=里程碑，P=优先级，S=状态）
# 优先级含义：P0 = 当前关键路径，P1 = 本里程碑内，P2 = 近期技术债/护栏，P3 = 有余力再做
# ---------------------------------------------------------------------------
$Tasks = @(
    # ---- M0 地基（全部已完成）----
    @{ T = '[M0] CI 能构建内核（core / android / desktop / version-consistency）'; M = 'M0'; P = 'P1'; S = 'Done'; B = '.github/workflows/ci.yml：四 job 并行，单轮约 3–4 分钟' }
    @{ T = '[M0] audiolink-types：协议常量、枚举、错误码、遥测结构'; M = 'M0'; P = 'P1'; S = 'Done'; B = 'docs/03-protocol.md §3/§4.1/§10/§11；serde 为可选 feature（默认零依赖）' }
    @{ T = '[M0] audiolink-proto：ALP/2 编解码 + §12 三组 golden vectors'; M = 'M0'; P = 'P1'; S = 'Done'; B = '数据报 / 控制帧 / 发现报文；L1 严格解码层，非法帧一律 1008 且绝不 panic' }
    @{ T = '[M0] 规格修订：锁定 L1/L2 分层、载荷表、MTU 实测与拒绝矩阵'; M = 'M0'; P = 'P1'; S = 'Done'; B = '§1.1 分层、§3 定长载荷 + QUIC 1162 B 实测、§4 长度约束、§9.2 字段格式、§12 拒绝矩阵' }
    @{ T = '[M0] tools/alp2-dump：协议解码（hex → 人类可读）'; M = 'M0'; P = 'P1'; S = 'Done'; B = '失败时打印 L1 拒绝上下文；结构预筛避免噪声' }
    @{ T = '[M0] tools/latency-probe：QUIC 数据报 RTT / 时钟偏移测量'; M = 'M0'; P = 'P1'; S = 'Done'; B = 'listen/probe；输出 RTT 分位、抖动代理、§6 best8 偏移与极差、§6.5 质量分级' }

    # ---- M1 单链路（关键路径）----
    @{ T = '[M1] WASAPI loopback 事件驱动采集（48 kHz / f32 / 2ch，10–20 ms 缓冲）'; M = 'M1'; P = 'P0'; S = 'Todo'; B = '最大关键路径风险：先把采集缓冲压到 10–20 ms 再谈其余' }
    @{ T = '[M1] Opus 编码（opus-rs，20 ms / 160 kbps / VBR / 48 kHz 锁定）'; M = 'M1'; P = 'P0'; S = 'Todo'; B = 'ADR-003：纯 Rust，不装 CMake；in-band FEC 默认关闭' }
    @{ T = '[M1] 桌面自环链路 + 四段延迟分解（采集/编码/解码/播放）'; M = 'M1'; P = 'P0'; S = 'Todo'; B = '不碰网络与 Android，先把两个未知量钉死（roadmap M1 第一周要求）' }
    @{ T = '[M1] QUIC 通道（quinn：控制流 #0 + 音频数据报）'; M = 'M1'; P = 'P1'; S = 'Todo'; B = '发送侧必须 min(1200, max_datagram_size())，默认初始 MTU 下实测 1162 B' }
    @{ T = '[M1] Android 播放（AudioTrack 低延迟模式 + JNI 环缓冲 + 欠载检测）'; M = 'M1'; P = 'P1'; S = 'Todo'; B = 'NFR-14 硬指标：PERFORMANCE_MODE_LOW_LATENCY + WRITE_NON_BLOCKING，须用 getPerformanceMode() 断言' }
    @{ T = '[M1] 最小 UI（桌面手工 IP 连接；Android 服务启停）'; M = 'M1'; P = 'P1'; S = 'Todo'; B = 'FR-29 / FR-35 的最小形态' }
    @{ T = '[M1] PIN 配对最小可用（白名单落盘）'; M = 'M1'; P = 'P1'; S = 'Todo'; B = 'FR-17；白名单可先为明文 JSON' }
    @{ T = '[M1] 验收：P50 ≤ 110 ms / P95 ≤ 150 ms、零重采样、低延迟模式生效'; M = 'M1'; P = 'P1'; S = 'Todo'; B = '两项硬指标不达标不得进入 M2（roadmap M1 备注）' }

    # ---- M2–M5 ----
    @{ T = '[M2] 稳定性与质量：抖动缓冲 + 双发/PLC/NACK + 自适应码率 + 遥测面板'; M = 'M2'; P = 'P2'; S = 'Todo'; B = 'docs/05-roadmap.md M2；验收含 8 h soak 与弱网（5 Mbps / 2% 丢包 / 30 ms 抖动）' }
    @{ T = '[M3] 多设备与同步：时钟同步全流程 + 预约播放 + 同步组 + sync-measure'; M = 'M3'; P = 'P2'; S = 'Todo'; B = '组内 ±10 ms（P95）；§6 的 200 样本窗口 + 回归漂移估计在此落地' }
    @{ T = '[M4] 网状与混音：Android 内录/麦克风 + PC 播放 + 混音器 + 能力协商'; M = 'M4'; P = 'P3'; S = 'Todo'; B = 'FR-06/07/08/12；内录受 allowAudioPlaybackCapture 限制需 UI 说明' }
    @{ T = '[M5] 产品化：托盘/自启/自动更新/双语/双 ABI 发布/合规清单'; M = 'M5'; P = 'P3'; S = 'Todo'; B = 'docs/05-roadmap.md M5' }

    # ---- 技术债与护栏 ----
    @{ T = '[护栏] 「定长载荷自洽」测试：LEN == 字段宽度之和'; M = 'M0'; P = 'P2'; S = 'Todo'; B = '针对 CLOCK_REPLY 那类「文档/常量算错」的机械护栏' }
    @{ T = '[文档] docs/10-handoff.md 状态更新（M0 已完成）'; M = 'M0'; P = 'P2'; S = 'Todo'; B = '交接文档当前仍写「M0-02 待做」，会误导下一个会话' }
    @{ T = '[安全] docs/06-dev-environment.md 本机路径脱敏'; M = 'M0'; P = 'P2'; S = 'Todo'; B = '仓库已 PUBLIC，文档里仍有 C:\Users\liuzh\... 路径' }
    @{ T = '[护栏] 跨端一致性夹具：同一组 golden vectors 在 Rust 与 FFI 双跑'; M = 'M1'; P = 'P3'; S = 'Todo'; B = '§12 唯一尚未验证的一条（需 audiolink-ffi 有导出路径后落地）' }
    @{ T = '[工具] alp2-dump 支持 pcap/pcapng 输入'; M = 'M1'; P = 'P3'; S = 'Todo'; B = '当前只吃 hex 文本；等真有抓包需求再加（需夹具验证）' }
    @{ T = '[CI] QUIC 依赖 feature 门控（回收 core job 的 ~2.5 min）'; M = 'M0'; P = 'P3'; S = 'Todo'; B = 'tools 引入 quinn/rustls/tokio 后 core job 由 1m36s 涨到 4m10s' }
    @{ T = '[工具] soak-runner（8 h 回环 + 指标采集 + 异常快照）'; M = 'M2'; P = 'P3'; S = 'Todo'; B = 'M2 验收依赖，建议 M1 一结束就有最简版' }
    @{ T = '[CI] Android job：预编译 cargo-ndk（省 ~2 min）'; M = 'M0'; P = 'P3'; S = 'Todo'; B = 'docs/10-handoff.md §7 待办' }
)

$STATUS_OPTIONS = @('Todo', 'In Progress', 'Done')
$MILESTONE_OPTIONS = @('M0', 'M1', 'M2', 'M3', 'M4', 'M5')
$PRIORITY_OPTIONS = @('P0', 'P1', 'P2', 'P3')

# ---------------------------------------------------------------------------
# 辅助
# ---------------------------------------------------------------------------

function Invoke-Gh {
    param([Parameter(Mandatory)][string[]]$Arguments, [switch]$AsJson)
    $output = & gh @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "gh $($Arguments -join ' ') 失败：`n$($output | Out-String)"
    }
    if ($AsJson) {
        return (($output | Out-String).Trim() | ConvertFrom-Json)
    }
    return $output
}

function Get-ProjectFields {
    param([Parameter(Mandatory)][int]$Number)
    return (Invoke-Gh @('project', 'field-list', "$Number", '--owner', $Owner, '--format', 'json') -AsJson).fields
}

function Ensure-SingleSelectField {
    param([Parameter(Mandatory)][int]$Number, [Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][string[]]$Options)
    $existing = Get-ProjectFields $Number | Where-Object { $_.name -eq $Name } | Select-Object -First 1
    if ($existing) {
        return $existing
    }
    if ($DryRun) {
        Write-Host "  [dry-run] 创建单选字段 '$Name'（选项：$($Options -join ', ')）"
        return $null
    }
    Write-Host "  + 创建单选字段 '$Name'（选项：$($Options -join ', ')）"
    Invoke-Gh @('project', 'field-create', "$Number", '--owner', $Owner, '--name', $Name, '--data-type', 'SINGLE_SELECT', '--single-select-options', ($Options -join ',')) | Out-Null
    return Get-ProjectFields $Number | Where-Object { $_.name -eq $Name } | Select-Object -First 1
}

function Item-Title {
    param($Item)
    if ($Item.title) { return "$($Item.title)" }
    if ($Item.content -and $Item.content.title) { return "$($Item.content.title)" }
    return ''
}

function Set-SingleSelect {
    param(
        [Parameter(Mandatory)][string]$ItemId,
        [Parameter(Mandatory)][string]$ProjectId,
        [Parameter(Mandatory)]$Field,
        [Parameter(Mandatory)][string]$OptionName,
        [Parameter(Mandatory)][int]$Number
    )
    if (-not $Field) { return }
    $option = $Field.options | Where-Object { $_.name -eq $OptionName } | Select-Object -First 1
    if (-not $option) {
        Write-Warning "字段 '$($Field.name)' 没有选项 '$OptionName'，跳过"
        return
    }
    Invoke-Gh @('project', 'item-edit', '--id', $ItemId, '--project-id', $ProjectId, '--field-id', "$($Field.id)", '--single-select-option-id', "$($option.id)") | Out-Null
}

# ---------------------------------------------------------------------------
# 主流程
# ---------------------------------------------------------------------------

Write-Host "AudioLink 任务看板同步（owner=$Owner, project='$ProjectTitle'$(if ($DryRun) { ', DRY-RUN' }))"

try {
    $projectList = Invoke-Gh @('project', 'list', '--owner', $Owner, '--limit', '100', '--format', 'json') -AsJson
}
catch {
    Write-Error "无法访问 GitHub Projects —— token 缺 scope。请先运行：`n    gh auth refresh -s project`n（交互式，需在浏览器完成授权）`n原始错误：$($_.Exception.Message)"
    exit 1
}

$project = $projectList.projects | Where-Object { $_.title -eq $ProjectTitle } | Select-Object -First 1
if (-not $project) {
    if ($DryRun) {
        Write-Host "  [dry-run] 创建看板 '$ProjectTitle'（并关联仓库 $Repo）"
        Write-Host "  [dry-run] 创建单选项字段：Status($($STATUS_OPTIONS -join '/'))、里程碑($($MILESTONE_OPTIONS -join '/'))、优先级($($PRIORITY_OPTIONS -join '/'))"
        Write-Host "  [dry-run] 将写入 $($Tasks.Count) 个条目："
        foreach ($task in $Tasks) {
            Write-Host ("      - [{0}/{1}/{2}] {3}" -f $task.M, $task.P, $task.S, $task.T)
        }
        exit 0
    }
    Write-Host "  + 创建看板 '$ProjectTitle'"
    Invoke-Gh @('project', 'create', '--owner', $Owner, '--title', $ProjectTitle) | Out-Null
    $projectList = Invoke-Gh @('project', 'list', '--owner', $Owner, '--limit', '100', '--format', 'json') -AsJson
    $project = $projectList.projects | Where-Object { $_.title -eq $ProjectTitle } | Select-Object -First 1
}
if (-not $project) {
    throw "看板创建后仍未找到（owner=$Owner, title=$ProjectTitle）"
}

$number = [int]$project.number
$projectId = "$($project.id)"
Write-Host "  看板：$($project.url)（number=$number）"

if (-not $NoLink -and -not $DryRun) {
    Invoke-Gh @('project', 'link', "$number", '--owner', $Owner, '--repo', $Repo) | Out-Null
    Write-Host "  已关联仓库 $Repo"
}

$statusField = Ensure-SingleSelectField -Number $number -Name 'Status' -Options $STATUS_OPTIONS
$milestoneField = Ensure-SingleSelectField -Number $number -Name '里程碑' -Options $MILESTONE_OPTIONS
$priorityField = Ensure-SingleSelectField -Number $number -Name '优先级' -Options $PRIORITY_OPTIONS

$items = @()
if (-not $DryRun) {
    $items = (Invoke-Gh @('project', 'item-list', "$number", '--owner', $Owner, '--limit', '200', '--format', 'json') -AsJson).items
}
$existing = @{}
foreach ($item in $items) {
    $title = Item-Title $item
    if ($title) { $existing[$title] = $item }
}
Write-Host "  现有条目 $($existing.Count) 个，任务表 $($Tasks.Count) 个"

$created = 0
$updated = 0
$unchanged = 0

foreach ($task in $Tasks) {
    if ($DryRun) {
        $mark = if ($existing.ContainsKey($task.T)) { '已存在' } else { '新建' }
        Write-Host ("  [dry-run] {0,-4} [{1}/{2}/{3}] {4}" -f $mark, $task.M, $task.P, $task.S, $task.T)
        continue
    }

    $item = $existing[$task.T]
    if (-not $item) {
        $createdItem = Invoke-Gh @('project', 'item-create', "$number", '--owner', $Owner, '--title', $task.T, '--body', $task.B, '--format', 'json') -AsJson
        $itemId = "$($createdItem.id)"
        Set-SingleSelect -ItemId $itemId -ProjectId $projectId -Field $statusField -OptionName $task.S -Number $number
        Set-SingleSelect -ItemId $itemId -ProjectId $projectId -Field $milestoneField -OptionName $task.M -Number $number
        Set-SingleSelect -ItemId $itemId -ProjectId $projectId -Field $priorityField -OptionName $task.P -Number $number
        $created++
        Write-Host ("  + [{0}/{1}/{2}] {3}" -f $task.M, $task.P, $task.S, $task.T)
        continue
    }

    $itemId = "$($item.id)"
    if ("$($item.status)" -ne $task.S) {
        Set-SingleSelect -ItemId $itemId -ProjectId $projectId -Field $statusField -OptionName $task.S -Number $number
        $updated++
        Write-Host ("  ~ 状态 {0} → {1}：{2}" -f $item.status, $task.S, $task.T)
    }
    else {
        $unchanged++
    }
}

Write-Host ""
if ($DryRun) {
    Write-Host "DRY-RUN 结束（没有做任何变更）。去掉 -DryRun 即真正同步。"
}
else {
    Write-Host "同步完成：新建 $created，状态更新 $updated，已是最新 $unchanged。"
    Write-Host "看板：$($project.url)"
}
