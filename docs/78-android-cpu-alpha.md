# Android 播放 CPU 优化与 1.0.2-alpha.1

日期：2026-11-06 开始排查，2026-11-09 完成最终包复核。设备：PHK110 / Android 16 / arm64-v8a。本文只覆盖接收路径开销和 alpha 测试交付，不重写历史延迟结论。

## 背景与定位

真实设备上的 `dumpsys cpuinfo` 曾测到 AudioLink 进程约 97–100%（100% 约等于一个逻辑核），不能把这个数字当作整机八核占用。`target/evidence/phk110-latency/` 保存了线程、服务、日志与 `device-link` 原始证据。

可直接定位的开销点：

- 状态灯以连续动画驱动 Compose 重组与 RenderThread；
- `LowLatencyPlayer` 空闲时每 1 ms 唤醒，且每轮调用 `AudioTrack.getUnderrunCount()` 进入 JNI；该欠载读数没有参与播放控制；
- PCM 跨 UniFFI 使用 `Vec<f32>` / `List<Float>`，20 ms / 双声道一帧对应 1920 个装箱样本，即每秒约 9.6 万个装箱对象。

`device-link` 的退出码证明连接、推流和对端遥测成立，不单独证明设备 PCM 回调或声学出声。因此本轮另用服务 `dump` 的 `pcmFeed`、PCM 环和 AudioTrack 计数交叉核对，而非只看工具退出码。

## 实施的修复

1. `StatusLamp.kt`：呼吸动画改为 250 ms 离散更新，仍保留 1.6 s 周期和系统减少动画设置。
2. `LowLatencyPlayer.kt`：只有真正空闲时才睡 4 ms；欠载计数并入既有的 32 轮快照。保留按水位补帧，不改成固定周期补帧。
3. `audiolink-ffi` 与 Kotlin 两侧：双向 PCM 改成 `Vec<u8>` / `ByteArray`，内容是小端 f32、48 kHz、双声道交错；用 `ByteBuffer` / `FloatBuffer` 批量搬运，重新生成并校验 UniFFI 绑定。这是进程内 FFI 接缝变更，**没有改冻结的 wire schema**。
4. 曾试过的 Rust 播放自旋改动因同步组与水位回归已撤回，最终 `core/crates/audiolink-engine/src/runtime.rs` 不在本交付的差异里。
5. 真机安装发现预发布的 `versionCode` 被旧派生逻辑错算为 10000：`2-alpha` 整段转整数失败后退为 patch=0。现在先移除 SemVer 预发布后缀，`1.0.2-alpha.1` 与 `1.0.2` 均为 10002；已重建、重装核验，不发布最初错误包。

## 最终包实测

命令使用真实 `device-link` 与合成 440 Hz 声源，10 ms 帧 / 10 ms 缓冲。该路径验证网络、解码和 PCM 搬运，不代表真实系统音频采集。

- `1.0.2-alpha.1` arm64 APK 覆盖安装成功；设备读取 `versionCode=10002`、`versionName=1.0.2-alpha.1`。
- 设备 APK SHA-256 与本地包一致：`232559cbb8e262dc3f5229e73f44eabb233e6366572e8305b27c82246f9ede8f`。
- 45 s 与 30 s 的真实 QUIC / Opus 观测均退出 0，丢包末值 0.00%。
- 播放中 `pcmFeed`：P50 10.0 ms / P95 10.1 ms / 最近窗口无 gaps；设备欠载、PCM 环溢出、引擎满队列拒收均为 0；不是只建立了网络连接。
- 最终包另跑 20 s 流（进程计时墙窗 23 s），`/proc/<pid>/stat` 的 user+system 增量为 6.89 CPU 秒，平均约 **0.300 核**。这是单个短窗，不能与不同长度、不同后台负载的历史读数当作严格同条件降幅，也不能声称已证明某个更低的 CPU 指标。
- 四环同源通过：Kotlin 绑定 ↔ Rust 源码、jniLibs、target 的 Android release 库、双 ABI APK 内嵌库均为 20/20 checksum。
- Android JVM 单测、assembleDebug 和双 ABI assembleRelease 通过；Windows 便携 ZIP 与 NSIS 安装包均来自 `1.0.2-alpha.1` 构建。
- 最终全量 Rust 门禁为 **575 通过、3 跳过、退出码 0**，其中 `queue_reject_burst` 首轮 B=12、自动重试 B=0，记 1 个已知负载敏感 flaky；没有把它写成每次都稳定通过。

原始证据：`target/evidence/phk110-latency/alpha-1.0.2-*`、`device-link-alpha-1.0.2*.json`；发布校验和：`out/test-1.0.2.alpha.1/SHA256SUMS.txt`。构建与证据目录不入库。

## 发布取舍与未验清单

- GitHub tag / Release 使用合法 SemVer **`1.0.2-alpha.1`**，并标记 **pre-release**；不是正式稳定版。用户早先写的 `1.0.2.alpha.1` 仅保留在本地便携 ZIP 文件名。
- pre-release 不进入 Android 的 `releases/latest` 稳定版更新查询，也不替换桌面稳定版的 `latest.json`。本轮未配置 Windows updater 签名，不能宣称已经推送 Windows 自动更新。
- 已存在的桌面实例会接收第二进程的单实例转交；本轮没有结束用户实例来完成全新独立启动验证。
- 未做声学端到端 P50/P95，也未做最终包长时间 soak；`device-link` 的模型下限不是声学端到端结果。
- 32 位 APK 只验证构建、绑定与内嵌库同源，未在 32 位真机安装。
- Android 全量 Lint 仍有既有 UniFFI `Cleaner` 的 API 33 检查错误；本改动未处理或掩盖它们，release 自身的 lintVital 已通过。
