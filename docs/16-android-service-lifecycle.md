# Android 服务启停与 PIN 真机复测（2026-09-16）

> ⚠️ **历史记录说明（2026-09-22 补）**：本文记录的是 **PIN 配对 / 信任库时代**的交付事实与证据。
> 该认证机制已在本轮整体移除（现在连上即用：没有配对码、没有白名单），本文中的配对步骤、PIN 验收数字与
> 信任判定均**不再对应当前实现**，只作为当时的交付记账保留 —— 现状见 `docs/72-remove-pairing.md`。

设备为 PHK110，Android 16 / API 36，arm64，Wi-Fi `192.168.3.75/24`；PC 为 `192.168.3.200`。
这是新的测试设备，不能用本轮数字回证 MI 8 Lite 的历史结果。

## 停止后界面仍显示运行中的修复

现场复现：启动后按“停止服务”，`dumpsys activity services com.gotkicry.audiolink` 已返回 `(nothing)`，
界面却显示“前台服务：运行中 / 播放：已停止 / 引擎：未启动”，按钮停在“停止服务”，无法再次启动。
证据在 `target/evidence/phk110-validation/top.xml`、`start-logcat.txt`。

根因是旧 `stopEngine()` 的 IO 协程在 `onDestroy()` 清空状态后，又调用 `refreshState()`，
后者无条件发布 `serviceRunning=true`。启动结果和 PIN 查询也可能在停止后回写；
不同 Service 实例分别向 IO 派发启停，还可能让新启动先于旧停止进入 Rust。

修复把原生启停放入**进程内按请求排序的队列**。队列在主线程登记顺序，JNI 调用切到 IO；
旧实例即使销毁，已登记的清理仍继续，新实例等待清理完成再启动。
每个实例的 `EngineLifecycle` 在停止时使旧代次失效，销毁后禁止发布任何结果。
`AudioLinkService` 的状态读写与发布统一到主线程；配对查询在 IO 执行，返回后核对代次。
销毁时关闭发布入口、取消该实例的配对查询，并清除 UI；进程级清理队列保留。

没有改变 Rust/UniFFI 导出接口。本轮原生库包含 `51e6d94` 的端口释放修复。

## 验证结果

新增 `EngineLifecycleTest` 四项纯 JVM 回归，使用挂起点控制真实协程交错：

| 场景 | 结果 |
|---|---|
| 旧服务启动未完成就销毁，新服务请求启动 | 旧启动、旧停止、新启动依次完成；旧实例不发布状态 |
| 同实例停止后马上重启 | 旧启动/停止结果与旧 PIN 查询代次失效，新状态不被覆盖 |
| 重复启动、重复停止、销毁后再启动 | 底层启停各执行一次，关闭后的实例不再启动 |
| 原生加载/启停抛错 | 当前错误可见，后续请求仍能执行；过期错误不覆盖新实例 |

Android **73 项 JVM 测试通过**，Debug/Release APK 构建、Release V2 签名校验通过。
仓库要求的 Rust fmt、Clippy 与内核全套测试通过。

手机验证通过：

1. 6 轮正常停止/重启：停止后系统服务消失，按钮恢复“启动”；每轮重新监听 `0.0.0.0:58290`。
2. 3 轮连续四次点击（停止/启动/停止/启动，ADB 点击间不插固定等待）：最终服务、引擎与播放均运行。
3. 推流中停止并立即重启：旧连接结束；同一 PC 身份免 PIN 重连，再次开流后实际写入音频。
4. 配对中停止并重启：旧 PIN 消失，原端口重新监听，没有残留卡片。

原始记录：`target/evidence/phk110-validation/restart-results.json`、`restart-*.xml`、`rapid-*.xml`、
`stream-restart.xml`、`stream-reconnected.xml`、`stream-reconnected-audio.xml`。

## PIN 验收

使用真实 Engine / QUIC / PIN 与手机 Release 包；从 UI XML 读取 PIN，并核对屏幕截图。
测试驱动位于 `target/evidence/phk110-validation/driver/`，脚本为同目录的 `pairing_regression.py`。

| 场景 | 结果与证据 |
|---|---|
| 未受信身份连接 | 屏幕出现完整六位 PIN，成功后对端名为 `PC-PIN-validation` |
| 错一次再正确提交 | 错误时原 PIN 保留；正确后卡片消失，显示“已信任” |
| 未配对主动断开 | PIN 清除；同身份重连后显示当前连接 PIN |
| 保持连接等待过期 | 55 s 后采样仍可见；62 s 后采样已消失（读取截图结束约 64.5 s） |
| 连续错五次 | 前四次返回剩余次数，第五次连接关闭，手机不再显示 PIN |
| 配对中停止/重启 | 卡片清除，服务可以再次启动并监听原端口 |

证据：`pairing-results.json`、`*-driver.log`、`auto-pin-*.xml/png`（均在上述本地目录）。
第一次锁定脚本错误地等待第五次 `PairCompleted(false)`，实际协议路径关闭了连接；按实际事件修正驱动后，
单独重跑锁定及尚未执行的停止场景通过。未把这个驱动等待超时当作产品通过或产品故障。

## APK 与版本核对

现场最初回拉的 APK SHA-256 为 `5a9f02aa1505e8836ceba3f1e1c564949d7ee1b624522083c91bd7af57d10ed9`，
它与旧默认路径的包一致，原生库不是最新构建。因此手机版本以后以**安装后回拉校验**为准，
不能仅凭 `adb install` 的 Success 或固定 `versionName=0.1.0` 判断。

修复后的行为验收使用 `28760fea9370694b99c8622ea1c1673a802eca075ec1147052046a478ade3229` 的包，
安装后回拉逐字节一致。随后仅整理源码注释/导入并为 JVM 测试加超时，重新打包的最终 Release 为：

`android/app/build/phk110-validation/outputs/apk/release/app-release.apk`

SHA-256：`818b9ac353a3fd657cb7eb2e30a16c8f1e67ffbd83a997f592ad2a7b893831fe`。
最终包也已安装并回拉核对，完成启动/低延迟/48000 Hz 检查，后续音频基线使用该包。
基线结束后，最终安装包再次通过手机内核自检 5/5；停止后 UI 与 Activity Manager 一致。
双 ABI 原生库与最终 APK 的 `.text/.rodata` 比对记录为 `final-apk-verification.json`。

本轮输出目录通过本地 init 脚本指定为 `android/app/build/phk110-validation`，仓库构建默认值不变。
本地命令：

```powershell
$env:GRADLE_USER_HOME = "$PWD/.gradle-home"
pwsh tools/gradlew.ps1 -JavaHome '<JDK 17 根目录>' testDebugUnitTest assembleDebug assembleRelease --init-script "$PWD/target/evidence/phk110-validation/apk-output.init.gradle" --no-configuration-cache --console=plain
```

## 音频定量验收

本轮已观察到 `LOW_LATENCY`、48 kHz / 2ch、AudioTrack `PCM_FLOAT` 与 `PRIMARY|FAST`，
实际缓冲容量仍为 3844 帧。推流恢复后音频写入增加，未发现设备写入错误。
这些结果证明管线与模式可用；完整端到端 P50/P95 和修复后的长跑仍须由对应验收任务核对。

180 s 合成 440 Hz 基线（20 ms 帧，最终安装包，默认设备队列档）：

| 项目 | 本轮证据 |
|---|---|
| 连接 | 180 份遥测，全程 streaming；工具 exit 0 |
| Rust 待播队列 | 首值 40 ms，P50 240 ms，末值 300 ms |
| 内核欠载 / 迟到丢弃 | 0 → 15 / 0 → 1 |
| §6 探针 RTT | 最近 200 个样本 P50 9.561 ms / P95 59.996 ms |
| 手机 PCM 环，同一推流区间约 100 s | 溢出 0 → 0、读空 362 → 362、水位两次均 960/2880 帧 |
| 设备写入，同一区间 | 音频帧 957600 → 5744160；静音帧 173760 → 173760；系统欠载、丢弃、写错均 0 |

队列是**阶跃增长**：t=10 s 约 160 ms，t=20–160 s 约 240 ms，t=170–180 s 约 300 ms；
不能继续把历史现象笼统解释为恒定“5500 ppm 速率漂移”。本轮两次 UI 采样含滚动，未隔离其调度影响。
PCM 双倍供给未再复现，但“接收待播队列增长”缺陷已确认，仍需修复后重新做延迟与长跑验收。
仅待播队列 P50 就超过 M1 总预算，不能把工具 exit 0 当作延迟达标。

证据：`baseline-180.json/log`、`baseline-start-stats.xml/png`、`baseline-end-stats.xml/png`。
手机计数区间的 UI 时长为 23 → 123 s（秒级取整）；不据此推断精确的采样时钟漂移。

## 接收待播队列增长修复与复测

根因是播放线程的本地时钟、待播队列与音频序号没有共同推进：某一拍队列为空时虽然写了静音，
随后才到的旧帧仍会在下一拍播放；平台提交或线程调度跨过多个播放拍时，代码只重置下一次提交时刻，
队列中的旧帧也会继续逐帧播放。因此每次短暂停顿都可能永久增加一帧或多帧延迟，直到 16 帧队列触顶。

修复以 `expected_seq` 作为播放时钟游标：正常播放、欠载静音和已经错过的播放拍都会推进游标；
迟到帧直接丢弃，未来帧保留到对应播放拍。线程落后超过一帧时按实际跨过的周期一次推进，
不会追赶式提交旧音频。序号比较使用半区间回绕规则，覆盖 `u32::MAX → 0`。

真实 Engine → QUIC → Opus → 播放线程回归在 sink 第 30 次写入时停顿 250 ms：旧代码恢复后的延迟
P50 为 271 ms；修复后连续运行 5 次，P95 为 20.6–40.7 ms。另有两项单元回归覆盖“欠载后迟到帧不再播放”
与序号回绕；正常 10/20 ms PCM 传输回归通过。Rust fmt、全 workspace Clippy 与测试通过，
`audiolink-engine` 为 78 项单元测试，新增跨管线回归 1 项。

PHK110 使用新双 ABI Release 包复测 180 s，包安装后回拉 SHA-256 一致：
`71e662553ddf32187261e67dc64472a3c93f91b1e8058575d3a0073782cb9f1f`（V2 签名）。

| 项目 | 修复前 | 修复后 |
|---|---:|---:|
| 待播队列 P50 / P95 / max | 240 / 300 / 320 ms | 40 / 40 / 60 ms |
| 待播队列首值 → 末值 | 40 → 300 ms | 40 → 40 ms |
| 内核欠载 / 迟到丢弃 | 0→15 / 0→1 | 0→143 / 0→143 |
| 链路丢包 | max 0.00% | max 0.00% |

手机播放环在同一推流内约 105 s 的两次采样均为 480/2880 帧；溢出、读空、系统欠载、设备丢弃和写错均无新增，
音频写入持续增长。新计数明确反映“错过播放拍后丢弃过期帧”，而不是继续把旧帧变成延迟；
它证明增长缺陷已消除，但不代表弱网听感已经完成。减少这 143 次欠载/迟到由 M2 自适应抖动缓冲与 PLC 继续处理。
仅按本轮已有段建模，队列修复后的 e2e 下限为约 73 ms —— **口径要说准**：这是**待播队列修复后**的**模型**下限，
**不含**设备侧 AudioTrack 输出（真机实测 3844 帧 = 80.08 ms，`docs/12` §2.1 与 §3）；对端解码与 AudioTrack
输出仍未纳入上限，所以不能据此把 M1 P50/P95 验收标为完成。
（对照：`docs/12` §0 的「≥ 115 ms」是**修复前** run3 的**实测**下限，且**已含**播放环水位 80 ms ——
两个数字都叫「e2e 下限」但口径不同，不可直接比较。）

证据：`target/evidence/playout-recovery/before.log`、`repeated-after.log`、`after-180.json/log`、
`playout-after-start-stats.xml/png`、`playout-after-mid-stats.xml/png`、`apk-verification.json`。

## CI 后续：测量探针并发漏配

服务修复提交 `12cf674` 的 [CI 35056343048](https://github.com/GotKiCry/AudioLink/actions/runs/35056343048)
中，Android、桌面和版本检查通过，内核失败于既有的 `concurrent_writers_do_not_lose_pairs`：
预期 2000 对时间戳，实际只配对 1992 对。

`MeasurementTap` 原来为发送记录、播放记录与样本分别加锁。两端并发时，可能都先查到对方队列为空，
再分别留下同序号记录，使这个序号永远无法配对。修复将查找、配对、留存与样本写入放在同一把状态锁内，
`reset()` 也原子清空全部记录；容量限制、延迟计算和公开 API 保持原有语义。

既有并发回归增加逐序号同步起跑，修复前复现为 645/2000，修复后完整配对 2000/2000、孤儿数为零，
全部样本准确为 25 ms；测量模块 8 项测试、Rust fmt、全内核 Clippy 与测试通过。
证据为 `measurement-before.log`、`measurement-after.log`、`measurement-clippy.log`、`measurement-core-tests.log`。
该探针仅由测量工具显式启用，Android 默认音频路径未启用；此修复不解释或解决上文的手机待播队列增长。
