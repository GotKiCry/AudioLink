# Android PIN 状态修复（2026-09-16）

看板任务：`[M1] Android PIN 卡有时不显示（已进入配对分支但屏幕无 PIN 卡）`。

## 原因与改动

旧版 FFI 的 `displayedPin()` 只读事件泵缓存。`DisplayPin` 是一次性广播，落后超过频道容量后会被丢弃；
Android 每 500 ms 轮询同一缓存也无法补回它。缓存还只在配对成功时清空，断开/失败后会留下失效 PIN，
旧事件泵也可能跨引擎重启继续写全局状态。

现在会话任务在写出 `PAIR_REQUIRED` 之前同步发布配对状态，FFI 直接读取当前 Engine 的会话快照，
删除额外的事件泵和全局 PIN 缓存。原有 Kotlin 导出签名保持不变：

- PIN 未出现前返回 `null`；进入配对后，查询不需要订阅事件。
- PIN 取自当前连接的 `Handshake/PinGate`，60 s 截止时间使用门禁原值。输错一次仍显示原 PIN，重试不续期。
- 成功、锁定、断开、到期或引擎停止后返回 `null`。同时有多条请求时优先显示最新仍有效的 PIN。
- 发起端的 `submitPin()` 也从当前会话定位待输入 PIN 的对端。
- 同一身份重连会停止旧会话；旧任务退出只移除自己的表项，不能删掉新会话或广播新会话已断开的假消息。
- 握手时及时发布对端名称，Android 配对卡不必等配对成功才取得名称。

Android 保留原有轮询及 PIN 卡；它原有的失效提示仍覆盖两次独立查询之间恰好发生断开的情况。

## 自动回归

`audiolink-engine/src/runtime/pairing_tests.rs` 新增三项回归，通过真实 `Handshake/PinGate` 推进状态：

1. 向广播发送 512 条遥测，确认收到 `Lagged` 且 `DisplayPin` 已被挤掉；当前 PIN 仍可查询。
   晚订阅者收不到历史事件，但读取同样成功；在门禁截止前 1 ns 仍有效，到期瞬间隐藏，无需新事件。
2. 错误 PIN 可重试，原始截止时间不变；第 5 次错误导致锁定，立即清除显示值。
3. 同一身份的新会话不会被旧任务清理；断开当前显示的请求后，另一条仍有效的配对状态保留。

`audiolink-ffi/tests/pairing_lifecycle.rs` 用真实 Engine/QUIC，经公开 FFI 查询与提交：
断开清 PIN、同身份重连、错误后重试成功、未完成配对时停止/重启，以及 FFI 作为发起端配对。
使用临时身份，不访问声卡。测试进程与原有 FFI 生命周期单测分离，避免争用全局引擎。

**旧代码对照**：仅把 FFI 桥恢复为 `8262bf5` 版本，同一个测试在“断开后不能返回旧 PIN”处失败；
修复版本通过。原始输出保存在本机 `target/evidence/pairing-state/old-ffi-regression.log`。

本轮检查通过：fmt、全 workspace Clippy（包含桌面）、内核完整测试、桌面 12 项测试（含真实配对接缝）、
Android 69 项 JVM 测试、双 ABI release 内核、Debug/Release APK。Release 签名 V2 验证通过。
两个 APK 内两种 ABI 的 `.text` / `.rodata` 都与本轮编译的原始 `.so` 一致，确认带入了新内核。

检查命令：

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --exclude audiolink-desktop
cargo test -p audiolink-desktop
pwsh android/scripts/build-rust.ps1
pwsh tools/gradlew.ps1 -JavaHome '<JDK 17 根目录>' assembleDebug assembleRelease testDebugUnitTest --console=plain
```

## 真机复测与独立遗留

代码修复当时 `adb devices -l` 无设备。以下为该阶段的构建记录；后续 PHK110 真机已通过验收，见下文。
当时使用新生成的
`android/app/build/outputs/apk/debug/app-debug.apk`（21.70 MiB）或
`android/app/build/pin-validation/outputs/apk/release/app-release.apk`（7.63 MiB）复测。
原目录的 Release APK 被占用，故本轮仅通过临时 Gradle init 脚本将 Release 输出到 `build/pin-validation`，
未更改仓库构建配置；旧 `build/outputs/apk/release/app-release.apk` 不是本轮交付物。
产物 SHA-256 与核对结果位于 `target/evidence/pairing-state/apk-verification.json`。

复测步骤：

1. 用未受信的 PC 身份连接，手机应在下一轮刷新出现 PIN 和 PC 名称；已受信连接无需 PIN。
2. 故意输错一次，原 PIN 保留；再输正确值应配对成功、卡片消失。
3. 未完成配对就退出 PC 客户端，卡片消失；重连时显示当前连接 PIN。
4. 等待 60 s 不提交，卡片消失。不同连接随机生成的 PIN 有小概率相同，不能仅凭数字不同判定重连成功。

后续已修复本轮发现的 **QUIC 端口释放竞态**（`engineStop()` 后立即同端口 `engineStart()` 报 10048）。
现在等待真实套接字和会话/音频线程释放；PIN 生命周期回归已移除换端口的绕行。
实现、取消/并发语义及回归证据见 `docs/15-engine-restart.md`。

### PHK110 真机验收完成（2026-09-16）

用户接入 PHK110（Android 16 / API 36）后，已验证屏幕 PIN 显示、输错后保留、输对后清除、
未配对断开清除、同身份重连、60 s 到期、五次输错关闭连接，以及配对中服务停止/重启。
UI XML、屏幕截图与真实 QUIC 客户端日志共同验证，PIN 看板条目可以关闭。
同时修复 Android 服务销毁后旧协程回写运行状态的问题，详见 `docs/16-android-service-lifecycle.md`。

当前 Release：`android/app/build/phk110-validation/outputs/apk/release/app-release.apk`，
安装后已从手机回拉并核对 SHA-256；不要再用旧默认输出路径作为最新版依据。
