#!/usr/bin/env pwsh
# M5 · 自动更新：本地端到端脚手架（一条命令跑完「能自动化的那一半」）
#
# 为什么需要它：看板 [M5] 真实更新流程验证要的是「干净机器装旧版 → 发布新版 → 客户端检测/下载/验签」。
# 其中「真发布 GitHub Release」是对外动作、「真安装」会改系统状态（Program Files / 注册表）——
# 这两条不适合自动做，收敛成人工清单（docs/42-m5-release-pipeline.md 第 12 节）。
# 本脚本把其余环节一条命令钉死：
#
#   S1 生成 latest.json          复用 tools/tauri-latest-json.ps1（不另写一份，避免两处真相）
#   S2 组装可托管目录            清单 + 安装包 + .sig 放进一个目录
#   S3 起本地静态托管            HttpListener，只在 127.0.0.1；不联网、不依赖 python
#   S4 走 HTTP 取回               可达性 + 清单字段完整性 + 字节一致（sha256 比对）
#   S5 版本比较语义              远端比本地新 / 相同 / 更旧 三种判定（与插件同一判据：remote > current）
#   S6 验签                      用 tauri.conf.json 的配置公钥对**从 HTTP 取回**的字节验签通过；
#                               再把取回的字节改 1 字节，必须被拒绝
#
# 与 tools/check-update.mjs 的分工：那个工具做的是「清单 ↔ 本地文件」的离线自洽验签（已进 release.yml）。
# 本脚本补的是它没覆盖的两件事：客户端真的走 HTTP 取、以及版本比较语义。
#
# 退出码（刻意区分「真的验证了」与「只是没报错」）：
#   0 = 全部通过（含验签正例与负例）
#   1 = 有断言失败（真问题）
#   2 = 缺少带签名的构建产物 → 验签环节**没被执行**（不算通过）
#
# 用法：
#   pwsh -File tools/updater-local-e2e.ps1
#   pwsh -File tools/updater-local-e2e.ps1 -Port 8400 -CleanWorkDir
[CmdletBinding()]
param(
    # 打包产物目录（相对仓库根）。
    [string]$BundleDir = 'target/x86_64-pc-windows-msvc/release/bundle/nsis',
    # 工作目录（托管内容与取回物留在这里当证据）。
    [string]$OutDir = 'target/evidence/updater-e2e',
    # 本地托管端口。
    [int]$Port = 8321,
    # 跑完删掉工作目录（默认保留，方便人工复核取回的字节）。
    [switch]$CleanWorkDir,
    # 破坏性自检：组装完成后把托管包改 1 字节 —— 预期 S6 验签失败、退出码 1。
    # 用途：证明「验签断言真的会红」，而不是只会打印 PASS。
    [switch]$TamperHostedPackage
)

$ErrorActionPreference = 'Stop'
try {
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    $OutputEncoding = [System.Text.Encoding]::UTF8
} catch { }

$root = Split-Path -Parent $PSScriptRoot
$passed = [System.Collections.Generic.List[string]]::new()
$failures = [System.Collections.Generic.List[string]]::new()

function Invoke-Stage {
    param([string]$Name, [scriptblock]$Body)
    Write-Host ''
    Write-Host "── $Name" -ForegroundColor Cyan
    try {
        & $Body
        $script:passed.Add($Name)
        Write-Host "   PASS  $Name" -ForegroundColor Green
    } catch {
        $script:failures.Add(("{0} :: {1}" -f $Name, $_.Exception.Message))
        Write-Host "   FAIL  $Name" -ForegroundColor Red
        Write-Host ("         " + $_.Exception.Message) -ForegroundColor Red
    }
}

# 取一个 JSON 端点：兼容 .Content 是 string 或 byte[] 两种情形（Content-Type 决定）。
function Read-JsonResponse {
    param([string]$Url)
    $response = Invoke-WebRequest -Uri $Url -TimeoutSec 15
    $content = $response.Content
    if ($content -is [byte[]]) {
        $content = [System.Text.Encoding]::UTF8.GetString($content)
    }
    return ($content | ConvertFrom-Json)
}

Write-Host 'M5 · 自动更新：本地端到端脚手架' -ForegroundColor Magenta
Write-Host "  仓库根：$root"
Write-Host "  产物目录：$BundleDir"

# ---------------------------------------------------------------- S0 产物前置检查
$bundlePath = Join-Path $root $BundleDir
$installer = $null
if (Test-Path -LiteralPath $bundlePath) {
    $installer = Get-ChildItem -LiteralPath $bundlePath -File -Filter '*-setup.exe' | Select-Object -First 1
}
$sigPath = if ($null -ne $installer) { $installer.FullName + '.sig' } else { '' }

if (($null -eq $installer) -or (-not (Test-Path -LiteralPath $sigPath))) {
    Write-Host ''
    Write-Host '缺少「带签名的安装包」：验签环节无法执行 —— 本脚本不假装通过。' -ForegroundColor Yellow
    Write-Host "  期望：$bundlePath\*-setup.exe 与同名 .sig"
    Write-Host '  怎么造：让 TAURI_SIGNING_PRIVATE_KEY 指向签名私钥，再跑 pwsh -File tools/tauri-build.ps1'
    Write-Host '  退出码 2 = 未执行（不等于通过）'
    exit 2
}

Write-Host ("  安装包：{0}（{1:N1} KB）" -f $installer.Name, ($installer.Length / 1KB))
Write-Host ("  签名文件：{0}（{1} B）" -f (Split-Path -Leaf $sigPath), (Get-Item -LiteralPath $sigPath).Length)

$workDir = Join-Path $root $OutDir
$serveDir = Join-Path $workDir 'serve'
$latestPath = Join-Path $root 'target/evidence/release/latest.json'
$baseUrl = "http://127.0.0.1:$Port"
$script:serverJob = $null
$exitCode = 1

try {
    # ------------------------------------------------------------ S1
    Invoke-Stage 'S1 生成更新清单（tools/tauri-latest-json.ps1）' {
        Remove-Item -LiteralPath $latestPath -Force -ErrorAction SilentlyContinue
        & (Join-Path $PSScriptRoot 'tauri-latest-json.ps1') -BundleDir $BundleDir -Out 'target/evidence/release/latest.json' -Notes 'M5 本地端到端脚手架（updater-local-e2e.ps1）'
        if (-not (Test-Path -LiteralPath $latestPath)) { throw "清单没有生成：$latestPath" }
        Write-Host "   清单：$latestPath"
    }

    # ------------------------------------------------------------ S2
    Invoke-Stage 'S2 组装可托管目录' {
        if (Test-Path -LiteralPath $serveDir) { Remove-Item -LiteralPath $serveDir -Recurse -Force }
        New-Item -ItemType Directory -Path $serveDir -Force | Out-Null
        Copy-Item -LiteralPath $installer.FullName -Destination $serveDir
        Copy-Item -LiteralPath $sigPath -Destination $serveDir
        Copy-Item -LiteralPath $latestPath -Destination $serveDir
        $names = (Get-ChildItem -LiteralPath $serveDir -File | Select-Object -ExpandProperty Name) -join ', '
        Write-Host "   托管目录：$serveDir"
        Write-Host "   文件：$names"
    }

    # ------------------------------------------------------------ S2.5（仅 -TamperHostedPackage）
    if ($TamperHostedPackage) {
        Invoke-Stage 'S2.5（破坏性自检）把托管包改 1 字节（预期 S6 失败）' {
            $target = Join-Path $serveDir $installer.Name
            $bytes = [System.IO.File]::ReadAllBytes($target)
            $bytes[$bytes.Length - 1] = $bytes[$bytes.Length - 1] -bxor 0x01
            [System.IO.File]::WriteAllBytes($target, $bytes)
            Write-Host ("   已翻转最后 1 个字节：{0}（清单里的签名一个字符都没动）" -f $installer.Name)
        }
    }

    # ------------------------------------------------------------ S3
    Invoke-Stage 'S3 在 127.0.0.1 起静态托管（HttpListener）' {
        $job = Start-ThreadJob -ScriptBlock {
            param($Directory, $PortNumber)
            $listener = [System.Net.HttpListener]::new()
            $listener.Prefixes.Add("http://127.0.0.1:$PortNumber/")
            $listener.Start()
            try {
                while ($listener.IsListening) {
                    $context = $listener.GetContext()
                    try {
                        # 按**文件名**匹配：这样清单里 releases/latest/download/xxx.exe 那种 URL
                        # 形状在本地也能命中，不需要改清单内容。
                        $name = [System.IO.Path]::GetFileName($context.Request.Url.AbsolutePath)
                        if ($name -eq '__shutdown') {
                            $context.Response.StatusCode = 200
                        } else {
                            $file = Join-Path $Directory $name
                            if (Test-Path -LiteralPath $file -PathType Leaf) {
                                $bytes = [System.IO.File]::ReadAllBytes($file)
                                # 按扩展名给 Content-Type：json 要能被 Invoke-WebRequest 当文本读，
                                # 否则 .Content 会是 byte[]，ConvertFrom-Json 拿到的不是 JSON。
                                $extension = [System.IO.Path]::GetExtension($name).ToLowerInvariant()
                                $contentType = if ($extension -eq '.json') { 'application/json; charset=utf-8' } else { 'application/octet-stream' }
                                $context.Response.StatusCode = 200
                                $context.Response.ContentType = $contentType
                                $context.Response.ContentLength64 = $bytes.Length
                                $context.Response.KeepAlive = $false
                                $context.Response.OutputStream.Write($bytes, 0, $bytes.Length)
                            } else {
                                $context.Response.StatusCode = 404
                                $context.Response.KeepAlive = $false
                            }
                        }
                    } finally {
                        try { $context.Response.Close() } catch { }
                    }
                    if ($name -eq '__shutdown') { break }
                }
            } finally {
                $listener.Stop()
                $listener.Close()
            }
        } -ArgumentList $serveDir, $Port

        $script:serverJob = $job
        $deadline = (Get-Date).AddSeconds(15)
        $ready = $false
        while ((-not $ready) -and ((Get-Date) -lt $deadline)) {
            try {
                $probe = Invoke-WebRequest -Uri "$baseUrl/latest.json" -TimeoutSec 3 -SkipHttpErrorCheck
                if ($probe.StatusCode -eq 200) { $ready = $true } else { Start-Sleep -Milliseconds 200 }
            } catch {
                Start-Sleep -Milliseconds 200
            }
        }
        if (-not $ready) { throw "托管服务在 15 秒内没有就绪：$baseUrl" }
        Write-Host "   服务就绪：$baseUrl（URL 形状与 releases/latest/download 一致）"
    }

    # ------------------------------------------------------------ S4
    Invoke-Stage 'S4 HTTP 可达 + 清单字段完整性 + 字节一致' {
        $manifest = Read-JsonResponse "$baseUrl/latest.json"

        foreach ($field in @('version', 'notes', 'pub_date')) {
            if ([string]::IsNullOrWhiteSpace([string]$manifest.$field)) { throw "latest.json 缺字段：$field" }
        }
        $entry = $manifest.platforms.'windows-x86_64'
        if ($null -eq $entry) { throw 'latest.json 缺 platforms.windows-x86_64' }
        if ([string]::IsNullOrWhiteSpace([string]$entry.url)) { throw 'latest.json 的 platforms.windows-x86_64.url 为空' }
        if ([string]::IsNullOrWhiteSpace([string]$entry.signature)) { throw 'latest.json 的 platforms.windows-x86_64.signature 为空' }

        $fileName = [System.IO.Path]::GetFileName(([uri]$entry.url).AbsolutePath)
        $hostedFile = Join-Path $serveDir $fileName
        if (-not (Test-Path -LiteralPath $hostedFile)) { throw "清单 url 指向 $fileName，但托管目录里没有这个文件" }

        $downloaded = Join-Path $workDir 'downloaded-installer.exe'
        Invoke-WebRequest -Uri "$baseUrl/releases/latest/download/$fileName" -OutFile $downloaded -TimeoutSec 180
        $localHash = (Get-FileHash -LiteralPath $hostedFile -Algorithm SHA256).Hash
        $remoteHash = (Get-FileHash -LiteralPath $downloaded -Algorithm SHA256).Hash
        if ($localHash -ne $remoteHash) { throw "HTTP 取回的字节与托管文件不一致（$localHash vs $remoteHash）" }

        Write-Host ("   version={0}  platform=windows-x86_64  file={1}（{2:N1} KB）" -f $manifest.version, $fileName, ((Get-Item -LiteralPath $downloaded).Length / 1KB))
        Write-Host ("   清单 url：{0}" -f $entry.url)
        Write-Host ("   签名前 40 字符：{0}…" -f $entry.signature.Substring(0, [Math]::Min(40, $entry.signature.Length)))
        Write-Host ("   字节一致：sha256={0}…" -f $localHash.Substring(0, 16))
    }

    # ------------------------------------------------------------ S5
    Invoke-Stage 'S5 版本比较语义（新 / 同 / 旧）' {
        function Get-VersionParts([string]$value) {
            $core = ($value -split '[-+]')[0]
            $parts = @($core.Split('.') | ForEach-Object { [int]($_ -replace '[^0-9]', '') })
            while ($parts.Count -lt 3) { $parts += 0 }
            return $parts
        }
        function Compare-VersionCore([string]$left, [string]$right) {
            $a = Get-VersionParts $left
            $b = Get-VersionParts $right
            for ($i = 0; $i -lt 3; $i++) {
                if ($a[$i] -gt $b[$i]) { return 1 }
                if ($a[$i] -lt $b[$i]) { return -1 }
            }
            return 0
        }

        $remote = [string]((Read-JsonResponse "$baseUrl/latest.json").version)
        if ([string]::IsNullOrWhiteSpace($remote)) { throw '清单里没有 version，无法判断版本语义' }

        # 判据与 tauri-plugin-updater 一致：只有 remote > current 才算「有更新」。
        $cases = @(
            @{ label = '远端更新'; current = '0.0.9'; expect = $true },
            @{ label = '版本相同'; current = $remote; expect = $false },
            @{ label = '远端更旧'; current = '0.9.9'; expect = $false }
        )
        foreach ($case in $cases) {
            $cmp = Compare-VersionCore $remote $case.current
            $shouldUpdate = $cmp -gt 0
            Write-Host ("   {0}：remote={1} current={2} → 比较={3} shouldUpdate={4}" -f $case.label, $remote, $case.current, $cmp, $shouldUpdate)
            if ($shouldUpdate -ne $case.expect) {
                throw ("版本语义判定不符：{0}（期望 shouldUpdate={1}，实际 {2}）" -f $case.label, $case.expect, $shouldUpdate)
            }
        }
    }

    # ------------------------------------------------------------ S6
    Invoke-Stage 'S6 验签：配置公钥 + 从 HTTP 取回的字节（正例通过 / 改 1 字节被拒）' {
        $env:AUDIOLINK_UPDATER_E2E_BASE = $baseUrl
        $env:AUDIOLINK_UPDATER_E2E_DIR = $serveDir
        Push-Location $root
        try {
            $output = & cargo test -p audiolink-desktop --test updater_hosted_e2e -- --nocapture 2>&1 | Out-String
            $code = $LASTEXITCODE
        } finally {
            Pop-Location
        }
        Write-Host $output
        if ($code -ne 0) { throw "Rust 端到端验签测试失败（exit $code）" }
        # 关键：必须看到两条标记行 —— 否则「没报错」可能只是测试被跳过了。
        if ($output -notmatch 'E2E-HOSTED: positive ok') { throw '没有看到正例标记行 E2E-HOSTED: positive ok —— 测试可能被跳过，不能算通过' }
        if ($output -notmatch 'E2E-HOSTED: negative rejected InvalidSignature') { throw '没有看到负例标记行 E2E-HOSTED: negative rejected InvalidSignature —— 负例没有真的执行' }
    }

    if ($failures.Count -gt 0) { $exitCode = 1 } else { $exitCode = 0 }
} finally {
    if ($null -ne $script:serverJob) {
        try { Invoke-WebRequest -Uri "$baseUrl/__shutdown" -TimeoutSec 3 -SkipHttpErrorCheck | Out-Null } catch { }
        Start-Sleep -Milliseconds 200
        try { Stop-Job -Job $script:serverJob -ErrorAction SilentlyContinue } catch { }
        try { Remove-Job -Job $script:serverJob -Force -ErrorAction SilentlyContinue } catch { }
    }
    if ($CleanWorkDir -and (Test-Path -LiteralPath $workDir)) {
        Remove-Item -LiteralPath $workDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Write-Host ''
Write-Host '── 汇总' -ForegroundColor Cyan
foreach ($name in $passed) { Write-Host "   PASS  $name" -ForegroundColor Green }
foreach ($item in $failures) { Write-Host "   FAIL  $item" -ForegroundColor Red }
Write-Host ''
if ($TamperHostedPackage) {
    Write-Host '  注意：本次是 -TamperHostedPackage 破坏性自检 —— S6 失败属于**预期结果**。' -ForegroundColor Yellow
}
if ($exitCode -eq 0) {
    Write-Host '本地端到端脚手架：全部通过（含验签正例与负例）' -ForegroundColor Green
    Write-Host '  仍未覆盖：真发布 GitHub Release、真安装 —— 见 docs/42-m5-release-pipeline.md 第 12 节的人工清单'
} else {
    Write-Host ("本地端到端脚手架：失败 {0} 项" -f $failures.Count) -ForegroundColor Red
}
Write-Host "  证据目录：$workDir"
exit $exitCode
