// AudioLink Android —— app 模块
import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

// 发布签名：从 android/keystore.properties 读取（该文件不入库，CI 由 Secret 生成）
val keystorePropsFile = rootProject.file("keystore.properties")
val keystoreProps = Properties().apply {
    if (keystorePropsFile.exists()) keystorePropsFile.inputStream().use { load(it) }
}

android {
    namespace = "com.gotkicry.audiolink"
    compileSdk = 37

    defaultConfig {
        applicationId = "com.gotkicry.audiolink"
        minSdk = 26 // API 26 = AudioTrack.PERFORMANCE_MODE_LOW_LATENCY 引入版本（低延迟的硬门槛）
        // ADR-009：必须停在 36。目标 SDK 37 会强制 ACCESS_LOCAL_NETWORK 运行时权限，
        // 未授权时局域网 UDP/TCP/组播全部失败（EPERM），且后台音频写入会被静默吞掉。
        targetSdk = 36

        versionName = "0.1.1" // CI 校验：必须与 Cargo.toml workspace version 一致
        // versionCode 从 versionName 派生（major*10000 + minor*100 + patch）：
        //  * 覆盖安装的硬要求是 versionCode **递增** —— 写死 1 会让「应用内更新」永远装不上去
        //    （同版本或降级，系统直接拒绝）；
        //  * 派生而不手写第二个数字：版本只有**一个**真相（check-version.ps1 校验三处里就有
        //    versionName 这处），不会出现「名字升了、code 忘了」。
        //  实际取值：0.1.0 → 100，0.1.1 → 101，0.2.0 → 200，1.0.0 → 10000。
        //  patch 到 99 为止（真实项目不会有 100 个补丁版）；到 100 会进位到 minor，可接受。
        // `versionName` 在这个 DSL 里是 String?（AGP 的属性类型）——`orEmpty()` 而不是 `!!`：
        // 万一将来有人删了上面那行，这里退化成 0（能被 CI 的 check-version.ps1 当场抓住），
        // 而不是让构建脚本在配置阶段崩掉。
        versionCode = versionName.orEmpty()
            .split(".")
            .let { parts ->
                val major = parts.getOrNull(0)?.toIntOrNull() ?: 0
                val minor = parts.getOrNull(1)?.toIntOrNull() ?: 0
                val patch = parts.getOrNull(2)?.toIntOrNull() ?: 0
                major * 10000 + minor * 100 + patch
            }

        // ABI 集合**只在一处声明**：下面的 `splits.abi.include`。
        // 不在这里再写 `ndk { abiFilters }`，是因为 AGP 直接拒绝二者并存：
        //   Conflicting configuration : ndk abiFilters cannot be present when splits abi filters are set
        // 两处各写一份的下场就是「改了一处、另一处悄悄还在」，所以留单点。

        // 由 android/scripts/build-rust.ps1 生成的 Rust 内核（UniFFI 绑定 + .so）
        // 产物路径：app/src/main/jniLibs/<abi>/libaudiolink_core.so
    }

    /**
     * 分 ABI 打包：`assembleRelease` 产出两个 APK，每个只带本 ABI 的本地库（路线图 M5：APK×2 ABI）。
     *
     * 支持的 ABI 是 FR-40 的契约（arm64-v8a + armeabi-v7a），也是**本文件的唯一声明处**：
     * 32 位 ARM 在 minSdk 26 的存量里仍占一部分，不要凭「现代设备都是 arm64」擅自收窄。
     *
     * 不发 universal 单包的理由：本项目 .so 是 3.9 MB 量级，JNA 的 libjnidispatch 也按 ABI 各一份 ——
     * 合包等于让每台设备多下载一份永远用不到的本地库。`isUniversalApk = true` 只在
     * 「分发渠道强制单包」时才有价值，本项目没有这个约束；真要临时要单包，
     * 把它改成 true 再 `assembleRelease` 即可。
     */
    splits {
        abi {
            isEnable = true
            reset()
            include("arm64-v8a", "armeabi-v7a")
            isUniversalApk = false
        }
    }

    // 发布签名只声明一次，release 与 debug 共用（keystore.properties 不入库，CI 由 Secret 生成）：
    // debug 只是「方便调试」的构建类型，不是第二个应用 —— 包名与签名都与 release 一致，
    // 避免「共存双包」带来的数据分裂（SharedPreferences 不互通）与覆盖安装被拒（签名不匹配）。
    val releaseSigning = if (keystorePropsFile.exists()) {
        signingConfigs.create("release").apply {
            storeFile = file(keystoreProps.getProperty("storeFile"))
            storePassword = keystoreProps.getProperty("storePassword")
            keyAlias = keystoreProps.getProperty("keyAlias")
            keyPassword = keystoreProps.getProperty("keyPassword")
        }
    } else {
        // 没有发布密钥时：release 产出未签名包（由分发流程另行签名），
        // debug 回退到 AGP 默认调试签名 —— 保持「没有密钥也能本地构建」。
        null
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
            if (releaseSigning != null) {
                signingConfig = releaseSigning
            }
        }
        debug {
            if (releaseSigning != null) {
                signingConfig = releaseSigning
            }
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    /**
     * Rust 内核的 UniFFI Kotlin 绑定：**不复制**，直接把生成物目录挂进源集。
     *
     * 生成物归 `core/crates/audiolink-ffi/bindings/kotlin`（ffi 流所有，重新生成见其 README）。
     * 为什么不 copy 一份到 android/：绑定必须与 `libaudiolink_ffi.so` **同版本** ——
     * 复制会让「内核更新了、APK 还在用旧绑定」变成只在运行期才炸的静默故障，单一来源更安全。
     */
    sourceSets["main"].kotlin.srcDir(
        rootProject.file("../core/crates/audiolink-ffi/bindings/kotlin"),
    )

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        resources.excludes += setOf("/META-INF/{AL2.0,LGPL2.1}")
        jniLibs.useLegacyPackaging = false
    }

    lint {
        // 旧版最大教训之一：空 catch 与吞异常遍地。这里把关键规则设为致命。
        warningsAsErrors = false
        abortOnError = true
        disable += setOf("MissingTranslation") // 中英双语初期允许缺项（CI 另跑一致性脚本）
    }
}

dependencies {
    // Compose 全家桶由 BOM 统一版本（不要给 compose.* 单独写版本号）
    val composeBom = platform("androidx.compose:compose-bom:2026.09.00")
    implementation(composeBom)
    androidTestImplementation(composeBom)

    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")

    // 以下不属于 Compose BOM 管辖，必须显式给版本号（漏写会得到 "Could not find xxx:." 这类空版本错误）
    // 版本查询日期：2026-09-11（源码：dl.google.com / repo1.maven.org 的 maven-metadata.xml）
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.11.0")
    implementation("androidx.lifecycle:lifecycle-service:2.11.0")
    implementation("androidx.datastore:datastore-preferences:1.2.1")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.11.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.11.0")

    // UniFFI 生成的绑定用 JNA 加载 libaudiolink_ffi.so。Android 上**必须**用 @aar 分类器：
    // 只有它带 libjnidispatch.so（plain jar 是 JVM 版，真机上会 UnsatisfiedLinkError）。
    // 版本查询日期：2026-09-14（来源 repo1.maven.org 的 net.java.dev.jna:jna maven-metadata.xml，release=5.19.1）
    implementation("net.java.dev.jna:jna:5.19.1@aar")

    // 注意：不引入 okhttp / ExoPlayer / AndroidAsync —— 旧版的整块 HTTP 管理面已被移除
    debugImplementation("androidx.compose.ui:ui-tooling")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
}

// ---- Rust 内核交叉编译：一键编到 app/src/main/jniLibs（避免忘记同步 .so）----------------
//
// **已接进构建图**（见文件末尾）：`assembleDebug/Release` 与 Android Studio 的 Run 都会先跑它。
// 「只注册任务、不挂接」等于把「.so 同步了吗」重新交回给人的记性 —— 本任务此前在仓库里零引用
// （只有 register，没有任何 dependsOn），这正是「core 改了、APK 里还是旧内核」的根因。
//
// 为什么用内置的 `Exec` 任务类型，而不是在脚本里自己写一个 abstract class：
// Kotlin DSL 脚本里声明的类会成为脚本类 `Build_gradle` 的**内部类**，Gradle 实例化时直接报
// 「Class Build_gradle.RustBuildTask is a non-static inner class」（实测）。旧版之所以没暴露，
// 是因为那个任务从来没人 dependsOn —— 任务注册是惰性的，直到真要执行才实例化。
// 内置 Exec 配置缓存友好，也不需要注入 ExecOperations。
//
// 为什么**不**声明 `@OutputDirectory`：声明了 Gradle 就会按「输入（脚本路径 / ABI）没变」判定
// up-to-date 并**跳过执行**，于是 Rust 源码改了也不重编 —— 比现在还糟。不声明输出 = 每次执行，
// 增量判断交给 cargo 自己（它才是唯一知道源码变没变的人）。
//
// 两个 Gradle property 开关：
//   * `-PskipRust=true`：不接进构建图（CI 用：android job 已显式 `cargo ndk` 编过 .so）；
//   * `-PrustAbi=arm64-v8a`：只编一个 ABI（Studio 日常 Run 只装自己手机时省一半时间）。
val rustAbi = providers.gradleProperty("rustAbi").getOrElse("arm64-v8a,armeabi-v7a")

tasks.register<Exec>("buildRustCore") {
    group = "audiolink"
    description = "用 cargo-ndk 交叉编译 Rust 内核到 app/src/main/jniLibs（已挂 preBuild，随构建自动执行）"
    commandLine(
        "pwsh", "-NoProfile", "-File",
        rootProject.layout.projectDirectory.file("scripts/build-rust.ps1").asFile.absolutePath,
        "-Abi", rustAbi,
    )
    // 编完顺手清 AGP 的 native 中间产物（merged_native_libs / merged_jni_libs 会把上一轮的 .so
    // 原样送进 APK）——这件事由 scripts/build-rust.ps1 自己收尾，不放在这里做：
    // Kotlin DSL 的 doLast 闭包会捕获脚本实例，配置缓存直接报「cannot serialize object of type DefaultProject」。
}

// ---- 接进构建图（这一步才是「core 改了、包装的还是旧内核」的解法）------------------
// 挂 AGP 的 preBuild：两个变体共用它，Android Studio 的 Run / Make Project 走的也是这条图。
// 为什么不靠 build-rust.ps1「自己保证」：Studio 只执行 Gradle 任务图，不会去跑仓库里的 .ps1。
val skipRustSync = providers.gradleProperty("skipRust").orNull?.toBoolean() ?: false
if (skipRustSync) {
    logger.lifecycle("buildRustCore: NOT wired into the build graph (-PskipRust=true) - .so sync is the caller's responsibility.")
} else {
    tasks.matching { it.name == "preBuild" }.configureEach { dependsOn("buildRustCore") }
    // 静默失效比报错更坏：AGP 若改了任务名，这里当场红，而不是「以为同步了、其实没同步」。
    afterEvaluate {
        check(tasks.names.contains("preBuild")) {
            "AGP preBuild task not found, so buildRustCore wiring is broken (AGP task graph changed). " +
                "Check the AGP version in android/build.gradle.kts, or pass -PskipRust=true to bypass."
        }
    }
}
