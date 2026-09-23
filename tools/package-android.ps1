#!/usr/bin/env pwsh
# AudioLink —— Android 发布包一条命令（含四环同源验收）
#
# 为什么需要它：这条链路上有四个**各自独立**的「同源」环节，任何一个漏掉都不会在构建期报错，
# 只会在真机上表现为「服务起不来」或「行为还是上一版」：
#   ① 绑定 ↔ core 源码     改了 Rust 就得重新生成 Kotlin 绑定（UniFFI 的 checksum 连文档注释都算）
#   ② jniLibs ↔ 本次源码   Gradle 不会替你编 Rust —— 现在由 buildRustCore 挂在 preBuild 上代劳
#   ③ APK 内嵌 .so         merged_native_libs 缓存会把旧库原样送进包（build-rust.ps1 编完顺手清）
#   ④ 版本号三处同源       Cargo.toml / tauri.conf.json / app/build.gradle.kts
#
# 顺序是有讲究的：绑定校验必须在编 .so **之前**（不同源时先修绑定，免得白编两轮）；
# 四环自检必须在打完包**之后**（它要读 APK 里那份 .so）。
#
# 用法：
#   pwsh tools/package-android.ps1                  # 双 ABI release APK + 四环自检 + 收产物到 out/
#   pwsh tools/package-android.ps1 -Install         # 额外 adb 覆盖安装到当前设备（按设备 ABI 选包）
#   pwsh tools/package-android.ps1 -Abi arm64-v8a   # 只出一个 ABI（快速迭代；发布必须双 ABI，FR-40）
param(
    # 打包 ABI（默认 FR-40 契约的双 ABI）。
    [string]$Abi = "arm64-v8a,armeabi-v7a",
    # 打完包后 adb 覆盖安装（只装与设备 ABI 匹配的那份）。
    [switch]$Install,
    # 产物收拢目录（相对仓库根）。
    [string]$OutDir = "out",
    # Gradle launcher JVM（必须 17+）。不传则自动探测。
    [string]$JavaHome
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

# Gradle 9 的 launcher JVM 必须 17+，而本机 JAVA_HOME 常年指向 JDK 11（tools/gradlew.ps1 会拦下来）。
# 与其让人每次手写 -JavaHome，这里按「显式参数 → JAVA_HOME → scoop corretto17 → Studio JBR → ~/.jdks」
# 的顺序找一个能用的；找不到就明确报错，而不是让 Gradle 抛一个看不懂的栈。
function Resolve-JavaHome([string]$explicit) {
    $candidates = @()
    if ($explicit) { $candidates += $explicit }
    if ($env:JAVA_HOME) { $candidates += $env:JAVA_HOME }
    $candidates += @(
        (Join-Path $env:USERPROFILE "scoop\apps\corretto17-jdk\current"),
        "C:\Program Files\Android\Android Studio\jbr"
    )
    $candidates += @(Get-ChildItem "$env:USERPROFILE/.jdks" -Directory -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName })
    foreach ($c in $candidates) {
        if (-not $c) { continue }
        $java = Join-Path $c "bin/java.exe"
        if (-not (Test-Path $java)) { continue }
        $verLine = "$(& $java -version 2>&1 | Select-Object -First 1)"
        $major = 0
        $seg = $verLine.Split([char]34)
        if ($seg.Count -ge 2 -and $seg[1] -match "^([0-9]+)") { $major = [int]$Matches[1] }
        if ($major -ge 17) { return $c }
    }
    return $null
}

Push-Location $root
try {
    $jh = Resolve-JavaHome $JavaHome
    if (-not $jh) {
        throw "找不到 JDK 17+：用 -JavaHome 指定路径，或设置 JAVA_HOME（Gradle 9 的 launcher JVM 必须 17+）"
    }
    Write-Host "JAVA_HOME    = $jh" -ForegroundColor DarkGray

    Write-Host "==> ① 版本号三处同源" -ForegroundColor Cyan
    & (Join-Path $root "tools/check-version.ps1") -Quiet

    Write-Host "==> ② 绑定 ↔ core 源码" -ForegroundColor Cyan
    & (Join-Path $root "core/crates/audiolink-ffi/bindings/check-so-symbols.ps1") -OnlySourceSync
    if ($LASTEXITCODE -ne 0) {
        throw "Kotlin 绑定落后于 core 源码：先按 core/crates/audiolink-ffi/bindings/README.md 重新生成，再重跑本脚本"
    }

    Write-Host "==> ③ assembleRelease（.so 由 buildRustCore 在 preBuild 阶段自动交叉编译并清缓存）" -ForegroundColor Cyan
    & (Join-Path $root "tools/gradlew.ps1") -JavaHome $jh -GradleArgs @("assembleRelease", "-PrustAbi=$Abi")
    if ($LASTEXITCODE -ne 0) { throw "assembleRelease 失败（exit $LASTEXITCODE）" }

    Write-Host "==> ④ 四环同源自检（含 APK 内嵌 .so 的 checksum 符号）" -ForegroundColor Cyan
    & (Join-Path $root "core/crates/audiolink-ffi/bindings/check-so-symbols.ps1")
    if ($LASTEXITCODE -ne 0) { throw "四环自检未过：见上面的红色行（标准动作 = 重新生成绑定 → 重编 .so → 重打包）" }

    Write-Host "==> ⑤ 收拢产物" -ForegroundColor Cyan
    $apkDir = Join-Path $root "android/app/build/outputs/apk/release"
    $apks = @(Get-ChildItem $apkDir -Filter "*.apk" -ErrorAction SilentlyContinue | Where-Object { $_.Name -notlike "*unaligned*" })
    if ($apks.Count -eq 0) { throw "没找到 release APK：检查 $apkDir" }

    $out = Join-Path $root $OutDir
    New-Item -ItemType Directory -Force -Path $out | Out-Null
    $sums = @()
    foreach ($apk in $apks) {
        Copy-Item $apk.FullName $out -Force
        $hash = (Get-FileHash $apk.FullName -Algorithm SHA256).Hash.ToLower()
        Write-Host ("    {0}  {1:N1} MB  SHA256={2}" -f $apk.Name, ($apk.Length / 1MB), $hash) -ForegroundColor Green
        $sums += ("{0}  {1}" -f $hash, $apk.Name)
    }
    $sums | Set-Content -Path (Join-Path $out "SHA256SUMS.txt") -Encoding utf8NoBOM
    Write-Host "产物目录：$out" -ForegroundColor Green

    if ($Install) {
        Write-Host "==> ⑥ adb 覆盖安装" -ForegroundColor Cyan
        $adb = (Get-Command adb -ErrorAction SilentlyContinue).Source
        if (-not $adb -and $env:ANDROID_HOME) {
            $cand = Join-Path $env:ANDROID_HOME "platform-tools/adb.exe"
            if (Test-Path $cand) { $adb = $cand }
        }
        if (-not $adb) { throw "找不到 adb：把 platform-tools 加进 PATH，或去掉 -Install" }
        $devAbi = "$(& $adb shell getprop ro.product.cpu.abi)".Trim()
        if (-not $devAbi) { throw "adb 看不到设备：先 adb devices 确认" }
        $target = @($apks | Where-Object { $_.Name -like "*$devAbi*" })[0]
        if (-not $target) { throw "本次只打了 $Abi，没有匹配设备 ABI（$devAbi）的包" }
        & $adb install -r $target.FullName
        if ($LASTEXITCODE -ne 0) { throw "adb install 失败（exit $LASTEXITCODE）" }
        Write-Host "已覆盖安装：$($target.Name)" -ForegroundColor Green
    }

    Write-Host ""
    Write-Host "全部完成。" -ForegroundColor Green
}
finally { Pop-Location }
