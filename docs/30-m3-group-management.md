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
| 桌面 / Android 的「勾选成组」面板 | 引擎侧账本与事件已就绪，~~UI 交互属 M3 的同步组那一项~~ → **桌面端已做、Android 侧未核到证据，见 §3.1** |
| 成员同步质量展示 | 需要每成员的时钟估计质量（Poor → UI 明示「该设备同步质量差」），~~尚未接线~~ → **已接线，见 §3.1** |
| 真机三台同时出声 | 组内 ±10 ms P95 的真机验收仍需设备；量具（sync-measure）与机制都在位 |
| §7 的 buffer_ms / dac_latency_ms 补偿项 | 要设备出厂 DAC 延迟估计 |
| 多会话（并行推 ≥ 8 台） | M3 交付物 1 ~~未开始~~ → **回环 8 台已覆盖、真机压测仍挂账，见 §3.1** |

### 3.1 更正（第 97 轮复核，2026-09-17）

| 原表述（§3） | 今天实况 | 证据 |
|---|---|---|
| 「桌面 / Android 的「勾选成组」面板｜UI 交互属 M3 的同步组那一项」 | **桌面端已做**（4 个命令 + 勾选建组 + 组卡片）；**Android 侧未核到证据**（`android/` 下 30 个 Kotlin 文件里 `group_id` / `GroupView` / `create_group` / `join_group` 零命中）| `desktop/src-tauri/src/lib.rs:134`（`list_groups`）；`desktop/src/components/GroupPanel.tsx`；面板那轮记录 `docs/31-m3-group-panel.md` |
| 「成员同步质量展示｜尚未接线」 | **已接线**：§6.5 分级随成员表出网，面板按分级上色 + 悬停显示偏移 | `core/crates/audiolink-engine/src/runtime.rs:686`（`GroupMember`）、`:690`（`quality: ClockQuality`）、`:1001`（`groups()`）、`:1038`（逐成员填分级）；`desktop/src-tauri/src/view.rs:64`；`desktop/src/components/GroupPanel.tsx:145`；`docs/32-m3-member-quality.md` |
| 「多会话（并行推 ≥ 8 台）｜M3 交付物 1 未开始」 | **回环 8 台已覆盖，真机压测仍挂账** | `core/crates/audiolink-engine/tests/engine/multi_session.rs:215`（`one_capture_feeds_eight_receivers`）；`docs/33-m3-multi-session.md` §9 |

**三行仍成立、不必再核**：「真机三台同时出声」、「§7 的 `buffer_ms` / `dac_latency_ms` 补偿项」（全仓 `dac_latency` 零命中）、§5.2 的两条（三组及以上与重叠成员语义 —— 本轮 grep 全仓无重叠处理，`create_group` 在 `core/crates/audiolink-engine/src/runtime.rs:851`、`join_group` 在 `:906`；真机多组听感）。

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

---

## 5. 多组共存：一个发送端同时服务两个组（2026-09-17 第 64 轮补）

组管理此前覆盖了「建组」「组内同步」「动态加入」，但**两个组同时存在**这条边界没有任何测试 ——
而它最容易写坏：组基准若是按「发送端」而不是按「组」存取，第二次 `create_group` 就会把第一个组的基准冲掉，
表现是「先建的组突然不同步了」，而单组测试全都抓不到。

新增 `tests/engine/group_multi.rs`：1 发 + 4 收，前两台一组、后两台一组，各自 `create_group`，四台一起开流。

### 5.1 判据（含一次自我更正）

第一版判据是「两组的目标时刻必须不同」——**立不住**：两次 `create_group` 相隔只有几十微秒，实测两组目标
只差 **27 µs**（而断言要求 > 1 ms，直接变红）。「各有各的基准」的直接证据是 **`epoch_id`**，不是目标时刻之差。
改判据之后：

```text
[group-multi] 四台 (epoch_id, target) [Some((11761359031561180005, 148579)), Some((11761359031561180005, 148579)), Some((16359494060613517758, 148624)), Some((16359494060613517758, 148624))]；非静音样本 [374400, 374400, 374400, 374400]
```

三条断言：① 组内两台的 `epoch_id` 相同；② 组间 `epoch_id` 不同；③ 四台都真的出声（各 374 400 = 3.88 s）。
连跑 3 次全绿；引擎集成 31 项 + 单测 150 项全绿。

### 5.2 仍未做

- 三组及以上，以及「一个设备同时属于两个组」的语义（当前 `create_group` 按成员表分发，重叠成员未定义取舍）；
- 真机多组同时出声的听感验收。


