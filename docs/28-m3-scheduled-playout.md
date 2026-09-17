# M3 · 预约播放（接收端按 epoch 排播）

> 日期 **2026-09-16**。一句话：把协议 §7 的「该包首样本的本地目标时刻」真的接进播放线程 ——
> 起播时刻不再由「队列攒够了」决定，而是由**组基准 epoch** 决定。这是「组内 ±10 ms」的另一半：
> 上一轮做出了量具（sync-measure），这一轮让接收端**照着同一个时刻表起播**。

---

## 1. 交付物

| 层 | 内容 | 位置 |
|---|---|---|
| 纯逻辑 | EpochSchedule（组基准）+ local_target_us（§7 换算）+ PlayoutAction（等待 / 播放 / 丢弃三分支） | core/crates/audiolink-engine/src/epoch.rs |
| 播放线程 | 有排播时：未到目标 → 补静音等待（**不推进游标**）；超过目标 20 ms → 丢弃并计 late_drops；到点 → 正常播 | runtime.rs（playout_main） |
| 换算基准 | 本流首个数据报的 (seq, sample_index) → 之后各包的样本序号由 48 kHz 帧长推算 | runtime.rs（note_stream_base） |
| 控制面 | Engine::schedule_playout(peer, Option<EpochSchedule>) → SessionCommand::SchedulePlayout → 会话任务写入排播状态 | runtime.rs |
| 时间线 | EngineEvent::PlayoutScheduled { epoch_id, target_local_us, wait_us }（首次排播时报一次） | runtime.rs |

**生效条件**：只有显式设置过排播（Some(schedule)）时才介入；没设置时完全走 M1 / M2 的本地游标排播
—— 老行为一个字都没改。

---

## 2. 公式与符号（符号错了整组偏两倍）

    设 offset = 对端 − 本机（时钟估计给出，µs）
    则 local(epoch) = epoch_local_us − offset
       target(sample_index) = local(epoch) + sample_index / 48000 + lead_ms
       age 判定：now < target → 等待；now − target > 20 ms → 丢弃；否则播

- epoch_local_us 是**发送端**时钟轴上的时刻，所以接收端要用 offset 换算到本机轴；
- offset 的符号一旦反了，组内偏差会整体偏出**两倍**偏移量 —— 单测里正负两侧都钉住了；
- 本机轴 = now_monotonic_us()（进程启动以来的单调 µs，刻意不用 SystemTime：NTP 校时不能影响组同步）。

---

## 3. 验证

| 层 | 内容 |
|---|---|
| 单测（epoch.rs） | **8 项**：样本序号换算（0 / 1 / 960 / 48000）、offset 正负两侧、lead_ms 叠加、等待 / 播放（含恰好 20 ms）/ 丢弃三分支、u32::MAX 样本序号不溢出且单调、epoch 早于本机开机时也给出确定结论 |
| 单测（播放面） | **3 项**：序号 → 样本序号换算（含早于基准的重传副本、无基准时返回 None）、peek 不消费帧、排播门控（无排播不判定 / 未来 → Wait 且给出等待量 / 正好到点 → Play / 过期 5 s → Drop 且给出迟到量 / 无帧不判定） |
| 回归 | 引擎 **133 项**单测全绿（新增 11 项，原有 122 项一项不少） |
| 质量门 | fmt / clippy(-D warnings) / 内核与桌面全量测试 |

提交 **20218cb** 已推送 main；CI run **35085530753**（core / android / desktop / version-consistency）四个 job 全绿。

---

## 4. 未做（记账，不假装）

| 项 | 说明 |
|---|---|
| §7 的 buffer_ms / dac_latency_ms 补偿项 | 需要设备出厂 DAC 延迟估计；当前只用 20 ms 迟到门限 |
| GROUP_EPOCH 控制帧接线 | Engine::schedule_playout 已就绪，但 ~~0x43 的载荷收发与 dispatch 分支还没做 —— 目前要靠上层显式调用，组管理打通后由 GROUP_EPOCH 触发~~ → **已接线，见 §4.1** |
| GROUP_CREATE / JOIN / LEAVE | 临时同步组（M3 交付物 4）~~尚未实现，那几个帧仍落在 dispatch 的「未实现」计数里~~ → **已实现，见 §4.1** |
| 真机双机验证 | 两台接收端在同一 epoch 下的起播对齐与 ±10 ms P95 需要第二台设备；量具与机制都已就位 |
| 多会话 | M3 交付物 1（并行推 ≥ 8 台）~~未开始~~ → **回环 8 台已覆盖、真机仍挂账，见 §4.1** |

### 4.1 更正（第 97 轮复核，2026-09-17）

| 原表述（§4） | 今天实况 | 证据 |
|---|---|---|
| 「GROUP_EPOCH 控制帧接线｜0x43 的载荷收发与 dispatch 分支还没做」 | 已接线：收（`ControlRequest::GroupEpoch` → 换算本机时钟 → 排播）与发（`Engine::announce_group_epoch`）两端都通 | `core/crates/audiolink-engine/src/runtime.rs:3678`（接收分支）、`:1911`（会话循环发送）、`core/crates/audiolink-engine/src/dispatch.rs:317`（**未实现只剩** `SET_VOLUME_LOCK` / `TELEMETRY_PUSH`）；该轮记录 `docs/29-m3-group-epoch.md` |
| 「GROUP_CREATE / JOIN / LEAVE｜临时同步组（M3 交付物 4）尚未实现，那几个帧仍落在 dispatch 的「未实现」计数里」 | 已实现：三帧都有载荷与处理分支，成员账本与动态加入/退出可用 | `core/crates/audiolink-engine/src/runtime.rs:3641`（`GroupCreate`）、`:3664`（`GroupJoin`）、`:3671`（`GroupLeave`）；`core/crates/audiolink-engine/src/dispatch.rs:317`（未实现计数里已无这三帧）、`:487`（覆盖度测试同步更新）；该轮记录 `docs/30-m3-group-management.md` |
| 「多会话｜M3 交付物 1（并行推 ≥ 8 台）未开始」 | **回环侧已覆盖，真机侧仍挂账**：1 发 8 收（同组、`start_send_many`）跑通 | `core/crates/audiolink-engine/tests/engine/multi_session.rs:215`；实测 `docs/33-m3-multi-session.md` §9 |

**两行仍成立、不必再核**：「§7 的 `buffer_ms` / `dac_latency_ms` 补偿项」（本轮全仓 grep `dac_latency` **零命中**；`buffer_ms` 只命中 WASAPI 的设备缓冲，如 `core/crates/audiolink-audio/src/wasapi/capture.rs:44`，与 §7 的补偿项不是一回事）；「真机双机验证」（要第二台设备；本轮复查 `docs/` 与 `target/evidence/` 下无 M3 真机验收记录）。
