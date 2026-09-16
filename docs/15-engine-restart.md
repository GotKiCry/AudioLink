# 同端口重启与停止完成语义（2026-09-16）

看板任务：`[M1] 引擎停止后立即重启：QUIC 端口尚未释放`。

## 问题与实现

原来的 `Engine::shutdown()` 只发关闭信号。FFI 等接受循环退出后就返回，但会话任务、音频线程和
QUIC 驱动仍可能持有 UDP 套接字，紧接着绑定同一端口会在 Windows 上报 `1008 / os error 10048`。
并发启动还可能越过“当前引擎为空”的检查；调用方取消等待时，清理和句柄安装也可能只完成一半。

现在分三层完成停止：

1. **Engine**：在同一把任务注册锁内禁止新任务并关闭 `TaskTracker`，向会话和 QUIC 发停止信号，
   等待接受循环、主动连接及会话全部退出。音频线程在阻塞池中 join，停止完成后不再调用旧音频回调。
   保留旧 `Arc<Engine>` 不会继续占用端口；停止后的连接/推流命令返回错误。
2. **net**：保留现有 `close()` 发信号语义，新增 `shutdown()`。取走端点句柄，等待 QUIC 连接排空，
   再等待真实 UDP 套接字析构通知。IO poller 也纳入该通知的持有关系；停止返回不依赖固定 sleep 或绑定重试。
   包装层原样转发分段与 MTU 能力，收发路径不新增锁或分配。
3. **FFI**：`engineStart/engineStop` 共用生命周期锁，按获锁顺序串行执行。
   同配置重复启动返回当前状态，不同配置返回 `1009 BUSY`；并发冷启动不再竞争绑定端口。
   停止时同步查询立即看到未启动，新启动则等全部旧资源释放。

取消语义明确为：**还在等生命周期锁时取消，不执行该请求；获锁并派发后取消等待，运行时继续完成整个操作**。
因此取消 `engineStop()` 的等待协程不会让随后启动的引擎越过旧资源清理。
UniFFI 导出签名不变，现有 Kotlin 绑定可直接使用。

底层 `AudioLinkEndpoint::shutdown()` 的调用方须先结束 accept/connect 任务并释放该端点的所有
`Connection/ControlChannel`；Engine 负责这些前置清理。它可在取消等待后再次调用，并支持幂等关闭。
停止可能需要等待 QUIC 排空及正在执行的音频回调；平台回调必须正常返回，不能把其无限阻塞解释为已停止。

## 回归证据

旧代码对照：移除 `pairing_lifecycle.rs` 中临时换端口的绕行，原代码稳定报 `10048`；
修复后同一用例直接复用原端口通过。旧日志：`target/evidence/engine-restart/before-fix.log`。

新增五项回归：

| 测试位置 | 实际验证 |
|---|---|
| `audiolink-net/src/endpoint/socket.rs` | 保留 IO poller 时不提前发释放通知；销毁最后持有者后立刻由 OS 重绑原端口 |
| `audiolink-net/tests/quic_pair.rs` | 真实 QUIC 连接持有期间关闭等待不提前完成；取消再继续关闭、保留旧 Endpoint 句柄、立即重绑与幂等 |
| `audiolink-engine/tests/engine_shutdown.rs` | 发出 QUIC Initial 后对端不回应，停止仍在 5 s 内结束；接受/连接任务退出，无迟到会话，旧 Engine 句柄不占端口 |
| `audiolink-ffi/src/engine_bridge.rs` | 取消已派发操作的等待不会中断操作；取消排队中的操作不会执行它 |
| `audiolink-ffi/tests/engine_restart.rs` | 同端口三轮 × 五状态（空闲/等 PIN/已连接/接收中/发送中）；每次停止后 OS 立即重绑、旧回调销毁；并发冷启动、重复停止，以及卡住真实播放回调后取消停止等待再重启 |

FFI 回归使用真实 Engine、QUIC、PIN、Opus 与 PCM 回调，音频源/输出为合成和计数实现，不访问物理声卡。
已有 PIN 生命周期回归也已恢复同端口重启，不再换端口规避问题。

检查命令：

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --exclude audiolink-desktop
cargo test -p audiolink-desktop
pwsh android/scripts/build-rust.ps1
$env:GRADLE_USER_HOME = "$PWD/.gradle-home"
pwsh tools/gradlew.ps1 -JavaHome '<JDK 17 根目录>' assembleDebug assembleRelease testDebugUnitTest --init-script "$PWD/target/evidence/engine-restart/apk-output.init.gradle" --no-configuration-cache --console=plain
```

上述 Rust 检查全部通过；Android 双 ABI Rust Release 库、Debug/Release APK 构建成功。
Kotlin 未改动，JVM 测试任务命中 Gradle 缓存；核对 XML 为 69 项通过、0 失败/错误/跳过。
两个 APK 中 arm64-v8a/armeabi-v7a 的 `.text` 和 `.rodata` 均与本轮构建的 JNI 库一致，
Release APK 的 V2 签名校验通过。核对记录：`target/evidence/engine-restart/apk-verification.json`。

| 产物（相对仓库根目录） | 字节 | SHA-256 |
|---|---:|---|
| `android/app/build/restart-validation/outputs/apk/debug/app-debug.apk` | 19,852,661 | `3e5c1bc16b2c5df746d6a7a8f535723ef0aa9155b6474ccad0b147443a0cfca0` |
| `android/app/build/restart-validation/outputs/apk/release/app-release.apk` | 8,079,628 | `41f215e3e81daacd0a1140c5eb21cd56452865414302be485b1a562891aadd42` |

本轮 ADB 无设备，Android 手机界面的快速启停及 PIN 显示仍由真机验收项追踪；本条关闭依据是 Windows
故障已复现、真实网络和音频生命周期回归通过。同一次停止之后继续保留已关闭对象也已覆盖。

后续 PHK110 真机已验证普通/快速启停、配对中和推流中同端口重启；另发现并修复了 Android 服务状态
回写竞态。最新 APK 与安装后核对结果见 `docs/16-android-service-lifecycle.md`；本文件上表保留原始构建记录。

为了保留正在使用的旧 APK，本轮通过临时 Gradle init 脚本将输出目录设为
`android/app/build/restart-validation`；仓库构建配置保持默认。

临时 init 脚本内容（复现时先创建上述路径）：

```groovy
gradle.beforeProject { project ->
    if (project.path == ':app') {
        project.layout.buildDirectory.set(new File(project.projectDir, 'build/restart-validation'))
    }
}
```
