// AudioLink Android —— 根构建脚本
// 版本矩阵依据：docs/04-tech-stack.md（AGP 9.4.0 / Gradle 9.7.1 / Kotlin 2.4.20 / Compose BOM 2026.09.00）
// 约束：compileSdk = 37（Compose 1.12+ 强制）、targetSdk = 36（ADR-009：升 37 会强制
//       ACCESS_LOCAL_NETWORK 运行时权限，导致局域网收发 EPERM）
plugins {
    id("com.android.application") version "9.4.0" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.4.20" apply false
}
