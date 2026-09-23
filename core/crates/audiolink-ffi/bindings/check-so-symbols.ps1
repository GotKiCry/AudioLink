# 校验「绑定 ↔ 当前源码 ↔ target ↔ jniLibs ↔ 最新 APK」同源 —— 每一环都要过。
#
# 四环各自能抓到的故障（都是真实踩过的，不是假想）：
#   ① 绑定 ↔ 当前源码：**本次事故的形态** —— 只改了 Rust 的文档注释，符号名不变、符号也都在，
#      但 UniFFI 的函数 checksum 变了（实测：给 set_local_peer_gain 的 KDoc 加一个空行 → 27074 → 23920）。
#      设备侧表现为 engineStart 抛 UniFFI API checksum mismatch，界面上是「服务没能启动」。
#      `findstr` 查符号**抓不到**这种漂移，只有「重新生成一次看有没有 diff」能抓到。
#   ② target ↔ jniLibs：16:36 那次「重建 .so」只把产物编进 target/，jniLibs/arm64-v8a 还是两天前的旧库。
#   ③ jniLibs ↔ APK：Gradle 的 merged_native_libs / merged_jni_libs 会吃缓存，assembleRelease 打过旧 .so。
#   ④ 绑定 ↔ 老 .so：旧 .so 里没有新函数的 checksum 符号（符号存在性检查能抓到这一种）。
#
# 用法：pwsh core/crates/audiolink-ffi/bindings/check-so-symbols.ps1 [-SkipSourceSync] [-OnlySourceSync]
#   -SkipSourceSync：跳过第 ① 环（省一次宿主 dll 编译）。
#   -OnlySourceSync：只跑第 ① 环就退出 —— 打包脚本（tools/package-android.ps1）拿它当「绑定量校验」前置。
# 退出码非 0 = 有环节不同源，先修再装机。

param(
    [switch]$SkipSourceSync,
    # 只跑第 ① 环就退出：给打包脚本当「绑定量校验」用 —— 那时 jniLibs / APK 还是上一轮的产物，
    # 跑 ②③④ 只会得到误导性的红。
    [switch]$OnlySourceSync
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.IO.Compression.FileSystem

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $here)))
$kt = Join-Path $here "kotlin\com\gotkicry\audiolink\core\audiolink_ffi.kt"
$jniLibs = Join-Path $repo "android\app\src\main\jniLibs"
$apkRoot = Join-Path $repo "android\app\build\outputs\apk"

if (-not (Test-Path $kt)) { throw "找不到生成的绑定：$kt（先按 README 重新生成）" }

$failed = @()

# ---- ① 绑定是否与**当前源码**同步：用当前源码编库 → 生成到临时目录 → 与入库的 .kt 逐字节比对 ----
Write-Host "── ① 绑定 ↔ 当前源码 ──"
if ($SkipSourceSync) {
    Write-Host "  已跳过（-SkipSourceSync）" -ForegroundColor DarkGray
} else {
    Push-Location $repo
    try {
        & cargo build -p audiolink-ffi --features bindgen 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "cargo build --features bindgen 失败（exit $LASTEXITCODE）" }
        $tmp = Join-Path $env:TEMP ("uniffi-sync-" + [guid]::NewGuid().ToString("N"))
        & cargo run -q -p audiolink-ffi --features bindgen --bin uniffi-bindgen -- generate `
            --library target/x86_64-pc-windows-msvc/debug/audiolink_ffi.dll --language kotlin `
            --out-dir $tmp --config core/crates/audiolink-ffi/uniffi.toml --no-format 2>&1 | Out-Null
        $fresh = Join-Path $tmp "com\gotkicry\audiolink\core\audiolink_ffi.kt"
        if (-not (Test-Path $fresh)) { throw "生成物没出现在临时目录：$fresh" }
        $a = (Get-FileHash $kt).Hash
        $b = (Get-FileHash $fresh).Hash
        if ($a -ne $b) {
            Write-Host "  入库的绑定与当前源码**不同源**（请重新生成：见 README「重新生成」）" -ForegroundColor Red
            $failed += 'bindings 落后于源码'
        } else {
            Write-Host "  入库的绑定与当前源码逐字节一致  OK" -ForegroundColor Green
        }
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    } finally { Pop-Location }
}

if ($OnlySourceSync) {
    if ($failed.Count -gt 0) {
        Write-Host "绑定与源码不同源：先按 README 重新生成，再重跑。" -ForegroundColor Red
        exit 1
    }
    Write-Host "第 ① 环同源（-OnlySourceSync）。" -ForegroundColor Green
    exit 0
}

$symbols = Select-String -Path $kt -Pattern "(uniffi_audiolink_ffi_checksum_[a-z_]+)\(" |
    ForEach-Object { $_.Matches[0].Groups[1].Value } | Sort-Object -Unique
Write-Host ""
Write-Host "绑定声明 $($symbols.Count) 个 checksum 符号"

function Test-SoBytes([byte[]]$bytes) {
    $text = [System.Text.Encoding]::ASCII.GetString($bytes)
    return @($symbols | Where-Object { -not $text.Contains($_) })
}

function Report-So([string]$label, [byte[]]$bytes, [string]$stamp) {
    $miss = Test-SoBytes $bytes
    if ($miss.Count -eq 0) {
        Write-Host ("{0,-42} {1}/{2}  OK   {3}" -f $label, $symbols.Count, $symbols.Count, $stamp) -ForegroundColor Green
    } else {
        Write-Host ("{0,-42} {1}/{2}  缺: {3}   {4}" -f $label, ($symbols.Count - $miss.Count), $symbols.Count, ($miss -join ","), $stamp) -ForegroundColor Red
        $script:failed += $label
    }
}

Write-Host ""
Write-Host "── ② jniLibs（会被打进 APK 的那一份）──"
if (Test-Path $jniLibs) {
    foreach ($dir in Get-ChildItem $jniLibs -Directory | Sort-Object Name) {
        $so = Join-Path $dir.FullName "libaudiolink_ffi.so"
        if (-not (Test-Path $so)) { Write-Host ("{0,-42} 缺 libaudiolink_ffi.so" -f $dir.Name) -ForegroundColor Yellow; $failed += $dir.Name; continue }
        Report-So ("jniLibs/" + $dir.Name) ([System.IO.File]::ReadAllBytes($so)) (Get-Item $so).LastWriteTime.ToString("MM-dd HH:mm:ss")
    }
} else { Write-Host '  jniLibs 目录不存在（先跑 android/scripts/build-rust.ps1）' -ForegroundColor Yellow; $failed += 'jniLibs(缺失)' }

Write-Host ""
Write-Host "── ③ target 里的 Android release 产物（「编了但没进 jniLibs」就死在这一步）──"
foreach ($pair in @(@('aarch64-linux-android','arm64-v8a'), @('armv7-linux-androideabi','armeabi-v7a'))) {
    $so = Join-Path $repo ("target\" + $pair[0] + "\release\libaudiolink_ffi.so")
    if (Test-Path $so) { Report-So ("target/" + $pair[1]) ([System.IO.File]::ReadAllBytes($so)) (Get-Item $so).LastWriteTime.ToString("MM-dd HH:mm:ss") }
    else { Write-Host ("{0,-42} 没有 release 产物" -f ("target/" + $pair[1])) -ForegroundColor Yellow }
}

Write-Host ""
Write-Host "── ④ 最新的已打包 APK（设备上装的就是它；同一 ABI 只看最新那份）──"
if (Test-Path $apkRoot) {
    $seen = @{}
    foreach ($apk in Get-ChildItem $apkRoot -Recurse -Filter *.apk | Sort-Object LastWriteTime -Descending) {
        $zip = [System.IO.Compression.ZipFile]::OpenRead($apk.FullName)
        try {
            foreach ($e in @($zip.Entries | Where-Object { $_.FullName -like "lib/*/libaudiolink_ffi.so" })) {
                $parts = $e.FullName.Split("/")
                if ($parts.Length -lt 3) { continue }
                $abi = $parts[1]
                if ($seen.ContainsKey($abi)) { continue }
                $seen[$abi] = $apk.Name
                $ms = New-Object System.IO.MemoryStream
                $s = $e.Open(); $s.CopyTo($ms); $s.Close()
                Report-So ($apk.Name + " [" + $abi + "]") $ms.ToArray() $apk.LastWriteTime.ToString("MM-dd HH:mm:ss")
                $ms.Dispose()
            }
        } finally { $zip.Dispose() }
    }
    if ($seen.Count -eq 0) { Write-Host '  outputs 下没有带 .so 的 APK' -ForegroundColor DarkGray }
} else { Write-Host '  还没有打过 APK（正常）' -ForegroundColor DarkGray }

if ($failed.Count -gt 0) {
    Write-Host ""
    Write-Host "以下环节不同源：$($failed -join ", ")" -ForegroundColor Red
    Write-Host "标准动作：① 重新生成绑定（README 的两条命令）② pwsh android/scripts/build-rust.ps1" -ForegroundColor Yellow
    Write-Host "          ③ 清 Gradle native 缓存后重打 APK（:app:clean 或删 build/intermediates/merged_*native*）④ 重装" -ForegroundColor Yellow
    exit 1
}
Write-Host ""
Write-Host "全部环节同源。" -ForegroundColor Green
