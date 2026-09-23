# AudioShare 与 AudioLink 延迟对照

后续实现：下文是优化前的分析快照；播放水位反馈、默认 30 ms 输出目标和更快的缓冲恢复已开始落地，当前行为及验证见 [docs/69](69-playout-latency-recovery.md)。

分析日期：2026-09-21。对照用户提供的 [HeHang0/AudioShare](https://github.com/HeHang0/AudioShare)，读取提交 `dac02fdc7fdb04a1301bb4784c85580ab3e74e73`（2024-05-05）。范围是 Windows 系统音频 → Android 播放，不包含它的在线音乐播放器。AudioLink 按当前工作树分析，包含 docs/65–67 的弱网修改。

用户已确认：对比使用相同环境、同一台 PC 和 Android 手机、同一 Wi-Fi。因此无需再以不同设备或 Wi-Fi / USB 路径差异解释此次体验差距。本次仍是源码与已有测量记录分析，没有运行 AudioShare 做声学 A/B，没有修改播放实现；两者的具体延迟、播放时长及网络波动后的恢复曲线尚未量化。

## 结论

源码支持的主要差异是**排队策略**：AudioShare 的 PCM 转发链路较短，在发送或输出繁忙时跳过新块；AudioLink 同时承担编码、包恢复、抖动缓冲和按时播放，并在 Android 侧继续排队。当前优先调查对象是整条接收链的实际积压、弱网升档后的恢复速度，而不是直接更换 QUIC 或删除 Opus。

用户确认的同环境对比加强了优先排查软件链路的依据，但尚不能量化各环节的贡献。下一步应区分「新连接后就偏慢」和「网络波动后变慢且迟迟不恢复」：前者优先核对整条输出链的基线水位，后者同时核对自适应缓冲目标及实际积压回落。

尤其不能只把 Android 队列目标从「满灌」改成 30 ms 就宣称总延迟降低：本项目已有真机 A/B 显示，下游省出的约 60 ms 被上游新增水位抵消。

## 1. 两条实际链路

| 环节 | AudioShare | AudioLink 当前实现 |
|---|---|---|
| 采集与封包 | NAudio WASAPI 回环，回调得到 PCM 后直接发送 | WASAPI 共享事件采集，按默认 20 ms 组帧，再用 Opus RestrictedLowDelay 编码 |
| 网络 | TCP，两端开启 TCP_NODELAY | QUIC 数据报，附带前帧冗余、有限重排和有条件 NACK |
| 过载处理 | 发送未完成或 AudioTrack 正在写入时跳过新音频块 | 保留时间轴，重排、补音、缓冲升档；降档时有限丢弃旧帧 |
| 接收缓冲 | 这条 PCM 路径没有独立的自适应抖动缓冲；仍有 socket / AudioTrack 缓冲 | 已解码待播队列 → Rust 播放调度 → Kotlin PCM 环 → AudioTrack |
| Android 输出 | PCM16，按 getMinBufferSize 创建 AudioTrack，来一块写一块 | PCM float，10 ms 写块，默认尽量填满 AudioTrack，缺数据时补零 |

AudioShare 的关键代码：

- [AudioManager.cs:57](https://github.com/HeHang0/AudioShare/blob/dac02fdc7fdb04a1301bb4784c85580ab3e74e73/windows/AudioManager.cs#L57)：回环采集与 PCM16 双声道格式。
- [Speaker.cs:175](https://github.com/HeHang0/AudioShare/blob/dac02fdc7fdb04a1301bb4784c85580ab3e74e73/windows/Speaker.cs#L175)：TCP_NODELAY；[283 行](https://github.com/HeHang0/AudioShare/blob/dac02fdc7fdb04a1301bb4784c85580ab3e74e73/windows/Speaker.cs#L283)中 `isBusy` 时直接返回。
- [TcpService.java:215](https://github.com/HeHang0/AudioShare/blob/dac02fdc7fdb04a1301bb4784c85580ab3e74e73/android/app/src/main/java/com/picapico/audioshare/TcpService.java#L215)：最小缓冲估计；[462 行](https://github.com/HeHang0/AudioShare/blob/dac02fdc7fdb04a1301bb4784c85580ab3e74e73/android/app/src/main/java/com/picapico/audioshare/TcpService.java#L462)中上一块未写完就跳过当前块。

这些跳块分支能避免应用层继续排队，但也可能损失声音；TCP 内部仍可能积压。不能从源码推断它在用户 Wi-Fi 上一定不会断音。

48 kHz / 双声道 / PCM16 的纯音频负载是 1.536 Mbps；AudioLink 默认 160 kbps Opus 加一份冗余约 320 kbps，均未计协议开销，也未计自适应码率变化。PCM 省去了应用层编解码，但增加网络负载，不适合作为弱网问题的唯一解决方案。

## 2. AudioLink 的主要延迟来源

### 2.1 Android 默认填满输出缓冲，上游看不到实际水位

- [LowLatencyPlayer.kt](../android/app/src/main/kotlin/com/gotkicry/audiolink/audio/LowLatencyPlayer.kt)：`DEFAULT_QUEUE_TARGET_FRAMES = QUEUE_TARGET_UNLIMITED`；目标为 0 时循环尽力写入，实际容量取请求值和系统最小值的较大者。
- [PlayoutLoop.kt](../android/app/src/main/kotlin/com/gotkicry/audiolink/audio/PlayoutLoop.kt)：读不到足量音频时补零，也会写进输出缓冲。因此刚来的真实声音可能排在已经写入的静音之后。
- [AudioLinkService.kt](../android/app/src/main/kotlin/com/gotkicry/audiolink/service/AudioLinkService.kt)：实际使用上述播放器；Kotlin 环容量 2880 帧，即 60 ms；AudioTrack 请求 960 帧，即 20 ms。**请求 20 ms 不表示实际缓冲或出声延迟就是 20 ms。**
- [audio_bridge.rs](../core/crates/audiolink-ffi/src/audio_bridge.rs)：`KotlinPlayoutSink::buffered_frames()` 返回 0，内核没有 Kotlin 环和 AudioTrack 的实际积压反馈。

多层容量不能简单相加当作实测延迟；真正影响延迟的是实际水位、音频年龄及调度等待。这里的缺口是缺少覆盖整条输出链的共同预算。

[已有真机记录 docs/12 §10](12-m1-device-acceptance.md#10-pcm-修复后的设备侧-ab-复测2026-09-18--phk110--android-16)（2026-09-18，PHK110，PCM 长度修复后）显示：

| 档位 | AudioFlinger 输出侧 Latency |
|---|---|
| 默认满灌 | 101.08 ms |
| 队列目标 30 ms | 39–47 ms |

但同时对端待播水位增加约 60 ms，总量推算基本不变；四段账本推算为 172.1–182.1 ms。**这是历史设备侧观测与账本估算，不是本轮声学端到端实测，也不是当前用户的测量结果。**

同文 §11 曾记录长跑水位上涨，以及两种简单水位护栏失败后被移除。当前弱网实现已有后续变化，不能声称历史 276 ms 水位仍是当前必现结果；但这些记录足以说明，修改单个缓冲常量或直接拒收帧不构成完整修复。

### 2.2 上一轮抗断音修改确实提高了弱网延迟

[jitter.rs](../core/crates/audiolink-engine/src/runtime/jitter.rs) 当前起步一帧，默认帧长 20 ms；docs/67 将最大目标从三帧提高到六帧，即 **60 → 120 ms**。升档建立更多真实余量，是用延迟换连续性，正常起步并非固定等待 120 ms。

控制器每秒观察一次，连续稳定 30 个窗口才降一帧。从六帧降到一帧，即使持续满足最低档条件，也需要约 **150 秒**。这是目标恢复时间，不保证整条链的实际积压同步清除。

120 ms 也不是实际待播队列的硬上限：上一轮 30 秒合成弱网实验（2% 丢包，15±15 ms 延迟）末次 `buffer_level_us = 160000`，见 [docs/67](67-audio-link-quality.md)。该实验没有 Android 输出，不能用它估算手机总延迟。

### 2.3 20 ms 组帧与播放节拍是次一级优化对象

[codec.rs](../core/crates/audiolink-audio/src/codec.rs) 默认 20 ms 帧，并已有 10 ms 配置入口；当前实际默认路径没有切到 10 ms。组满一帧会带来等待，但不能把 20 ms 当作编码计算耗时。

[runtime.rs](../core/crates/audiolink-engine/src/runtime.rs) 普通播放起步还会对齐下一播放拍；包重排和 NACK 等待主要在缺包时触发，不能每个包都加上一份固定重传延迟。预约组播放走 epoch 时间轴，不能用单端自由缩缓冲破坏同步。

## 3. 两个容易误判的点

**AudioShare 并没有特殊的超低延迟采集器。** 它锁定 NAudio 2.2.1，`WasapiLoopbackCapture(device)` 继承默认采集配置。该版本 [WasapiCapture](https://github.com/naudio/NAudio/blob/v2.2.1/NAudio.Wasapi/WasapiCapture.cs) 默认请求 100 ms、使用非事件轮询，循环按实际缓冲时长的一半休眠后取包。如果系统给出 100 ms，轮询间隔约 50 ms；实际值取决于设备，不能据此断言它固定有 100 ms 延迟。参见 [WasapiLoopbackCapture](https://github.com/naudio/NAudio/blob/v2.2.1/NAudio.Wasapi/WasapiLoopbackCapture.cs)。这进一步削弱了「它只是采集更快」的解释。

**播放中的 flush 不是低延迟秘诀。** AudioShare 在 write 后调用 flush，但 Android [AudioTrack.flush 官方说明](https://developer.android.com/reference/android/media/AudioTrack#flush())规定，流模式且处于停止或暂停状态才有效，播放态调用不会按预期清空积压。`getMinBufferSize` 同样只是创建缓冲的估计，不能作为实际出声延迟。

## 4. 建议的优化顺序与验证

1. **先闭合全链水位观测和总延迟预算。** 把 Rust 待播队列、Kotlin 环、AudioTrack 实际排队量及音频年龄一起记录。结合下游消费位置做调度和积压恢复，避免延迟从一个队列搬到另一个队列。单端低延迟与多端预约同步应分别处理。
2. **随后一起调整输出目标和弱网恢复。** 低延迟模式采用受控的 AudioTrack 水位，网络稳定后更快回落；稳定模式允许更大抖动余量。现有诊断页已有 20/30/40/60 ms 队列目标，适合做分段观测，不能据此承诺单独切到 30 ms 就能解决总延迟。
3. **补足过期音频的平滑追赶。** 丢弃必须同步推进播放时间轴，避免历史准入护栏的序号缺口问题；优先研究小幅速率修正或有交叉淡化的有限跳过，衡量声音连续性，不能照搬 AudioShare 的忙时整块跳过。
4. **最后做 10 ms 帧长 A/B。** 检查跨端协商、采集组帧和播放节拍；包率翻倍，应同时比较 CPU、丢包、欠载与听感。暂没有证据支持为了降延迟而更换 QUIC。

验收沿用用户已确认的同一套 PC、Android 和 Wi-Fi，并保持音频输出一致，覆盖新连接、稳定播放、短时网络波动及恢复后长跑。使用已有 [声学延迟测具](60-p1-acceptance-notes.md)测出声 P50/P95，同时记录各队列水位、欠载和迟到；不能只凭 RTT、NullPlayout 回环测试或某一级缓冲下降判定完成。

当前界面显示的网络延迟是 **RTT**，不是 PC 声音到手机出声的端到端延迟。恢复后的音频丢包率为 0% 也不排除迟到、欠载或过长缓冲。
