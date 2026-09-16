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
