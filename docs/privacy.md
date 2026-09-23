# 隐私说明（Privacy）

> 事实基线：**git be09ab1**（2026-09-17）｜ 本文每一条都能指到代码或清单，行号可用 `git show be09ab1:<path>` 复核
> 　　本轮变更（整体移除 PIN 配对 / 信任库 / AUTH_* 挑战应答）涉及的条目已同步更新，交付记录见 `docs/72-remove-pairing.md`；未受影响的条目仍以 be09ab1 为准。
> 上游要求：`docs/05-roadmap.md` §M5 交付物 5「合规清单：`LICENSE` / `NOTICE` / 隐私说明」
> 交叉材料：`docs/manual/user-guide.zh-CN.md`（§1 一句话）、`docs/03-protocol.md`（协议）、`docs/41-compliance-license-audit.md`（许可证）

## 0. 一句话

**AudioLink 没有服务器、没有账号、没有云端。** 声音只在你选定的设备之间**点对点直连**传输，
**不经过任何中转**，**不落盘**，**不上报任何遥测**。（依据见 §2、§3、§4。）
**不经过任何中转**，**不落盘**，**不上报任何遥测**。（依据见 §2、§3、§4。）

---

## 1. 采集什么（以及明确不采集什么）

采集**只在发送端**发生，且必须由用户显式选源 + 完成系统授权。

| 端 | 可选采集源 | 触发方式 | 证据 |
|---|---|---|---|
| 桌面（Windows） | **系统输出端点**（WASAPI loopback，即"电脑正在播放的声音"）| 用户选定输出端点后点「开始推流」 | `core/crates/audiolink-audio/src/wasapi/`（capture/render）；桌面契约的 `list_capture_devices` / `active_capture_device` 只列**输出端点**：`docs/11-m1-contract.md` §6、`docs/13-desktop-capture-selection.md` |
| Android | 三态：**关闭 / 系统内录 / 麦克风**（FR-38）| 用户在界面选源；内录拉系统 MediaProjection 授权页，麦克风走 `RECORD_AUDIO` 运行时授权 | `android/app/src/main/kotlin/com/gotkicry/audiolink/capture/CaptureStateMachine.kt:24`（发送源三态）、`CaptureController.kt:85-89`（`startMicrophone()` / `startLoopback(projection)`）、`MediaProjectionRequestActivity.kt:74`（系统授权页）、`android/app/src/main/AndroidManifest.xml`（`RECORD_AUDIO` + 两个采集类前台服务权限）|

**明确不采集的东西**（每条都给"为什么可以这么断言"）：

- **不采集屏幕画面**：Android 的 MediaProjection 只用来取得**音频播放捕获**配置 —— `SystemLoopbackCaptureSource.kt:42` 构造 `AudioPlaybackCaptureConfiguration`、`:56` `setAudioPlaybackCaptureConfig(..)`；全仓 grep `VirtualDisplay` **零命中**（没有录屏对象）。
  注意：系统授权页本身叫「屏幕捕获」（`MediaProjectionRequestActivity.kt:74` 用 `createScreenCaptureIntent()`），那是 Android 取 MediaProjection 的**唯一**入口；本项目只从中拿音频。
- **桌面不采集麦克风**：`core/crates/audiolink-audio/src/` 里 grep `microphone|Microphone` **零命中**（只有输出端点内录）。
- **不采集键盘输入 / 文件 / 通讯录 / 位置**：Android 清单里没有对应权限（见 §5 全量清单）；桌面端不申请任何权限。
- **音频不落盘**：引擎实现里 grep `fs::write|File::create|OpenOptions` **零命中** —— 采集到的 PCM 只在内存里编码、发送、播放。

**一处诚实的现状（未验证）**：Android 采集是**新落地**的能力（提交 `1c4ac55` / `4c2d21a` / `a0158a2`），
代码自身列出了尚未实测的部分：真实内录是否真的抓到系统音频、麦克风实际采样与延迟、授权弹窗流程、
MIUI 后台限制（见 `capture/CaptureController.kt:40` 的清单）。**真机验收未做** —— 本文不声称它已验证。

---

## 2. 音频去了哪里

**只去你连上的那台设备，直连，加密，不经中转。**

| 事实 | 证据 |
|---|---|
| 传输是 QUIC（UDP）+ **TLS 1.3**，且**双向出示证书**（服务端索取客户端证书） | `core/crates/audiolink-net/src/tls.rs:46-56`（`with_client_cert_verifier`）、`:66-79`（`with_client_auth_cert`）；依赖见 `audiolink-net/Cargo.toml:17,19`（quinn + rustls，ring 后端）|
| 证书是**本机自签**，不是任何 CA 签发；身份 = **证书指纹**（`SHA-256(证书 DER)` 前 8 字节 = `NodeId`） | `core/crates/audiolink-net/src/tls.rs:35-38`、`docs/03-protocol.md` §2 |
| 通道**仍然加密**：TLS 层只负责加密，以及“对端就是它出示的那张证书、且持有对应私钥”的证明 | `core/crates/audiolink-net/src/tls.rs:88`、`:176-184`；协议 `docs/03-protocol.md` §2 |
| **不再做信任裁决**：**TLS 仍然证明「对端就是它出示的那张证书」，只是不再有人问「这张证书值不值得信任」。** 没有信任库、没有白名单、没有 6 位配对码 | 本版本整体移除，见 `docs/72-remove-pairing.md`；协议 `docs/03-protocol.md` §5 |
| **没有服务器、没有中继**：仓库里不存在服务端组件；音频数据报只在两端 QUIC 连接之间流动 | 音频路径：`core/crates/audiolink-engine/src/runtime.rs` 会话循环的 `transmit_audio` / `read_datagram_into`；协议 `docs/03-protocol.md` §2（QUIC 传输映射）、§3（音频数据报格式）|
| 设计上就不出局域网：**跨网段、跨公网明确不做** | `docs/manual/troubleshooting.zh-CN.md:88` |
| 本机监听 `0.0.0.0:58290`（默认全网卡）：**同一局域网内的任何设备都能发起 TLS 握手并建立会话** —— 本版本没有准入校验（有意的取舍：局域网内不设安全需求），能连上即通 | `core/crates/audiolink-engine/src/runtime.rs:206,252`（默认监听）、`core/crates/audiolink-types/src/lib.rs:30`（端口常量）、`docs/72-remove-pairing.md` |
| 多设备推送（FR-12/M3）只是"把同一路声音同时发给**你点名的**那几台"，不含任何第三方 | `docs/33-m3-multi-session.md`（`start_send_many`）|

---

## 3. 本地存了什么

**只存“设备身份 / 界面偏好”这两类，不存音频**（旧版本还存过信任库文件，见下表「历史遗留」行）。

| 位置 | 内容 | 为什么 | 证据 |
|---|---|---|---|
| 桌面 `%APPDATA%\com.gotkicry.audiolink\identity\` | 本机自签证书 + 私钥（`cert.pem` / `key.pem`） | 设备身份（指纹即 `NodeId`），下次启动复用 | `desktop/src-tauri/src/engine_bridge.rs:855-875`；`runtime.rs:202-203, 741` |
| 桌面 `...\trust.json`（**历史遗留**） | 旧版本留下的**信任库**文件（曾配对对端的指纹、显示名、平台、配对时间）。**当前版本不读、不写、不生成它**，留着也无影响；介意可手工删除 | 它服务的「免 PIN 重连」判据已随配对机制一起移除，见 `docs/72-remove-pairing.md` |
| 桌面 `...\settings.json` | `auto_connect`（启动自动连上次的主机）、`auto_broadcast`（有人接入时自动开始推流，**默认开**）、`locale`（语言）、`last_peer`（**上次成功连接**的地址）| 界面偏好与便利性 | `engine_bridge.rs:44-52`（键名）、`:206-228`（读写）、`:466-513`（命令）、`auto_broadcast_state` / `set_auto_broadcast` |
| Android 应用私有目录（`filesDir`） | 同上：本机证书 + 私钥（以及旧版本可能留下的信任库文件，当前不读写） | 同上；清单声明 `allowBackup="false"`，**不参与系统云备份** | `android/.../service/AudioLinkService.kt:253, 435`；`AndroidManifest.xml`（`allowBackup`）|
| 桌面 `%USERPROFILE%\AudioLink\telemetry-<时间戳>.csv` | 遥测数字（丢包/抖动/缓冲水位…），**仅在你点"导出"时写入** | 你自己要看/要发给别人排障 | `engine_bridge.rs:1317-1331`（目录与写文件）、`:632`（命令）、`view.rs:255`（CSV 渲染）|

**没有的东西**（同样是可核对的"不存在"）：没有账号/登录、没有云端配置同步、没有音频缓存文件、
**没有日志文件**（引擎不写盘；Android 侧 `Log.*` 调用零命中）、没有崩溃上报 SDK（§4）。

---

## 4. 有没有遥测上报：**没有**

| 类别 | 事实 | 证据 |
|---|---|---|
| 链路遥测（RTT / 丢包 / 抖动 / 缓冲水位 / 欠载）| **只在本机界面显示**；另外两端引擎之间以 1 Hz 互发 `STREAM_STATS`（控制帧），那是**与你连接的对端**交换，不是发给第三方 | `docs/03-protocol.md` §10（定义）；`runtime.rs` 会话循环的 1 Hz ticker；面板 `docs/25-m2-telemetry-panel.md` |
| 远程上报 / 分析 / 崩溃收集 | **不存在**：全仓 grep `analytics` / `sentry` / `crashlytics` / `firebase` 等零命中 | 依赖清单见 `docs/41-compliance-license-audit.md` 与 `docs/compliance/THIRD-PARTY-NOTICES.md` |
| 唯一的对外网络请求 | 桌面「**检查更新**」——**只由你点击触发**，GET GitHub Releases 的 `latest.json`（后端**不做**启动期自动检查）| `desktop/src/lib/ipc.ts:110-116`（注释与命令）、`UpdatePanel.tsx:36`、`desktop/src-tauri/src/update.rs:87`、端点 `desktop/src-tauri/tauri.conf.json`（`plugins.updater.endpoints`）|
| 连带的第三方可见信息（如实写）| 点「检查更新」时，**GitHub 会看到你的 IP 与请求**（这是任何"向某地址要文件"的固有代价）；Windows 首次运行时系统会弹一次**防火墙放行提示**（UDP 监听） | `docs/manual/troubleshooting.zh-CN.md:28`（防火墙提示）、`docs/08-ui-spec.md:256` |
| 安装期 | 安装包若需下载 WebView2 运行时，由**安装器**向微软下载（不是应用行为） | `desktop/src-tauri/tauri.conf.json`（`webviewInstallMode: downloadBootstrapper`）|

---

## 5. 权限逐条（Android；桌面端不申请任何权限）

来源：`android/app/src/main/AndroidManifest.xml`（**全量清单，一条不多**）。

| 权限 | 为什么需要 | 代码/清单依据 |
|---|---|---|
| `INTERNET` | 局域网内建立 QUIC 连接、收发音频 | 会话建立 `runtime.rs:1389-1400`（`Engine::connect`）|
| `ACCESS_NETWORK_STATE` / `ACCESS_WIFI_STATE` | 判断网络可用性与当前 Wi-Fi 状态（连接、显示地址）| 清单注释「局域网通信」|
| `CHANGE_WIFI_MULTICAST_STATE` | 为 mDNS 组播接收预留（`MulticastLock`）| **如实说明**：发现协议的信标编解码已在 `core/crates/audiolink-proto/src/discovery.rs:24,163` 定义，但**当前产品没有接入发现**（全仓没有构造/收发信标的调用点，Android 侧也没有 `NsdManager` / `MulticastLock` 代码）→ 该权限目前**未被使用** |
| `FOREGROUND_SERVICE` + `FOREGROUND_SERVICE_MEDIA_PLAYBACK` + `FOREGROUND_SERVICE_CONNECTED_DEVICE` | 常驻前台服务，保证后台受控运行、通知可见（不被系统静默杀掉）| 服务声明 `foregroundServiceType="mediaPlayback\|connectedDevice\|microphone\|mediaProjection"` |
| `FOREGROUND_SERVICE_MICROPHONE` | **麦克风采集期间**必须处于该类型前台服务（Android 14+ 强制）| 清单注释（FR-07）；服务侧类型映射 `service/AudioLinkService.kt:617-636` |
| `FOREGROUND_SERVICE_MEDIA_PROJECTION` | **系统内录**：系统要求"取投影之前"已进入该类型前台服务 | 同上（FR-06）|
| `RECORD_AUDIO` | 麦克风采集（**只有你选「麦克风」时才申请**，运行时授权）| `capture/CaptureController.kt:85-86`；拒绝/被收回走 `CaptureError.kt:14,142` |
| `POST_NOTIFICATIONS` | 显示播放/采集通知（Android 13+ 运行时授权）；被拒也有媒体通知豁免 | 清单注释 |
| `WAKE_LOCK` | 播放/采集期间防止 CPU 休眠造成断音 | 清单 |

---

## 6. 怎么关闭 / 撤回

| 你想做的事 | 怎么做 | 依据 |
|---|---|---|
| 停止发送声音 | 界面「停止推流 / 断开」→ 采集线程停止、采集源释放（麦克风/内录随即不再被读取）| `runtime.rs:1465`（`stop_send`）、`:3011`（`stop_capture`）、Android `CaptureController.kt:95`（停止会唤醒阻塞的读取）|
| 回收 Android 的录音权限 | 系统设置 → 应用 → 权限 → 麦克风/屏幕录制 关闭；**内录每次会话都要重新授权**（系统行为，绕不过）| `capture/CaptureError.kt:20,142`（`PermissionRevoked` 是正常路径）、`CaptureStateMachine.kt:116`、`SystemLoopbackCaptureSource.kt:16` |
| 停止后台服务 | 通知栏停止（前台服务）| `AndroidManifest.xml` 服务声明 + 通知 |
| 关掉开机自动连接 | 设置面板的「启动时自动连接上次的主机（本机作为接收端）」开关 | `engine_bridge.rs:503-513`（`set_auto_connect`）、`settings.rs` |
| 关掉「接入即自动推流」 | 设置面板的「有人接入时自动开始推流」开关（默认开）；关掉后回到手动点「开始推流」 | `set_auto_broadcast` 命令、`settings.json` 的 `auto_broadcast` 键 |
| **限制谁能接入（没有信任列表）** | 本版本既没有配对码、也没有信任列表：**任何与本机网络互通的设备都能连上并接入**。想限制只有两条路：① 关掉「有人接入时自动开始推流」（`settings.json` 的 `auto_broadcast`）—— 该开关**只对桌面端生效**（桌面默认开、接入即出声；**Android 端当前不会自动开始推流**，要出声须手动点「开始发送」，见 `docs/72-remove-pairing.md` §5）；② 不要让 AudioLink 的 QUIC 端口（默认 UDP 58290）暴露在你不信任的网络上 | 见 `docs/72-remove-pairing.md`；设置键见 §3 的 `settings.json` 行 |
| 清掉旧版本的信任库文件（历史遗留） | 不需要清理 —— 程序**不再读写** `trust.json`。想清干净可手工删除（桌面 `%APPDATA%\com.gotkicry.audiolink\trust.json`、Android 应用私有目录内同名文件），或在系统里「清除数据 / 卸载」 | 信任库随本轮去配对整体移除，见 `docs/72-remove-pairing.md` |
| 换一个设备身份 | 删除身份目录（桌面 `...\identity\`）→ 下次启动重新生成证书，指纹随之改变，对端下次连接会把它认成新节点 | `runtime.rs:741`（`load_or_create`）|
| 清掉"上次设备"记录 | ⚠️ 同样没有界面出口；可编辑/删除 `settings.json` 里的 `last_peer` 键 | `engine_bridge.rs:50, 216-218` |

---

## 7. 核对结论：哪些说法成立、哪些要改（不美化）

**成立（本轮逐条核过）**：

1. `docs/manual/user-guide.zh-CN.md:13`「只在局域网内工作：不经过任何服务器，不上传音频内容，不采集遥测到云端」——
   "不经过服务器" ✔（§2）、"不采集遥测到云端" ✔（§4）。其中"不上传音频内容"的**准确含义**应是
   「音频不发给任何第三方，只发给你连上的对端」（音频当然会被本机采集，否则功能不成立）。

**需要改的地方（本文只记录，不改上游文档）**：

| # | 位置 | 问题 | 建议 |
|---|---|---|---|
| 1 | `docs/05-roadmap.md` §M5 交付物 5 | 「隐私说明（**不采集音频内容**、不上传遥测）」——**前半句与产品核心功能矛盾**：本产品的功能就是采集音频并定向推送。“不采集”若被当真，等于说功能不存在 | 改为「音频只在你连上的设备之间直连传输，不上传第三方」 |
| 2 | `docs/manual/user-guide.zh-CN.md:152` | 「Android 端系统内录与麦克风采集：**尚未实现**」——**过时**：采集已实现并接进服务生命周期（提交 `1c4ac55` / `4c2d21a` / `a0158a2`；`CaptureController.kt:85-89`；清单里的 `FOREGROUND_SERVICE_MICROPHONE` / `FOREGROUND_SERVICE_MEDIA_PROJECTION`）。**真机验证仍未做** | 改为「已实现（内录需 Android 10+），真机验收未完成」 |
| 3 | 产品侧 | **取消配对没有界面出口**（§6）—— 该缺口已随本轮「移除配对与信任库」整体消解：不再存在“配对的设备”这个概念，连接即用、断开即止 | 无需新增「移除设备」入口；旧版本残留的 `trust.json` 可手工删除（§3 / §6）|

---

## 8. English summary

AudioLink has **no server, no account, no cloud**. Audio is captured only on a sending device,
only from a source you explicitly select (Windows system-output loopback; on Android: system
playback capture via MediaProjection, or the microphone), and is sent **peer-to-peer** over QUIC
with TLS 1.3 and mutual certificates to the device you connect to. Nothing is relayed through any
intermediary, nothing is written to disk (the engine makes no filesystem writes), and **no
telemetry is ever uploaded** — the only outbound request is the update check that **you** trigger,
which fetches a manifest from GitHub Releases. Locally it stores only its own certificate/private
key and UI preferences (an older build also kept a trust-store file, which this revision neither
reads nor writes); Android declares `allowBackup="false"`. TLS still proves “the peer is the
certificate it presents” — it just no longer decides whether that certificate is worth trusting:
there is no trust store, no whitelist and no pairing code.
There is no allow-list that could restrict peers either: any device that can reach this machine QUIC port
(UDP 58290 by default) is accepted. On the desktop, "start streaming automatically when a device connects" is
on by default; on Android streaming never starts by itself today - you must tap "Start sending" (see
`docs/72-remove-pairing.md`). Turn the setting off, or keep the port off untrusted networks, if that matters to you.
The MediaProjection consent screen is used **only** to capture audio playback, never the screen
(no `VirtualDisplay` anywhere in the tree). Known gaps, stated plainly: Android capture is
implemented but **not yet device-validated**.

---

## 9. 维护纪律

本文是**可核对的说明书**，不是营销材料：

1. 任何"我们不做什么"的句子，必须能指到代码/清单；**发现与实现不符时改本文或改实现，而不是改措辞**；
2. 新增权限、新增落盘文件、新增任何外发请求 → 必须在同一次变更里更新本文（§3/§4/§5）；
3. 发现协议（mDNS）若将来接入产品，§5 的 `CHANGE_WIFI_MULTICAST_STATE` 与 §1/§2 必须同步说明"会向局域网广播什么"。
