# M1 桌面采集端点选择（2026-09-16）

对应 Project「桌面 UI 缺采集端点选择器」。此前桌面只采系统默认输出；现在可先选择输出设备，再给对端推流。

## 使用方式

1. 在「采集声音 → 输出设备」选择系统默认、扬声器、显示器音频或虚拟输出设备。播放器也应使用同一个输出。
2. 连接并配对对端，点击「开始推流」。界面显示实际打开的端点名字、采样率与声道数；日志同时记录完整端点 ID 和混音格式。
3. 更换耳机或声卡后点击「刷新设备」。刷新保留显式选择；设备已断开时提示重新选择，不偷偷切到其他设备。
4. 更换音源前先停止推流。系统默认选项在每次开流时解析，Windows 默认设备改变不会迁移当前采集。选择在当前界面会话中保留，尚不跨应用重启保存。

设备名称重复时显示短 ID 以便区分，传给后端的始终是完整 ID。无法满足当前 48 kHz / 双声道 / f32 混音格式的端点会给出调整提示并禁止开流。

## 实现与错误处理

- `desktop/src-tauri/src/capture.rs` 在阻塞工作线程枚举活动输出端点，向前端只传可序列化的信息。
- `list_capture_devices` 提供端点列表；`start_send` 增加兼容的可选 `capture_device_id` 参数；`active_capture_device` 返回实际端点快照。契约同步于 `docs/11-m1-contract.md` §6。
- 采集工厂在自己的线程里创建 WASAPI 对象。设置和开流串行化，显式 ID 精确校验，缺失端点不会回退默认设备。
- 修正原有的开流错误回传：`Engine::start_send` 先前仅表示命令已入队；现在等待本地采集/编码初始化与 OPEN_STREAM 写出。采集失败直接返回错误，界面不标成推流中；同一连接可重试，重复开流返回 BUSY 并保留已有采集线程。成功仍不等于远端已经开始播放。

## 验证

- `pnpm build`、workspace Clippy（全部 targets/features，`-D warnings`）、内核完整测试通过。
- 桌面 11 个单测和 1 个真实 QUIC 接缝测试通过。接缝测试覆盖采集设备打开失败的错误码/原因回传、同连接重试、成功返回前设备已初始化、重复开流拒绝以及实际音频到达接收端。
- 显式执行 WASAPI 设备测试通过：Realtek 扬声器、NVIDIA 显示器音频、网易虚拟音频、AudioRelay Virtual Speakers 共 4 个端点；选择 ID 与实际打开 ID 一一匹配，不存在的 ID 明确失败。此测试不播放声音、不保存 PCM，CI 默认忽略。
- 实际启动 Tauri 应用，经 WebView 自动化读取 4 个真实端点，选择 NVIDIA 显示器输出并刷新，确认选择保留；1100×720、最小窗口 880×560，以及深色模式截图已检查，无横向溢出。
- `pnpm tauri build --debug --no-bundle` 生成带前端资源的桌面测试版：`target/x86_64-pc-windows-msvc/debug/audiolink-desktop.exe`。

设备测试复跑：

```powershell
cargo test -p audiolink-desktop selected_endpoint_is_the_one_opened_by_wasapi -- --ignored --nocapture
```

本地窗口验证记录与截图：`target/evidence/capture-ui/`（不入库）。自动化使用的调试端口只在临时构建配置中开启，交付测试版使用仓库正常配置。

本轮验证了端点选择、实际设备打开和采集启动失败处理；未进行不同输出设备之间的手机试听对照，M1 的端到端延迟与长时间队列验收仍独立追踪。
