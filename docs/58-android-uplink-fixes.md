# Android「手机 → 电脑」链路：真机缺陷定位与修复（2026-09-18）

> 日期 **2026-09-18** ｜ 设备 OnePlus PHK110（Android 16 / API 36 / arm64-v8a）｜ PC 与手机同子网（192.168.3.200 ↔ 192.168.3.75）
> 上游：看板 [M4] 条目、`docs/55-device-acceptance-runbook.md` §5.2、`docs/16-android-service-lifecycle.md`
> 触发：真机实测发现「发送（手机 → 电脑）」这条链路**不可用** —— 用户操作时表现为「地址填好了按钮点不动」「输入框进不去」

## 0. 一句话

这条链路在本轮真机上**从未真正可用**：看板写的「实现侧已补齐」（6f04cbf）只补了 UI 骨架，
而**发送源入口、连接门禁、前台服务类型位**三层各有一处硬缺陷，任何一处都足以让链路走不通。
本轮定位 3 处阻断/崩溃级缺陷并修复（外加 2 处同源崩溃点），**实测验证了前四处的效果**（§3）。

## 1. 缺陷与修复（按因果链顺序）

### 1.1 发送源 UI 从未接线（阻断）

**事实**：`MainActivity.kt` 只传了 4 个回调（connect / submitPin / startSend / stopSend），
`PlaybackScreen` 没有发送源卡片，`AudioLinkService.setCaptureSource` 除测试外**零调用点**。
⇒ `captureSelection` 恒为 `null` ⇒ `SenderStateMapper.canStartSend` 恒判 `CaptureOff` ⇒「开始发送」永久禁用。
**服务侧是完整的**（`setCaptureSource → applyCaptureSource → CaptureController` + MediaProjection 回执），缺的只是 UI 这一层。

**修法**：新增 `PlaybackScreen.CaptureSourceCard`（三态：关闭 / 麦克风 / 系统内录，零新组件类型）+
`MainActivity.onSelectCapture`（麦克风走 RECORD_AUDIO 运行时权限；内录先登记再拉起 `MediaProjectionRequestActivity`）。

### 1.2 连接门禁死锁（阻断）

**事实**：`SenderStateMapper.canConnect` 读 `sender.targetAddr`（**服务侧**回填副本），
而它只在 `connect()` 被触发之后才有值 ⇒ 地址进了输入框但没进服务 → 按钮禁用 → 连接触发不了 → 服务侧永远是空 → **按钮永远点不动**，
界面还挂着「先填电脑的地址（例如 192.168.1.5）」。
单测全绿是因为 `sender()` 辅助函数默认给了非空地址，把错误设计一起固化了。

**修法**：`canConnect(sender, engineRunning, targetAddr)` 显式收**输入框当前值**；
新增回归测试 `connectGateReadsTheTextFieldNotTheServiceCopy`（真机场景：服务侧副本为空 + 输入框有值 → 必须 Allowed）。

### 1.3 前台服务类型位：授权前带上 mediaProjection → 崩进程（崩溃）

**症状串（adb logcat crash buffer 原文）**：

    java.lang.SecurityException: Starting FGS with type mediaProjection callerApp=... targetSDK=36
    requires permissions: allOf=true [android.permission.FOREGROUND_SERVICE_MEDIA_PROJECTION]
    any of the permissions allOf=false [android.permission.CAPTURE_VIDEO_OUTPUT, android:project_media]
    at com.gotkicry.audiolink.service.AudioLinkService.onStartCommand

**根因**：错误串里的 `android:project_media` 是 **AppOp**（permission 会写成 `android.permission.…`），
只在用户于 MediaProjection 授权页点「允许」**之后**才由系统授予。
而旧代码在**用户选内录那一刻**就把该类型位加进 `startForeground` ⇒ **必崩**。

**两个放大因素**：
1. `ACTION_SET_CAPTURE_SOURCE` 里 `startForegroundWithTypes()` 排在 `applyCaptureSource(requested)` **之前** ⇒
   从内录切到麦克风的那一拍，仍以**旧的 SystemLoopback** 刷了一次前台 —— 这就是「选麦克风也崩」的真机制；
2. Manifest 已声明 `FOREGROUND_SERVICE_MEDIA_PROJECTION` 也没用 —— 缺的是 AppOp 不是 permission。

**修法（三处，配套）**：
- `loopbackAuthorized` 标志：类型位**只在授权回执到达后**才带（`foregroundServiceTypes()` 里门禁）；
- `handleCaptureGranted` 顺序改为「先 startForeground(带位) → 再 `getMediaProjection()`」——
  Android 14+ 的校验正在这一步，而 AppOp 此时已在；
- `ACTION_SET_CAPTURE_SOURCE` 改为「先落新选择 → 再刷前台」。
- 对称崩溃点：microphone 类型位加 `RECORD_AUDIO` 运行时门禁（用户撤权后同样会崩）；
  切到麦克风时清 `loopbackAuthorized` + 停旧投影（Android 14+ 每会话重新授权，旧投影复用会再崩或静默无声）。

### 1.4 采集状态机：PermissionGranted 从未被发送（阻断，且**让排障盲飞**）

**事实**：`CaptureStateMachine` 的 `Request` 直接把状态置为 `AwaitingPermission`，而 `PermissionGranted` 事件
在生产代码里**没有任何发送方**（只在 test 里），`StartSucceeded/StartFailed` 又都不接受 `AwaitingPermission`。
⇒ 状态**永远出不去**：UI/通知恒显示「等待授权」；**失败完全静默**（权限被拒、设备被占用、格式不支持全看不见）；
`nextRetryAtMs` 恒为 null ⇒ 退避重试形同不存在（一次即放弃）。

**修法**：`CaptureController.start()` 里 `onEvent(Request(kind))` 之后补一发 `PermissionGranted`（调用方已保证授权到手，见其 KDoc）。

### 1.5 connecting 不收敛 → 输入框永久锁死（阻断）

**事实**：`connectSender` 设 `connecting = true` 后，协程的早退分支 `if (destroyed || !engineLifecycle.isCurrent(generation)) return@launch`
**不收敛 connecting**；而 `EngineLifecycle.start()/stop()` 都 `++generation`，切发送源就会 `stopEngine(); startEngine()`。
⇒ 一次「连接飞行中切源」就让 `connecting` 永远为 true：输入框受 `enabled = !sender.connecting` 控制而**永久禁用**、
按钮恒显示「连接中…」，用户从此既改不了地址也点不动按钮，**只能重启服务**。

**修法**：早退分支无条件 `senderState = senderState.copy(connecting = false)` 并刷新。

### 1.6 卡片顺序：先连接后选源 = 连接被引擎重启掐断（阻断）

**事实**：「关闭 ⇄ 非关闭」必须重启引擎（`capture` 是引擎启动参数），而引擎 shutdown 会关掉**全部会话**。
UI 原先把发送卡排在发送源卡之前 ⇒ 用户按自然顺序「先连接、后选源」必然踩中，刚配对好的连接被掐断。

**修法**：UI 调换两张卡顺序（先选源、后连接）。**内核侧解耦 capture 留待后续**（见 §4）。

## 2. 修复清单（文件 → 改动）

| 文件 | 改动 |
|---|---|
| `ui/PlaybackScreen.kt` | 新增 `CaptureSourceCard` + `SourceButton` + `sourceLabel`；`onSelectCapture` 参数；发送源卡排到发送卡之前 |
| `MainActivity.kt` | `onSelectCapture`（RECORD_AUDIO 权限请求 / 内录授权页拉起）+ 权限回调 |
| `service/SenderUiState.kt` | `canConnect` 收 `targetAddr`（读输入框，不读服务侧副本） |
| `service/AudioLinkService.kt` | `loopbackAuthorized` 标志与门禁；授权回执顺序；`ACTION_SET_CAPTURE_SOURCE` 顺序；`connecting` 收敛；切麦克风清投影 |
| `capture/CaptureController.kt` | `start()` 补发 `PermissionGranted` |
| `test/…/SenderStateMapperTest.kt` | 3 处调用点更新 + 新增真机回归用例 |

## 3. 实测验证（不是推演）

| 项 | 证据 |
|---|---|
| 服务不再崩 | 装包后启动服务 → `adb logcat -b crash` **空**；`dumpsys activity services` 显示 `isForeground=true` |
| 类型位正确 | `types=0x00000092` = mediaPlayback \| connectedDevice \| **microphone** |
| 采集真的起来了 | UI 显示 **「麦克风：正在采集」**（修复前恒为「麦克风：等待授权」，且那是**假状态**） |
| 卡片顺序生效 | dump 中发送源卡 y=475，发送卡 y=1394 |
| 单测 | `testDebugUnitTest` BUILD SUCCESSFUL（含新增回归） |
| APK | `assembleRelease` BUILD SUCCESSFUL |

## 4. 已知未修（已定位，留给后续）

按严重度：
1. **引擎重启时序洞**：引擎启动窗口内切源不会重启 ⇒ 引擎以 `capture=null` 落地 ⇒ 无 `CAN_SEND` ⇒ 点发送得
   `cap_unsupported`，而 `canStartSend` **不看** `engineCanSend` ⇒ 按钮照旧可点（「选了源却不工作」的持久坏态）；
2. **「假已连接」**：引擎重启后会话表长期为空，UI 仍显示「已连接 · 未发送」；
3. 地址输入体验：提示文本排在输入框**之前**的条件子项会让 Column 槽位位移、丢焦点/IME；edge-to-edge 下缺 `imePadding`；
4. 重复连接同址恒 1009 BUSY，文案却说「可以再试一次」；麦克风权限被拒**零反馈**；
5. `CaptureRing` 的 close 口径不一致（KDoc 要求 stop 即 close，实现只有 close 才关）。

## 5. M4 根因已定位：**问题在桌面端（Tauri 外壳）的接收路径**，不在手机、不在网络

「手机 → 电脑」在真机上超时，本轮用**四组独立实验**把它钉死：

| # | 实验 | 结果 | 排除了什么 |
|---|---|---|---|
| 1 | 裸 UDP 探针占住 PC 的 58290，手机点「连接电脑」 | **收到 9 个 1200 B QUIC Initial 包**（`cd 00 00 00 01 …`，来自 192.168.3.75） | 「手机没发包」「包到不了 PC」「AP 客户端隔离」「Windows 防火墙」 |
| 2 | `latency-probe probe 192.168.3.200:58290`（PC 自连） | 5/5 成功，RTT P50 648 µs | 「PC 侧没人监听 / 引擎没起来」 |
| 3 | `latency-probe probe 192.168.3.75:58290`（PC 探手机） | 5/5 成功，RTT P50 11.2 ms | 「PC → 手机 方向不通」 |
| 4 | **`link-loop` / `soak-runner`（同进程两个真引擎，完整 §5 + PIN）** | **全部通过**（link-loop P50 49.3 ms；soak 判定 ok、丢包/欠载/掩盖全 0） | 「内核的 §5 / Responder 路径有缺陷」 |

**再加上直接证据**（桌面端 stdout 的 TRACE 日志，用 `cmd /c "AudioLink.exe" > log 2>&1` 抓到）：

- TLS 握手**完整成功**：收到 `ClientHello` → `ServerHello` → `CertificateRequest` →
  **`client CertificateVerify OK`** → `Finished` + `NewSessionTicket`；
- QUIC 连接**已经建立**，并有应用数据往返（`got Data packet (40 bytes) from 192.168.3.75:58290`）；
- **但整份日志里没有任何 accept / 会话 / playout 的应用层记录**，只有 `quinn::connection drive; id=0`
  （id=0 是**出站**那条连接的序号）；
- 最后手机关闭：`got frame Close(Application(ApplicationClose { error_code: 0, reason: b"" }))`。

**结论**：入站连接**没有被 `Engine::spawn_accept_loop` 接受**（否则至少会留下 `inbound without cert` 或会话建立的痕迹），
于是应用层 HELLO 永远不交换，双方各自超时。

**独立复现**：用 `device-link run --peer 192.168.3.200`（PC 自己做发起方，与手机同角色）连同一个桌面接收端 ——
**同样 `1008 handshake timed out`**。所以要修的是**桌面端**，与 Android 侧无关。

**注**：桌面端 `engine_bridge.rs:804` 确实调了 `engine.spawn_accept_loop()`，配置里 `playout` / `capture` 工厂都在
（`engine_config` 955-967），日志也有 `engine ready addr=0.0.0.0:58290`。**它之后的路径没有留下任何证据** ——
这是下一步的入口：accept loop 是否真在跑、其 task 是否被 Tauri 运行时饿死、
或 `Endpoint::accept()` 返回后 `create_session` / `run_session` 是否静默失败。

⇒ **M4 的状态**：Android 侧已修到**可执行且实测有效**（§3），**桌面接收端仍缺一次成功的入站会话**；不能写「双向达标」。

## 6. 附带发现：键盘遮挡「连接电脑」按钮（真机可见性缺陷）

真机截图证据：填入地址后软键盘弹起，**「连接电脑」按钮被挤出屏幕**（截图里只见输入框与键盘）。
根因是 `PlaybackScreen` 的 Column 只有 `fillMaxSize() + verticalScroll()`，**没有 `imePadding()`**，
而 targetSdk 36 + Android 15+ 强制 edge-to-edge ⇒ Manifest 的 `adjustResize` 失效。
**这一条同时解释了人工操作时的「输入地址后按钮点不动」**，也与 autotest 侧「adb 点不动输入框」是两件事
（后者已由 §1.5 的 `connecting` 收敛修复解决 —— 修复后 adb 输入实测可用）。
