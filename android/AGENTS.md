# AGENTS.md — android(Android 端)

Compose UI + 前台服务承载收发,Rust 内核经 UniFFI(JNA)接入。单模块 `:app`,包名 `com.gotkicry.audiolink`。仓库级约定见根 `AGENTS.md`。

## 布局地图

| 路径 | 是什么 |
|---|---|
| `app/src/main/kotlin/com/gotkicry/audiolink/AudioLinkApp.kt` | Application(刻意不承载业务状态)→ `MainActivity.kt` |
| `…/service/AudioLinkService.kt` | 核心前台服务(FGS: mediaPlayback\|connectedDevice\|microphone\|mediaProjection) |
| `…/audio/` | 播放:`LowLatencyPlayer`(AudioTrack 低延迟)、`PlayoutLoop`、`PcmRingBuffer`、`FfiPcmFeed` |
| `…/capture/` | 采集:麦克风 / `SystemLoopbackCaptureSource`(MediaProjection)/ 状态机 |
| `…/service/` | `EngineLifecycle`、`LanDiscovery`、UI 状态映射(`PeerUiState` 等) |
| `…/ui/` | `screens/`(Console/Device/Sender/Receiver/Settings/Update/Licenses)、`components/`、`theme/`、`i18n/` |
| `…/update/` | 应用内更新(GitHub Releases,FileProvider 安装) |
| `app/src/main/jniLibs/{arm64-v8a,armeabi-v7a}/` | `libaudiolink_ffi.so`(JNA 加载) |
| `app/src/test/` | 30 个纯 JVM 单测(JUnit4) |
| `app/build.gradle.kts` | SDK/ABI/签名/版本;UniFFI 绑定挂为 source set |
| `core/crates/audiolink-ffi/bindings/` | Kotlin 绑定(入库交付物)+ README(再生成纪律) |
| `android/scripts/build-rust.ps1` | Rust .so 交叉编译脚本 |
| `docs/61-android-self-update.md` | 应用内更新设计 |

## 命令(仓库根执行)

```powershell
pwsh android/scripts/build-rust.ps1 [-Abi arm64-v8a] [-Debug]  # 交叉编译 .so,默认 release 双 ABI
pwsh tools/gradlew.ps1 assembleDebug      # 务必走包装(自动 JDK 17+/ANDROID_HOME)
pwsh tools/gradlew.ps1 testDebugUnitTest  # JVM 单测
pwsh tools/gradlew.ps1 lint
pwsh tools/check-version.ps1              # versionName 与根 Cargo.toml 一致
pwsh core/crates/audiolink-ffi/bindings/check-so-symbols.ps1  # FFI 四环自检
```

## 硬约束与坑

- **改完 Rust 必须重新生成 UniFFI 绑定**(checksum 连注释变化都变);绑定由 Rust 侧 `kotlin_guard.rs` 测试守护,再生成方法见 `bindings/README.md`。
- 进 APK 的 .so 必须 **release profile**;打 APK 前防 `merged_native_libs/` 缓存旧 .so;`-Debug` 产物 ~94MB 不可发布。
- `targetSdk` 锁 **36**(ADR-009),`minSdk` 26;ABI 仅 arm64-v8a + armeabi-v7a,分 ABI 出包(FR-40)。
- JNA 必须 `jna:5.19.1@aar`(只有它带 `libjnidispatch.so`);不引 okhttp/ExoPlayer。
- R8 必须 keep JNA 与 `com.gotkicry.audiolink.core.**`(否则 release 启动 UnsatisfiedLinkError)。
- 签名:`keystore.properties` 不入库;debug/release 共用同一签名与包名;Manifest 不声明 usesCleartextTraffic、不开机自启。
- JDK 路径禁写进 `android/gradle.properties`;`sdk.dir` 走不入库的 `local.properties`;Gradle 9 无任务级 `exec {}`,用注入的 `ExecOperations`。

## 风格

- `kotlin.code.style=official`;无 ktlint/detekt,静态检查靠 Android Lint + cargo 门禁。
- 中文注释记「为什么」与踩坑日期;单点声明(ABI 只在 splits.abi、签名一次、版本号单一来源)。
- Android 改动门禁 = 根四条 cargo 门禁 + `build-rust.ps1` + `assembleDebug`。
