# 局域网主机广播与接收端发现

## 行为

Windows 打开 AudioLink、QUIC 监听就绪后立即广播主机信息；Android 选择发送源并启动服务后同样广播，之后每 3 秒在各 IPv4 网卡的子网广播地址发送一次 UDP 58280 报文。网卡地址每次重新枚举，可覆盖以太网、Wi-Fi 和移动热点。已连接、推流中和手动暂停时继续广播；退出时回收线程。广播不触发连接或采集。

Android 接收入口和桌面「收听其他设备的声音」显示「局域网主机」列表，并提供「刷新主机」按钮。刷新会清空旧结果、重新建立 UDP 浏览器；搜索期间按钮禁用，失败后可再次刷新。条目包含名称与地址，点击后复用现有连接及 PIN 配对流程。手动地址输入保留。Android 的扫描不依赖音频服务启动，只在接收入口可见且应用处于前台时运行；取消收集时停止 Rust 浏览器并释放 Wi-Fi 接收锁。

## 实现与接口

- `audiolink-discovery` 复用 `audiolink-proto::DiscoveryBeacon`。`Advertiser` 与 `Browser` 独立，扫描端口占用不阻断主机广播或音频会话。工作线程不进入音频实时路径。
- 使用报文来源 IP 与广播携带的 QUIC 端口组成地址，不能把广播源端口当成连接端口。按短指纹 + 地址去重；10 秒过期；最多保留 128 条；过滤本机、接收专用节点、非法报文和端口 0。
- 版本不兼容的主机仍显示，但禁止从列表连接。匹配要求与当前握手一致，为完整 ALP 版本相等。
- Tauri `discovered_hosts({ refresh?: boolean })` 返回 `DiscoveredHost[]`：`idShort / name / addr / platform / protoVersion / compatible`。界面每秒刷新一次，退出接收入口停止轮询，失败时清空过期画面并自动重试。
- UniFFI `DiscoveryBrowser()`、`hosts()`、`stop()` 为 Android 提供同一份协议实现；生成的 Kotlin 绑定与双 ABI 原生库一起更新。
- 内核 `PeerStatus.initiated_locally` 记录每条真实会话的角色，并映射到桌面 `PeerView.receiving`。收听主机时不触发自动推流；入站接收设备连接、配对完成后仍自动推流。角色来自会话本身，不靠地址缓存推断。

发现报文未经认证，只提供候选地址；信任仍由证书身份和 PIN 决定。无音频、PIN、密钥或信任列表进入发现报文。当前接通 IPv4 UDP 广播，mDNS / 单播查询仍是预留方案；手动 IPv6 地址连接保持可用。

新增 `if-addrs 0.14.0` 用于跨平台安全枚举网卡（MIT OR BSD-3-Clause），`socket2 0.6.x` 用于 UDP 监听地址复用（MIT OR Apache-2.0，已有传递依赖）。许可报告及随包声明已重新生成。

## 验证

- 公共模块覆盖真实 UDP 接收、来源 IP / QUIC 端口、重复报文刷新、过期、异常报文、本机过滤、接收节点过滤、列表容量、多网卡地址和协议不兼容。
- `cargo nextest run --workspace --exclude audiolink-desktop --lib --tests`：546 项通过，2 项需硬件的测试默认跳过。
- 另行运行 `cargo test -p audiolink-discovery live_lan_advertisement_is_discovered -- --ignored --nocapture`：通过，本机网卡实际广播能被浏览器读到。
- Rust 全工作区 Clippy 通过；桌面库 39 项通过、1 项 WASAPI 硬件测试跳过。全量桌面测试命令受正在运行的开发程序锁定 EXE 影响，改用 `--lib` 完成此次改动相关测试。
- 桌面构建、中英文本一致性检查与 172 项前端测试通过；另覆盖发现连接地址、重复点击、失效刷新、失败恢复、卸载后停止轮询和接收方向不回传音频。
- Playwright 检查 1024×640 中文浅色、英文深色界面，验证长名称、版本不兼容、选择连接与错误后的手动入口；证据在 `target/evidence/lan-discovery/`。
- Android `arm64-v8a` / `armeabi-v7a` 原生库与 Debug APK 构建通过，237 项 JVM 测试通过。Kotlin 绑定与当前源码重新生成结果逐字节一致，21 个 FFI 校验符号完整；两份 APK 内原生库与当前构建经过 NDK strip 后的结果逐字节一致，许可声明亦与仓库生成物一致。记录见 `target/evidence/lan-discovery/verification.json`。

已在 PHK110 与 Windows 之间验证发现与刷新，详细证据见 [64-discovery-device-refresh.md](64-discovery-device-refresh.md)。本轮不测音频联动。跨子网、访客 Wi-Fi、AP 客户端隔离或防火墙拦截广播时，列表可能为空；保留手动连接路径。发现需 UDP 58280，默认音频端口为 UDP 58290。
