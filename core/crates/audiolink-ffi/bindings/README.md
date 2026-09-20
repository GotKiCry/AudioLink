# Kotlin 绑定（UniFFI 生成物）

本目录是 **`audiolink-ffi` 的 Kotlin 绑定的落地位置**，由 `uniffi-bindgen` 从 cdylib 里嵌的
元数据生成（proc-macro 模式，没有 UDL 文件）。

> **本 crate 的写域只到 `core/crates/audiolink-ffi/**`**。`android/**` 归 `android-playback`，
> 所以这里是「交付物 + 说明书」，不是「已接线」。谁来接、怎么接见下。

## 重新生成

```powershell
# 1) 产出宿主平台的 cdylib（`.cargo/config.toml` 把默认目标钉成了 x86_64-pc-windows-msvc）
cargo build -p audiolink-ffi --features bindgen

# 2) 从库里的元数据生成 Kotlin（包名 / 库名见 ../uniffi.toml）
cargo run -p audiolink-ffi --features bindgen --bin uniffi-bindgen -- generate `
  --library target/x86_64-pc-windows-msvc/debug/audiolink_ffi.dll `
  --language kotlin `
  --out-dir core/crates/audiolink-ffi/bindings/kotlin `
  --config core/crates/audiolink-ffi/uniffi.toml
```

`bindgen` 是 crate 的 feature（不是默认开），所以**不会**把 `uniffi_bindgen` / `clap` 带进
Android 产物。

> **改完 Rust 就重新生成绑定 —— 不要凭「我没动导出面」判断**（2026-09-20 的真实事故，见下方「上线前必跑」）：
>
> 1. **checksum 不只看导出面清单**：签名 / **文档注释** / 类型定义一变它就变。实测：只是想消掉一条 clippy 的文档
>    缩进告警、给 `set_local_peer_gain` 的 KDoc 加了一个空行，它的 checksum 就从 **27074** 变成 **23920**
>    （其余 15 个函数的值一字未动）。那份没同步的绑定配新 `.so`，设备侧就是 `engineStart` 抛
>    `UniFFI API checksum mismatch`、界面显示「服务没能启动」。**`findstr` 查符号抓不到这种漂移**
>    （符号名没变、符号也都在），只有「重新生成一次看有没有 diff」抓得到。
> 2. **导出面变化的实测**：新增 `stopSendTo` / `setPeerGain` 时 `engine_start` 从 **32272** → **16758**；
>    新增 `disconnectPeer` / `setLocalPeerGain` / `localPeerGain` 后，三个新符号分别是 **34397 / 23920 / 35079**。
> 3. 所以判断「同源」**不能靠肉眼比对数字**，只能靠**动作**：改完 Rust → 重新生成 → 重建 `.so` → 跑自检脚本。

## 落地位置（给 android 侧的建议，不是要求）

| 生成物 | 建议去处 |
|---|---|
| `kotlin/com/gotkicry/audiolink/core/*.kt` | `android/app/src/main/kotlin/com/gotkicry/audiolink/core/`（或把它挂成一个额外 source set） |
| `libaudiolink_ffi.so`（`cargo ndk -o android/app/src/main/jniLibs`） | `android/app/src/main/jniLibs/<abi>/libaudiolink_ffi.so` |

依赖：生成的绑定用 **JNA** 加载动态库（`net.java.dev.jna:jna:<ver>@aar`，Android 上必须 `@aar`：
只有它带 `libjnidispatch.so`），异步函数用 **kotlinx-coroutines**（`suspend`）。

**进 APK / 上真机必须用 `-Release`**：`build-rust.ps1` 默认是 `dev` profile，产出的 `.so` 带 debuginfo
（本机实测 **94.0 MB / 74.6 MB**），release 后是 **3.70 MB / 2.50 MB**。

## 上线前必跑：四环同源自检（2026-09-20 事故的护栏）

```powershell
pwsh core/crates/audiolink-ffi/bindings/check-so-symbols.ps1
```

四环逐层核对，任一环不同源就 exit 1（第 ① 环会让它跑约 20 秒编译，赶时间可以 `-SkipSourceSync`）：

| 环 | 查什么 | 抓得到哪一种故障 |
|---|---|---|
| ① 绑定 ↔ 当前源码 | 用当前源码编库 → 重新生成到临时目录 → 与入库的 `.kt` 逐字节比对 | **只改文档注释导致的 checksum 漂移**（符号都在，`findstr` 抓不到） |
| ② jniLibs | 每个 ABI 的 `.so` 是否含绑定声明的全部 checksum 符号 | 「重建只进了 `target/`，`jniLibs` 还是旧的」 |
| ③ target release 产物 | 同上 | 「编了但没拷进 `jniLibs`」 |
| ④ 最新的 APK 内嵌 `.so` | 同上（每个 ABI 只看最新那份） | 「Gradle 吃缓存，APK 里装的是旧 `.so`」 |

### 两条纪律（这次踩的坑，值得永久记下）

1. **改完 Rust 一律重新生成绑定**，不管「有没有动导出面」—— checksum 也随**文档注释**变化（实测见文件头那段记录）。
   别用「我这次只改了实现」来判断。
2. **Android 侧打包必须清掉 Gradle 的 native 缓存**：`android/app/build/intermediates/merged_native_libs/` 与
   `merged_jni_libs/` 会把上一轮的 `.so` 原样送进 APK（实测：09-18 的旧库就这样被装到手机上演过一轮）。
   判别方法：**解包 APK、对里面的 `.so` 查新符号** —— 上面脚本的第 ④ 环替你做了；手搓版是
   `unzip app-arm64-v8a-release.apk lib/arm64-v8a/libaudiolink_ffi.so` 之后 `findstr /C:"uniffi_audiolink_ffi_checksum_func_disconnect_peer"` 那个 `.so`。

四个「一致」是四件事：**绑定与源码一致 ≠ `target/` 产物新 ≠ `jniLibs/` 新 ≠ 装进设备的 APK 新**；
再往下一层是「设备上装的 APK」—— 装完也值得跑一次第 ④ 环（脚本读的是仓库里最新的 APK）。

## 验证生成物（两条都跑过，别只靠 review）

### 1) Rust 侧的机械断言（`cargo test` 自动跑）

`../src/kotlin_guard.rs` 有三条：

| 用例 | 盯什么 |
|---|---|
| `源码文档注释不含_kotlin_危险序列` | rustdoc 里不许出现「斜杠紧跟星号 / 星号紧跟斜杠」—— 那段文本会被原样搬进 KDoc，而 **Kotlin 块注释可以嵌套**，一个路径通配符足以把整份生成物吞到文件尾（实测吞掉 1779–2343 行） |
| `生成物块注释闭合` | 按 Kotlin 词法（嵌套块注释 / 字符串 / 字符字面量）走一遍本目录的 `.kt`，必须以「正常状态」收尾 |
| `生成物包含契约_s8_的导出面` | §8 的导出函数（含 `stopSendTo` / `setPeerGain`）+ 两个回调接口 + `FfiException` + `messageText` 真的还在文件里 |

第二条同时兜住「源码改了却忘了重新生成绑定」：`.kt` 是**入库交付物**，它坏了必须在 `cargo test` 就红。

### 2) 真 Kotlin 编译器（本机没有 `kotlinc` 时，用 Gradle 缓存里的编译器 jar）

```powershell
# jar 从 ~/.gradle/caches/modules-2/files-2.1 下取；版本对齐 android/build.gradle.kts（Kotlin 2.4.20）
$compiler    = "...\org.jetbrains.kotlin\kotlin-compiler-embeddable\2.4.20\...jar"
$stdlib      = "...\org.jetbrains.kotlin\kotlin-stdlib\2.4.20\...jar"
$coroutines  = "...\org.jetbrains.kotlinx\kotlinx-coroutines-core-jvm\1.11.0\...jar"
$annotations = "...\org.jetbrains\annotations\24.0.1\...jar"        # 缺它 → 编译器内部 NoClassDefFoundError
$jnaClasses  = "<把 net.java.dev.jna:jna:5.19.1 的 aar 解压出来的 classes.jar>"

java -cp "$compiler;$stdlib;$coroutines;$annotations" `
  org.jetbrains.kotlin.cli.jvm.K2JVMCompiler -no-stdlib -no-reflect `
  -classpath "$jnaClasses;$coroutines;$stdlib" -d target\kt-probe\out <生成的 .kt>
```

两个坑：`coroutines` 和 `annotations` 必须同时出现在**编译器自己的** `-cp` 上（只放 target classpath 会得到
`NoClassDefFoundError: kotlinx/coroutines/CoroutineScope` / `org/jetbrains/annotations/NotNull`）。
本机实测：Kotlin 2.4.20 + JNA（JVM jar 与 aar 的 classes.jar 两种形态）→ 各 **119 个 class，零错误零警告**。

### 3) 发布前 30 秒自检：`.so` 里该有的符号真的在吗

UniFFI 的 **checksum 校验发生在第一次 FFI 调用**（`uniffi_audiolink_ffi_checksum_...`），
所以「符号在不在」要提前看得见，而不是等真机第一次 `feedPcm` 才暴露。
把生成物声明的每个 checksum 符号拿去 `.so` 里找 —— 全部命中才算同源：

```powershell
$kt = "core/crates/audiolink-ffi/bindings/kotlin/com/gotkicry/audiolink/core/audiolink_ffi.kt"
$symbols = Select-String -Path $kt -Pattern "(uniffi_audiolink_ffi_checksum_[a-z_]+)\(" |
  ForEach-Object { $_.Matches[0].Groups[1].Value } | Sort-Object -Unique
foreach ($abi in @("arm64-v8a", "armeabi-v7a")) {
  $so = "android\app\src\main\jniLibs\$abi\libaudiolink_ffi.so"
  $text = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($so))
  $miss = @($symbols | Where-Object { -not $text.Contains($_) })
  "{0,-14} {1}/{2}  缺失=[{3}]" -f $abi, ($symbols.Count - $miss.Count), $symbols.Count, ($miss -join ",")
}
```

本机实测（release 版 `.so`，2026/09/14 14:14:06 / 14:15:12）：

```text
arm64-v8a      13/13  缺失=[]
armeabi-v7a    13/13  缺失=[]
```

抽一个字段名一起搜（`message_text`）还能顺带确认「这份 `.so` 是改名之后编的」——
比对比时间戳可靠。

## 与内核的约定（改之前先读）

* 所有 PCM 都是 **48 000 Hz / f32 / 2ch 交错**（零重采样；内核会断言设备格式，不达标直接拒绝推流）；
* `PcmFeed.feedPcm` 会被**内核的 Rust 播放线程**（`audiolink-playout`，非 UI 线程）调用 →
  实现必须线程安全；
* 生成的 Kotlin 里 `Vec<f32>` 是 `List<Float>`（UniFFI 0.29 的映射，不是 `FloatArray`），
  装箱代价见 `../src/audio_bridge.rs` 顶部的说明；
* 删改本目录里的 `.kt` 没有意义 —— 下次生成会覆盖，要改的是 Rust 侧的 `#[uniffi::export]` 面。
