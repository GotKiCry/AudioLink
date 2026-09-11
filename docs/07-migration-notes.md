# 迁移说明：从 AudioPipe 到 AudioLink

> 本文回答两个问题：**继承什么**、**绝不再犯什么**。
> 证据来源（只读分析报告，共 4 份，约 3800 行）：
> - `C:\_Project\_cache\AudiolinkPlan\windows-analysis.md`（Windows/WPF 端，565 行）
> - `C:\_Project\_cache\AudiolinkPlan\android-analysis.md`（Android/Java 端，1501 行）
> - `C:\_Project\_cache\AudiolinkPlan\musiche-and-build-analysis.md`（musiche / 构建 / 上游关系，970 行）
> - `C:\_Project\_cache\AudiolinkPlan\rust-stack-research.md`、`android-stack-research.md`、`benchmark-research.md`（技术选型调研）

---

## 1. 上游事实（必须先说清楚）

| 事实 | 结论 |
|---|---|
| `C:\_Project\AudioPipe` = `HeHang0/AudioShare` **master tip 的逐字节镜像**（37 commits 全属原作者，tree SHA 与上游一致，自有提交 0） | AudioLink 是一次**从零重写**，不背历史包袱，也不存在"改造旧分支"的路线 |
| 许可证 **Apache-2.0**（非 GPL，无 copyleft 传染） | 允许二开与再发布，义务为：保留版权与许可声明、附 LICENSE、**标注修改**、不得使用其商标 |
| 上游最后 tag `v1.7.9`，2024-05 停更；`windows/` 自 2024-03-28 起零演进 | 上游不会替我们修任何东西 |
| 上游 `origin` = `GotKiCry/AudioPipe` → GitHub API 404（私有或已删） | 不作为依赖来源 |

**合规动作（AudioLink 仓库必须带）**：
1. `LICENSE` = Apache-2.0 全文；
2. `NOTICE` = 声明"本项目基于 HeHang0/AudioShare 的功能与协议设计重新实现，原项目版权归原作者，许可证 Apache-2.0"，并列出被参考的设计点；
3. `README` 顶部一句话标注来源与差异；
4. **不使用** AudioShare / AudioPipe / picapico / phicomm 等名称与图标；
5. **不复刻**任何第三方非官方 API 调用代码（见 §4）。

---

## 2. 继承：值得保留的设计思想

> 说明：AudioLink 换语言栈（Rust + Kotlin）从零实现，**不拷贝上游代码**；下表是"思想级继承"，并在继承时逐条修正其缺陷。

| 上游设计 | 继承方式 | 修正点 |
|---|---|---|
| **声道分流**（立体声 / 左 / 右） | 语义保留为 FR-03，并扩展"左→A 设备 + 右→B 设备"双路 | 上游在采集回调里手写 4 字节步长拆包；新实现用 SIMD 友好的 `deinterleave` |
| **远端音量遥控** | FR-24：发送端可调接收端音量 | 上游用"每次命令新建 TCP 短连接"，音量滑块可造成连接风暴；改为 QUIC 可靠流上的持久会话 |
| **托盘常驻 + 开机自启 + 单实例** | FR-30/31 | 上游 `ShellLink.CreateShortcut(path,"startup")` 第二参实际是命令行参数（用错）；Tauri 插件实现 |
| **UDP 局域网自发现** | FR-16 兜底发现（mDNS 为主） | 上游 Windows 侧因 26 字节阈值 bug **100% 失效**；新协议载荷带长度前缀与版本字段 |
| **单文件便携分发**（免安装） | 桌面端同时产"安装包 + 便携版" | 上游 Costura 单文件 + 5 个文件同目录约定（含 apk/adb）；新方案不依赖同名约定 |
| **配置持久化在用户目录** | FR-33 窗口状态/白名单持久化 | 上游 `%APPDATA%\AudioShare\audio.share.config.json` 全静态单例；改为显式配置模块 + 原子写 |
| **双语 + 深浅色** | FR-33/39 | 上游 24 个语言 key 硬编码在 XAML；新方案用 i18n 资源文件 + 双语齐备校验 |

---

## 3. 摒弃：上游缺陷清单与根治方案

### 3.1 协议层（旧协议无法演进）

| 上游缺陷 | AudioLink 方案 |
|---|---|
| 无版本号、无能力协商、无鉴权字段（魔数 + 1 字节命令） | 握手携带 `proto_version` + 能力位图 + 节点身份（证书指纹） |
| 控制命令每条新建 TCP 连接、无状态码、无应答 | QUIC 可靠流上的持久控制会话 + 请求/响应 ID + 明确错误码 |
| 音频帧无序号、无时间戳，丢帧只能"整帧丢弃" | 每包带 `stream_id + seq + capture_ts`，支持重排、PLC、时钟同步 |
| 心跳语义混乱（4 字节零长帧；Windows 忽略对端心跳） | 独立 `keepalive` 数据报 + RTT 探测，双向都用于存活判定与时钟同步 |
| 音量 0/静音处理靠 `lastPCVolume` 猜测（曾把音量锁在 1/15） | 显式 `gain` 与 `mute` 状态，双向同步且幂等 |

### 3.2 音频链路

| 上游缺陷 | AudioLink 方案 |
|---|---|
| 无抖动缓冲，"忙则丢整帧"（PC 与 Android 各丢一次） | 自适应抖动缓冲（目标深度 2 帧，动态 20–60 ms） |
| 采集强制 `WaveFormat(48000,16,2)`，引擎缓冲 100 ms 且无重采样策略 | 采集用事件驱动 + 内部统一 48 kHz f32 重采样，显式配置缓冲 |
| 位置同步靠事后 `seek`（150 ms 死区），有可听瑕疵 | 预约播放：按校正时钟在目标时刻提交首帧（FR-21） |
| `AudioTrack.flush()` 在 PLAYING 下是 no-op，仍被当作清缓冲手段 | 播放器状态机显式管理：`STOP→FLUSH→REBUILD`，仅重建路径调用 |
| `mAudioTrack` 跨线程无同步、`isWriting` 由主线程复位且无 try/catch → **一次异常永久静音** | 播放器封装为单线程 actor（消息驱动），异常必走上报与重建（FR-28） |
| `setVolume(0)` + `lastPCVolume=1` 初始化缺陷 | 音量状态显式初始化为"跟随发送端/本机"二选一，无魔法值 |
| 采样率/位深不可配（192 kHz 源用 16 bit 白耗带宽） | FR-02 档位化 + 遥测可见 |

### 3.3 网络与发现

| 上游缺陷 | AudioLink 方案 |
|---|---|
| Windows 侧 UDP 发现 100% 失效（`Buffer.Length > 26` 阈值未随三字段报文更新） | 发现报文带版本+长度前缀，协议变更走单元测试与跨端契约测试 |
| `UdpClient` 端口扫描循环不 break → 句柄泄漏 | 单 socket + 组播/广播复用，生命周期由 RAII 管理 |
| `Speaker.ReadAsync` 对端关闭后无限空转烧 CPU | 读取任务在流关闭时退出，用 `select!/await` + 明确的连接状态机 |
| 无 KeepAlive、无重连退避 | QUIC 连接迁移 + 指数退避重连（上限 3 s 恢复，FR-27） |
| 无加密无鉴权（Android `LocalServerSocket` 全局命名空间，同机任意 App 可推流） | QUIC TLS1.3 + PIN 配对 + 白名单；本机回环也走身份校验 |

### 3.4 Android 端安全问题（旧版最大攻击面）

| 上游缺陷 | AudioLink 方案 |
|---|---|
| 内置 HTTP 服务器 45 条路由**无任何鉴权**，`/storage?key=` 可明文读回音乐平台 cookie，`/proxy` 是开放代理（SSRF），CORS `*` + `Allow-Credentials: true` | **整块删除**：v1 不提供任何 HTTP 服务面；节点通信只走 QUIC + 已配对身份 |
| `android/sign.jks` 与明文口令入库 | 密钥不入库，CI 用 Secret 注入；本地用 `~/.audiolink/keystore.jks` |
| `usesCleartextTraffic=true` | 禁止明文流量（QUIC 本身加密） |
| 声明 `RECORD_AUDIO` 却从不申请；`requestPermissions` 申请的全是 normal 权限 | 权限按需申请且与功能路径一一对应（FR-07） |
| 通知 `PendingIntent` 指向 `BroadcastReceiver` 类 → 点击必崩 | 通知动作统一走 service/activity 的显式 Intent |
| 两套通知共用 `NOTIFICATION_ID=1` 互相覆盖 | 单一通知通道，ID 常量化并加测试 |
| `startService` 而非 `startForegroundService`；`startForeground` 只在播放时调用 | 服务启动即进入前台并声明 `mediaPlayback` 类型（FR-36） |

### 3.5 平台与工程

| 上游缺陷 | AudioLink 方案 |
|---|---|
| 功能正确性依赖"用户打开过 App"（`MainActivity.onServiceConnected` 才注入依赖） | 服务自持依赖，无 Activity 前置条件（无隐性生命周期耦合） |
| 仅 `armeabi-v7a` 单 ABI；`minifyEnabled false` | 双 ABI + R8 混淆（FR-40） |
| God Class（`TcpService` 640 行）、30+ 硬编码魔数、15+ 处吞异常、**0 测试** | 单一职责模块 + 常量集中 + 禁止空 catch（lint 规则）+ 内核单元测试 + 跨端契约测试 |
| 版本号三处人工同步（csproj / build.gradle / git tag） | 单一来源（workspace 版本 + 脚本生成），CI 校验一致性 |
| CI 只用 tag 触发、actions 未 pin、adb 每次拉最新不可复现 | CI 全流程 pin + 缓存 + 锁定工具链（`rust-toolchain.toml`、Gradle wrapper） |
| Release 版 Logger 因 `#if DEBUG` 全废 | 分级日志默认开启（不含音频内容），支持导出（FR-32） |

---

## 4. 合规红线（不得复刻）

| 项 | 处置 |
|---|---|
| 网易云 `weapi`（硬编码 `encSecKey`/AES key/iv）、QQ `musicu.fcg` vkey、咪咕 `listen-url` 直链 | **不复刻**。v1 不提供云音乐能力，不做第三方平台私有接口调用 |
| 斐讯 R1：`System.loadLibrary("ledLight-jni")` + `su` 执行 **`setenforce 0`**（全局关闭 SELinux）+ 冒用 `com.phicomm.*` 包名 | **不复刻**。如未来要灯效，走公开 `Visualizer` API + 可选插件，且不要求 root |
| USB 模式下 adb `pm uninstall` / `pm install -r` 静默安装（对他人设备属未授权操作） | **不复刻**。分发改为用户手动安装 APK |
| 上游签名密钥与口令 | 不复用（且上游密钥已泄露，其产物不可作为信任基础） |

---

## 5. 兼容性立场

- **不兼容旧协议**：AudioLink 与 AudioShare/AudioPipe 之间**不能互通**（用户已确认）。
- 若未来需要互通，必须新增独立的"兼容适配层"模块，且不得把旧协议细节泄漏进内核主干（旧协议细节留在适配层内部，可单独删除）。
- 桌面端与 Android 端**必须同版本号**握手：`proto_version` 不匹配时明确拒绝并提示升级（避免旧版静默错误行为）。
