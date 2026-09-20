// AudioLink Android —— app 模块
import java.util.Properties
import javax.inject.Inject
import org.gradle.process.ExecOperations

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

        versionName = "0.1.0" // CI 校验：必须与 Cargo.toml workspace version 一致
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

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
            if (keystorePropsFile.exists()) {
                signingConfig = signingConfigs.create("release").apply {
                    storeFile = file(keystoreProps.getProperty("storeFile"))
                    storePassword = keystoreProps.getProperty("storePassword")
                    keyAlias = keystoreProps.getProperty("keyAlias")
                    keyPassword = keystoreProps.getProperty("keyPassword")
                }
            }
        }
        debug {
            applicationIdSuffix = ".debug" // 允许与正式版共存于同一台设备
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

/**
 * 一键交叉编译 Rust 内核到 `app/src/main/jniLibs`（避免忘记同步 .so）。
 *
 * 注意：Gradle 9 已移除任务/项目级的 `exec {}` DSL（会报 Unresolved reference 'exec'）。
 * 官方推荐注入 `ExecOperations` —— 这样在配置缓存与 Isolated Projects 下同样安全，
 * 且路径在**配置阶段**解析好，执行期不再访问 rootProject。
 */
abstract class RustBuildTask : DefaultTask() {
    @get:Inject
    abstract val execOps: ExecOperations

    @get:Input
    abstract val scriptPath: Property<String>

    @get:Input
    abstract val abis: Property<String>

    @TaskAction
    fun build() {
        execOps.exec {
            commandLine("pwsh", "-NoProfile", "-File", scriptPath.get(), "-Abi", abis.get())
        }
    }
}

tasks.register<RustBuildTask>("buildRustCore") {
    group = "audiolink"
    description = "用 cargo-ndk 交叉编译 Rust 内核到 app/src/main/jniLibs"
    scriptPath.set(
        rootProject.layout.projectDirectory.file("scripts/build-rust.ps1").asFile.absolutePath
    )
    abis.set("arm64-v8a,armeabi-v7a")
}
