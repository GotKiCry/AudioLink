# 开发环境与构建

> 本机（开发机）实测于 **2026-09-11**。凡标 ✅ 为已具备，标 ⚠️ 为**必须处理**，标 ❌ 为需安装。

---

## 1. 本机现状

| 组件 | 状态 | 详情 |
|---|---|---|
| Rust | ✅ | `rustc`/`cargo` **1.96.1**（MSVC host）；`aarch64-linux-android` target 已装 |
| MSVC 工具链 | ✅ | VS Build Tools 2026，MSVC **14.50.35717**；Windows SDK 10.0.26100 |
| WebView2 Runtime | ✅ | **152.0.4191.66**（Tauri 运行必需） |
| Node / pnpm | ✅ | Node **24.13.0** / pnpm **11.22.0** |
| NSIS | ✅ | Windows 安装包打包用 |
| 长路径支持 | ✅ | 已开启（Rust/Android 构建路径较深，必需） |
| Android SDK | ✅ | `C:\Users\liuzh\AppData\Local\Android\Sdk`；platforms 到 **android-37.0**；build-tools 到 **37.0.0**；NDK **28.2.13676358**、27.0.12077973；licenses 已接受 |
| adb | ✅ | platform-tools 36.0.2（当前无设备连接） |
| Tauri CLI | ❌ | 需装（项目内 devDependency 即可，**不要全局装**） |
| JDK 17 | ⚠️ | 本机有 **Corretto 17.0.20.1**（`C:\Users\liuzh\scoop\apps\corretto17-jdk\current`），但 `JAVA_HOME` 指向 **JDK 11** → Gradle 9 会**直接拒绝启动** |
| `ANDROID_HOME` / `NDK_HOME` | ⚠️ | 未设置（`cargo-ndk` 与 Gradle 需要） |
| cmake / NASM / perl / vcpkg | ✅（不需要） | 纯 Rust Opus 方案已绕开，**不要为此安装** |

---

## 2. 待办清单（一次性）

```powershell
# 1) 让 Gradle 能启动（本机 JAVA_HOME 是 JDK 11 → Gradle 9 直接拒绝启动）
#    ⚠️ 不要把 JDK 路径写进 android/gradle.properties：CI 跑在 Ubuntu，路径不存在会直接失败。
#    本地用包装脚本（自动校验 JAVA_HOME 是否 17+，并注入 ANDROID_HOME）：
#       pwsh tools\gradlew.ps1 assembleDebug
#    也可显式指定：-JavaHome 'C:\path\to\jdk-17'
# 验证（新开终端）
java -version            # 期望 17.x

# 2) Android 环境变量
[Environment]::SetEnvironmentVariable('ANDROID_HOME',"$env:LOCALAPPDATA\Android\Sdk",'User')
[Environment]::SetEnvironmentVariable('NDK_HOME',"$env:LOCALAPPDATA\Android\Sdk\ndk\28.2.13676358",'User')
$env:Path += ";$env:LOCALAPPDATA\Android\Sdk\platform-tools;$env:LOCALAPPDATA\Android\Sdk\cmdline-tools\latest\bin"

# 3) Rust 交叉编译目标（arm64 已装，补 armv7）
rustup target add armv7-linux-androideabi

# 4) cargo-ndk（把 Rust 内核编成 Android .so）
cargo install cargo-ndk --locked

# 5) Tauri CLI（项目内）
cd C:\_Project\AudioLink\desktop; pnpm add -D @tauri-apps/cli@2.11
```

**Gradle 隔离（重要）**：`~/.gradle/gradle.properties` 中含公司私服地址与**明文账号密码**。本项目**必须**使用独立 Gradle 用户目录，避免把内部凭证带进构建日志或缓存：

```powershell
$env:GRADLE_USER_HOME = 'C:\_Project\AudioLink\.gradle-home'   # 建议写入 android/gradle.properties 或 CI 环境
```

---

## 3. 构建流程

### 3.1 内核（Rust）

```powershell
cd C:\_Project\AudioLink\core
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo run -p audiolink-tools --bin alp2-dump -- --help    # 协议解码工具
```

### 3.2 桌面端（Tauri）

```powershell
cd C:\_Project\AudioLink\desktop
pnpm install
pnpm tauri dev            # 开发
pnpm tauri build          # 产物：NSIS 安装包 + 绿色版 exe
```

### 3.3 Android

```powershell
cd C:\_Project\AudioLink\android
# 1) 先编内核 .so（脚本会调 cargo-ndk，产出到 app/src/main/jniLibs/<abi>/）
pwsh .\scripts\build-rust.ps1 -Abi arm64-v8a,armeabi-v7a
# 2) 再编 APK —— 用包装脚本（保证以 JDK 17 启动 Gradle；本机直接 .\gradlew.bat 会因 JAVA_HOME=11 失败）
pwsh ..\tools\gradlew.ps1 assembleDebug
pwsh ..\tools\gradlew.ps1 assembleRelease   # 需本地 keystore 属性（见下）
```

`android/local.properties`（**不入库**）：
```properties
sdk.dir=C\:\\Users\\liuzh\\AppData\\Local\\Android\\Sdk
```

发布签名（**不入库**）：`android/keystore.properties`
```properties
storeFile=C\:\\Users\\liuzh\\.audiolink\\release.jks
storePassword=<从 CI Secret 注入，本地手动填>
keyAlias=audiolink
keyPassword=<同上>
```

---

## 4. 常见坑（血泪清单）

| 坑 | 现象 | 处置 |
|---|---|---|
| `JAVA_HOME` 指 JDK 11 | Gradle 9.x 启动即报错 | 改成 Corretto 17（§2 第 1 条） |
| `targetSdk` 升到 37 | 局域网收发全部失败（UDP EPERM），且后台音频被**静默吞掉** | **锁 `targetSdk = 36`**（见 ADR-009） |
| Compose BOM 与 compileSdk 不匹配 | 编译期报"要求 compileSdk 37" | 保持 `compileSdk 37` + BOM 2026.09.00；或整体回退 AGP 8.13.2 + BOM 2026.06.01 |
| `cmdline-tools` 目录为空 | 无 `sdkmanager`，命令行装 SDK 包失败 | 所需包已齐，必要时用 Android Studio 的 SDK Manager 补 |
| Gradle 缓存无 9.6/9.7 | 首次构建需联网下载（约 100 MB+） | 预留网络；CI 侧用缓存 |
| 本机有 3 个同名"扬声器"端点 + 虚拟声卡 | 采集到空流/无声 | 设备选择显示**唯一实例 ID**；默认设备是虚拟声卡时警告（FR-01） |
| 事件驱动 loopback 不触发 | 采集无数据 | 需 Win10 **1703+**（更低版本用 `StreamMode::PollingShared`） |
| 独占模式 + loopback | `wasapi` crate 明确报错 | 不要用独占模式做 loopback |
| 路径过长 | Rust/NDK 构建失败 | 已开启长路径；仓库路径保持短（`C:\_Project\AudioLink` 即可） |
| 端口占用 | QUIC 58290 / 发现 58280 冲突 | 端口可配置；启动时检测占用并提示 |
| PATH 里有 GNU coreutils 的 `link.exe` | 可能劫持 MSVC 链接步骤 | 用 `cargo build`（rustc 自带精确工具链路径）；不要手写 `link` 调用 |
| 想装 CMake 编译 Opus | 不必要 | 本项目走 **纯 Rust `opus-rs`**（ADR-003）→ **CMake / NASM / Perl / pkg-config / vcpkg 全都不需要**；仅当切换到 `opus` 0.4.0 路线才需要 CMake |
| `cargo build` 报 `aws-lc-sys` / 缺 CMake、NASM | `rustls` 的**默认**加密后端是 `aws_lc_rs`（需要 CMake 构建） | workspace 里已把 `rustls` 固定为 `default-features = false, features = ["ring", "std", "tls12", "logging"]`；`quinn` 默认后端即 `rustls-ring`（无需改动）。**不要**给 `rustls` 打开默认 features，也不要显式启用 `quinn/rustls-aws-lc-rs` |

### 4.1 首次跑通时实测踩到的坑（2026-09-11 已全部修复）

| 现象 | 根因 | 处置 |
|---|---|---|
| `cargo ndk` 报 `profile name 'debug' is reserved` | Cargo 的开发 profile 叫 **`dev`**，`debug` 是保留名 | `android/scripts/build-rust.ps1` 已改用 `dev`/`release`；同时避免用 `$profile` 作变量名（与 PowerShell 自动变量 `$PROFILE` 撞名） |
| `术语 '.\gradlew.bat' 不会被识别为…` | 仓库缺 wrapper 三件套（只写了 `gradle-wrapper.properties`） | 已补齐 `gradlew`、`gradlew.bat`、`gradle/wrapper/gradle-wrapper.jar`（标准文件；`distributionUrl` 仍锁 9.7.1） |
| Gradle 配置期报 `Unresolved reference 'exec'` | **Gradle 9 移除了项目/任务级 `exec {}` DSL** | `app/build.gradle.kts` 改为注入 `ExecOperations` 的 `RustBuildTask`（配置缓存与 Isolated Projects 下同样安全） |
| `error: OUT_DIR env var is not set, do you have a build script?`（指向 `tauri::generate_context!`） | `desktop/src-tauri/` 缺 **`build.rs`** | 已补 `build.rs` 调用 `tauri_build::build()`（Tauri 2 硬要求） |
| `gradlew` 启动即失败 | **launcher JVM** 由 `JAVA_HOME`/PATH 决定，`org.gradle.java.home` 只影响 **daemon JVM** | 已提供 **`tools/gradlew.ps1`** 包装（自动校验 JDK 17+、注入 ANDROID_HOME）；并**禁止**把本机 JDK 路径写进 `android/gradle.properties`（会让 Ubuntu CI 直接失败） |
| `无法覆盖变量 home，因为它是只读变量或常量` | PowerShell 把 `$home` / `$profile` / `$args` / `$host` 等视为**只读自动变量**，不能用作参数名或赋值目标 | 已改名；写 PS 脚本时避开这些名字（本项目已在 `build-rust.ps1` 的 `$profile` 与 `gradlew.ps1` 的 `$home` 各踩一次） |

---

## 5. CI 设计（`.github/workflows/`）

| 工作流 | 触发 | 内容 |
|---|---|---|
| `rust-check.yml` | push / PR | fmt + clippy(`-D warnings`) + test + 协议 golden vectors |
| `android-build.yml` | push / PR（`android/**`、`core/**`） | 装 JDK 17 + NDK → `build-rust` → `assembleDebug` → 上传 APK（debug） |
| `desktop-build.yml` | push / PR（`desktop/**`、`core/**`） | pnpm 缓存 → `tauri build` → 上传安装包 artifact |
| `release.yml` | tag `v*.*.*` | 全量构建 → NSIS 安装包 + 绿色版 + APK（双 ABI，Secret 签名）→ 生成更新清单（签名）→ GitHub Release |

**规则**：所有 `uses:` 固定到 commit SHA（旧版用未 pin 的 `v3` actions）；工具链版本全部来自 `04-tech-stack.md`；版本号单一来源（`core/Cargo.toml` 的 workspace version + 脚本同步到 Tauri 与 Android），CI 校验三处一致（旧版靠人工同步）。

---

## 6. 本地验收工具链

| 工具 | 用途 | 归属里程碑 |
|---|---|---|
| `alp2-dump` | 协议抓包解码（hex 文本 → 人类可读字段；拒绝用例的现场取证） | M0 ✅ |
| `latency-probe` | QUIC 数据报 RTT/抖动 + §6 四时间戳时钟偏移（`listen` / `probe` 两个子命令） | M0 ✅（音频各环节分解留到 M1） |
| `netem-sim` | 丢包/抖动/带宽注入（Windows 侧代理或 WFP） | M2 |
| `sync-measure` | 双机同期录音 + 波形对齐，输出同步偏差报告 | M3 |
| `soak-runner` | 8 h 连续运行 + 指标采集 + 异常自动快照 | M2 |

**用法（M0 已可用；可执行文件在 `target/x86_64-pc-windows-msvc/debug/`）**：

```powershell
# 1) 协议解码：每行一个报文；可直接粘 docs/03-protocol.md §12 的向量行
"02 01 00 00 | 01 00 00 00 | 2A 00 00 00 | C0 03 00 00 | 88 77 66 55 44 33 22 11 | DE AD BE EF" |
    cargo run -q -p audiolink-tools --bin alp2-dump
cargo run -q -p audiolink-tools --bin alp2-dump -- dump.txt        # 文件输入
cargo run -q -p audiolink-tools --bin alp2-dump -- --single       # 整个输入当作一个报文
# 退出码：0 = 全部解码成功；1 = 有报文被 L1 拒绝；2 = 用法/输入错误

# 2) 链路测量（两个终端；先服务端后客户端）
cargo run -q -p audiolink-tools --bin latency-probe -- listen --bind 0.0.0.0:58290
cargo run -q -p audiolink-tools --bin latency-probe -- probe 192.168.1.20 --count 50 --interval-ms 100
# 输出：RTT 的 min/P50/P95/P99/max、相邻 RTT 波动、§6 的 best8 偏移估计与极差、§6.5 质量分级
# 启动时会打印 QUIC 的 max_datagram_size() —— §3 的 1200 B 预算必须与它取 min（默认初始 MTU 下实测 1162 B）
```
