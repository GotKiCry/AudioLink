#Requires -Version 7.0
<#
.SYNOPSIS
    [M1] 声学端到端延迟量具 —— 激励 / 播放+录音 / 定位 / P50-P95 / 判读 / 落盘，一条命令走完。

.DESCRIPTION
    量法**不是本脚本发明的**：口径照抄 docs/55-device-acceptance-runbook.md §2（已写死，不要改）：
      §2.1 验收线 = e2e P50 <= 110 ms 且 P95 <= 150 ms（docs/05 §M1）
      §2.2 激励信号 = 48 kHz / 16-bit / 单声道 / 6 个宽带噪声突发（20 ms 窗、0.4 ms 指数衰减、
           幅度 0.8）/ 间隔 500 ms / 总长 4000 ms
      §2.3 拓扑 = 同源双路出声 + 单支麦克风：
           PC 播放激励 ──┬── PC 喇叭直接出声（参考峰，声程 P）
                         └── device-link 用 WASAPI loopback 采同一路 → AudioLink → 手机喇叭（被测峰，声程 M）
           两个峰都落进**同一次录音**，因此麦克风延迟、录音启动偏移、PC 播放链全部自相消。
      §2.4 时间轴纪律：必须是同一次录音。本脚本只做单文件分析，不做跨文件相减。
      §2.5 定位「发出/听到」：在一个 500 ms 周期内，时间上先到的是参考峰（PC 喇叭），后到的是被测峰（手机）。
      §2.7 误差来源与量级：见 .NOTES。
      §2.8 判读：P50 <= 110 且 P95 <= 150 → 达标；样本对数 >= 6。

    本脚本量到的是「手机链路相对 PC 直连播放的**净增量**」（口径 A），
    **不是**「PC 采集封口 → 手机出声」的绝对值（口径 B）。两者相差 PC 播放链
    （共享模式缓冲下限 22 ms + DAC ~1 ms，docs/05 §M1 / docs/06 §4.2）。报告里两个数字绝不混读。

    三条纪律（与 sync-measure 同款）：
      1) 检测不到峰 / 峰对数不足 → **明确失败**，绝不输出任何 P50/P95；
      2) 一切数字都从录音里读出来，脚本不估算、不填充；
      3) 实际使用的检测阈值、每个峰的时刻、每个被丢弃的 slot 都写进报告 —— 降级要留痕。

.PARAMETER SelfTest
    自检（不需要麦克风，不碰任何音频设备）：程序自己合成一段带**已知偏移**的双峰录音（含底噪与反射干扰），
    走完整的「写 WAV → 读回 → 包络 → 定位 → 配对 → 分位 → 判读」，断言每个用例的读数与真值之差 <= 1 ms；
    另有一组反例（静音 / 只有参考峰 / 只有被测峰 / 被测峰弱到检不出）必须**失败**。
    自检不过 = 量具不可用，不许拿它的数字去量真机。

.PARAMETER DryRun
    只打印将要执行的步骤、命令、人工动作清单与误差表；会枚举 dshow 输入设备并对选定设备做
    **只读**电平探测（3 s），不播放、不改任何设备设置。

.PARAMETER Run
    正式测量：生成激励 → 校验麦克风 → 打印摆位清单 → device-link 推流 → 播放 + 录音 → 分析 → 落盘。

.PARAMETER AnalyseWav
    离线分析一段已有录音（同一次录音里含两个峰的那种），走与 Run 完全相同的定位/配对/分位/判读逻辑。

.EXAMPLE
    pwsh tools/acoustic-latency.ps1 -SelfTest
    pwsh tools/acoustic-latency.ps1 -DryRun
    pwsh tools/acoustic-latency.ps1 -Run -MicDevice "麦克风 (USB Audio Device)" -Peer 192.168.31.231
    pwsh tools/acoustic-latency.ps1 -AnalyseWav target/evidence/m1-device-p1/20260917/rec.wav

.NOTES
    退出码（与项目其它工具一致）：0 达标 · 1 不达标（测到了但超阈） · 2 用法错误 ·
    3 检测失败（峰数不足 / 录音不可分析 —— **没有**数字） · 4 设备不可用（无麦克风 / 端点静音 / 缺 ffmpeg）。

    误差来源与量级（docs/55 §2.7 原表，报告里也打印）：
      #1 PC 播放链（共享模式缓冲 22 ms + DAC ~1 ms）~23 ms —— 被对消（参考与被测共享），代价是口径偏移
      #2 麦克风采集延迟 / 录音启动偏移（数 ms–数十 ms，未知）—— 被对消（同一次录音、同一设备）
      #3 两个声源到麦克风的声程差：10 cm ≈ 0.29 ms；1 m ≈ 2.9 ms —— 量距离，可用 -ExtraPathCm 修正
      #4 手机输出侧（被测对象，不扣）：AudioTrack 缓冲 3844 帧 = 80.08 ms，flinger track latency 101 ms
      #5 手机待播队列水位（被测对象，测前记录）：实测 40–200 ms，30 min 时 P50 200 ms / 上限 320 ms
      #6 环境混响/反射：首达峰外的反射可能被当成竞争峰 —— 关窗、近场、看报告里的 extra peaks
      #7 网络抖动：跨三层 RTT P50 11.7 / P95 81.1 ms；同子网 P50 6.65 ms —— 记录当轮路径形态
      #8 时钟漂移（两台设备不同源）：±50 ppm 量级，30 min ≈ 90 ms —— 本次用 20–60 s 窗口，影响小
#>
[CmdletBinding(DefaultParameterSetName = 'Usage')]
param(
    [Parameter(ParameterSetName = 'SelfTest', Mandatory = $true)][switch]$SelfTest,
    [Parameter(ParameterSetName = 'DryRun', Mandatory = $true)][switch]$DryRun,
    [Parameter(ParameterSetName = 'Run', Mandatory = $true)][switch]$Run,
    [Parameter(ParameterSetName = 'Analyse', Mandatory = $true)][string]$AnalyseWav,

    [string]$MicDevice,
    [string]$Peer,
    [string]$PcDevice = 'default',
    [ValidateRange(1, 20)][int]$Rounds = 1,
    [ValidateRange(30, 3600)][int]$PeerSeconds = 90,
    [ValidateRange(0, 600)][int]$RecordSeconds = 0,
    [ValidateRange(-120, 0)][double]$MicSilenceDb = -80,
    [ValidateRange(1, 30)][int]$MicProbeSeconds = 3,
    [ValidateRange(1, 100)][int]$MinPairs = 6,
    [double]$ExtraPathCm = 0,
    [ValidateRange(0.05, 0.95)][double]$ThresholdRatio = 0.5,
    [int]$Channel = 0,
    [string]$EvidenceDir = 'target/evidence/m1-device-p1',
    [string]$StimulusWav,
    [string]$Ffmpeg = 'ffmpeg',
    [string]$Ffplay = 'ffplay',
    [string]$DeviceLinkExe,
    [switch]$SkipMicProbe,
    [switch]$NonInteractive,
    [switch]$NoEvidence,
    [string]$SelfTestLog,
    [switch]$Quiet
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3.0

# ============================ 量法常量（写死；出处 docs/55 §2）============================
$SCRIPT:Cfg = @{
    SampleRate    = 48000    # §2.2
    Bits          = 16
    Channels      = 1
    PulseCount    = 6        # §2.2：6 个宽带噪声突发
    PulseMs       = 20.0     # §2.2：20 ms 窗
    DecayMs       = 0.4      # §2.2：0.4 ms 指数衰减
    Amplitude     = 0.8      # §2.2：幅度 0.8
    GapMs         = 500.0    # §2.2：间隔 500 ms
    TotalMs       = 4000.0   # §2.2：6 x 500 + 1000
    ReleaseTauMs  = 4.0      # 与 core/crates/audiolink-tools/src/syncmeasure.rs 的 DetectConfig 同口径
    MinGapMs      = 5.0      # 同上（脉冲内振荡不二次触发）
    P50LimitMs    = 110.0    # §2.8 / docs/05 §M1
    P95LimitMs    = 150.0    # §2.8
    MinDeltaMs    = 10.0     # 一周期内「被测峰必须晚于参考峰」的下限
    MaxDeltaMs    = 375.0    # 0.75 x 500 ms：超出即判「配对不可信」（§2.5 说超过 250 ms 就该把激励间隔加大到 1000 ms）
    MicSilenceDb  = -80.0    # 判据：max_volume <= -80 dB 说明该端点拿不到声学信号（本机两个虚拟端点实测 -91.0 / -90.3 dB）
    RelaxLadder   = @(0.35, 0.25, 0.18, 0.12)  # 默认阈值配不满对时的降级阶梯（每次降级都写进报告）
}

$SCRIPT:Log = [System.Collections.Generic.List[string]]::new()
$SCRIPT:ExitCode = 0
$SCRIPT:QuietMode = [bool]$Quiet
$SCRIPT:EvidenceRoot = $EvidenceDir

# ============================ 输出与日志 ============================
function Emit {
    param([string]$Text = '', [ValidateSet('info', 'step', 'ok', 'warn', 'err', 'head')][string]$Kind = 'info')
    $SCRIPT:Log.Add($Text)
    if (-not $SCRIPT:QuietMode -or $Kind -ne 'info') {
        switch ($Kind) {
            'head' { Write-Host $Text -ForegroundColor Cyan }
            'step' { Write-Host $Text }
            'ok'   { Write-Host $Text -ForegroundColor Green }
            'warn' { Write-Host $Text -ForegroundColor Yellow }
            'err'  { Write-Host $Text -ForegroundColor Red }
            default { Write-Host $Text }
        }
    }
}

function Emit-Banner {
    param([string]$Title)
    Emit ''
    Emit ('=' * 78) 'head'
    Emit "  $Title" 'head'
    Emit ('=' * 78) 'head'
}

function Save-LogFile {
    param([string]$Path)
    if ($Path) {
        $dir = Split-Path -Parent $Path
        if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
        Set-Content -LiteralPath $Path -Value ($SCRIPT:Log -join [Environment]::NewLine) -Encoding utf8
        Write-Host "日志：$Path"
    }
}

function Get-Sha256 {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function New-EvidenceDir {
    param([string]$Stamp)
    $dir = Join-Path $SCRIPT:EvidenceRoot $Stamp
    if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    return $dir
}

# ============================ 外部进程 ============================
function Invoke-Native {
    <# 跑一个外部进程并把 stdout+stderr 一起收进日志文件（避开 PS 对 native stderr 的重定向坑）。 #>
    param([string]$Exe, [string[]]$Arguments, [string]$LogPath)
    $prev = $ErrorActionPreference
    $code = -1
    try {
        $ErrorActionPreference = 'Continue'
        & $Exe @Arguments *> $LogPath
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $prev
    }
    $text = ''
    if (Test-Path -LiteralPath $LogPath) { $text = (Get-Content -LiteralPath $LogPath -Raw) }
    return [pscustomobject]@{ ExitCode = $code; Output = $text; LogPath = $LogPath }
}

function Resolve-Tool {
    param([string]$Name)
    $cmd = Get-Command -Name $Name -ErrorAction SilentlyContinue
    if ($null -eq $cmd) { return $null }
    return $cmd.Source
}

function Get-DshowAudioDevices {
    <# 枚举 DirectShow 音频输入设备名（本机实测只有两个虚拟端点，没有物理麦克风）。 #>
    param([string]$FfmpegExe, [string]$LogPath)
    $res = Invoke-Native -Exe $FfmpegExe -Arguments @('-hide_banner', '-list_devices', 'true', '-f', 'dshow', '-i', 'dummy') -LogPath $LogPath
    $devices = [System.Collections.Generic.List[string]]::new()
    foreach ($m in [regex]::Matches($res.Output, '"([^"]+)"\s+\(audio\)')) { $devices.Add($m.Groups[1].Value) }
    return @($devices.ToArray())
}

function Test-MicSignal {
    <#
        麦克风可用性判据（Lead 实测口径）：录 N 秒、只看电平。
        ffmpeg -f dshow -i "audio=<设备名>" -t N -af volumedetect -f null NUL
        max_volume <= -80 dB ⇒ 该端点拿不到声学信号（物理麦在安静房间也会录到 -60~-40 dB 的环境噪声；
        本机两个虚拟端点无播放时实测 -91.0 / -90.3 dB）⇒ 明确报错，让用户插物理麦克风。
    #>
    param([string]$FfmpegExe, [string]$Device, [int]$Seconds, [string]$LogPath, [double]$SilenceDb = -80.0)
    $ffArgs = @(
        '-hide_banner', '-nostdin', '-f', 'dshow', '-i', ("audio=" + $Device),
        '-t', "$Seconds", '-af', 'volumedetect', '-f', 'null', 'NUL'
    )
    $res = Invoke-Native -Exe $FfmpegExe -Arguments $ffArgs -LogPath $LogPath
    $maxDb = $null
    $meanDb = $null
    if ($res.Output -match 'max_volume:\s*(-?\d+(?:\.\d+)?)\s*dB') { $maxDb = [double]$Matches[1] }
    if ($res.Output -match 'mean_volume:\s*(-?\d+(?:\.\d+)?)\s*dB') { $meanDb = [double]$Matches[1] }
    $opened = ($null -ne $maxDb)
    $usable = $opened -and ($maxDb -gt $SilenceDb)
    $reason = ''
    if (-not $opened) { $reason = "ffmpeg 打不开该输入端点（设备名写错？被别的程序独占？）" }
    elseif (-not $usable) { $reason = ("最高电平 {0:F1} dB <= 判据 {1:F1} dB：该端点拿不到声学信号" -f $maxDb, $SilenceDb) }
    return [pscustomobject]@{
        Device     = $Device
        Opened     = $opened
        MaxDb      = $maxDb
        MeanDb     = $meanDb
        Usable     = $usable
        Reason     = $reason
        ExitCode   = $res.ExitCode
        LogPath    = $LogPath
    }
}

# ============================ WAV 读写 ============================
function Write-Wav16 {
    <# 写 16-bit PCM 单声道 WAV（与 sync_measure.rs 的 write_wav_16 同格式）。 #>
    param([string]$Path, [double[]]$Samples, [int]$SampleRate)
    $n = $Samples.Count
    $i16 = [int16[]]::new($n)
    for ($i = 0; $i -lt $n; $i++) {
        $v = [Math]::Round($Samples[$i] * 32768.0)
        if ($v -gt 32767.0) { $v = 32767.0 } elseif ($v -lt -32768.0) { $v = -32768.0 }
        $i16[$i] = [int16]$v
    }
    $dataLen = $n * 2
    $bytes = [byte[]]::new(44 + $dataLen)
    [System.Text.Encoding]::ASCII.GetBytes('RIFF').CopyTo($bytes, 0)
    [System.BitConverter]::GetBytes([uint32](36 + $dataLen)).CopyTo($bytes, 4)
    [System.Text.Encoding]::ASCII.GetBytes('WAVE').CopyTo($bytes, 8)
    [System.Text.Encoding]::ASCII.GetBytes('fmt ').CopyTo($bytes, 12)
    [System.BitConverter]::GetBytes([uint32]16).CopyTo($bytes, 16)
    [System.BitConverter]::GetBytes([uint16]1).CopyTo($bytes, 20)
    [System.BitConverter]::GetBytes([uint16]1).CopyTo($bytes, 22)
    [System.BitConverter]::GetBytes([uint32]$SampleRate).CopyTo($bytes, 24)
    [System.BitConverter]::GetBytes([uint32]($SampleRate * 2)).CopyTo($bytes, 28)
    [System.BitConverter]::GetBytes([uint16]2).CopyTo($bytes, 32)
    [System.BitConverter]::GetBytes([uint16]16).CopyTo($bytes, 34)
    [System.Text.Encoding]::ASCII.GetBytes('data').CopyTo($bytes, 36)
    [System.BitConverter]::GetBytes([uint32]$dataLen).CopyTo($bytes, 40)
    if ($dataLen -gt 0) { [System.Buffer]::BlockCopy($i16, 0, $bytes, 44, $dataLen) }
    $dir = Split-Path -Parent $Path
    if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    [System.IO.File]::WriteAllBytes($Path, $bytes)
}

function Read-WavFile {
    <# 解析 WAV 并取出指定声道；支持 PCM 16/24/32 与 IEEE float 32/64（与 syncmeasure::parse_wav 同口径）。 #>
    param([string]$Path, [int]$Channel = 0)
    if (-not (Test-Path -LiteralPath $Path)) { throw "录音文件不存在：$Path" }
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 44) { throw "文件太短，不是合法 WAV：$Path（$($bytes.Length) 字节）" }
    if ([System.Text.Encoding]::ASCII.GetString($bytes, 0, 4) -ne 'RIFF' -or [System.Text.Encoding]::ASCII.GetString($bytes, 8, 4) -ne 'WAVE') {
        throw "不是 RIFF/WAVE 容器：$Path"
    }
    $pos = 12
    $fmt = $null
    $dataOff = -1
    $dataLen = 0
    while ($pos + 8 -le $bytes.Length) {
        $id = [System.Text.Encoding]::ASCII.GetString($bytes, $pos, 4)
        $size = [int][System.BitConverter]::ToUInt32($bytes, $pos + 4)
        $body = $pos + 8
        if ($id -eq 'fmt ') {
            if ($size -lt 16) { throw "fmt 块不合法：$Path" }
            $codec = [System.BitConverter]::ToUInt16($bytes, $body)
            $ch = [System.BitConverter]::ToUInt16($bytes, $body + 2)
            $sr = [System.BitConverter]::ToUInt32($bytes, $body + 4)
            $bits = [System.BitConverter]::ToUInt16($bytes, $body + 14)
            if ($codec -eq 0xFFFE -and $size -ge 40) { $codec = [System.BitConverter]::ToUInt16($bytes, $body + 24) }
            if ($ch -eq 0 -or $sr -eq 0) { throw "fmt 块不合法（声道/采样率为 0）：$Path" }
            $fmt = @{ Codec = $codec; Channels = [int]$ch; SampleRate = [int]$sr; Bits = [int]$bits }
        } elseif ($id -eq 'data') {
            $dataOff = $body
            $dataLen = $size
            if ($dataOff + $dataLen -gt $bytes.Length) { $dataLen = $bytes.Length - $dataOff }
        }
        $pos = $body + $size + ($size % 2)
    }
    if ($null -eq $fmt) { throw "缺少 fmt 块：$Path" }
    if ($dataOff -lt 0) { throw "缺少 data 块：$Path" }
    if ($Channel -ge $fmt.Channels) { throw "请求的声道 $Channel 超出范围（文件只有 $($fmt.Channels) 声道）：$Path" }
    $bps = [int]($fmt.Bits / 8)
    if ($bps -le 0) { throw "不支持的位深：$($fmt.Bits)" }
    $frameBytes = $bps * $fmt.Channels
    $frames = [int][Math]::Floor($dataLen / $frameBytes)
    if ($frames -lt 2) { throw "录音太短，无法分析（$frames 帧）：$Path" }
    $samples = [double[]]::new($frames)
    $codec = $fmt.Codec
    $bits = $fmt.Bits
    if ($codec -eq 1 -and $bits -eq 16) {
        $tmp = [int16[]]::new($frames * $fmt.Channels)
        [System.Buffer]::BlockCopy($bytes, $dataOff, $tmp, 0, $frames * $frameBytes)
        for ($i = 0; $i -lt $frames; $i++) { $samples[$i] = $tmp[$i * $fmt.Channels + $Channel] / 32768.0 }
    } elseif ($codec -eq 1 -and $bits -eq 32) {
        $tmp = [int32[]]::new($frames * $fmt.Channels)
        [System.Buffer]::BlockCopy($bytes, $dataOff, $tmp, 0, $frames * $frameBytes)
        for ($i = 0; $i -lt $frames; $i++) { $samples[$i] = $tmp[$i * $fmt.Channels + $Channel] / 2147483648.0 }
    } elseif ($codec -eq 3 -and $bits -eq 32) {
        $tmp = [single[]]::new($frames * $fmt.Channels)
        [System.Buffer]::BlockCopy($bytes, $dataOff, $tmp, 0, $frames * $frameBytes)
        for ($i = 0; $i -lt $frames; $i++) { $samples[$i] = [double]$tmp[$i * $fmt.Channels + $Channel] }
    } elseif ($codec -eq 3 -and $bits -eq 64) {
        $tmp = [double[]]::new($frames * $fmt.Channels)
        [System.Buffer]::BlockCopy($bytes, $dataOff, $tmp, 0, $frames * $frameBytes)
        for ($i = 0; $i -lt $frames; $i++) { $samples[$i] = $tmp[$i * $fmt.Channels + $Channel] }
    } elseif ($codec -eq 1 -and $bits -eq 24) {
        for ($i = 0; $i -lt $frames; $i++) {
            $b = $dataOff + $i * $frameBytes + $Channel * 3
            $v = $bytes[$b] -bor ($bytes[$b + 1] -shl 8) -bor ($bytes[$b + 2] -shl 16)
            if (($v -band 0x800000) -ne 0) { $v = $v -bor (-16777216) }
            $samples[$i] = $v / 8388608.0
        }
    } else {
        throw "不支持的 WAV 编码：codec=$codec bits=$bits（$Path）"
    }
    return [pscustomobject]@{
        Path       = $Path
        SampleRate = $fmt.SampleRate
        Channels   = $fmt.Channels
        Bits       = $bits
        Frames     = $frames
        Seconds    = $frames / [double]$fmt.SampleRate
        Samples    = $samples
    }
}

# ============================ 激励信号生成（docs/55 §2.2）============================
function New-StimulusSamples {
    <#
        6 个宽带噪声突发：确定性 LCG 噪声 + 0.4 ms 指数衰减、20 ms 窗、幅度 0.8、间隔 500 ms、总长 4000 ms。
        与 core/crates/audiolink-tools/src/bin/sync_measure.rs 的 synth_pulses 同形状（同一套 LCG 常数与衰减），
        因此本脚本的激励与 sync-measure --self-test --write 产出的 self-test-a.wav 可互相印证。
        用宽带瞬态而不是单一频率正弦：后者的互相关有周期歧义（docs/27 §2.2 实测偏差散开约 1 ms）。
    #>
    param(
        [int]$SampleRate = 48000,
        [int]$Count = 6,
        [double]$GapMs = 500.0,
        [double]$PulseMs = 20.0,
        [double]$DecayMs = 0.4,
        [double]$Amplitude = 0.8,
        [double]$TailMs = 1000.0
    )
    $totalMs = $Count * $GapMs + $TailMs
    $n = [int][Math]::Round($SampleRate * $totalMs / 1000.0)
    $buf = [double[]]::new($n)
    $pulseLen = [int][Math]::Floor($SampleRate * $PulseMs / 1000.0)
    $decay = [Math]::Max($SampleRate * $DecayMs / 1000.0, 1.0)
    for ($i = 0; $i -lt $Count; $i++) {
        # LCG 一律走 uint64 + 取模：PowerShell 对 int64/uint64 混用会提升成 decimal，显式统一类型才不会翻车
        # 注意：0x9E3779B9 这种「高位为 1 的 8 位十六进制字面量」在 PowerShell 里会被当 int32 负数，必须写十进制
        [uint64]$state = ([uint64]2654435769 -bxor (([uint64]$i * 2654435761) % 4294967296))
        $start = [int][Math]::Round($SampleRate * ($i * $GapMs) / 1000.0)
        for ($k = 0; $k -lt $pulseLen; $k++) {
            $idx = $start + $k
            if ($idx -ge $n) { break }
            $state = (($state * 1664525) + 1013904223) % 4294967296
            $noise = [double]($state -shr 9) / 4194304.0 - 1.0
            $env = [Math]::Exp(-1.0 * $k / $decay)
            $buf[$idx] = $buf[$idx] + ($Amplitude * $noise * $env)
        }
    }
    return , $buf
}

function Add-ClickTrack {
    <# 把一串同形状 click 叠加进既有缓冲（自检时用来合成「录音」里的参考峰/被测峰/反射）。 #>
    param(
        [double[]]$Buf,
        [int]$SampleRate,
        [double[]]$OnsetsSec,
        [double]$Amp,
        [int]$Seed,
        [double]$PulseMs = 20.0,
        [double]$DecayMs = 0.4
    )
    $n = $Buf.Count
    $pulseLen = [int][Math]::Floor($SampleRate * $PulseMs / 1000.0)
    $decay = [Math]::Max($SampleRate * $DecayMs / 1000.0, 1.0)
    $j = 0
    foreach ($onset in $OnsetsSec) {
        [uint64]$state = ((([uint64]($Seed + $j) * 22695477) + 1) % 4294967296)
        $start = [int][Math]::Round($SampleRate * $onset)
        for ($k = 0; $k -lt $pulseLen; $k++) {
            $idx = $start + $k
            if ($idx -ge $n) { break }
            if ($idx -lt 0) { continue }
            $state = (($state * 1664525) + 1013904223) % 4294967296
            $noise = [double]($state -shr 9) / 4194304.0 - 1.0
            $env = [Math]::Exp(-1.0 * $k / $decay)
            $Buf[$idx] = $Buf[$idx] + ($Amp * $noise * $env)
        }
        $j++
    }
}

function New-SynthRecording {
    <#
        自检用：合成一段「麦克风录音」。
        参考峰 = PC 喇叭（声程 P），被测峰 = 手机喇叭（全链路 + 声程 M）；两者都可带反射副本与底噪。
        底噪按 8 样本一块生成（块内常值）：循环量少 8 倍，而 83 us 的块长在 4 ms 快释放包络上仍只是起伏 —— 
        足以验证「噪声不会凭空造出峰、也不会把穿越点明显拉走」。
    #>
    param(
        [double]$LengthSec,
        [int]$SampleRate,
        [double[]]$RefOnsetsSec,
        [double[]]$MicOnsetsSec,
        [double]$RefAmp = 0.8,
        [double]$MicAmp = 0.6,
        [double]$NoiseDb = -60.0,
        [double]$ReflMs = 4.0,
        [double]$ReflGain = 0.30,
        [int]$Seed = 12345
    )
    $n = [int][Math]::Round($LengthSec * $SampleRate)
    $buf = [double[]]::new($n)
    Add-ClickTrack -Buf $buf -SampleRate $SampleRate -OnsetsSec $RefOnsetsSec -Amp $RefAmp -Seed $Seed
    Add-ClickTrack -Buf $buf -SampleRate $SampleRate -OnsetsSec $MicOnsetsSec -Amp $MicAmp -Seed ($Seed + 101)
    if ($ReflGain -gt 0) {
        $d = $ReflMs / 1000.0
        $refDelayed = @($RefOnsetsSec | ForEach-Object { $_ + $d })
        $micDelayed = @($MicOnsetsSec | ForEach-Object { $_ + $d })
        Add-ClickTrack -Buf $buf -SampleRate $SampleRate -OnsetsSec $refDelayed -Amp ($RefAmp * $ReflGain) -Seed ($Seed + 211)
        Add-ClickTrack -Buf $buf -SampleRate $SampleRate -OnsetsSec $micDelayed -Amp ($MicAmp * $ReflGain) -Seed ($Seed + 307)
    }
    if ($NoiseDb -gt -200.0) {
        $amp = [Math]::Pow(10.0, $NoiseDb / 20.0)
        [uint64]$state = (([uint64]$Seed * 7919) % 4294967296)
        $block = 8
        for ($i = 0; $i -lt $n; $i += $block) {
            $state = (($state * 1664525) + 1013904223) % 4294967296
            $v = (([double]($state -shr 9) / 4194304.0 - 1.0)) * $amp
            $end = [Math]::Min($i + $block, $n)
            for ($k = $i; $k -lt $end; $k++) { $buf[$k] = $buf[$k] + $v }
        }
    }
    return , $buf
}

# ============================ 检测 / 配对 / 分位 ============================
function Get-PulseOnsets {
    <#
        脉冲起始时刻（秒）。与 syncmeasure::detect_pulse_onsets 逐行同口径：
        去均值 → 快攻击慢释放包络（release = exp(-1/(tau*fs))）→ 阈值 = 包络峰值 x ratio →
        上升穿越 + 线性插值 → 最小间隔去重。
    #>
    param(
        [double[]]$Samples,
        [int]$SampleRate,
        [double]$ThresholdRatio,
        [double]$ReleaseTauMs = 4.0,
        [double]$MinGapMs = 5.0
    )
    $n = $Samples.Count
    if ($n -lt 8) { throw '录音太短，无法分析' }
    $sum = 0.0
    for ($i = 0; $i -lt $n; $i++) { $sum += $Samples[$i] }
    $mean = $sum / $n
    $tau = [Math]::Max($ReleaseTauMs / 1000.0, 1e-4)
    $release = [Math]::Exp(-1.0 / ($tau * $SampleRate))
    $env = [double[]]::new($n)
    $cur = 0.0
    $peak = 0.0
    for ($i = 0; $i -lt $n; $i++) {
        $x = $Samples[$i] - $mean
        if ($x -lt 0.0) { $x = -$x }
        if ($x -gt $cur) { $cur = $x } else { $cur = $cur * $release }
        $env[$i] = $cur
        if ($cur -gt $peak) { $peak = $cur }
    }
    if ($peak -le 1e-9) { throw '录音是静音（包络峰值为 0）—— 没有可比的东西' }
    $threshold = $peak * $ThresholdRatio
    $minGap = $MinGapMs / 1000.0
    $onsets = [System.Collections.Generic.List[double]]::new()
    if ($env[0] -ge $threshold) { $onsets.Add(0.0) }
    for ($i = 1; $i -lt $n; $i++) {
        $prev = $env[$i - 1]
        $curE = $env[$i]
        if ($prev -lt $threshold -and $curE -ge $threshold) {
            $den = $curE - $prev
            $frac = 0.0
            if ($den -gt 0.0) { $frac = ($threshold - $prev) / $den }
            $pos = ($i - 1) + $frac
            if ($pos -lt 0.0) { $pos = 0.0 }
            $secs = $pos / $SampleRate
            if ($onsets.Count -eq 0 -or ($secs - $onsets[$onsets.Count - 1]) -ge $minGap) { $onsets.Add($secs) }
        }
    }
    return [pscustomobject]@{
        Onsets    = @($onsets.ToArray())
        PeakEnv   = $peak
        Threshold = $threshold
        Frames    = $n
        MeanDc    = $mean
    }
}

function Get-NearestRankPercentile {
    <# 与 syncmeasure::percentile_nearest_rank 同定义：rank = ceil(q*n) - 1，clamp 到 [0, n-1]。 #>
    param([double[]]$Sorted, [double]$Q)
    if ($null -eq $Sorted -or $Sorted.Count -eq 0) { return [double]::NaN }
    $rank = [int][Math]::Ceiling($Q * $Sorted.Count) - 1
    if ($rank -lt 0) { $rank = 0 }
    if ($rank -ge $Sorted.Count) { $rank = $Sorted.Count - 1 }
    return $Sorted[$rank]
}

function Get-GroupRegularity {
    <# 一组峰的「组内间隔是否都接近 gap」评分（中位绝对偏差，秒）；NaN = 峰太少无法评。 #>
    param([double[]]$Times, [double]$GapSec)
    if ($null -eq $Times -or $Times.Count -lt 2) { return [double]::NaN }
    $devs = [System.Collections.Generic.List[double]]::new()
    for ($i = 0; $i -lt $Times.Count - 1; $i++) { $devs.Add([Math]::Abs(($Times[$i + 1] - $Times[$i]) - $GapSec)) }
    $sorted = @($devs.ToArray() | Sort-Object)
    return $sorted[[int][Math]::Floor($sorted.Count / 2)]
}

function Group-Pairs {
    <#
        在同一段录音里把峰分成「参考峰（PC 喇叭）」与「被测峰（手机喇叭）」并按序号配对。
        为什么按序号：syncmeasure::measure 的纪律就是这个 —— 第 i 个对第 i 个，不按最近邻乱配。
        做法：以第一个峰为锚（前提：录音先于播放启动，见 Run 流程的时序），按 k x 500 ms 的期望时刻找参考峰，
        再在这个参考峰之后 (MinDeltaMs, MaxDeltaMs] 内找最早的未使用峰作为被测峰。
        找不到就记为 dropped（不猜、不补）；额外峰（反射/噪声）单独列出，绝不静默丢弃。
    #>
    param(
        [double[]]$Onsets,
        [double]$GapMs = 500.0,
        [int]$SlotCount = 6,
        [double]$MinDeltaMs = 10.0,
        [double]$MaxDeltaMs = 375.0,
        [double]$RefToleranceFrac = 0.25
    )
    $gap = $GapMs / 1000.0
    $tol = $RefToleranceFrac * $gap
    $pairs = [System.Collections.Generic.List[object]]::new()
    $dropped = [System.Collections.Generic.List[string]]::new()
    $extra = [System.Collections.Generic.List[double]]::new()
    if ($null -eq $Onsets -or $Onsets.Count -eq 0) {
        return [pscustomobject]@{
            Pairs = @(); Dropped = @('录音里一个峰都没检测到'); Extra = @()
            RegularityA = [double]::NaN; RegularityB = [double]::NaN; Ref0Secs = $null
        }
    }
    $used = [bool[]]::new($Onsets.Count)
    $ref0 = $Onsets[0]
    for ($k = 0; $k -lt $SlotCount; $k++) {
        $expect = $ref0 + $k * $gap
        $bestIdx = -1
        $bestErr = [double]::MaxValue
        for ($i = 0; $i -lt $Onsets.Count; $i++) {
            if ($used[$i]) { continue }
            $err = [Math]::Abs($Onsets[$i] - $expect)
            if ($err -lt $bestErr) { $bestErr = $err; $bestIdx = $i }
        }
        if ($bestIdx -lt 0 -or $bestErr -gt $tol) {
            $dropped.Add(("slot {0}：期望参考峰 {1:F3} s 附近没有峰（容差 {2:F0} ms）" -f $k, $expect, ($tol * 1000.0)))
            continue
        }
        $used[$bestIdx] = $true
        $refT = $Onsets[$bestIdx]
        $minT = $refT + $MinDeltaMs / 1000.0
        $maxT = $refT + $MaxDeltaMs / 1000.0
        $moIdx = -1
        for ($i = 0; $i -lt $Onsets.Count; $i++) {
            if ($used[$i]) { continue }
            if ($Onsets[$i] -gt $minT -and $Onsets[$i] -le $maxT) { $moIdx = $i; break }
        }
        if ($moIdx -lt 0) {
            $dropped.Add(("slot {0}：只有参考峰（{1:F3} s），其后 {2:F0}–{3:F0} ms 内没有被测峰" -f $k, $refT, $MinDeltaMs, $MaxDeltaMs))
            continue
        }
        $used[$moIdx] = $true
        $moT = $Onsets[$moIdx]
        $pairs.Add([pscustomobject]@{
            index          = $k
            ref_secs       = [Math]::Round($refT, 5)
            mic_secs       = [Math]::Round($moT, 5)
            delta_ms       = [Math]::Round(($moT - $refT) * 1000.0, 4)
            ref_slot_err_ms = [Math]::Round(($refT - $expect) * 1000.0, 3)
        })
    }
    for ($i = 0; $i -lt $Onsets.Count; $i++) { if (-not $used[$i]) { $extra.Add($Onsets[$i]) } }
    $even = [System.Collections.Generic.List[double]]::new()
    $odd = [System.Collections.Generic.List[double]]::new()
    for ($i = 0; $i -lt $Onsets.Count; $i++) {
        if ($i % 2 -eq 0) { $even.Add($Onsets[$i]) } else { $odd.Add($Onsets[$i]) }
    }
    return [pscustomobject]@{
        Pairs       = @($pairs.ToArray())
        Dropped     = @($dropped.ToArray())
        Extra       = @($extra.ToArray())
        RegularityA = (Get-GroupRegularity -Times @($even.ToArray()) -GapSec $gap)
        RegularityB = (Get-GroupRegularity -Times @($odd.ToArray()) -GapSec $gap)
        Ref0Secs    = $ref0
    }
}

function Measure-Acoustic {
    <#
        分析一段「同一次录音」并给出 P50/P95 与判读。**这是量具唯一对外产出数字的地方。**
        失败时返回 ok=$false 且所有延迟字段为 $null —— 绝不输出伪造的 P50/P95。
    #>
    param(
        [string]$WavPath,
        [int]$Channel = 0,
        [double]$ThresholdRatio = 0.5,
        [int]$MinPairs = 6,
        [int]$SlotCount = 6,
        [double]$ExtraPathCm = 0.0,
        [bool]$AutoRelax = $true
    )
    $res = [ordered]@{
        ok = $false; wav_path = $WavPath; sample_rate = $null; seconds = $null; frames = $null
        pulses_detected = $null; onsets_secs = @(); pairs = @(); pairs_count = 0
        dropped = @(); extra_peaks_secs = @(); threshold_ratio = $null; threshold_attempts = @()
        peak_env = $null; env_threshold = $null; ref0_secs = $null
        mean_ms = $null; p50_ms = $null; p95_ms = $null; min_ms = $null; max_ms = $null
        path_correction_ms = $null; p50_corrected_ms = $null; p95_corrected_ms = $null
        verdict = 'insufficient'; errors = @(); notes = @()
    }
    try {
        $wav = Read-WavFile -Path $WavPath -Channel $Channel
    } catch {
        $res.errors += ("读不了录音：{0}" -f $_.Exception.Message)
        return [pscustomobject]$res
    }
    $res.sample_rate = $wav.SampleRate
    $res.seconds = [Math]::Round($wav.Seconds, 3)
    $res.frames = $wav.Frames
    if ($wav.SampleRate -ne $SCRIPT:Cfg.SampleRate) {
        $res.notes += ("录音采样率 {0} Hz 与量法口径 {1} Hz 不一致：本工具**不做重采样**（量具不把自己的重采样误差算进去），结论只在同采样率下可比。" -f $wav.SampleRate, $SCRIPT:Cfg.SampleRate)
    }
    $ratios = [System.Collections.Generic.List[double]]::new()
    $ratios.Add($ThresholdRatio)
    if ($AutoRelax) { foreach ($r in $SCRIPT:Cfg.RelaxLadder) { if ($r -lt $ThresholdRatio) { $ratios.Add($r) } } }
    $attempts = [System.Collections.Generic.List[object]]::new()
    $chosen = $null
    foreach ($ratio in $ratios) {
        try {
            $det = Get-PulseOnsets -Samples $wav.Samples -SampleRate $wav.SampleRate -ThresholdRatio $ratio -ReleaseTauMs $SCRIPT:Cfg.ReleaseTauMs -MinGapMs $SCRIPT:Cfg.MinGapMs
        } catch {
            $attempts.Add([pscustomobject]@{ ratio = $ratio; onsets = 0; pairs = 0; note = $_.Exception.Message })
            continue
        }
        $grp = Group-Pairs -Onsets $det.Onsets -GapMs $SCRIPT:Cfg.GapMs -SlotCount $SlotCount -MinDeltaMs $SCRIPT:Cfg.MinDeltaMs -MaxDeltaMs $SCRIPT:Cfg.MaxDeltaMs
        $attempts.Add([pscustomobject]@{
            ratio     = $ratio
            onsets    = $det.Onsets.Count
            pairs     = $grp.Pairs.Count
            peak_env  = [Math]::Round($det.PeakEnv, 6)
            threshold = [Math]::Round($det.Threshold, 6)
            note      = ''
        })
        if ($null -eq $chosen -or $grp.Pairs.Count -gt $chosen.Grp.Pairs.Count) {
            $chosen = [pscustomobject]@{ Ratio = $ratio; Det = $det; Grp = $grp }
        }
        if ($grp.Pairs.Count -ge $MinPairs) { $chosen = [pscustomobject]@{ Ratio = $ratio; Det = $det; Grp = $grp }; break }
    }
    $res.threshold_attempts = @($attempts.ToArray())
    if ($null -eq $chosen) {
        $why = '检测不出任何脉冲：录音可能是静音，或被削波成一坨'
        if ($attempts.Count -gt 0) { $why = [string]$attempts[0].note }
        $res.errors += $why
        return [pscustomobject]$res
    }
    $res.threshold_ratio = $chosen.Ratio
    $res.peak_env = [Math]::Round($chosen.Det.PeakEnv, 6)
    $res.env_threshold = [Math]::Round($chosen.Det.Threshold, 6)
    $res.pulses_detected = $chosen.Det.Onsets.Count
    $res.onsets_secs = @($chosen.Det.Onsets | ForEach-Object { [Math]::Round($_, 4) })
    $res.pairs = @($chosen.Grp.Pairs)
    $res.pairs_count = $chosen.Grp.Pairs.Count
    $res.dropped = @($chosen.Grp.Dropped)
    $res.extra_peaks_secs = @($chosen.Grp.Extra | ForEach-Object { [Math]::Round($_, 4) })
    $res.ref0_secs = [Math]::Round([double]$chosen.Grp.Ref0Secs, 4)

    if ($chosen.Ratio -lt $ThresholdRatio) {
        $res.notes += ("默认阈值 {0} 配不满 {1} 对，已降级到 {2} 才检出；降级意味着可能把噪声/反射峰算进来 —— 先调 PC 音量与手机音量让两个峰幅度接近，再重测。" -f $ThresholdRatio, $MinPairs, $chosen.Ratio)
    }
    if ($res.ref0_secs -lt 0.2) {
        $res.notes += ("第一个峰出现在录音开头 {0:F3} s 处：录音起点可能切掉了参考峰。Run 流程要求录音先于播放启动（前导 >= 1 s）。" -f $res.ref0_secs)
    }
    if ($res.pairs_count -lt $MinPairs) {
        $res.errors += ("有效配对只有 {0} 对（要求 >= {1}）。可能原因：手机没出声 / 待播队列没起来 / PC 喇叭没放出来 / 麦克风摆得太偏 / 两侧音量差太大（包络阈值 = 全局峰值 x {2}，弱的那一侧被吞掉）。" -f $res.pairs_count, $MinPairs, $chosen.Ratio)
        return [pscustomobject]$res
    }
    $deltas = @($res.pairs | ForEach-Object { [double]$_.delta_ms })
    $sorted = @($deltas | Sort-Object)
    $sum = 0.0
    foreach ($d in $deltas) { $sum += $d }
    $mean = $sum / $deltas.Count
    $res.mean_ms = [Math]::Round($mean, 3)
    $res.p50_ms = [Math]::Round((Get-NearestRankPercentile -Sorted $sorted -Q 0.5), 3)
    $res.p95_ms = [Math]::Round((Get-NearestRankPercentile -Sorted $sorted -Q 0.95), 3)
    $res.min_ms = [Math]::Round($sorted[0], 3)
    $res.max_ms = [Math]::Round($sorted[$sorted.Count - 1], 3)
    $corr = (($ExtraPathCm / 100.0) / 343.0) * 1000.0
    $res.path_correction_ms = [Math]::Round($corr, 3)
    $res.p50_corrected_ms = [Math]::Round($res.p50_ms - $corr, 3)
    $res.p95_corrected_ms = [Math]::Round($res.p95_ms - $corr, 3)

    $regA = [double]$chosen.Grp.RegularityA
    $regB = [double]$chosen.Grp.RegularityB
    if (-not [double]::IsNaN($regA) -and ($regA * 1000.0) -gt 40.0) {
        $res.notes += ("偶数组的组内间隔中位偏差 {0:F1} ms：峰序列不规整（可能漏检/多检），配对应人工核对。" -f ($regA * 1000.0))
    }
    if (-not [double]::IsNaN($regB) -and ($regB * 1000.0) -gt 40.0) {
        $res.notes += ("奇数组的组内间隔中位偏差 {0:F1} ms：峰序列不规整（可能漏检/多检），配对应人工核对。" -f ($regB * 1000.0))
    }
    if ($res.extra_peaks_secs.Count -gt 0) {
        $head = @($res.extra_peaks_secs | Select-Object -First 8)
        $res.notes += ("除配对峰外还检出 {0} 个额外峰（反射/噪声）：{1} —— 关窗、靠近声源重测。" -f $res.extra_peaks_secs.Count, ($head -join ', '))
    }
    $sigma = 0.0
    if ($deltas.Count -gt 1) {
        $acc = 0.0
        foreach ($d in $deltas) { $acc += ($d - $mean) * ($d - $mean) }
        $sigma = [Math]::Sqrt($acc / ($deltas.Count - 1))
    }
    $res.notes += ("样本 {0} 对；极差 {1:F1} ms；样本标准差 {2:F1} ms（抖动主要来自手机待播队列水位与 Wi-Fi，见 docs/55 §2.7 #5/#7）。" -f $deltas.Count, ([double]$res.max_ms - [double]$res.min_ms), $sigma)
    $res.ok = $true
    if (([double]$res.p50_ms -le $SCRIPT:Cfg.P50LimitMs) -and ([double]$res.p95_ms -le $SCRIPT:Cfg.P95LimitMs)) {
        $res.verdict = 'pass'
    } else {
        $res.verdict = 'fail'
    }
    return [pscustomobject]$res
}

function Show-Measurement {
    <# 把一次测量结果打印成人看的样子；失败时明确说「没有数字」。 #>
    param($M, [string]$Title = '测量结果')
    Emit ""
    Emit "── $Title ──" 'step'
    if (-not $M.ok) {
        Emit ("  ❌ 没有可用读数：{0}" -f ($M.errors -join ' / ')) 'err'
        Emit ("  录音：{0}（{1} 帧 / {2} s）" -f $M.wav_path, $M.frames, $M.seconds) 'warn'
        if ($M.pulses_detected -ne $null) { Emit ("  检出峰 {0} 个：{1}" -f $M.pulses_detected, ($M.onsets_secs -join ', ')) 'warn' }
        if ($M.dropped.Count -gt 0) { foreach ($d in $M.dropped) { Emit ("  · 丢弃 {0}" -f $d) 'warn' } }
        Emit "  **本工具不会在没有录音证据时输出 P50/P95。**" 'err'
        return
    }
    Emit ("  录音 {0}：{1} Hz / {2} s / 检出峰 {3} 个" -f (Split-Path -Leaf $M.wav_path), $M.sample_rate, $M.seconds, $M.pulses_detected) 'info'
    Emit ("  阈值比 {0}（包络峰值 {1:F4} → 阈值 {2:F4}）" -f $M.threshold_ratio, $M.peak_env, $M.env_threshold) 'info'
    Emit ("  有效配对 {0} 对（每对：参考峰 → 被测峰）" -f $M.pairs_count) 'info'
    foreach ($p in $M.pairs) {
        Emit ("    #{0}  {1:F4} s → {2:F4} s   差 {3:F2} ms（参考峰偏离 500 ms 栅格 {4:+0.0;-0.0;0.0} ms）" -f $p.index, $p.ref_secs, $p.mic_secs, $p.delta_ms, $p.ref_slot_err_ms) 'info'
    }
    foreach ($n in $M.notes) { Emit ("  · {0}" -f $n) 'warn' }
    Emit ""
    Emit ("  P50 = {0:F1} ms   P95 = {1:F1} ms   （均值 {2:F1} / 最小 {3:F1} / 最大 {4:F1} ms）" -f $M.p50_ms, $M.p95_ms, $M.mean_ms, $M.min_ms, $M.max_ms) 'step'
    if ($M.path_correction_ms -ne 0.0) {
        Emit ("  声程差修正 {0:+0.000;-0.000;0.000} ms 后：P50 = {1:F1} ms，P95 = {2:F1} ms" -f $M.path_correction_ms, $M.p50_corrected_ms, $M.p95_corrected_ms) 'info'
    }
    Emit ("  口径 A：手机链路相对 **PC 直连播放** 的净增量（不含 PC 播放链 22 ms + DAC ~1 ms，二者已被对消）。" ) 'info'
    Emit ("  验收线（docs/05 §M1）：P50 <= {0:F0} ms 且 P95 <= {1:F0} ms，样本 >= 6 对。" -f $SCRIPT:Cfg.P50LimitMs, $SCRIPT:Cfg.P95LimitMs) 'info'
    if ($M.verdict -eq 'pass') {
        Emit "  ✅ 判定：达标（pass）" 'ok'
    } else {
        Emit "  ❌ 判定：不达标（fail）—— 按 docs/55 §2.8 的三因排查：① 手机播放链吃满（AudioTrack 80.08 ms）② 待播队列水位（40–200 ms）③ 网络路径形态（是否同子网）" 'err'
    }
}

function Write-MeasurementReport {
    param($M, [string]$Path, [hashtable]$Meta = @{})
    $obj = [ordered]@{
        tool              = 'acoustic-latency'
        generated_at      = (Get-Date).ToString('s')
        method            = 'docs/55-device-acceptance-runbook.md §2（同源双路出声 + 单支麦克风，同一次录音）'
        calibration       = '口径 A = 手机链路相对 PC 直连播放的净增量（PC 播放链已被对消）'
        acceptance        = @{ p50_limit_ms = $SCRIPT:Cfg.P50LimitMs; p95_limit_ms = $SCRIPT:Cfg.P95LimitMs; min_pairs = $MinPairs }
        stimulus          = @{
            sample_rate = $SCRIPT:Cfg.SampleRate; bits = $SCRIPT:Cfg.Bits; channels = $SCRIPT:Cfg.Channels
            pulses = $SCRIPT:Cfg.PulseCount; pulse_ms = $SCRIPT:Cfg.PulseMs; decay_ms = $SCRIPT:Cfg.DecayMs
            amplitude = $SCRIPT:Cfg.Amplitude; gap_ms = $SCRIPT:Cfg.GapMs; total_ms = $SCRIPT:Cfg.TotalMs
        }
        meta              = $Meta
        ok                = $M.ok
        verdict           = $M.verdict
        sample_rate       = $M.sample_rate
        seconds           = $M.seconds
        frames            = $M.frames
        threshold_ratio   = $M.threshold_ratio
        threshold_attempts = $M.threshold_attempts
        peak_env          = $M.peak_env
        env_threshold     = $M.env_threshold
        ref0_secs         = $M.ref0_secs
        pulses_detected   = $M.pulses_detected
        onsets_secs       = $M.onsets_secs
        pairs_count       = $M.pairs_count
        pairs             = $M.pairs
        dropped           = $M.dropped
        extra_peaks_secs  = $M.extra_peaks_secs
        mean_ms           = $M.mean_ms
        p50_ms            = $M.p50_ms
        p95_ms            = $M.p95_ms
        min_ms            = $M.min_ms
        max_ms            = $M.max_ms
        path_correction_ms = $M.path_correction_ms
        p50_corrected_ms  = $M.p50_corrected_ms
        p95_corrected_ms  = $M.p95_corrected_ms
        notes             = $M.notes
        errors            = $M.errors
    }
    $dir = Split-Path -Parent $Path
    if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    ($obj | ConvertTo-Json -Depth 8) | Set-Content -LiteralPath $Path -Encoding utf8
    return $Path
}

# ============================ 自检（不需要麦克风）============================
function Invoke-SelfTest {
    <#
        自检的断言目标：**定位 + 配对 + 分位 + 判读**这一段逻辑正确。
        做法：程序自己合成一段双峰录音（已知 Δ 真值、带底噪与反射），写成真实 WAV 文件再读回来，
        走与现场测量**完全相同**的 Measure-Acoustic 路径，比对读数与真值。
        反例必须失败：静音 / 只有参考峰 / 只有被测峰 / 被测峰弱到检不出 —— 这四种情况下一律不得输出 P50/P95。
    #>
    param([string]$LogPath, [string]$ArtifactDir)
    Emit-Banner '[M1] 声学延迟量具自检（合成录音回放；不碰麦克风、不碰手机）'
    Emit (@'
自检覆盖：① 激励形状（§2.2）② 峰值定位 + 按序号配对 + 分位数（nearest-rank，与 syncmeasure 同定义）
          ③ 判读阈值（§2.8）④ 反例必须失败（绝不输出伪造的 P50/P95）
'@) 'info'

    $results = [System.Collections.Generic.List[object]]::new()
    $script:SelfTestFailed = 0

    function Add-Result {
        param([string]$Name, [bool]$Pass, [string]$Detail)
        $tag = if ($Pass) { '[PASS]' } else { '[FAIL]' }
        if ($Pass) { Emit ("  {0} {1} —— {2}" -f $tag, $Name, $Detail) 'ok' }
        else { Emit ("  {0} {1} —— {2}" -f $tag, $Name, $Detail) 'err'; $script:SelfTestFailed = $script:SelfTestFailed + 1 }
        $results.Add([pscustomobject]@{ name = $Name; pass = $Pass; detail = $Detail })
    }

    # ---------- T1：激励形状（§2.2）----------
    Emit ''
    Emit 'T1 激励信号形状（48 kHz / 16-bit / 单声道 / 6 个 20 ms 宽带突发 / 间隔 500 ms / 幅度 0.8）' 'step'
    $stim = New-StimulusSamples -SampleRate $SCRIPT:Cfg.SampleRate -Count $SCRIPT:Cfg.PulseCount -GapMs $SCRIPT:Cfg.GapMs -PulseMs $SCRIPT:Cfg.PulseMs -DecayMs $SCRIPT:Cfg.DecayMs -Amplitude $SCRIPT:Cfg.Amplitude
    $t1Problems = [System.Collections.Generic.List[string]]::new()
    $expectedSamples = [int][Math]::Round($SCRIPT:Cfg.SampleRate * $SCRIPT:Cfg.TotalMs / 1000.0)
    if ($stim.Count -ne $expectedSamples) { $t1Problems.Add("总样本 $($stim.Count) != $expectedSamples（4000 ms）") }
    for ($i = 0; $i -lt $SCRIPT:Cfg.PulseCount; $i++) {
        $start = [int][Math]::Round($SCRIPT:Cfg.SampleRate * ($i * $SCRIPT:Cfg.GapMs) / 1000.0)
        $peak = 0.0
        for ($k = $start; $k -lt [Math]::Min($start + 960, $stim.Count); $k++) {
            $v = [Math]::Abs($stim[$k])
            if ($v -gt $peak) { $peak = $v }
        }
        if ($peak -lt 0.5 -or $peak -gt 0.81) { $t1Problems.Add(("第 {0} 个突发峰值 {1:F4} 不在 (0.5, 0.81]" -f $i, $peak)) }
        $gStart = $start + 1440
        $gEnd = [Math]::Min($start + 22000, $stim.Count)
        $maxGap = 0.0
        for ($k = $gStart; $k -lt $gEnd; $k++) { $v = [Math]::Abs($stim[$k]); if ($v -gt $maxGap) { $maxGap = $v } }
        if ($maxGap -ne 0.0) { $t1Problems.Add(("第 {0} 个突发之后的间隙不静音（max={1:E3}）" -f $i, $maxGap)) }
    }
    $stimPath = Join-Path $ArtifactDir 'stimulus.wav'
    Write-Wav16 -Path $stimPath -Samples $stim -SampleRate $SCRIPT:Cfg.SampleRate
    $stimBack = Read-WavFile -Path $stimPath -Channel 0
    if ($stimBack.Frames -ne $stim.Count) { $t1Problems.Add("写回的帧数 $($stimBack.Frames) != $($stim.Count)") }
    if ($stimBack.SampleRate -ne 48000 -or $stimBack.Bits -ne 16 -or $stimBack.Channels -ne 1) {
        $t1Problems.Add("WAV 头不对：$($stimBack.SampleRate) Hz / $($stimBack.Bits) bit / $($stimBack.Channels) ch")
    }
    $maxRoundTripErr = 0.0
    for ($i = 0; $i -lt $stim.Count; $i += 7) {
        $e = [Math]::Abs($stim[$i] - $stimBack.Samples[$i])
        if ($e -gt $maxRoundTripErr) { $maxRoundTripErr = $e }
    }
    if ($maxRoundTripErr -gt 1.0 / 32768.0) { $t1Problems.Add(("写入-读回量化误差 {0:E3} 超 1 LSB" -f $maxRoundTripErr)) }
    $detail1 = ("{0} 个突发、总长 {1:F3} s、峰值 {2:F3}；写回 WAV {3}（SHA-256 {4}）" -f $SCRIPT:Cfg.PulseCount, ($stim.Count / 48000.0), ($stim | ForEach-Object { [Math]::Abs($_) } | Measure-Object -Maximum).Maximum, (Split-Path -Leaf $stimPath), (Get-Sha256 $stimPath).Substring(0, 16))
    if ($t1Problems.Count -eq 0) { Add-Result -Name 'T1-stimulus-shape' -Pass $true -Detail $detail1 }
    else { Add-Result -Name 'T1-stimulus-shape' -Pass $false -Detail ($t1Problems -join '; ') }

    # ---------- T2/T3/T4：合成录音回放 ----------
    Emit ''
    Emit 'T2 定位 + 配对 + 分位 + 判读（真值已知，断言 |读数 - 真值| <= 1 ms）' 'step'
    $cases = @(
        @{ name = 'T2A-const-92';       lead = 0.7; length = 4.8; truth = @(92.0, 92.0, 92.0, 92.0, 92.0, 92.0); refAmp = 0.80; micAmp = 0.60; noise = -62.0; refl = 0.30; expectOk = $true; expectVerdict = 'pass'; tolerMs = 1.0 },
        @{ name = 'T2B-const-1184';     lead = 0.7; length = 4.8; truth = @(118.4, 118.4, 118.4, 118.4, 118.4, 118.4); refAmp = 0.80; micAmp = 0.60; noise = -62.0; refl = 0.30; expectOk = $true; expectVerdict = 'fail'; tolerMs = 1.0 },
        @{ name = 'T2C-ramp-95-145';    lead = 0.7; length = 4.8; truth = @(95.0, 105.0, 115.0, 125.0, 135.0, 145.0); refAmp = 0.80; micAmp = 0.60; noise = -62.0; refl = 0.30; expectOk = $true; expectVerdict = 'fail'; tolerMs = 1.0 },
        @{ name = 'T2D-jitter-88-108';  lead = 0.7; length = 4.8; truth = @(88.0, 108.0, 95.5, 101.0, 92.3, 104.7); refAmp = 0.80; micAmp = 0.55; noise = -50.0; refl = 0.45; expectOk = $true; expectVerdict = 'pass'; tolerMs = 1.0 },
        @{ name = 'T2E-weak-mic-relax'; lead = 0.7; length = 4.8; truth = @(100.0, 100.0, 100.0, 100.0, 100.0, 100.0); refAmp = 0.90; micAmp = 0.25; noise = -70.0; refl = 0.20; expectOk = $true; expectVerdict = 'pass'; tolerMs = 1.0; expectRelax = $true },
        @{ name = 'T2F-p95-branch';     lead = 0.7; length = 4.8; truth = @(100.0, 100.0, 100.0, 100.0, 100.0, 160.0); refAmp = 0.80; micAmp = 0.60; noise = -62.0; refl = 0.30; expectOk = $true; expectVerdict = 'fail'; tolerMs = 1.0 },
        @{ name = 'T2G-large-300ms';    lead = 0.7; length = 4.8; truth = @(300.0, 300.0, 300.0, 300.0, 300.0, 300.0); refAmp = 0.80; micAmp = 0.60; noise = -62.0; refl = 0.30; expectOk = $true; expectVerdict = 'fail'; tolerMs = 1.0 },
        @{ name = 'T4-late-recording';  lead = 2.1; length = 6.2; truth = @(92.0, 92.0, 92.0, 92.0, 92.0, 92.0); refAmp = 0.80; micAmp = 0.60; noise = -62.0; refl = 0.30; expectOk = $true; expectVerdict = 'pass'; tolerMs = 1.0 }
    )
    $t2Readings = @{}
    foreach ($case in $cases) {
        $refOnsets = [System.Collections.Generic.List[double]]::new()
        $micOnsets = [System.Collections.Generic.List[double]]::new()
        for ($i = 0; $i -lt 6; $i++) {
            $t = $case.lead + $i * 0.5
            $refOnsets.Add($t)
            $micOnsets.Add($t + $case.truth[$i] / 1000.0)
        }
        $samples = New-SynthRecording -LengthSec $case.length -SampleRate 48000 -RefOnsetsSec @($refOnsets.ToArray()) -MicOnsetsSec @($micOnsets.ToArray()) -RefAmp $case.refAmp -MicAmp $case.micAmp -NoiseDb $case.noise -ReflMs 4.0 -ReflGain $case.refl -Seed 424242
        $path = Join-Path $ArtifactDir ($case.name + '.wav')
        Write-Wav16 -Path $path -Samples $samples -SampleRate 48000
        $m = Measure-Acoustic -WavPath $path -Channel 0 -ThresholdRatio 0.5 -MinPairs 6 -SlotCount 6
        if (-not $m.ok) {
            Add-Result -Name $case.name -Pass $false -Detail ("应当量出结果，实际失败：{0}" -f ($m.errors -join ' / '))
            continue
        }
        $problems = [System.Collections.Generic.List[string]]::new()
        $worst = 0.0
        for ($i = 0; $i -lt $m.pairs.Count; $i++) {
            $err = [Math]::Abs([double]$m.pairs[$i].delta_ms - $case.truth[$i])
            if ($err -gt $worst) { $worst = $err }
            if ($err -gt $case.tolerMs) { $problems.Add(("第 {0} 对读数 {1:F3} ms vs 真值 {2:F3} ms（误差 {3:F3} ms）" -f $i, $m.pairs[$i].delta_ms, $case.truth[$i], $err)) }
        }
        $truthSorted = @($case.truth | Sort-Object)
        $truthP50 = Get-NearestRankPercentile -Sorted $truthSorted -Q 0.5
        $truthP95 = Get-NearestRankPercentile -Sorted $truthSorted -Q 0.95
        $p50Err = [Math]::Abs([double]$m.p50_ms - $truthP50)
        $p95Err = [Math]::Abs([double]$m.p95_ms - $truthP95)
        if ($p50Err -gt $case.tolerMs) { $problems.Add(("P50 = {0:F3} ms vs 真值 {1:F3} ms" -f $m.p50_ms, $truthP50)) }
        if ($p95Err -gt $case.tolerMs) { $problems.Add(("P95 = {0:F3} ms vs 真值 {1:F3} ms" -f $m.p95_ms, $truthP95)) }
        if ($m.verdict -ne $case.expectVerdict) { $problems.Add(("判定 {0} != 期望 {1}" -f $m.verdict, $case.expectVerdict)) }
        if ($m.pairs_count -ne 6) { $problems.Add(("配对数 {0} != 6" -f $m.pairs_count)) }
        if ($case.ContainsKey('expectRelax') -and $case.expectRelax -and $m.threshold_ratio -ge 0.5) {
            $problems.Add(("弱信号用例应当发生阈值降级，实际阈值比 {0}" -f $m.threshold_ratio))
        }
        $t2Readings[$case.name] = @{ p50 = [double]$m.p50_ms; p95 = [double]$m.p95_ms; deltas = @($m.pairs | ForEach-Object { [double]$_.delta_ms }) }
        $detail = ("读数 P50 {0:F1} / P95 {1:F1} ms（真值 {2:F1} / {3:F1}），单对最大误差 {4:F3} ms，阈值比 {5}，判定 {6}" -f $m.p50_ms, $m.p95_ms, $truthP50, $truthP95, $worst, $m.threshold_ratio, $m.verdict)
        if ($problems.Count -eq 0) { Add-Result -Name $case.name -Pass $true -Detail $detail }
        else { Add-Result -Name $case.name -Pass $false -Detail ($detail + ' | ' + ($problems -join '; ')) }
    }

    # ---------- T4b：同一次录音口径 —— 起点整体后移后读数必须不变（§2.4）----------
    Emit ''
    Emit 'T4 时间轴纪律（录音起点整体后移 ⇒ 读数不变，§2.4）' 'step'
    if ($t2Readings.ContainsKey('T2A-const-92') -and $t2Readings.ContainsKey('T4-late-recording')) {
        $a = $t2Readings['T2A-const-92']
        $b = $t2Readings['T4-late-recording']
        $dP50 = [Math]::Abs($a.p50 - $b.p50)
        $same = $dP50 -le 0.05
        Add-Result -Name 'T4b-start-shift-invariance' -Pass $same -Detail ("录音起点后移 1.4 s：P50 {0:F3} ms → {1:F3} ms（差 {2:F3} ms，判据 <= 0.05 ms）" -f $a.p50, $b.p50, $dP50)
    } else {
        Add-Result -Name 'T4b-start-shift-invariance' -Pass $false -Detail '前置用例没跑出来，无法比较'
    }

    # ---------- T3：反例必须失败（绝不撒谎）----------
    Emit ''
    Emit 'T3 反例必须失败（不得输出任何 P50/P95）' 'step'
    $negCases = @(
        @{ name = 'T3A-silence';        lead = 0.7; length = 4.8; truth = @(92.0, 92.0, 92.0, 92.0, 92.0, 92.0); refAmp = 0.00; micAmp = 0.00; noise = -200.0; refl = 0.0 },
        @{ name = 'T3B-ref-only';       lead = 0.7; length = 4.8; truth = @(92.0, 92.0, 92.0, 92.0, 92.0, 92.0); refAmp = 0.80; micAmp = 0.00; noise = -62.0; refl = 0.30 },
        @{ name = 'T3C-mic-only';       lead = 0.7; length = 4.8; truth = @(92.0, 92.0, 92.0, 92.0, 92.0, 92.0); refAmp = 0.00; micAmp = 0.80; noise = -62.0; refl = 0.30 },
        @{ name = 'T3D-mic-too-weak';   lead = 0.7; length = 4.8; truth = @(92.0, 92.0, 92.0, 92.0, 92.0, 92.0); refAmp = 0.80; micAmp = 0.06; noise = -70.0; refl = 0.20 }
    )
    foreach ($case in $negCases) {
        $refOnsets = [System.Collections.Generic.List[double]]::new()
        $micOnsets = [System.Collections.Generic.List[double]]::new()
        for ($i = 0; $i -lt 6; $i++) {
            $t = $case.lead + $i * 0.5
            if ($case.refAmp -gt 0) { $refOnsets.Add($t) }
            if ($case.micAmp -gt 0) { $micOnsets.Add($t + $case.truth[$i] / 1000.0) }
        }
        $samples = New-SynthRecording -LengthSec $case.length -SampleRate 48000 -RefOnsetsSec @($refOnsets.ToArray()) -MicOnsetsSec @($micOnsets.ToArray()) -RefAmp $case.refAmp -MicAmp $case.micAmp -NoiseDb $case.noise -ReflMs 4.0 -ReflGain $case.refl -Seed 909090
        $path = Join-Path $ArtifactDir ($case.name + '.wav')
        Write-Wav16 -Path $path -Samples $samples -SampleRate 48000
        $m = Measure-Acoustic -WavPath $path -Channel 0 -ThresholdRatio 0.5 -MinPairs 6 -SlotCount 6
        $noNumbers = ($null -eq $m.p50_ms) -and ($null -eq $m.p95_ms)
        if ((-not $m.ok) -and $noNumbers) {
            $why = @($m.errors)[0]
            if (-not $why) { $why = '（无 errors 文本）' }
            Add-Result -Name $case.name -Pass $true -Detail ("按时失败且没有数字：{0}" -f $why)
        } elseif (-not $m.ok) {
            Add-Result -Name $case.name -Pass $false -Detail '虽然 ok=false 但延迟字段不是 null（可能残留了数字）'
        } else {
            Add-Result -Name $case.name -Pass $false -Detail ("不该有结果却量出了 P50 {0} ms / P95 {1} ms（{2} 对）" -f $m.p50_ms, $m.p95_ms, $m.pairs_count)
        }
    }

    # ---------- 汇总 ----------
    $total = $results.Count
    $passed = @($results | Where-Object { $_.pass }).Count
    Emit ''
    Emit ('-' * 78) 'head'
    if ($script:SelfTestFailed -eq 0) {
        Emit ("自检通过：{0}/{1}（量具的定位/配对/分位/判读逻辑可用）" -f $passed, $total) 'ok'
        Emit '未覆盖的部分（必须如实交代）：真实麦克风采集、真实手机链路、PC 播放链与 device-link 的时序 —— 这些本机没有条件验证。' 'warn'
    } else {
        Emit ("自检失败：{0}/{1} 通过，{2} 项不过 —— 量具不可用，不许拿它的数字去量真机。" -f $passed, $total, $script:SelfTestFailed) 'err'
    }
    Emit ('-' * 78) 'head'
    Save-LogFile -Path $LogPath
    if ($script:SelfTestFailed -eq 0) { return 0 } else { return 1 }
}

# ============================ 人工清单与误差表 ============================
function Show-SetupChecklist {
    param([string]$MicName, [string]$PeerIp, [string]$PcDev, [string]$PinFile)
    Emit ''
    Emit '【现场摆位与操作清单】（docs/55 §2.3 / §2.10 —— 这些必须现场定，脚本定不了）' 'step'
    Emit '  1) 插上物理麦克风（USB 麦或 3.5 mm 麦）；本机 dshow 里只有两个虚拟端点，测不出声学信号。' 'info'
    Emit '  2) 关窗、关风扇/空调；手机静音其它通知；房间尽量安静（反射峰会被当成竞争峰，§2.7 #6）。' 'info'
    Emit '  3) 麦克风摆在 **PC 喇叭** 与 **手机扬声器** 之间，尽量等距；量出两者到麦的距离差。' 'info'
    Emit '     声程差每 10 cm ≈ 0.29 ms、每 1 m ≈ 2.9 ms（§2.7 #3）；差多少就用 -ExtraPathCm 传多少（可留 0）。' 'info'
    Emit '  4) 调音量，让两个 click 在录音里**幅度接近**：PC 音量别开太大（它的峰最强），手机媒体音量适中。' 'info'
    Emit '     判据：包络阈值 = 全局峰值 × 0.5；若手机声音弱于 PC 的一半，被测峰会被整个吞掉。' 'info'
    Emit '  5) 手机与 PC 必须**同一子网**（§1.1）；手机 App 启动接收，界面显示监听状态。' 'info'
    Emit ('  6) 系统默认输出端点必须就是 device-link 采集的端点（--device {0}）：' -f $PcDev) 'info'
    Emit '     ffplay 放到的就是系统默认输出端点；两者不一致 = 参考声与被测声不是同一路，前功尽弃。' 'info'
    Emit '     测完之前不要切换音频设备、不要插拔耳机。' 'info'
    Emit ('  7) 手机侧配对：把手机屏幕上的 6 位 PIN 写进 {0}（脚本会轮询读取）。' -f $PinFile) 'info'
    Emit ("  8) 记录到 notes.md：手机型号/系统/包名、PC 端点名（{0}）、两端距离、房间、音量、git hash、手机待播队列水位。" -f $PcDev) 'info'
    Emit ("  9) 目标 peer：{0}；本轮期间手机不要切后台、不要锁屏省电。" -f $PeerIp) 'info'
}

function Show-ErrorBudget {
    Emit ''
    Emit '【误差来源与量级】（docs/55 §2.7 原表 —— 报告里也要逐项交代）' 'step'
    Emit '  #1 PC 播放链（共享模式缓冲 22 ms + DAC ~1 ms）~23 ms  → 被对消；代价是量到的是「口径 A」净增量' 'info'
    Emit '  #2 麦克风采集延迟 / 录音启动偏移（数 ms–数十 ms，未知）→ 被对消（同一次录音、同一设备）' 'info'
    Emit '  #3 两个声源到麦克风的声程差：10 cm ≈ 0.29 ms；1 m ≈ 2.9 ms → 用 -ExtraPathCm 修正' 'info'
    Emit '  #4 手机输出侧（被测对象，不扣）：AudioTrack 实际缓冲 3844 帧 = 80.08 ms；flinger track latency 101 ms' 'info'
    Emit '  #5 手机待播队列水位（被测对象）：实测 40–200 ms，30 min 时 P50 200 ms / 上限 320 ms' 'info'
    Emit '  #6 环境混响/反射：看报告里的 extra_peaks_secs；有关窗/近场要求' 'info'
    Emit '  #7 网络抖动：跨三层 RTT P50 11.7 / P95 81.1 ms；同子网 P50 6.65 ms → 记录当轮路径形态' 'info'
    Emit '  #8 时钟漂移：±50 ppm 量级，30 min ≈ 90 ms → 本次窗口 20–60 s，影响小' 'info'
    Emit '  本机自带的检测误差（自检实测）：单对定位误差 <= 1 ms 量级（合成真值比对）。' 'info'
}

function Resolve-DeviceLink {
    param([string]$Explicit)
    if ($Explicit) {
        if (-not (Test-Path -LiteralPath $Explicit)) { return $null }
        return [pscustomobject]@{ Exe = (Resolve-Path -LiteralPath $Explicit).Path; Prefix = @() }
    }
    # Lead 复核补（P1-A1）：本项目 .cargo/config.toml 锁了 build.target = x86_64-pc-windows-msvc，
    # 实际产物在 target\<triple>\debug\ 而不是 target\debug\ —— 原版只找后者，会让现场白白回退到
    # `cargo run` 并现场编译（首跑多等数分钟）。两个位置都找，triple 优先。
    foreach ($c in @(
            'target\x86_64-pc-windows-msvc\debug\device-link.exe',
            'target\x86_64-pc-windows-msvc\release\device-link.exe',
            'target\debug\device-link.exe',
            'target\release\device-link.exe'
        )) {
        if (Test-Path -LiteralPath $c) { return [pscustomobject]@{ Exe = (Resolve-Path -LiteralPath $c).Path; Prefix = @() } }
    }
    $cargo = Resolve-Tool -Name 'cargo'
    if ($cargo) { return [pscustomobject]@{ Exe = $cargo; Prefix = @('run', '-q', '-p', 'audiolink-tools', '--bin', 'device-link', '--') } }
    return $null
}

function Quote-Arg {
    param([string]$Text)
    if ($Text -match '[\s"]') { return ('"' + ($Text -replace '"', '\"') + '"') }
    return $Text
}

# ============================ DryRun ============================
function Invoke-DryRun {
    param([string]$FfmpegExe, [string]$FfplayExe, [string]$Mic, [string]$PeerIp, [string]$PcDev, [string]$DeviceLinkExePath, [bool]$ProbeMic, [string]$Stamp)
    Emit-Banner '[M1] -DryRun：预演（不播放、不录音、不改任何设备设置）'
    Emit '本模式下只做三件事：枚举音频设备、对选定麦克风做**只读**电平探测、把将要执行的步骤与命令打印出来。' 'info'

    Emit ''
    Emit '【步骤】' 'step'
    Emit '  1. 生成激励信号（48 kHz / 16-bit / 单声道 / 6 个 20 ms 宽带突发 / 间隔 500 ms / 总长 4000 ms，§2.2）' 'step'
    Emit '  2. 校验麦克风：枚举 dshow 输入端点 → 录 3 s 看电平；max_volume <= -80 dB 即判「拿不到声学信号」并退出' 'step'
    Emit '  3. 打印摆位清单（麦与两个声源的距离、音量匹配、关窗、同子网……）并等操作者确认' 'step'
    Emit '  4. 启动 device-link（WASAPI loopback 采 PC 输出端点 → AudioLink → 手机）；用 pin.txt 送 PIN' 'step'
    Emit '  5. **先**启动 ffmpeg 录音（前导 >= 1.5 s），**后**播放激励（ffplay 到系统默认输出端点）' 'step'
    Emit '  6. 播放结束 + 尾巴静音后收录音；停止 device-link' 'step'
    Emit '  7. 分析这一段录音：包络 → 阈值穿越定位峰 → 按序号配对（参考峰 vs 被测峰）→ P50/P95（nearest-rank）→ 判读' 'step'
    Emit '  8. 落盘：rec.wav / stimulus.wav / acoustic-latency.json / notes.md（+ SHA-256）' 'step'

    Emit ''
    Emit '【将会执行的命令】（渲染自当前参数）' 'step'
    $stimBase = if ($StimulusWav) { $StimulusWav } else { (Join-Path (Join-Path $SCRIPT:EvidenceRoot $Stamp) 'stimulus.wav') }
    Emit ('  激励：' + $(if ($StimulusWav) { "复用现成文件 $StimulusWav（不重新生成）" } else { 'New-StimulusSamples → Write-Wav16（脚本内置，见 §2.2 常量）' })) 'info'
    Emit ('  麦克风：ffmpeg -hide_banner -nostdin -f dshow -i "audio=<麦名>" -t 3 -af volumedetect -f null NUL') 'info'
    if ($Mic) { Emit ('    <麦名> = ' + $Mic) 'info' } else { Emit '    <麦名> = （未指定：需要 -MicDevice，见下面的候选列表）' 'warn' }
    # 预览必须与实际执行走同一条解析（Lead 复核补）：否则预演印 `cargo run`，实跑却在用 triple 下的 exe。
    $dlPreview = Resolve-DeviceLink -Explicit $DeviceLinkExePath
    $dlShown = if ($dlPreview) { ($dlPreview.Exe + ' ' + ($dlPreview.Prefix -join ' ')).Trim() } else { 'cargo run -q -p audiolink-tools --bin device-link --' }
    Emit ('  推流：' + $dlShown + ' run --peer <手机IP> --seconds <观测秒数> --capture wasapi --device ' + $PcDev + ' --pin-file <证据目录>\pin.txt --json <证据目录>\device-link.json') 'info'
    Emit '  录音：ffmpeg -hide_banner -nostdin -y -f dshow -i "audio=<麦名>" -ar 48000 -ac 1 -t <录音秒数> -c:a pcm_s16le <证据目录>\rec.wav' 'info'
    Emit ('  播放：ffplay -nodisp -autoexit -hide_banner -loglevel warning ' + $stimBase + '   （每轮一次，共 ' + $Rounds + ' 轮）') 'info'
    Emit ('  分析：pwsh tools/acoustic-latency.ps1 -AnalyseWav <证据目录>\rec.wav') 'info'

    Emit ''
    Emit '【工具链与本机事实】' 'step'
    Emit ('  ffmpeg : ' + $(if ($FfmpegExe) { $FfmpegExe } else { '✗ 没找到（-Ffmpeg 指定）' })) $(if ($FfmpegExe) { 'info' } else { 'err' })
    Emit ('  ffplay : ' + $(if ($FfplayExe) { $FfplayExe } else { '✗ 没找到（-Ffplay 指定）' })) $(if ($FfplayExe) { 'info' } else { 'err' })
    $dlResolved = Resolve-DeviceLink -Explicit $DeviceLinkExePath
    if ($dlResolved) { Emit ('  device-link : ' + $dlResolved.Exe + ' ' + ($dlResolved.Prefix -join ' ')) 'info' }
    else { Emit '  device-link : ✗ 既没有 target\<triple>\debug\device-link.exe（也没有 target\debug），也没有 cargo —— 先 cargo build -p audiolink-tools --bin device-link' 'err' }
    Emit ('  peer（手机 IP）：' + $(if ($PeerIp) { $PeerIp } else { '（未指定：Run 需要 -Peer <手机IP>；脚本不碰 adb，手机侧由你操作）' })) 'info'

    Emit ''
    Emit '【本机音频输入端点（dshow 枚举）】' 'step'
    $probeLog = Join-Path $env:TEMP ('acoustic-latency-dryrun-devices-{0}.log' -f $Stamp)
    $devices = @()
    if ($FfmpegExe) { $devices = Get-DshowAudioDevices -FfmpegExe $FfmpegExe -LogPath $probeLog }
    if ($devices.Count -eq 0) {
        Emit '  ✗ 一个音频输入端点都没枚举到' 'err'
    } else {
        foreach ($d in $devices) { Emit ('  · ' + $d) 'info' }
    }
    Emit '  注：本机（2026-09-17 实测）只有 「Virtual Mic (Virtual Mic for AudioRelay)」与「麦克风阵列 (网易虚拟音频设备)」' 'warn'
    Emit '      —— 两个虚拟端点，没有物理麦克风；它们无播放时 max_volume ≈ -91.0 / -90.3 dB（数字静音）。' 'warn'

    $micUsable = $false
    if ($ProbeMic -and $Mic -and $FfmpegExe) {
        Emit ''
        Emit ('【麦克风电平探测】audio=' + $Mic + '（只读 3 s，不播放）') 'step'
        $probeLog2 = Join-Path $env:TEMP ('acoustic-latency-dryrun-probe-{0}.log' -f $Stamp)
        $micProbe = Test-MicSignal -FfmpegExe $FfmpegExe -Device $Mic -Seconds $MicProbeSeconds -LogPath $probeLog2 -SilenceDb $MicSilenceDb
        if ($micProbe.Opened) {
            Emit ("  mean_volume = {0:F1} dB / max_volume = {1:F1} dB（判据：max_volume > {2:F1} dB 才算拿到声学信号）" -f $micProbe.MeanDb, $micProbe.MaxDb, $MicSilenceDb) 'info'
        } else {
            Emit '  ffmpeg 打不开该端点（设备名写错？被别的程序独占？）' 'err'
        }
        if ($micProbe.Usable) {
            $micUsable = $true
            Emit '  ✅ 该端点能拿到声学信号' 'ok'
        } else {
            Emit ('  ❌ ' + $micProbe.Reason) 'err'
            Emit '  ⇒ 先插上物理麦克风（USB 麦 / 3.5 mm 麦），再用 -MicDevice 指定它的 dshow 名字。' 'err'
        }
    } elseif (-not $Mic) {
        Emit ''
        Emit '【麦克风电平探测】跳过：没有指定 -MicDevice' 'warn'
    } else {
        Emit ''
        Emit '【麦克风电平探测】跳过：-SkipMicProbe' 'warn'
        $micUsable = $true
    }

    Show-SetupChecklist -MicName $Mic -PeerIp $PeerIp -PcDev $PcDev -PinFile (Join-Path (Join-Path $SCRIPT:EvidenceRoot $Stamp) 'pin.txt')
    Show-ErrorBudget

    Emit ''
    Emit '【退出码】0 达标 · 1 不达标 · 2 用法错误 · 3 检测失败（没有数字） · 4 设备不可用' 'step'
    Emit ''
    if ($micUsable -or $SkipMicProbe) {
        Emit '✅ DryRun 结束：脚本这一半齐备；麦克风与手机那一半要现场做。' 'ok'
        return 0
    }
    Emit '❌ DryRun 结束：本机当前**不可测量**（上面那条麦克风判据没过）。这不是脚本的错，是缺硬件。' 'err'
    return 4
}

# ============================ Run ============================
function Invoke-MeasurementRun {
    param([string]$FfmpegExe, [string]$FfplayExe)
    Emit-Banner '[M1] 声学端到端延迟测量（Run）'
    Emit '⚠️ 本流程含「未在本机验证过」的现场步骤（无麦克风、无手机）：device-link 推流、ffmpeg 采集、ffplay 播放的时序。' 'warn'
    Emit '   本机已验证的部分：激励生成、WAV 读写、定位/配对/分位/判读（见 -SelfTest 日志）。' 'warn'

    if (-not $Peer) {
        Emit '缺少 -Peer <手机IP>：Run 需要手机侧在接收（脚本不碰 adb，手机操作由你做）。' 'err'
        return 2
    }
    if (-not $FfmpegExe) { Emit '找不到 ffmpeg（用 -Ffmpeg 指定路径）。' 'err'; return 4 }
    if (-not $FfplayExe) { Emit '找不到 ffplay（用 -Ffplay 指定路径）。' 'err'; return 4 }
    $dl = Resolve-DeviceLink -Explicit $DeviceLinkExe
    if (-not $dl) {
        Emit '找不到 device-link：先 cargo build -p audiolink-tools --bin device-link，或用 -DeviceLinkExe 指定 exe。' 'err'
        return 4
    }

    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $dir = New-EvidenceDir -Stamp $stamp
    Emit ("证据目录：{0}" -f $dir) 'info'

    # --- 激励 ---
    $stimPath = $StimulusWav
    if ($stimPath) {
        if (-not (Test-Path -LiteralPath $stimPath)) { Emit ("-StimulusWav 不存在：{0}" -f $stimPath) 'err'; return 2 }
        Emit ("复用现成激励：{0}" -f $stimPath) 'ok'
    } else {
        $stim = New-StimulusSamples -SampleRate $SCRIPT:Cfg.SampleRate -Count $SCRIPT:Cfg.PulseCount -GapMs $SCRIPT:Cfg.GapMs -PulseMs $SCRIPT:Cfg.PulseMs -DecayMs $SCRIPT:Cfg.DecayMs -Amplitude $SCRIPT:Cfg.Amplitude
        $stimPath = Join-Path $dir 'stimulus.wav'
        Write-Wav16 -Path $stimPath -Samples $stim -SampleRate $SCRIPT:Cfg.SampleRate
        Emit ("已生成激励：{0}（{1} 样本 / {2:F3} s，SHA-256 {3}）" -f $stimPath, $stim.Count, ($stim.Count / 48000.0), (Get-Sha256 $stimPath).Substring(0, 16)) 'ok'
    }

    # --- 麦克风 ---
    $devLog = Join-Path $dir 'dshow-devices.log'
    $devices = Get-DshowAudioDevices -FfmpegExe $FfmpegExe -LogPath $devLog
    Emit ''
    Emit '本机 dshow 音频输入端点的候选：' 'step'
    foreach ($d in $devices) { Emit ('  · ' + $d) 'info' }
    $mic = $MicDevice
    if (-not $mic) {
        Emit '必须用 -MicDevice 指定麦克风（上面是候选）。本机默认没有物理麦克风 —— 先插上再查一次。' 'err'
        return 4
    }
    if ($devices.Count -gt 0 -and ($devices -notcontains $mic)) {
        Emit ("⚠️ -MicDevice「{0}」不在枚举结果里：名字要完全一致（区分中英文括号与空格）。" -f $mic) 'warn'
    }
    if (-not $SkipMicProbe) {
        Emit ''
        Emit ("麦克风电平探测：audio={0}（{1} s）" -f $mic, $MicProbeSeconds) 'step'
        $micProbe = Test-MicSignal -FfmpegExe $FfmpegExe -Device $mic -Seconds $MicProbeSeconds -LogPath (Join-Path $dir 'mic-probe.log') -SilenceDb $MicSilenceDb
        if (-not $micProbe.Opened) {
            Emit ("❌ 打不开输入端点「{0}」。插上物理麦克风后用 ffmpeg -list_devices true -f dshow -i dummy 查准确名字。" -f $mic) 'err'
            return 4
        }
        Emit ("  端点已打开：mean_volume = {0:F1} dB / max_volume = {1:F1} dB" -f $micProbe.MeanDb, $micProbe.MaxDb) 'info'
        if (-not $micProbe.Usable) {
            Emit ("❌ {0}" -f $micProbe.Reason) 'err'
            Emit '   判据：max_volume <= -80 dB 说明这个端点拿不到声学信号（虚拟端点无播放时就是数字静音）。' 'err'
            Emit '   **本工具不会在没有麦克风的情况下输出任何 P50/P95。**' 'err'
            return 4
        }
        Emit '  ✅ 麦克风能拿到声学信号' 'ok'
    } else {
        Emit '（-SkipMicProbe：跳过电平探测 —— 后果自负，静音录音会在分析阶段被明确判为失败）' 'warn'
    }

    Show-SetupChecklist -MicName $mic -PeerIp $Peer -PcDev $PcDevice -PinFile (Join-Path $dir 'pin.txt')
    if (-not $NonInteractive) {
        Emit ''
        Emit '按上面的清单摆好、手机 App 开始接收后，按回车开始测量（Ctrl+C 放弃）……' 'step'
        Read-Host | Out-Null
    } else {
        Emit ''
        Emit '（-NonInteractive：不等待人工确认，5 s 后自动开始）' 'warn'
        Start-Sleep -Seconds 5
    }

    $oneRound = $SCRIPT:Cfg.TotalMs / 1000.0 + 1.0
    $recSecs = if ($RecordSeconds -gt 0) { $RecordSeconds } else { [int][Math]::Ceiling(1.6 + $Rounds * $oneRound + 1.2) }
    $recPath = Join-Path $dir 'rec.wav'
    $pinFile = Join-Path $dir 'pin.txt'

    # --- device-link ---
    $dlArgs = @($dl.Prefix) + @(
        'run', '--peer', $Peer, '--seconds', "$PeerSeconds", '--capture', 'wasapi',
        '--device', $PcDevice, '--pin-file', $pinFile, '--json', (Join-Path $dir 'device-link.json')
    )
    $dlArgLine = (($dlArgs | ForEach-Object { Quote-Arg $_ }) -join ' ')
    Emit ''
    Emit ("启动 device-link：{0} {1}" -f $dl.Exe, $dlArgLine) 'step'
    $dlOut = Join-Path $dir 'device-link.out.log'
    $dlErr = Join-Path $dir 'device-link.err.log'
    $dlProc = Start-Process -FilePath $dl.Exe -ArgumentList $dlArgLine -PassThru -NoNewWindow -RedirectStandardOutput $dlOut -RedirectStandardError $dlErr
    Emit ("  它要把手机屏幕上的 6 位 PIN 写进：{0}（脚本每 300 ms 轮询；这一步由 device-link 自己做）" -f $pinFile) 'info'
    if (-not $NonInteractive) {
        Emit '  等 device-link 打印出「已连接 / 开始推流」后按回车继续（若它报错，先解决再继续）……' 'step'
        Read-Host | Out-Null
    } else {
        Emit '（-NonInteractive：等 20 s 让配对与推流建立）' 'warn'
        Start-Sleep -Seconds 20
    }
    if ($dlProc.HasExited) {
        Emit ("⚠️ device-link 已经退出（退出码 {0}）—— 看 {1} / {2}。" -f $dlProc.ExitCode, $dlOut, $dlErr) 'warn'
    }

    # --- 录音 + 播放 ---
    $recArgs = @(
        '-hide_banner', '-nostdin', '-y', '-f', 'dshow', '-i', ('audio=' + $mic),
        '-ar', '48000', '-ac', '1', '-t', "$recSecs", '-c:a', 'pcm_s16le', $recPath
    )
    $recArgLine = (($recArgs | ForEach-Object { Quote-Arg $_ }) -join ' ')
    Emit ''
    Emit ("启动录音（{0} s，先跑 1.6 s 前导再播放）：ffmpeg {1}" -f $recSecs, $recArgLine) 'step'
    $recErr = Join-Path $dir 'ffmpeg-record.log'
    $recProc = Start-Process -FilePath $FfmpegExe -ArgumentList $recArgLine -PassThru -NoNewWindow -RedirectStandardError $recErr
    Start-Sleep -Milliseconds 1600
    for ($r = 1; $r -le $Rounds; $r++) {
        $playArgLine = (('−nodisp', '-autoexit', '-hide_banner', '-loglevel', 'warning', $stimPath) | ForEach-Object { Quote-Arg $_ }) -join ' '
        $playArgLine = $playArgLine -replace '^−', '-'
        Emit ("  第 {0}/{1} 轮播放：ffplay {2}" -f $r, $Rounds, $playArgLine) 'step'
        $playProc = Start-Process -FilePath $FfplayExe -ArgumentList $playArgLine -PassThru -NoNewWindow -RedirectStandardError (Join-Path $dir ("ffplay-{0}.log" -f $r))
        $null = $playProc.WaitForExit(120000)
        Emit ("    播放进程退出（退出码 {0}）" -f $playProc.ExitCode) 'info'
        if ($r -lt $Rounds) { Start-Sleep -Milliseconds 900 }
    }
    Emit '  等尾巴静音写完……' 'step'
    $null = $recProc.WaitForExit(($recSecs + 20) * 1000)
    if (-not $recProc.HasExited) { Stop-Process -Id $recProc.Id -Force; Emit '  录音进程超时被强杀（WAV 尾部可能被截，工具按实际帧数分析）' 'warn' }
    else { Emit ("  录音进程正常退出（退出码 {0}）" -f $recProc.ExitCode) 'info' }
    if (-not $dlProc.HasExited) {
        Stop-Process -Id $dlProc.Id -Force
        Emit '  已停止 device-link' 'info'
    }

    if (-not (Test-Path -LiteralPath $recPath)) {
        Emit ("❌ 没生成录音文件：{0} —— 看 {1}" -f $recPath, $recErr) 'err'
        return 3
    }
    $recSize = (Get-Item -LiteralPath $recPath).Length
    Emit ("录音：{0}（{1:N0} 字节，SHA-256 {2}）" -f $recPath, $recSize, (Get-Sha256 $recPath).Substring(0, 16)) 'ok'

    # --- 分析 ---
    Show-ErrorBudget
    $m = Measure-Acoustic -WavPath $recPath -Channel $Channel -ThresholdRatio $ThresholdRatio -MinPairs $MinPairs -SlotCount $SCRIPT:Cfg.PulseCount -ExtraPathCm $ExtraPathCm
    Show-Measurement -M $m -Title '声学端到端延迟（口径 A）'

    $meta = [ordered]@{
        evidence_dir  = $dir
        peer          = $Peer
        mic_device    = $mic
        pc_endpoint   = $PcDevice
        rounds        = $Rounds
        record_secs   = $recSecs
        stimulus_wav  = $stimPath
        stimulus_sha256 = (Get-Sha256 $stimPath)
        rec_wav       = $recPath
        rec_sha256    = (Get-Sha256 $recPath)
        git_head      = (Get-GitHead)
        device_verified = $false
        scope         = '口径 A = 手机链路相对 PC 直连播放的净增量（PC 播放链 ~23 ms 已被对消，不含在内）'
    }
    $jsonPath = Join-Path $dir 'acoustic-latency.json'
    Write-MeasurementReport -M $m -Path $jsonPath -Meta $meta | Out-Null
    Emit ("报告：{0}" -f $jsonPath) 'ok'
    Write-NotesMarkdown -Path (Join-Path $dir 'notes.md') -M $m -Meta $meta
    return (Get-VerdictExitCode -M $m)
}

# ============================ 记录用的小工具 ============================
function Get-GitHead {
    try {
        $prev = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        $head = (& git rev-parse HEAD 2>$null)
        $dirty = (& git status --porcelain 2>$null)
        $ErrorActionPreference = $prev
        if ($head) {
            $s = ([string]$head).Trim()
            if ($dirty) { return ($s + ' (+dirty)') }
            return $s
        }
    } catch {
        return $null
    }
    return $null
}

function Get-VerdictExitCode {
    param($M)
    if (-not $M.ok) { return 3 }
    if ($M.verdict -eq 'pass') { return 0 }
    return 1
}

function Write-NotesMarkdown {
    param([string]$Path, $M, [hashtable]$Meta)
    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add('# 声学端到端延迟测量记录（自动生成，现场请补全「现场工况」一节）')
    $lines.Add('')
    $lines.Add(("- 时间：{0}" -f (Get-Date).ToString('s')))
    $lines.Add(("- 量法：docs/55-device-acceptance-runbook.md §2（同源双路出声 + 单支麦克风，同一次录音）"))
    $lines.Add(("- 口径：{0}" -f $Meta.scope))
    $lines.Add(("- peer（手机）：{0}" -f $Meta.peer))
    $lines.Add(("- 麦克风：{0}" -f $Meta.mic_device))
    $lines.Add(("- PC 采集端点（device-link --device）：{0}（必须 == 系统默认输出端点）" -f $Meta.pc_endpoint))
    $lines.Add(("- git HEAD：{0}" -f $Meta.git_head))
    $lines.Add('')
    $lines.Add('## 读数（全部来自录音，未做任何估算）')
    $lines.Add('')
    if ($M.ok) {
        $lines.Add(("- **P50 = {0:F1} ms**，**P95 = {1:F1} ms**（nearest-rank，与 syncmeasure 同定义）" -f $M.p50_ms, $M.p95_ms))
        $lines.Add(("- 均值 {0:F1} ms / 最小 {1:F1} ms / 最大 {2:F1} ms；样本 **{3} 对**" -f $M.mean_ms, $M.min_ms, $M.max_ms, $M.pairs_count))
        $lines.Add(("- 判定：**{0}**（验收线 P50 <= 110 ms 且 P95 <= 150 ms，§2.8）" -f $M.verdict))
    } else {
        $lines.Add('- **本次没有可用读数**（没有输出任何 P50/P95），原因：')
        foreach ($e in $M.errors) { $lines.Add(("- {0}" -f $e)) }
    }
    $lines.Add(("- 检测阈值比：{0}（包络峰值 {1}）" -f $M.threshold_ratio, $M.peak_env))
    $lines.Add(("- 录音：{0}（{1} 帧 / {2} s，SHA-256 {3}）" -f $Meta.rec_wav, $M.frames, $M.seconds, $Meta.rec_sha256))
    $lines.Add(("- 激励：{0}（SHA-256 {1}）" -f $Meta.stimulus_wav, $Meta.stimulus_sha256))
    $lines.Add('')
    $lines.Add('## 逐对读数')
    $lines.Add('')
    $lines.Add('| # | 参考峰时刻 (s) | 被测峰时刻 (s) | 差 (ms) |')
    $lines.Add('|---|---|---|---|')
    foreach ($p in $M.pairs) { $lines.Add(("| {0} | {1:F4} | {2:F4} | {3:F2} |" -f $p.index, $p.ref_secs, $p.mic_secs, $p.delta_ms)) }
    $lines.Add('')
    $lines.Add('## 误差项（docs/55 §2.7）')
    $lines.Add('')
    $lines.Add('- #1 PC 播放链 ~23 ms：**被对消**（参考与被测共享）→ 因此这是口径 A 的净增量')
    $lines.Add('- #2 麦克风采集延迟/录音启动偏移：**被对消**（同一次录音、同一设备）')
    $lines.Add(("- #3 声程差：现场量并填 -ExtraPathCm（当前 {0} cm ⇒ 修正 {1:F3} ms）" -f $ExtraPathCm, $M.path_correction_ms))
    $lines.Add('- #4 手机输出侧（被测对象，不扣）：AudioTrack 3844 帧 = 80.08 ms；flinger 101 ms')
    $lines.Add('- #5 手机待播队列水位（被测对象）：**现场填**（测前读遥测面板）')
    $lines.Add('- #6 环境反射：见 JSON 的 extra_peaks_secs')
    $lines.Add('- #7 网络路径形态（同子网/跨三层）：**现场填**')
    $lines.Add('- #8 时钟漂移：本次窗口短，影响小')
    $lines.Add('')
    $lines.Add('## 现场工况（请补全）')
    $lines.Add('')
    $lines.Add('- 手机型号 / Android 版本 / 包名（debug|release）：')
    $lines.Add('- PC 输出端点名 / 音量 / 手机媒体音量：')
    $lines.Add('- 麦克风与 PC 喇叭距离 / 与手机扬声器距离：')
    $lines.Add('- 房间 / 门窗状态 / 时段：')
    $lines.Add('- 手机待播队列水位（测前 / 测后）：')
    $lines.Add('- 网络：手机 IP / PC IP / 同子网？（是|否）')
    $dir = Split-Path -Parent $Path
    if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    Set-Content -LiteralPath $Path -Value ($lines -join [Environment]::NewLine) -Encoding utf8
    Write-Host "现场记录：$Path"
}

# ============================ 用法 ============================
function Show-Usage {
    Emit-Banner 'tools/acoustic-latency.ps1 —— [M1] 声学端到端延迟量具（docs/55 §2 口径）'
    Emit '  pwsh tools/acoustic-latency.ps1 -SelfTest                     # 自检（不需要麦克风，证明定位/配对/分位/判读正确）'
    Emit '  pwsh tools/acoustic-latency.ps1 -DryRun                       # 预演：打印步骤、命令、人工清单、误差表；只读探测麦克风'
    Emit '  pwsh tools/acoustic-latency.ps1 -Run -MicDevice "<麦名>" -Peer <手机IP>   # 正式测量（现场：插麦克风 + 手机在接收）'
    Emit '  pwsh tools/acoustic-latency.ps1 -AnalyseWav <rec.wav>          # 离线分析一段已有录音'
    Emit ''
    Emit '  常用参数：-PcDevice default|id:<子串>|name:<友好名>（device-link 采集端点，必须 == 系统默认输出端点）'
    Emit '            -Rounds N（重复几轮，合并样本） -MinPairs 6 -ExtraPathCm <声程差 cm> -Channel 0'
    Emit '            -EvidenceDir target/evidence/m1-device-p1 -MicSilenceDb -80 -SkipMicProbe -NonInteractive'
    Emit ''
    Emit '  退出码：0 达标 · 1 不达标 · 2 用法错误 · 3 检测失败（没有数字） · 4 设备不可用'
}

# ============================ 入口 ============================
$SCRIPT:EntryStamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$exitCode = 0
switch ($PSCmdlet.ParameterSetName) {
    'Usage' {
        Show-Usage
    }
    'SelfTest' {
        $logPath = $SelfTestLog
        $artifactDir = Join-Path $SCRIPT:EvidenceRoot 'selftest'
        if (-not $logPath) {
            if ($NoEvidence) { $logPath = Join-Path $env:TEMP ('acoustic-selftest-{0}.log' -f $SCRIPT:EntryStamp) }
            else { $logPath = Join-Path $SCRIPT:EvidenceRoot 'acoustic-selftest.log' }
        }
        if ($NoEvidence) { $artifactDir = Join-Path $env:TEMP ('acoustic-selftest-artifacts-{0}' -f $SCRIPT:EntryStamp) }
        if (-not (Test-Path -LiteralPath $artifactDir)) { New-Item -ItemType Directory -Force -Path $artifactDir | Out-Null }
        Emit ("自检产物目录：{0}" -f $artifactDir) 'info'
        $exitCode = Invoke-SelfTest -LogPath $logPath -ArtifactDir $artifactDir
    }
    'DryRun' {
        $ffmpegExe = Resolve-Tool -Name $Ffmpeg
        $ffplayExe = Resolve-Tool -Name $Ffplay
        $exitCode = Invoke-DryRun -FfmpegExe $ffmpegExe -FfplayExe $ffplayExe -Mic $MicDevice -PeerIp $Peer -PcDev $PcDevice -DeviceLinkExePath $DeviceLinkExe -ProbeMic (-not $SkipMicProbe) -Stamp $SCRIPT:EntryStamp
    }
    'Analyse' {
        $m = Measure-Acoustic -WavPath $AnalyseWav -Channel $Channel -ThresholdRatio $ThresholdRatio -MinPairs $MinPairs -SlotCount $SCRIPT:Cfg.PulseCount -ExtraPathCm $ExtraPathCm
        Show-Measurement -M $m -Title ("离线分析：" + (Split-Path -Leaf $AnalyseWav))
        Show-ErrorBudget
        if (-not $NoEvidence) {
            $dir = New-EvidenceDir -Stamp $SCRIPT:EntryStamp
            $jsonPath = Join-Path $dir 'acoustic-latency-analyse.json'
            $meta = [ordered]@{ analysed_wav = $AnalyseWav; analysed_sha256 = (Get-Sha256 $AnalyseWav); source_wav = $AnalyseWav; scope = '口径 A = 手机链路相对 PC 直连播放的净增量' }
            Write-MeasurementReport -M $m -Path $jsonPath -Meta $meta | Out-Null
            Emit ("报告：{0}" -f $jsonPath) 'ok'
        }
        $exitCode = Get-VerdictExitCode -M $m
    }
    'Run' {
        $ffmpegExe = Resolve-Tool -Name $Ffmpeg
        $ffplayExe = Resolve-Tool -Name $Ffplay
        $exitCode = Invoke-MeasurementRun -FfmpegExe $ffmpegExe -FfplayExe $ffplayExe
    }
    default {
        Show-Usage
    }
}
exit $exitCode
