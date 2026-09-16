# M3 · 临时同步组：组管理帧与成员账本（GROUP_CREATE / JOIN / LEAVE）

> 日期 **2026-09-16**。0x43 让接收端能排播了，但「谁和谁是一组」还没有着落。
> 这一轮把组管理三帧接上，并给发送端一个真实的成员账本。

---

## 1. 交付物

| 层 | 内容 | 位置 |
|---|---|---|
| 载荷 | GroupCreatePayload{group_id, epoch_id, epoch_local_us, lead_ms, members}、GroupJoinPayload{group_id, member}、GroupLeavePayload{group_id, member}（postcard，0x40–0x42） | audiolink-engine/src/payload.rs |
| 分发 | ControlRequest::GroupCreate/Join/Leave + op()/encode_body() 映射；三帧从「本里程碑不实现」计数里移出；覆盖度测试样本表同步补齐 | audiolink-engine/src/dispatch.rs |
| 账本 | Engine 级 group_id → {epoch, lead_ms, members}（BTreeSet，便于有序快照） | audiolink-engine/src/runtime.rs |
| API | Engine::create_group(members, lead_ms) -> group_id；join_group / leave_group；groups() -> Vec<GroupSnapshot> | 同上 |
| 事件 | EngineEvent::GroupUpdated { group_id, epoch_id, members } | 同上 |
| 接收侧 | 收到 GROUP_CREATE → 用本机时钟偏移换算并排播（与 0x43 同一条路径）+ 上报 GroupUpdated | 同上 |
| 端到端 | 真双 Engine + 真 QUIC + 真 PIN：建组 → 成员排播 → 成员表可查 → 退出后账本收敛 | core/crates/audiolink-engine/tests/group_management.rs |

**成员动态加入的语义**：新成员除了收到 JOIN，还会补一条 GROUP_EPOCH —— 它需要组基准才能排播；
老成员不受影响（§7 的 FR-22「运行中第 3 台加入，前两台不中断」）。

---

## 2. 验证

| 层 | 内容 |
|---|---|
| 单测（dispatch） | 11 项：三帧纳入覆盖度测试、L1 往返逐字节一致、未实现计数只剩 SET_VOLUME_LOCK / TELEMETRY_PUSH |
| 端到端 | tests/group_management.rs：建组后账本里恰好一个组、成员表 == 被点名的那个对端、lead_ms 原样保留；**成员排播用的 epoch 必须就是建组帧里的那个**（跨引擎一致性）；成员退光后组条目被清掉 |
| 回归 | 引擎与桌面全量测试、fmt、clippy（-D warnings）全绿 |

---

## 3. 未做（记账，不假装）

| 项 | 说明 |
|---|---|
| 桌面 / Android 的「勾选成组」面板 | 引擎侧账本与事件已就绪，UI 交互属 M3 的同步组那一项 |
| 成员同步质量展示 | 需要每成员的时钟估计质量（Poor → UI 明示「该设备同步质量差」），尚未接线 |
| 真机三台同时出声 | 组内 ±10 ms P95 的真机验收仍需设备；量具（sync-measure）与机制都在位 |
| §7 的 buffer_ms / dac_latency_ms 补偿项 | 要设备出厂 DAC 延迟估计 |
| 多会话（并行推 ≥ 8 台） | M3 交付物 1 未开始 |

---

## 4. 动态加入同步组的回环验证（2026-09-17 第 61 轮补）

`Engine::join_group`（成员动态加入同步组）此前**没有任何自动化证据**：`dynamic_join.rs` 测的是
「第 3 台加入**会话**」，本文档 §2 覆盖的是建组与退出。而 join_group 的语义正是
「新成员除 JOIN 外补一条 GROUP_EPOCH —— 它需要组基准才能排播」，这条路径断掉不会有任何测试变红。

新增 `tests/engine/group_join.rs`：1 发 + 3 收，A、B 先成组并开流，随后**先让 C 加入组、再给它开流**
（刻意让组基准早于播放句柄到达，覆盖「暂存 → 开流后补应用」这条路径）。判据三条：

1. C 必须收到 `PlayoutScheduled`（空 = 没排播）；
2. C 的**非静音样本量**必须与 A 同量级（只看事件抓不住「排播了但目标时刻错位」）；
3. A、B 不被新成员打断。

### 4.1 顺带抓到的第二个真缺陷：新成员的基准用错时刻

第一次跑这条测试时它**通过**了，但打印露了馅：

```text
[group-join] 第 3 台加入后：A 新增 385920 · B 新增 385920 · C 新增 30720 个非静音样本
```

4 s 窗口里 A、B 各播了 4 s 音频，C 只播了 **0.32 s**。

根因：`join_group` 补发的 `GROUP_EPOCH` 复用了组里的 `epoch_id`，却把 `epoch_local_us` 取成
**当前时刻**。而 `epoch_local_us` 的语义是「样本序号 0 在发送端时钟上的时刻」—— 新成员拿它换算
`target = local(epoch) + sample_index/48000 + lead`，其中 `sample_index` 是**流开始以来累计的**
（加入时已是大数），于是目标时刻被推到未来好几秒，C 大半时间都停在「还没到点」的等待里。

修法：整份基准都复用组里存的那份（`state.epoch.epoch_id` + `state.epoch.epoch_local_us` + `state.lead_ms`）。

**实测**：C 的非静音样本从 30 720 → **374 400**（A、B 各 384 000），连跑 5 轮全绿；
引擎全套集成 28 项 + 单测 150 项全绿。

**教训（与 `docs/49` §5.7 同源）**：断言写 `> 0` 太松了 —— 「几乎不出声」与「完全不出声」在它眼里一样。
计数类判据要写**量级**，不要写「有没有」。

