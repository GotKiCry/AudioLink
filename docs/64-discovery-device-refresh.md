# 局域网发现真机排查与刷新

> ⚠️ **历史记录说明（2026-09-22 补）**：本文记录的是 **PIN 配对 / 信任库时代**的交付事实与证据。
> 该认证机制已在本轮整体移除（现在连上即用：没有配对码、没有白名单），本文中的配对步骤、PIN 验收数字与
> 信任判定均**不再对应当前实现**，只作为当时的交付记账保留 —— 现状见 `docs/72-remove-pairing.md`。

验证日期：2026-09-21。Windows 主机 `GOTKICRY_WORK`，以太网地址 `192.168.3.200/24`；Android 真机 PHK110，Wi-Fi 地址 `192.168.3.75/24`。两端处于同一子网。

## 原因与修复

1. 上轮只构建了 Debug，手机仍运行旧的正式包 `com.gotkicry.audiolink`。从手机拉取 APK 后，确认它既没有主机列表，也没有 `uniffi_audiolink_ffi_fn_constructor_discoverybrowser_new` 原生符号。此次同时更新 Debug、正式签名 Release 和双 ABI 原生库，并覆盖安装正式包。
2. Windows 广播实际上正常：本机 UDP 捕获和 Android 的 UDP 监听均收到真实主机报文。Android 作为发送主机时尚未启动广播；现在 FFI 引擎在具备 `CAN_SEND` 能力并监听成功后启动广播，停止引擎时释放广播器。只有接收能力的手机不会出现在主机列表中。
3. 桌面和 Android 主机列表均增加「刷新主机」。点击会清空旧结果、释放并重建浏览器，继续监听广播。搜索状态避免重复点击，发生错误后可再次刷新；桌面忽略旧请求返回的数据，Android 取消旧扫描并等待资源释放后启动新扫描。
4. Android 原生构建显式使用 API 26，与应用 `minSdk=26` 一致。发现层枚举网卡使用的 `getifaddrs` / `freeifaddrs` 从 API 24 起提供；cargo-ndk 的默认 API 21 导致独立探针无法链接。

## 真机验证

- 在 Android 通过 UDP 58280 收到 Windows 当前运行程序发出的广播；未修改路由器或防火墙设置。
- 安装更新后的正式包，真实应用界面显示 `GOTKICRY_WORK` 和 `192.168.3.200:58290`。
- 连续操作「刷新主机」3 次，每次都能重新发现电脑；切到后台后返回，也能再次发现。
- 最终 API 26 构建完成后再次覆盖安装正式包，复测刷新前后主机均可见。从手机拉取安装后的 APK，与本地最终 Release APK 逐字节一致。
- 反向使用 PHK110 上的原生探针调用生产 FFI `engine_start`，提供合成静音源，启动实际广播；Windows 生产 Rust 浏览器发现 `192.168.3.75:58299`，名称为 `AudioLink discovery test (synthetic audio)`。该检查验证手机广播和电脑接收链路，未通过桌面 WebView 操作反向连接，也没有打开麦克风。
- 未进行连接与音频回归；这些不属于此次发现验证结果。

本地原始证据保存在 `target/evidence/discovery-refresh/`，不提交二进制产物。UIAutomator 获取快照包含等待界面空闲的耗时，不能将脚本耗时当作发现延迟。

| 证据 | SHA-256 |
| --- | --- |
| `installed-before.apk`，原手机正式包 | `4940f32fe5ec314c3dc925a7babf79569c7a3439504815dfb3201dca95ba179b` |
| `installed-after.apk`，最终正式包 | `d8b30fe38d896a17406398d5a6c5dadceb549e634c2483037e4e696de95edfe2` |
| `phone-final.png`，最终主机列表截图 | `913d69e1fa2b80ade7f7f4e7cb141ffaaaf6fa73d8db0aa8b598a26c74534d09` |
| `phone-refresh-results.json`，连续刷新与前后台检查 | `91c198c20208f328629abe690be04058b199a4a250329c7286f0550a739be8e8` |
| `windows-received-phone.txt`，电脑发现手机 | `a44314c43fd61765dc6eb9360ad9ed506961e47257ae3aa7b27868e8837fa63b` |

`verification.json` 记录两种 ABI、Debug / Release 共 4 份 APK 的一致性检查：当前 Rust 输出、JNI 输入、Gradle 合并库逐字节一致；APK 内库与 NDK strip 后的当前 Rust 输出一致。生成的 Kotlin 绑定也与当前源码重新生成结果一致，21 个 FFI 校验符号完整，随包许可声明一致。`final-device-check.json` 记录最终安装包匹配和刷新复测。

## 自动检查

- 桌面构建和中英文本检查通过，174 项前端测试通过；新增刷新重建、旧响应丢弃、重复点击抑制、后续普通轮询和失败恢复覆盖。
- 桌面 Rust 库测试 39 项通过，1 项硬件测试跳过。
- 发现与 FFI 模块测试 35 项通过，1 项硬件测试跳过。
- 发现、FFI、桌面 Rust 的全部 target / feature Clippy 通过；探针 Clippy 与 Rust 格式检查通过。
- Android 237 项 JVM 测试通过；双 ABI 原生库和 Debug / Release APK 均构建通过。

## 复现原生链路

在仓库根目录构建 Android 探针，然后通过 ADB 放入设备的临时目录：

```powershell
cargo ndk --platform 26 -t arm64-v8a build -p audiolink-ffi --example discovery_probe --release
adb push target/aarch64-linux-android/release/examples/discovery_probe /data/local/tmp/audiolink-discovery-probe
adb shell chmod 755 /data/local/tmp/audiolink-discovery-probe
adb shell /data/local/tmp/audiolink-discovery-probe scan 15
```

检查反向广播时，在一个终端让手机启动合成源主机：

```powershell
adb shell /data/local/tmp/audiolink-discovery-probe advertise 30 /data/local/tmp/audiolink-discovery-identity
```

同时在 Windows 另一终端运行浏览器：

```powershell
cargo run -p audiolink-ffi --example discovery_probe -- scan 15
```

探针的运行时间限制为 1–60 秒，到期自动停止；测试使用 UDP 58299 监听引擎，发现仍使用 UDP 58280。`advertise` 会在指定临时目录生成独立测试身份，不使用应用身份。
