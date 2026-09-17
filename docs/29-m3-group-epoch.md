# M3 · GROUP_EPOCH 载荷与接线（发送端指定组基准 → 接收端排播）

> 日期 **2026-09-16**。上一轮做完了「接收端能按 epoch 排播」，但那个排播只能靠上层显式调用去设置；
> 这一轮把**协议通道**接上：发送端广播 0x43，接收端解出载荷 → 换算 → 排播生效。

---

## 1. 交付物

| 层 | 内容 | 位置 |
|---|---|---|
| 载荷 | GroupEpochPayload { epoch_id, epoch_local_us, lead_ms }（postcard，§4.1 的 0x43） | audiolink-engine/src/payload.rs |
| 分发 | ControlRequest::GroupEpoch + op()/encode_body() 映射；0x43 从「本里程碑不实现」计数里**移出** | audiolink-engine/src/dispatch.rs |
| 接收侧 | 收到 GROUP_EPOCH → 取本机时钟偏移 → 写入排播（与 schedule_playout 同一条路径） | audiolink-engine/src/runtime.rs |
| 发送侧 | Engine::announce_group_epoch(peer, epoch_id, lead_ms)：epoch_local_us 取本机 now_monotonic_us() | 同上 |
| 端到端 | 真双 Engine + 真 QUIC + 真 PIN：0x43 送达 → epoch_id 原样到达 → PlayoutScheduled 时间线 | core/crates/audiolink-engine/tests/group_epoch.rs |

---

## 2. 一个容易想错的地方（本轮实测踩到）

协议的目标时刻是：

    target = local(epoch) + sample_index / 48000 + lead_ms

也就是说接收端等的**不是** lead，而是「这一帧在流里的时刻 + lead」：测试里推流 2 s 后广播，
等待量就是约 2 s + 120 ms —— 公式正常工作时看起来像「等太久」，其实完全正确
（e2e 的断言最初就是这么写错的，已改成量级断言并在注释里写明语义）。

**推论（真机部署必须注意）**：epoch 必须与当前推流位置对齐。若发送端给一个「很久以前」的 epoch
基准，换算出来的目标时刻会落在过去，于是 §7 的 age 判定会把帧**全部丢弃**（late_drops 一路涨）。
这不是缺陷，而是「宁可丢一帧，也不延迟出声破坏组同步」的既定语义 —— 但配置 epoch 的人必须知道。

---

## 3. 验证

| 层 | 内容 |
|---|---|
| 单测（dispatch） | 11 项：包含「每个已实现命令码都有样本覆盖」的覆盖度测试、控制帧 L1 往返逐字节一致、未实现命令码与坏载荷分开计数 —— GROUP_EPOCH 从此归入已实现，样本表也补上了 |
| 端到端 | tests/group_epoch.rs：真实双 Engine + 真 QUIC + 真 PIN + NullPlayout；发送端 announce_group_epoch(0x0BAD_C0DE_1234_5678, lead 120 ms) → 接收端收到 PlayoutScheduled，断言 epoch_id 原样到达、target_local_us 是换算后的本机 µs、等待量落在「流内时刻 + 提前量」量级 |
| 回归 | 引擎与桌面全量测试、fmt、clippy（-D warnings）全绿 |

---

## 4. 未做（记账，不假装）

| 项 | 说明 |
|---|---|
| GROUP_CREATE / JOIN / LEAVE（0x40–0x42） | 组管理（成员表、动态加入退出、质量展示）~~仍未实现~~ → **已实现，见 §4.1**；0x43 是本轮唯一接上的一条 |
| §7 的 buffer_ms / dac_latency_ms 补偿项 | 要设备出厂 DAC 延迟估计 |
| 真机双机 | 两台接收端在同一 epoch 下同时出声、±10 ms P95，仍需第二台设备；量具（sync-measure）与机制都在位 |
| 多会话 | M3 交付物 1（并行推 ≥ 8 台）~~未开始~~ → **回环已覆盖、真机仍挂账，见 §4.1** |

### 4.1 更正（第 97 轮复核，2026-09-17）

| 原表述（§4） | 今天实况 | 证据 |
|---|---|---|
| 「GROUP_CREATE / JOIN / LEAVE（0x40–0x42）｜组管理（成员表、动态加入退出、质量展示）仍未实现」 | 已实现：三帧都有载荷与处理分支；成员账本与动态加入/退出（`docs/30`）、桌面勾选成组面板（`docs/31`）、成员质量展示（`docs/32`）都在 | `core/crates/audiolink-engine/src/runtime.rs:3641` / `:3664` / `:3671`；`core/crates/audiolink-engine/src/dispatch.rs:317`（未实现只剩 `SET_VOLUME_LOCK` / `TELEMETRY_PUSH`）、`:487` |
| 「多会话｜M3 交付物 1（并行推 ≥ 8 台）未开始」 | **回环已覆盖，真机仍挂账** | `core/crates/audiolink-engine/tests/engine/multi_session.rs:215`；`docs/33-m3-multi-session.md` §9 |

**两行仍成立、不必再核**：「§7 的 `buffer_ms` / `dac_latency_ms` 补偿项」（全仓 `dac_latency` 零命中）、「真机双机」（要第二台设备；无 M3 真机验收记录）。
