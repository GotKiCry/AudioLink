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

## 落地位置（给 android 侧的建议，不是要求）

| 生成物 | 建议去处 |
|---|---|
| `kotlin/com/gotkicry/audiolink/core/*.kt` | `android/app/src/main/kotlin/com/gotkicry/audiolink/core/`（或把它挂成一个额外 source set） |
| `libaudiolink_ffi.so`（`cargo ndk -o android/app/src/main/jniLibs`） | `android/app/src/main/jniLibs/<abi>/libaudiolink_ffi.so` |

依赖：生成的绑定用 **JNA** 加载动态库（`net.java.dev.jna:jna:<ver>@aar`），异步函数用
**kotlinx-coroutines**（`suspend`）。

## 验证生成物（两条都跑过，别只靠 review）

### 1) Rust 侧的机械断言（`cargo test` 自动跑）

`../src/kotlin_guard.rs` 有三条：

| 用例 | 盯什么 |
|---|---|
| `源码文档注释不含_kotlin_危险序列` | rustdoc 里不许出现「斜杠紧跟星号 / 星号紧跟斜杠」—— 那段文本会被原样搬进 KDoc，而 **Kotlin 块注释可以嵌套**，一个路径通配符足以把整份生成物吞到文件尾（实测吞掉 1779–2343 行） |
| `生成物块注释闭合` | 按 Kotlin 词法（嵌套块注释 / 字符串 / 字符字面量）走一遍本目录的 `.kt`，必须以「正常状态」收尾 |
| `生成物包含契约_s8_的导出面` | 十个导出函数 + 两个回调接口 + `FfiException` + `messageText` 真的还在文件里 |

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

## 与内核的约定（改之前先读）

* 所有 PCM 都是 **48 000 Hz / f32 / 2ch 交错**（零重采样；内核会断言设备格式，不达标直接拒绝推流）；
* `PcmFeed.feedPcm` 会被**内核的 Rust 播放线程**（`audiolink-playout`，非 UI 线程）调用 →
  实现必须线程安全；
* 生成的 Kotlin 里 `Vec<f32>` 是 `List<Float>`（UniFFI 0.29 的映射，不是 `FloatArray`），
  装箱代价见 `../src/audio_bridge.rs` 顶部的说明；
* 删改本目录里的 `.kt` 没有意义 —— 下次生成会覆盖，要改的是 Rust 侧的 `#[uniffi::export]` 面。
