# 77 · B/C 线正反馈与播放锁死 —— 根治设计

> **状态：S1 + S3 + S4 已实施（2026-10-20），真机验证待做；S2 未实施。** 设计稿写于同日。
>
> 前置阅读：[`73-playout-queue-full-lockup.md`](73-playout-queue-full-lockup.md)（B 线首次直接观测）、
> [`74-underrun-breakdown.md`](74-underrun-breakdown.md)（A/B/C 分量口径）、
> [`75-playout-rejected-gap-recovery.md`](75-playout-rejected-gap-recovery.md)（跳队头恢复）、
> [`76-queue-reject-burst-fix.md`](76-queue-reject-burst-fix.md)（多槽标记 + 墙钟容量）。
>
> **注意**：75/76 治的是「容量 / 让位线 / 多槽标记」三条。本文处理它们**之后仍然存在**的一条
> 无出口正反馈。结论不推翻 75/76，只补它们的盲区。

---

## 1. 背景

`queue_rejects`（B 线）与 `cursor_catchups`（C 线）在抖动爆发时以**每秒十几**的速度上涨，
且**不会自愈 —— 必须停止播放再重新开始才能恢复**（2026-10-20 用户现场确认）。

这一条把问题定性从「计数器噪声」升级为**状态锁死**：

- 若是瞬态，几秒后自己回落，最多影响听感；
- 实测是**只能手动停止播放**才能退出，说明状态机里**没有出口**。

本文给出根因链、为什么 73/75/76 都没堵住它、以及根治设计。

---

## 2. 现场证据

### 2.1 装置

- 设备：PHK110 / Android 16 / arm64-v8a，Wi-Fi 192.168.3.75
- 安装包：`versionName=1.0.0`，**DEBUGGABLE**，`lastUpdateTime=2026-10-15 17:11`（含时序探针）
- 方向：PC → 手机（手机为接收侧），帧长 `10 ms`
- 进程已运行 **5.7 h**（`/proc/<pid>/stat` starttime 换算）

### 2.2 稳态读数（`dumpsys activity service`，2026-10-20 16:16–16:22）

```text
pcmFeed: n=256 win=2.5s p50=10.0ms p95=10.8ms p99=11.7ms max=14.8ms gaps=0
events : ringOvf=0  ringUdr=61(131367ms 前)  silence=62(131367ms 前)
         trkUdr=5(5877412ms 前)  capTrunc=0  pullEmpty=0
ui     : playing=true engine=true trackUnderruns=9 sourceUnderruns=107
         ring=480/2880 queued=672/960
ffi    : rtt=7.8~14.3ms jitterP50=2.1~9.4ms jitterP95=10.0~31.0ms
         loss=0.0% frame=10ms underruns=6147 lateDrops=5793 plc=6
```

**关键观察**：

1. **供给节拍完全健康**：`p50=10.0 ms`、`gaps=0`、`ringOvf=0`、`trackUnderruns` 98 分钟前才动过。
   → 内核播放线程没有被卡，Kotlin 播放侧也没有欠载。**这不是「供给跟不上」型的问题。**
2. **稳态增速极慢**：20 s 采样下 `underruns +1 / lateDrops +1`（≈0.05 次/s）；另一轮 10 s 采样 +3（≈0.3 次/s）。
3. **爆发窗口**：抓到一个采样点 `jitterP50=9.4 ms / jitterP95=31.0 ms` —— **到达抖动冲到 3 倍帧长**。
   分布上 `underruns(6147) ≈ lateDrops(5793) + 354`，两者同步增长。
4. **用户观察**：爆发时 B/C 每秒十几，且**必须停止播放**才恢复。

> 结论：稳态无害、爆发锁死。治理目标不是「把 0.3/s 降到 0」，而是**切断爆发后的锁死环**。

### 2.3 为什么「每秒十几」这个量级是自洽的

10 ms 帧 = 100 拍/s。锁死场景下队列贴满，每一拍是「投递被拒」与「消费腾格」交替，
B 的理论上限约为帧率的 50% = **50/s**；实际到达被 NACK/重排抑制，落在**十几/s**量级。
C 则等于「每秒处于追赶状态的拍数」，与洞大小直接挂钩。

---

## 3. 根因链

### 3.1 触发阈值：**只有 >120 ms 的洞才追赶**

```text
max_conceal_frames = CONCEAL_FADE_MS(120 ms) / frame_ms
```

- 定义：`runtime.rs:3797`；`CONCEAL_FADE_MS = 120`（`core/crates/audiolink-audio/src/conceal.rs:11`）
- 10 ms 档 → **12 帧**

于是：

| 洞大小 | `receive` 的动作 | 队里是否留序号缺口 | 是否触发 C 线 |
|---|---|---|---|
| ≤ 12 帧（≤120 ms） | 连发 N 帧掩盖填满（`runtime.rs:3834-3854`） | **否** | **否**（`distance` 恒 0） |
| > 12 帧（>120 ms） | 超出部分不补，**重建解码器**（`runtime.rs:3855-3861`） | **是** | **是** |

**这就是全部矛盾所在**：能触发逐拍追赶的，只有「超过 120 ms 的大洞」。
而恰恰在这个分支里，接收侧**已经判定这是时间轴重建**（重建了 Opus 解码器）——
它知道，但**没有告诉播放侧**。

### 3.2 逐拍状态：追赶期队列只进不出

以一个 `D = 10` 的洞为例（10 ms 帧、容量 30 帧、进入前沿水位 20、洞内无标记），
按 `take_due_frame_after_reject`（`runtime.rs:5760-5817`）实际判定跑出来的表：

| 拍 | 队列长(取帧前) | pending | expected_seq | 判定 | B | C |
|---|---|---|---|---|---|---|
| 1 | 21 | 1010 | 1001 | Missing（帧挪进 pending） | 0 | 1 |
| 2 | 21 | 1010 | 1002 | Missing（同一帧再取再放回） | 0 | 2 |
| … | 21→29 | 1010 | +1/拍 | Missing | 0 | … |
| 10 | 29 | 1010 | 1010 | Missing（`distance` 已归零） | 0 | 10 |
| 11 | 30 | 空 | 1011 | Ready | 0 | 10 |
| 12 | 30 | — | — | **投递被拒 / 消费腾格 交替** | **1 并持续** | 10 |

三条机制读数（都是代码事实，不是估算）：

1. **每拍只推进 1 个序号**：`*pending = Some(frame)` + `*expected_seq = expected+1`（`runtime.rs:5803-5804`），
   下一拍 `pending.take()` 又把同一帧取回来（`runtime.rs:5768`）→ 必须 D 拍才追平。
2. **每拍各记一次 C 与一次 underrun**：`runtime.rs:5807-5809` 记 `cursor_catchup`；
   `runtime.rs:5591→5601` 同一拍记 `underrun`。**所以 C 的数字是「洞大小 × 洞频次」，不是「洞的个数」。**
3. **追赶期队列只有进没有出**：队头帧在第 1 拍被 `try_recv` 取走后，再无任何消费 → **净增 D-1 帧**。

### 3.3 水位棘轮（本文的核心定量结论）

- **上行**：每出现一个洞，水位净增 `D - 1` 帧。
- **下泄**：唯一的稳态回调是 `SETTLE_EXCESS_WINDOW = 500 ms` 丢 **1 帧**
  （`jitter.rs:69/72`、实现 `jitter.rs:289-300`）→ **2 帧/s**。

```text
上行速率 = 洞频次 × (D - 1)
下泄速率 ≈ 2 帧/s
```

**只要洞频次 > 2/s（几乎必然），水位单调爬升 → 必撞容量（30 帧）→ B 线开始滴漏。**
撞满之后每拍「一拒一取」交替，正是「每秒十几」的形态。

### 3.4 `covers_gap` 的盲区：真实大洞永远走不了快路径

快路径判据（`runtime.rs:3273-3289`）：16 个槽里必须有一个「**被本地丢弃的序号**」
严格落在眼前这段洞内部（`expected ≤ m < 队头帧`）。

问题在于**标记的来源只有两个**：

| 写入点 | 触发条件 |
|---|---|
| `try_push` 队满（`runtime.rs:3214-3216`） | 手上有一帧但队满 |
| `note_dropped_seq` 掩帧让位（`runtime.rs:3205-3207` ← `3903-3905`） | 占位帧触到让位线 |

两者都要求「**我们手上有一帧但没让它进队**」。而真实大洞是**数据报根本没到**，
`receive` 对它不产出任何帧 → **永远没有标记 → `covers_gap` 恒 false → 只能逐拍追**。

> 这就是 75/76 治不到的那块：它们解决了「我们主动丢的帧」，没解决「发送端丢的帧」。

### 3.5 为什么锁死无出口

水位撞满之后，四条可能的退出路径**全部被堵死**：

| 可能的出口 | 为什么走不通 |
|---|---|
| `PlayoutGapState::resync_cursor`（`runtime.rs:5733-5739`） | 要求 `empty == true`（队列空**且** pending 空）。追赶期 `pending` 非空 → `empty_since` 每拍被清空 → **恒 false** |
| `raise_depth` 升档（`runtime.rs:5604-5611`） | 被 `!scheduled_mode` 门控；且升档只会**加深**队列，满队时帮助为负 |
| 稳态结算下泄（2 帧/s） | 比上行慢一到两个数量级；且它丢的是**真实音频** |
| 全链积压回收（`runtime/backlog.rs`） | 门槛是「设备水位 + 500 ms」，被 `Missing` 事件本身反复打断 |

**四条全堵 = 只能手动停止播放**（`stop_playout` 销毁句柄 → 队列/游标/pending 整体重建）。
这与用户现场观察完全一致。

---

## 4. 老路为什么不能走

| 老路 | 真机代价 | 出处 |
|---|---|---|
| guardA「水位超目标+2 就每拍丢最旧」 | underruns **7×**、late_drops 6.8×，把引擎环抽到 0–1 帧 | `docs/12:504/506`、commit `c719499`→`b6b0f90` |
| guardB「软上限处拒绝入队」 | **105×/111×**（22.6 次/s），`receive` 先推 `expected` 再 emit ⇒ 缺口留给游标 ⇒ Missing 那拍不消费 ⇒ **队列自锁** | `docs/12:520/532` |
| 上界取「target + 常数」 | 判据口径错误，应跟随实测抖动分位 | commit `dab50d5` 正文 |
| 「队列满时丢队首」 | `PlayoutHandle` 只持 `Sender`，够不到队首 ⇒ **不可实现** | `docs/73:69` |
| 把 B/C 塞进 `StreamStats` | 破坏旧节点解码（postcard 严格解码） | `telemetry.rs:95-100` |

**尤其注意**：guardB 的自锁机制与当前 B 线**同构** —— 现在的 `try_push` 满队拒收就是
「真满版的 guardB 自锁环」。所以**任何「再加一道准入/水位闸」的改法都会重蹈覆辙**。

本文方案与两条老路的本质区别：

| | guardA | guardB | **本文 S1** |
|---|---|---|---|
| 丢的是什么 | 真实音频 | 真实音频（准入拒绝） | **本来就用掩盖填的洞**（无真实帧） |
| 动作频率 | 每拍 | 每次到达 | **每个大洞一次** |
| 是否消耗队列 | 是 | **否**（自锁根源） | **是**（跳队头立即消费） |

---

## 5. 设计目标与硬约束

### 5.1 目标

1. **切断棘轮**：一个洞对水位的净贡献从 `D-1` 降为 0。
2. **保证有出口**：任何病态都能在有限时间内自愈，不再需要用户手动停止播放。
3. **数字可解释**：B/C 能从「每秒十几」拆出「几个洞、多大、被拒的是真帧还是掩帧」。
4. **不引入新老路**：不加水位闸、不做准入拒绝、不每拍丢帧。

### 5.2 硬约束（全部继承，不得放宽）

| 约束 | 出处 |
|---|---|
| 任何回收/结算必须**速率受限**，且目标之上保留**一整帧**摆幅 | `core/AGENTS.md:32`；实现 `jitter.rs:69/72`、`backlog.rs:7` |
| wire schema 冻结；进程内计数走 **Engine 独立出口** | `AGENTS.md:52`、`core/AGENTS.md:30`、`telemetry.rs:95-100`；先例 `runtime.rs:1427/1439` |
| 排播模式下不许自行丢帧/改时间轴 | `runtime.rs:5461-5472`（整帧差 → 组内偏差 20.38 ms） |
| 让位 ≠ 拒收（掩帧不进队不记 B） | `runtime.rs:3893-3908` |
| 容量是余量不是延迟 | `jitter.rs:61`、`docs/76:39` |
| 不承诺零欠载 | `docs/75:58-59` |

---

## 6. 方案总览

| 编号 | 名称 | 治什么 | 改动面 | 风险 |
|---|---|---|---|---|
| **S1** | 大洞标记 → 走已有快路径跳队头 | 棘轮上行 + 无出口（主修） | `receive` + `push_playback_frame`，约 25 行 | 时间轴前移 |
| **S2** | 水位口径扣除游标落后量 + 锁死自愈 | 误结算 + 最后一道保险 | 播放循环，约 40 行 | 一次可听断音（罕见） |
| **S3** | 取证：B/C 拆分 + 洞直方图 | 数字可解释（先决条件） | `telemetry.rs` + Engine 出口 + dumpsys | 无（只读口径） |
| **S4** | 诊断工具三处缺陷 | 锁嵌套 / 实时线程 monitor / dump 漏项 | `PcmRingBuffer`、`IntervalRing`、`EventRing`、`AudioLinkService` | 低 |

**实施顺序**：S3 与 S1 同批上（一轮同时取证与验证），S2 紧随，S4 独立可并行。

---

## 7. 详细设计

### S1 —— 大洞标记，复用既有快路径

#### 7.1 核心思想

接收侧在 `gap > conceal_frames` 时**已经判定「时间轴重建」**（重建了 Opus 解码器）。
那就让播放侧用**同一个判据**：这个洞，播放侧也不该逐拍补掩盖，而应该**一次性跳过**。

跳过的不是真实音频 —— 是**本来就要用掩盖填的洞**。走的是 75/76 已经建好、
且被测试覆盖的 `ReadyAfterRejectedGap` 恢复路径。

#### 7.2 改动点

**(a)** `Emission` 增加一个变体（`runtime.rs:3633-3639`）：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Emission {
    /// 接收侧掩盖生成的占位帧（可让位）。
    Conceal,
    /// 真实解码帧（队列的最后一格永远留给它）。
    Real,
    /// 接收侧判定「超过掩盖上限」的洞：`frame.seq` 是洞的首序号，`samples` 为空。
    /// 播放侧据此走跳队头快路径，而不是逐拍补掩盖（见 docs/77 §7.1）。
    Hole,
}
```

**(b)** `receive` 在重建解码器的同一分支里投递一次洞标记（`runtime.rs:3855-3861`）：

```rust
let hole_start = expected; // 进入 receive 时缓存（3830 行会推进它）
// ...
if gap > conceal_frames {
    if let Ok(decoder) = OpusDecoder::new(self.decoder.config()) {
        self.decoder = decoder;
    }
    // 大洞：把洞首交给播放侧跳队头（与「重建解码器」同一判据、同一时刻）。
    let _ = emit(
        PlaybackFrame { seq: hole_start, samples: Vec::new() },
        Emission::Hole,
    );
}
```

**(c)** `push_playback_frame` 识别 Hole 并让位（`runtime.rs:3897-3908`）：

```rust
fn push_playback_frame(
    handle: &PlayoutHandle,
    frame: PlaybackFrame,
    emission: Emission,
    conceal_ceiling: usize,
) -> bool {
    match emission {
        // 大洞标记：不进队（不是音频），只把序号交给播放端跳队头。
        // 让位 ≠ 拒收：返回 true，不污染 queue_rejects（与掩帧让位同一条纪律）。
        Emission::Hole => {
            handle.note_dropped_seq(frame.seq);
            true
        }
        Emission::Conceal if handle.should_yield_conceal(conceal_ceiling) => {
            handle.note_dropped_seq(frame.seq);
            true
        }
        _ => handle.try_push(frame),
    }
}
```

播放侧**零改动**：`covers_gap` 会命中这个标记（`m = 洞首 = expected`，`rd = 0 < distance`），
走 `ReadyAfterRejectedGap`，**D 拍压成 1 拍**。

#### 7.3 速率与摆幅

- **一个洞只跳一次**：`covers_gap` 命中即清槽（`!covered` 门控，`runtime.rs:3283`）。
- **洞本身稀少**：只有 >120 ms 的洞才标记，正常网络下几乎不出现。
- **跳队头即时消费一帧**，队列立刻腾出空间，不存在 guardB 式的「不消费」。
- 16 槽容量对一个「>120 ms 洞」的频次而言绰绰有余。

#### 7.4 时间轴影响与取舍

| 场景 | 影响 |
|---|---|
| **非排播（单机收发，当前使用形态）** | 无害。跳过的是永远不会有帧的序号空间，播放立刻恢复。 |
| **排播（组内同步）** | 游标前移 D 帧 ⇒ 该设备相对组基准错位，破坏 ≤10 ms 组同步。 |

**本轮决定**：用户 2026-10-20 确认「目前没用组同步，先设定为单机收发」，
故 S1 **不做 `scheduled_mode` 门控，全模式生效**；
但保留一条 `if scheduled_mode` 的分支注释与 TODO，未来启用组同步时必须回来重新评估
（届时可选：门控 S1、或把跳队头改为「跳队头 + 重新锚定 epoch」）。

> ⚠️ 这是本文**唯一已知会改变用户可感知行为**的取舍，必须写进看板。

### S2 —— 水位口径修正 + 锁死自愈

#### 7.5 水位口径扣除「游标落后量」

现状（`runtime.rs:5400`）：

```rust
let buffered_frames = frames.len() + usize::from(pending.is_some());
```

追赶期队列里的 D 帧是「**游标还没走到的等待帧**」，它们不是积压。
把它们算进水位 → 虚高 D 帧 → 误触发降档/结算 → 结算丢的却是真实音频
（这正是 `docs/12:509` 记录的「late_drops 与 underruns 同源同步」的成因）。

改为：

```rust
// 游标落后量：队头帧序号 - 当前游标。追赶期这部分是「等待帧」，不是积压。
let catchup_lag = expected_seq
    .zip(peek_frame(&frames, &mut pending).map(|f| f.seq))
    .map_or(0, |(e, head)| usize::try_from(head.wrapping_sub(e)).unwrap_or(0))
    .min(frames.len());
let buffered_frames =
    (frames.len() + usize::from(pending.is_some())).saturating_sub(catchup_lag);
```

**判据**：稳态下 `catchup_lag == 0`，行为与现状完全一致（回归安全）；
只有追赶期才有差异。

#### 7.6 锁死自愈（最后一道保险）

即使有 S1，仍可能出现「病态到达形状」让队列贴满且不收敛。加一道**速率极低**的兜底：

```rust
/// 锁死判定与自愈复位（docs/77 §7.6）。
///
/// 为什么需要它：73/75/76 之后仍有四条出口全堵死的组合（§3.5），
/// 现状唯一解是用户手动停止播放。这道保险把「必须手动重启」换成「自动复位一次」。
const LOCKUP_GRACE: Duration = Duration::from_secs(3);
const RESET_MIN_INTERVAL: Duration = Duration::from_secs(10);

// 每拍末尾：
let locked = in_gap_since.is_some_and(|t| t.elapsed() >= LOCKUP_GRACE)
    && frames.len() + usize::from(pending.is_some()) + 1 >= capacity;
if locked && last_reset_at.is_none_or(|t| t.elapsed() >= RESET_MIN_INTERVAL) {
    // 软复位：清队列 + 重锚游标。这是「用一次几十毫秒的断音换掉整条锁死」。
    while frames.try_recv().is_ok() {}
    pending = None;
    expected_seq = None;
    backlog.reset();
    playout_gap = PlayoutGapState::default();
    last_reset_at = Some(Instant::now());
    if let Ok(mut telemetry) = telemetry.lock() {
        telemetry.record_playout_reset();
    }
}
```

**为什么这不是 guardA/guardB 的翻版**：

| 维度 | guardA | guardB | S2 自愈 |
|---|---|---|---|
| 频率 | 每拍 | 每次到达 | **10 s 上限一次** |
| 触发条件 | 水位超目标+2 | 到软上限 | **连续 3 s 贴满不收敛** |
| 动作 | 丢 1 帧 | 拒绝入队 | **一次整体复位** |
| 后果 | 环被抽干 | 队列自锁 | 一次可听断音，之后归零 |

诚实记账：**代价 = 一次几十~几百 ms 的可听断音**（复位后队列清空、需要重新攒水位）。
但这远优于现状的「必须手动停止播放」。

---

### S3 —— 取证：把「每秒十几」拆开

#### 7.7 新增计数（Engine 独立出口，**不进 `StreamStats`**）

按 `depth_drops`（`runtime.rs:1427`）与 `underrun_breakdown`（`runtime.rs:1439`）先例：

```rust
/// B/C 线的可解释分解（docs/77 §7.7；进程内口径，理由同 depth_drops）。
pub struct CatchupBreakdown {
    /// C 线的「洞个数」—— 与 `cursor_catchups`（拍数）分开，回答「十几个是几个洞」。
    pub catchup_gaps: u32,
    /// C 线跳过的帧数合计。
    pub catchup_frames: u32,
    /// B 线里被拒的是掩盖占位帧的数量（说明该动让位线）。
    pub reject_conceal: u32,
    /// B 线里被拒的是真实解码帧的数量（说明该动容量/洞）。
    pub reject_real: u32,
    /// 接收侧判定的洞大小直方图：[0, 1-3, 4-12, 13-30, >30] 帧。
    /// 4-12 档在掩盖上限内（不触发追赶）；13-30/>30 才是 S1 的目标。
    pub hole_hist: [u32; 5],
    /// 队列贴满的拍数（棘轮的直接影响面）。
    pub queue_full_beats: u32,
    /// S2 自愈复位次数。
    pub playout_resets: u32,
}
```

#### 7.8 出口

- **Kotlin 侧**：`AudioLinkService.dump()` 补上这七个字段（当前只打总 `underruns/lateDrops`）。
- **诊断页**：`DiagnosticsDeck` 增加「洞直方图」一行。
- **不碰** `StreamStats` / `ReceiveReport` 的 wire 面。

#### 7.9 先决地位与自我否证条件

`docs/73:91-93` 的 P0 纪律：**分量数字出来前不写生产代码**。
本方案的处置是：**S3 与 S1 同批提交**，用一轮真机同时取证并验证因果 —— 既不违背纪律意图，
也不多等一轮。

**自我否证条件**：真机 600 s 后若 `hole_hist` 显示 >12 帧的洞占比为零，
则说明 **S1 的触发条件从未发生，本轮根因判定错误，必须回退 S1 重新定位**。

---

### S4 —— 诊断工具三处缺陷

| # | 位置 | 问题 | 修法 |
|---|---|---|---|
| 1 | `PcmRingBuffer.write`（`android/.../audio/PcmRingBuffer.kt:150`） | `events.record()` 在 `synchronized(lock)` **内部**，形成 `ring.lock → EventRing.monitor` 嵌套；而内核每拍 `playoutBufferState()` 要抢 `ring.lock` ⇒ overflow 一旦发生就是正反馈 | 移到锁外（与 `readInto` 的 `:219-223` 一致） |
| 2 | `IntervalRing.record` / `EventRing.record` | `@Synchronized` 实例 monitor 落在实时线程（FFI 回调 / 播放线程），且 `EventRing` 一把锁被三线程共用 ⇒ 优先级反转风险 | 改无锁：`AtomicLongArray` + `AtomicInteger` 头指针（record 路径零锁、零分配） |
| 3 | `AudioLinkService.dump()` | 只打总 `underruns/lateDrops`，缺 A/B/C 分量 ⇒ 看 dump 分不清「播放线程卡」还是「帧迟到」 | 与 S3 一并补 |

当前 `ringOvf=0`，第 1 条尚未触发，属**未爆的雷**。

---

## 8. 取舍与已知风险

| 风险 | 评估 | 缓解 |
|---|---|---|
| S1 让时间轴前移 D 帧 | 非排播无害；排播会破坏组同步 | 用户确认暂不用组同步；留 TODO 与看板条目 |
| S2 自愈会丢一次音频 | 一次几十~几百 ms 断音 | 10 s 速率上限 + 3 s 持续判据；远优于手动停止 |
| S1 触发条件（>12 帧洞）可能从未发生 | 若从未发生则本轮根因判定错误 | **S3 的 `hole_hist` 就是自证/否证装置** |
| 两个 S 同时上线，归因困难 | S3 计数能把两者分开 | `playout_resets` 单独计数；先看 S1 效果再观察自愈触发次数 |

---

## 9. 验收标准

### 9.1 真机（600 s，抖动爆发场景）

| 指标 | 目标 |
|---|---|
| `queue_rejects` 增量 | **< 0.5 次/s**（稳态量级；不接受爆发期持续上涨） |
| `playout_resets` | 0（理想）或 ≤1（说明 S1 未完全覆盖，但不再锁死） |
| 爆发后**无需手动停止播放**即可恢复 | **必须** |
| 水位 | P50 ≤ 80 ms；后半段不比前半段高出一帧 |
| `pcmFeed` 节拍 | `gaps == 0`、`max < 20 ms`（不许把供给侧带坏） |

### 9.2 门禁（提交前全绿）

```powershell
cargo fmt --all --check
cargo clippy --workspace --exclude audiolink-desktop --all-targets --all-features -- -D warnings
cargo nextest run --workspace --exclude audiolink-desktop --lib --tests
pwsh tools/gradlew.ps1 assembleDebug
```

重点盯的抗改护栏：

- `playout_tests.rs:511-527`：`ReceiveReport {...}` **穷举字面量** —— 加字段会编译失败（刻意保留）。
- `playout_tests.rs:184-190`：未标记的普通缺帧**必须逐拍掩盖** —— S1 只在 `Emission::Hole` 时跳，不得放宽这条。
- `queue_reject_burst.rs:242`：`queue_rejects == 0` 硬零；`:248` `schedule_slips <= 2`。
- jitter.rs 五条结算窗口用例（`943/970/988/1001`、`931`）—— S2 不得改动结算语义。
- `android_backlog_feedback_reduces_total_queue_without_output_underruns`（`playout_tests.rs:1042`）。

---

## 10. 未验清单

- [ ] **S1 未实施、未真机验证**。`hole_hist` 中 >12 帧洞的真实占比**未知**。
- [ ] **爆发场景无复现装置**：现有 `queue_reject_burst.rs` 造的是批式到达，不是「>120 ms 大洞 + 追赶锁死」，可能需要新装置。
- [ ] **S2 自愈的触发阈值**（3 s / 10 s）纯属设计取值，无实测依据。
- [ ] **排播模式完全未评估**：一旦启用组同步，S1 与 S2 都需重新评审。
- [ ] 手机当前 load average **13**、内存 14.4G/15.1G —— 负载本身是抖动源，验证结论可能被环境放大。
- [ ] `e2e=0us` 的读数异常，未查（与本问题无关，但属同类「计数器不可信」信号）。

---

## 11. 不做清单（历史已否决）

1. 水位层「每拍丢最旧」（guardA，7×）—— `docs/12:504`
2. 水位层「准入拒绝」（guardB，105×）—— `docs/12:520/532`
3. 上界取「target + 常数」—— `dab50d5`
4. 「队列满时丢队首」—— `PlayoutHandle` 只持 `Sender`，不可实现（`docs/73:69`）
5. 把 B/C 塞进 `StreamStats` —— 破坏 wire 兼容
6. 诊断页再加「队列目标滑杆 / 容量收缩」类一次性实验动作 —— 已整套删除（`docs/12:536-545`）
7. 承诺「修完零欠载」—— `docs/75:58`

---

## 12. 实施顺序与回滚

1. **S3**（只增计数，零行为改动）→ 单独提交，可安全 cherry-pick。
2. **S1**（大洞标记 → 跳队头）→ 与 S3 同批；若 `hole_hist` 显示无大洞，**回退 S1**。
3. **S2**（口径扣除 + 自愈）→ 在 S1 真机验证后实施。
4. **S4**（诊断工具缺陷）→ 独立，可与其他并行。

回滚粒度：S3 与 S1/S2 分开提交，任一不达标可单独 revert，不影响其余。

---

## 13. 实施记录（2026-10-20）

### 13.1 落地清单

| 方案 | 文件 | 改动 |
|---|---|---|
| S1 | `core/crates/audiolink-engine/src/runtime.rs` | `Emission::Hole` 变体；`receive` 在 `gap > conceal_frames` 分支投递洞首；`push_playback_frame` 对 Hole 让位（`note_dropped_seq`，不记 B） |
| S3 | `core/crates/audiolink-engine/src/telemetry.rs` | `UnderrunBreakdown` +7 字段与对应 `record_*`；`saturating_delta_since` / `underrun_breakdown` / `reset` 同步 |
| S3 | `core/crates/audiolink-engine/src/runtime.rs` | `ReceiveReport` +`reject_conceal`/`reject_real`；`receive_audio` 记帧类拆分与洞直方图；播放循环记 `catchup_gaps`/`catchup_frames`；`note_playout_beat` 记 `queue_high_water` |
| S3 | `core/crates/audiolink-ffi/src/engine_bridge.rs` + 重新生成的 `bindings/kotlin` | `PeerView` +7 字段（`catchupGaps`/`catchupFrames`/`rejectConceal`/`rejectReal`/`holeHist`/`queueHighWater`/`playoutResets`） |
| S3 | `android/.../service/PeerUiState.kt`、`AudioLinkService.kt`、`ui/screens/DiagnosticsDeck.kt`、`ui/i18n/UiStrings.kt` | 映射 + `dump()` 新增 `kernel.bc:` 一行 + 诊断页 4 行 + 中英文案 |
| S4 | `android/.../diagnostics/IntervalRing.kt`、`EventRing.kt`、`audio/PcmRingBuffer.kt` | 探针改无锁（发布戳 + 已发号窗口裁剪）；`write()` 的 overflow 探针移出 `synchronized(lock)` |

新增测试：`long_gap_emits_a_hole_mark_for_the_player_cursor_jump`（S1 正向）+ Android 侧 9 个探针用例。

### 13.2 与设计的偏差（**必须记账**）

`queue_high_water` 原设计是「播放循环每拍记一次」。**实测不可行**：

- 每拍多抢一次 `telemetry` 锁，而接收侧每包要抢**同一把**；播放线程被拖慢 ⇒ 队列更早撞满；
- 后果直接可见：`queue_reject_burst.rs:242` 的硬零断言从 **B=0 变 B=4~8**（`A=0 · C=0`，与追赶链无关）；
- 去掉这次加锁后立刻回到 B=0。

改为**搭 `note_playout_beat` 已有的加锁点**（它每拍必调，加一个 `queue_depth` 形参），零额外锁。

> 教训：在播放线程上「多抢一次锁」不是微优化问题，是**可观测的回归**。这条对后续任何往播放循环里塞统计的改动都成立。

### 13.3 门禁结果（本机，2026-10-20）

| 项 | 结果 |
|---|---|
| `cargo fmt --all --check` | ✅ |
| `cargo clippy --workspace --exclude audiolink-desktop --all-targets --all-features -- -D warnings` | ✅ |
| `cargo xt --workspace --exclude audiolink-desktop --lib` | **424/424** |
| `cargo xt -p audiolink-engine --test engine -j 1` | **62/62**（EXIT=0） |
| UniFFI 绑定再生成 | ✅（`PeerView` +7 字段） |
| `pwsh android/scripts/build-rust.ps1 -Abi arm64-v8a` | ✅ 4.2 MB |
| `gradlew assembleDebug` | ✅ BUILD SUCCESSFUL |
| `testDebugUnitTest` | **273/273** |

**关于假红**：`queue_reject_burst::queue_rejects_under_wifi_style_bursts` 与
`larger_buffer_preserves_audio_during_repeated_eighty_ms_delivery_bursts` 在**全量并行**跑时会红，
单独跑或 `-j 1` 串行跑全绿 —— 与 `docs/76:74-79` 记的「宿主 CPU 饥荒会造 B 假红」同源。
门禁验收必须以串行结果为准。

### 13.4 S4 顺带发现的、未修复的隐患

均由 Android 侧实施者报告，**本轮未改**（不在 S1/S3/S4 范围内）：

1. `FfiPcmFeed.feedPcm` 是 `@Synchronized` 且锁内调 `ring.write()`（→ `ring.lock`）：
   `FfiPcmFeed.monitor → ring.lock` 的嵌套落在**内核实时回调线程**上。目前无环（播放线程不回调 feedPcm），属死锁面。
2. `CapturePcmPull.readPcm` 同样 `@Synchronized` 内取 `CaptureRing` 的 `ReentrantLock`，采集实时线程上同构。
3. `PcmRingBuffer.sizeFrames` 仍是内核播放线程每拍唯一的阻塞点（UI 诊断页也抢同一把）。临界区只有一次 arraycopy，有界。
4. `tools/gradlew.ps1` 的首个位置参数被绑到 `-JavaHome`（`[Parameter(ValueFromRemainingArguments)]` 没能接管），
   而 `android/AGENTS.md` 与根 `AGENTS.md` 写的正是 `pwsh tools/gradlew.ps1 testDebugUnitTest` ——
   实测必须显式 `-JavaHome <JDK17+>` 才能跑。**这是一条与文档不一致的工具缺陷**，建议下一轮修 `tools/`。

### 13.5 仍未验（承接 §10）

- [ ] **真机 600 s 未做**。`hole_hist` 的 13-30 / >30 两档是否非零，直接决定 S1 是否命中根因。
- [ ] `queue_high_water` 是否随爆发爬升（棘轮的现场证据）未采。
- [ ] S2 未实施：水位口径扣除 `catchup_lag` 与 3 s/10 s 锁死自愈都还没写。
- [ ] 自愈阈值（3 s / 10 s）仍是设计取值，无实测依据。
- [ ] 排播模式仍未评估（用户当前不用组同步）。


