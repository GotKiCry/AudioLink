# Wi-Fi 波动时的播放连续性修复

## 背景与根因

2026-09-21：检查共享 Rust 内核，复现两处会放大短时网络波动的缺陷。

1. 发送端先发当前主包，再发上一帧的冗余副本。接收端在 20/40 ms 缓冲档且没有 NACK 等待窗口时立即跳过序号洞，导致紧随其后的副本被判迟到。默认起步档就是 20 ms。
2. 解码侧 `AudioReceiver` 的 PCM 掩盖只在后续包到达后才确认缺包。播放线程在此之前遇到空队列，以及升档、补水的 Hold 阶段，直接输出全零 PCM。事后补出的掩盖帧可能已经过期，无法消除已经播放的静音。

另有两处相关时序问题：RTT ≥ 30 ms 时虽然不发送 NACK，接收端仍可能等待重传；预约播放在队列暂空时会误认成普通播放，触发不应该发生的本地深度升档。

## 做法与取舍

- 20/40 ms 档只在有序号洞时留 5 ms 给乱序包或冗余副本；连续帧立即交付，60 ms 档仍有一帧重排预算，待排包数量上限保持 3。
- NACK 等待与 NACK 发送使用相同的 RTT < 30 ms 门槛。
- 播放线程单独保存增益前的 PCM 历史。欠载或 Hold 时立即用已有 `PcmConcealer` 生成过渡帧，连续缺帧在 120 ms 内淡出，恢复后以 2.5 ms 交叉淡化接回真实音频。稳态无新增堆分配。
- 真帧、掩盖帧共用两层音量渐变，断流期间调音量或静音立即按原 ramp 生效。播放器重建、预约等待/丢弃时清空历史，避免旧音频被再次播放。
- 预约播放无论队列是否有帧都保持 epoch 约束，不因本地深度调整冻结或丢弃播放拍。
- 协议、帧格式、FFI 与缓冲目标范围不变。Windows、Android 共用此内核，接收端需要重新构建才能生效。

`underruns` 仍如实记录缺帧/补水拍，`plc_count` 保持接收解码侧的序号缺口计数，避免同一缺包事前、事后重复计数。实际静音应读取播放输出的连续静音统计，不能再把欠载数直接解释成全零输出拍数；旧 soak-runner 的欠载预算仍作为保守供给质量门槛使用。

## 验证

修复前，新回归在真实 `playout_main` 中按已输出拍数注入 PCM，三个测试均失败：短时断流/补水输出为零、长断流第一拍直接静音、静音操作之前没有任何掩盖音频。默认缓冲档的冗余恢复测试也失败（丢包 1，预期 0）。这些测试不依赖 UDP 丢包概率。

修复前 30 s QUIC 弱网基线（2% 丢包、15±15 ms 延迟、5 Mbps、seed=1）：欠载 17、迟到 15、解码掩盖 0、NACK 0，欠载 1.13% 超过工具的 1% 门槛。证据：`target/evidence/wifi-before.json`。短跑数字受 OS 调度影响，不能单凭一次前后对比认定改善幅度。

修复后检查：

- `cargo fmt --all --check`、内核与桌面 `clippy --all-targets --all-features -- -D warnings` 通过。
- `cargo test -p audiolink-engine --lib runtime::`：64 项通过，包括新增的 3 项播放截止回归，以及低档冗余恢复和有界等待回归。
- `cargo nextest run --workspace --exclude audiolink-desktop --lib --tests --test-threads 2`：执行 549 项，首次 545 项通过、4 项失败，另有 2 项跳过。该次与桌面/Android 编译重叠；构建结束后将 4 项失败逐个单独运行，全部通过（无代码或阈值变更）：重丢包组同步 P95 0.03 ms、250 ms 播放停顿恢复 P95 32.99 ms、重连回执 1、20 s 播放水位前后 P50 均为 20 ms 且欠载/迟到增量均为 0。保留这次受负载影响的失败记录，不将首轮描述为全绿。
- 真实 QUIC + 成对丢包的 NACK 集成测试通过；新增断言要求实际播放静音拍占比低于 5%，防止只修复包计数却仍频繁硬静音。
- 桌面后端 49 项测试通过（另有 1 项忽略）；首次遇到 MSVC 匿名符号链接失败，设置进程内 `CARGO_INCREMENTAL=0` 后重建通过。`pnpm build`（含 TypeScript、i18n、Vite）通过。后续已将桌面禁用增量编译写入 workspace 配置，并实跑开发窗口启动，见 `66-desktop-dev-linker.md`。
- Android `arm64-v8a` / `armeabi-v7a` release 内核及 `assembleDebug` 通过。逐 ABI 核对源 `.so` 与 Gradle 合并产物、剥离后的 `.so` 与 APK 内条目的 SHA-256，一致。安装包位于 `android/app/build/outputs/apk/debug/app-{arm64-v8a,armeabi-v7a}-debug.apk`。
- 相同弱网参数的修复后 30 s 运行通过（exit 0）：欠载 9（0.60%）、迟到 6、解码掩盖 0、NACK 0、最终丢包率 0。注入器丢弃 63 个包，与修复前相同；证据：`target/evidence/wifi-after.json` 及对应 `.netem.json`。这是一轮模拟前后对照，不能外推成真机听感改善比例。

弱网复测命令（在 workspace 根目录执行）：

```powershell
cargo run -p audiolink-tools --bin soak-runner -- run --seconds 30 --tolerant --netem-loss-pct 2 --netem-delay-ms 15 --netem-jitter-ms 15 --netem-bandwidth-kbps 5000 --netem-seed 1 --report target/evidence/wifi-after.json --quiet
```

## 未验

- 真实路由器 Wi-Fi 漫游、Android 真机听感、不同音源与蓝牙输出。
- 8 h 长跑。120 ms 掩盖只能平滑短暂缺帧，不能重建持续断网期间丢失的原始声音。
