# 满队列拒收后的播放游标恢复

日期：2026-09-29。范围：普通单流播放的 Rust 待播队列；续接 `docs/73` 和 `docs/74`。

## 背景

现场截图为 PC 向 Android 播放时 `underruns=14821`、`buffer_level=160 ms`、丢包 0%。
160 ms 正好等于 10 ms 帧下 16 格待播队列的硬上限。历史真机记录（`docs/12` §11.14）
已经证明过一种自锁：队列拒收一帧时接收序号先前进，播放游标遇到缺口后逐拍输出掩盖音，
不消费队列；队列保持满载，又拒收后续帧。`docs/74` 的本机对照未复现 Android 路径。
本轮接入 PJZ110 后直接读到了满队列拒收，但不能据此把原截图的全部欠载归因于这一条路径。

## 做法与取舍

- 满队列拒收时记录**第一次**被拒帧的序号。连续拒收不能覆盖它，否则播放端无法识别最早的缺口。
- 普通播放走到确实包含该序号的队头缺口时，游标一次对齐到队头并播放真帧；拼接处使用现有短淡入。
  这样不再为了已拒收的帧逐拍补音并延长队列满载。拒收仍按原口径计入 `late_drops` 和
  `queue_rejects`，没有伪造网络丢包或清零欠载计数。
- 网络缺帧、空队列以及预约同步播放仍按原时间轴处理。该恢复不提高播放线程的吞吐量；
  若真机主要是 FFI 回调耗时或设备调度造成持续节拍超时，仍需按 `docs/74` 的 A/B/C 分量定位。
- 不改冻结的 `StreamStats` wire schema，不改变 Android `AudioTrack` 的目标水位。
- 已有的 A/B/C 分量通过 `PeerView` 进程内快照传到 Android 诊断页；每个对端分开读，
  不把本机分量混入对端回传的 `StreamStats`。同时修正 UniFFI 同源检查对生成器末尾空白的误报。

## 验证

- Rust 引擎库：200/200 通过；新增用例覆盖满队列拒收后恢复、连续拒收保留首个缺口、
  普通网络缺帧维持掩盖、序号回绕。
- `cargo fmt --all --check`、非桌面与桌面 Clippy、非桌面 nextest（563 通过、3 跳过）、
  `cargo test -p audiolink-desktop`、`desktop/pnpm build` 通过。
- Android 双 ABI release `.so` 和 debug APK 已重建并安装到 PJZ110；UniFFI 同源检查四环通过，
  jniLibs、target、APK 均检出同一组 20 个 checksum 符号。
- `assembleDebug`、`testDebugUnitTest` 通过；初次单测受旧 HTML 报告权限阻塞，保留旧报告并移到
  `target/evidence/testDebugUnitTest-before-playout-fix` 后重跑通过。

### PJZ110 实测（PC → Android，10 ms 帧，Wi-Fi）

| 时段 | 本机节拍超时 A | 满队列拒收 B | 游标追赶 C | AudioTrack 欠载 | PCM 供给欠载 |
|---|---:|---:|---:|---:|---:|
| 14:05 首次读数 | 0 | 5 | 0 | 0 | 3 |
| 14:08，电脑执行 Rust 全量测试后 | 0 | 225 | 192 | 0 | 3 |
| 14:10，电脑空闲约 2.5 分钟后 | 0 | 232 | 193 | 0 | 3 |

14:08 桌面显示对端累计欠载 341、缓冲中位 30 ms、丢包 0%；与原现场 160 ms 顶格、
欠载近 50 次/秒的持续恶化不同。B/C 在电脑执行测试时显著增长，空闲后分别只增 7/1；
这支持负载扰动的解释，但不是对每一次序号缺口的来源证明。A 持续为 0，排除了这轮复测中
Android 播放线程大批量跳拍。手机侧系统与 PCM 供给欠载在两个间隔里均未增加。

重装前同一修复版本跑了约 9 分钟：13:44 到 13:47，AudioTrack 欠载 0、供给欠载 4、
PCM 环溢出累计 45,120 帧均未增加；桌面缓冲中位 30→40 ms，未回到 160 ms。
原始截图在 `target/evidence/audiolink-counters1.png`、`audiolink-counters2.png`、
`audiolink-counters3.png`、`desktop-pjz110-second-run.png` 及重装前的
`audiolink-diagnostics-queue-later.png`。

## 未验

- 没有受控重现原截图的 160 ms 顶格状态，也未进行声学连续性录音或 8 小时稳定性验收。
- 本轮 B/C 在电脑测试负载下仍增长，修复不承诺零欠载；若以后在空闲负载下持续上升，
  应连同 PCM 环的区间溢出及网络重排/迟到帧一起排查。
