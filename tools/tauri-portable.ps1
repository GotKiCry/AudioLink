#!/usr/bin/env pwsh
# M5：桌面端绿色版（便携包）—— exe + 第三方声明 + 说明 → 一个 zip。
#
# 与安装包（NSIS）的分工：
#   · 安装包负责「装进系统」：注册表、开始菜单、WebView2 引导、卸载入口；
#   · 绿色版负责「解压即用」：不写系统目录，删掉目录即卸载。
#
# **一个真实的正确性点**：Tauri 的 resource_dir() 在便携形态下就是 **exe 所在目录**。
# 所以 THIRD-PARTY-NOTICES.md 必须摆在**与 exe 同级**，否则「关于 / 第三方声明」面板会显示
# 「未找到声明文件」。安装版里这件事由 bundle.resources 处理，绿色版必须自己摆。
#
# 另一件事写进 README 而不是猜：配置落在**用户目录**（%APPDATA%\com.gotkicry.audiolink），
# 不跟着 zip 走 —— 路线图 M5 的验收也是这么写的（「绿色版解压即用，配置写用户目录」）。
#
# 用法：
#   pwsh tools/tauri-portable.ps1              # 打 zip + 校验和
#   pwsh tools/tauri-portable.ps1 -Verify      # 再解压到临时目录、真启动一次（本地验证用）
[CmdletBinding()]
param(
    # 产物路径；默认 target/evidence/release/portable/AudioLink_<版本>_x64_portable.zip
    [string]$Output = "",

    # 打包后解压到临时目录并启动一次 exe，确认「解压即用」。CI 不跑这个开关。
    [switch]$Verify
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

$version = (Get-Content -Raw -LiteralPath (Join-Path $root "desktop/src-tauri/tauri.conf.json") | ConvertFrom-Json).version
$exeSrc = Join-Path $root "target/x86_64-pc-windows-msvc/release/audiolink-desktop.exe"
$noticesSrc = Join-Path $root "docs/compliance/THIRD-PARTY-NOTICES.md"

if (-not (Test-Path -LiteralPath $exeSrc)) {
    throw "找不到桌面端 exe：$exeSrc —— 先跑 pwsh tools/tauri-build.ps1（或 pnpm tauri build）"
}
if (-not (Test-Path -LiteralPath $noticesSrc)) {
    throw "找不到第三方声明：$noticesSrc —— 先跑 pwsh tools/license-audit.ps1 -Notices"
}
if ([string]::IsNullOrEmpty($Output)) {
    $Output = Join-Path $root ("target/evidence/release/portable/AudioLink_" + $version + "_x64_portable.zip")
}

$readme = @'
AudioLink 绿色版（便携包）
==========================

形态：解压即用 —— 不需要安装，删掉整个目录即卸载。

怎么用
------
1. 把本目录放到任意位置（桌面、D 盘、U 盘都行）；
2. 双击 AudioLink.exe；
3. 若提示缺少 WebView2：Windows 11 自带；Windows 10 请先安装一次 Microsoft Edge WebView2
   Runtime（Evergreen Bootstrapper），装完再启动。

配置与数据放在哪
----------------
写在【用户目录】，不在本目录：
    %APPDATA%\com.gotkicry.audiolink
里面有 settings.json（界面语言 / 开机自启 / 上次设备）与日志。
也就是说：把本目录复制到另一台机器，设置不会跟着走；要清空设置就删掉那个目录。

与安装版的区别
--------------
  绿色版：解压即用；更新靠手动替换 exe；没有 WebView2 引导。
  安装版：装进系统、有卸载入口；应用内自动更新（带签名校验）。
两者配置位置相同（都在用户目录）。

第三方声明
----------
THIRD-PARTY-NOTICES.md 与 exe 同级，「关于 / 第三方声明」面板会读它。

---
Portable build. Unzip and run AudioLink.exe.
Requires the Microsoft Edge WebView2 Runtime (bundled with Windows 11).
Settings live in %APPDATA%\com.gotkicry.audiolink - they do NOT travel with this folder.
'@

$staging = Join-Path ([IO.Path]::GetTempPath()) ("audiolink-portable-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
New-Item -ItemType Directory -Force -Path $staging | Out-Null
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Output) | Out-Null

try {
    Copy-Item -LiteralPath $exeSrc -Destination (Join-Path $staging "AudioLink.exe")
    Copy-Item -LiteralPath $noticesSrc -Destination (Join-Path $staging "THIRD-PARTY-NOTICES.md")
    Set-Content -LiteralPath (Join-Path $staging "README-portable.txt") -Value $readme -Encoding utf8NoBOM
    if (Test-Path -LiteralPath $Output) { Remove-Item -LiteralPath $Output -Force }
    Compress-Archive -Path (Join-Path $staging "*") -DestinationPath $Output -CompressionLevel Optimal
}
finally {
    Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
}

$zip = Get-Item -LiteralPath $Output
$hash = (Get-FileHash -LiteralPath $Output -Algorithm SHA256).Hash
$sumFile = Join-Path (Split-Path -Parent $Output) "SHA256SUMS.txt"
Set-Content -LiteralPath $sumFile -Value ($hash + "  " + $zip.Name) -Encoding utf8NoBOM

Write-Host ("portable: {0}  {1:N1} KB" -f $zip.Name, ($zip.Length / 1KB))
Write-Host "  内容：AudioLink.exe / THIRD-PARTY-NOTICES.md / README-portable.txt"
Write-Host ("  SHA256：{0}" -f $hash)
Write-Host ("  校验和：{0}" -f $sumFile)

if ($Verify) {
    $unpack = Join-Path ([IO.Path]::GetTempPath()) ("audiolink-verify-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
    Expand-Archive -LiteralPath $Output -DestinationPath $unpack -Force
    try {
        $proc = Start-Process -FilePath (Join-Path $unpack "AudioLink.exe") -PassThru
        Start-Sleep -Seconds 8
        if ($proc.HasExited) {
            throw ("解压后启动失败：进程在 8 s 内退出（exit code " + $proc.ExitCode + "）")
        }
        Stop-Process -Id $proc.Id -Force
        Write-Host "  ✓ 解压后启动成功（进程存活 8 s），已关闭"
        Write-Host ("  配置目录：" + (Join-Path $env:APPDATA "com.gotkicry.audiolink"))
    }
    finally {
        Remove-Item -LiteralPath $unpack -Recurse -Force -ErrorAction SilentlyContinue
    }
}
