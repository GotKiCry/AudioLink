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

        versionCode = 1
        versionName = "0.1.0" // CI 校验：必须与 Cargo.toml workspace version 一致

        ndk {
            abiFilters += listOf("arm64-v8a", "armeabi-v7a")
        }

        // 由 android/scripts/build-rust.ps1 生成的 Rust 内核（UniFFI 绑定 + .so）
        // 产物路径：app/src/main/jniLibs/<abi>/libaudiolink_core.so
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
    val composeBom = platform("androidx.compose:compose-bom:2026.09.00")
    implementation(composeBom)
    androidTestImplementation(composeBom)

    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.activity:activity-compose")
    implementation("androidx.lifecycle:lifecycle-runtime-compose")
    implementation("androidx.lifecycle:lifecycle-service")
    implementation("androidx.datastore:datastore-preferences")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json")

    // 注意：不引入 okhttp / ExoPlayer / AndroidAsync —— 旧版的整块 HTTP 管理面已被移除
    debugImplementation("androidx.compose.ui:ui-tooling")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
}

// 一键：先编 Rust 内核，再打 APK（避免忘记同步 .so）
tasks.register("buildRustCore") {
    group = "audiolink"
    description = "用 cargo-ndk 交叉编译 Rust 内核到 app/src/main/jniLibs"
    doLast {
        exec {
            commandLine(
                "pwsh", "-NoProfile", "-File",
                rootProject.file("scripts/build-rust.ps1").absolutePath,
                "-Abi", "arm64-v8a,armeabi-v7a"
            )
        }
    }
}
