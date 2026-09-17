# M3 · 成员同步质量展示（§6.5 分级随成员表出网）

> 日期 **2026-09-16**。协议 §7 里有一句很硬的要求：「若某成员 Poor 质量 → UI **必须**明示
> 『该设备同步质量差』」。上一轮的面板只显示成员短码 —— 它没法明示任何事，因为质量压根没出网。

---

## 1. 交付物

| 层 | 内容 | 位置 |
|---|---|---|
| 引擎 | GroupSnapshot 的成员从裸指纹改为 GroupMember { id, quality, offset_us }；groups() 为每个成员填 §6.5 分级 | core/crates/audiolink-engine/src/runtime.rs |
| 口径 | 还没有时钟估计时按 **Poor** 处理（没有估计就等于不可同步，不能默认 Good） | 同上 |
| 锁序 | groups() 先放掉组表锁、再去查会话表 —— 避免「组表 ⇄ 对端表」两把锁被同时持有 | 同上 |
| 桌面视图 | GroupMemberView 增加 quality（good / fair / poor 字符串）与 offsetUs | desktop/src-tauri/src/{view.rs, engine_bridge.rs} |
| 面板 | 成员旁显示质量：poor 红字「同步质量差」、fair 琥珀、good 绿；悬停显示偏移 µs（或「还没有时钟估计」） | desktop/src/components/GroupPanel.tsx |

---

## 2. 验证

| 层 | 内容 |
|---|---|
| 引擎端到端 | group_management：成员表按 id 比对（形状变了，断言跟着改），并断言质量分级落在三个确定值之一 |
| 桌面单测 | 视图转换：短码口径、**quality 字符串透传**、offset 原样出网 |
| 前端 | tsc --noEmit（严格）+ vite build |
| 质量门 | 内核与桌面 clippy（-D warnings）、两边全量测试、fmt |

---

## 3. 未做（记账，不假装）

| 项 | 说明 |
|---|---|
| 事件驱动的自动刷新 | 面板目前是打开/操作后刷新；~~GroupUpdated 事件驱动刷新与 PeerUpdated 一起做~~ → **已接，见 §4** |
| 质量变化时的高亮/提示 | 现在只在渲染时按分级上色，没有「刚刚变差」的提示 |
| 真机三台同时出声 | 组内 ±10 ms P95 仍需设备；量具（sync-measure）与机制都在位 |
| 多会话（并行推 ≥ 8 台） | M3 交付物 1 ~~未开始 —— 它是引擎里最后一块结构性工作（共享采集 → N 会话编码发送）~~ → **两句都已过时，见 §4** |
| 面板观感人工复核 | 无头环境点不了，与 docs/13、docs/25 同一口径 |

---

## 4. 更正（第 96 轮复核，2026-09-17）

§3 的两条「未做」在写下时是真的，**此后已被做掉**；原文保留（划掉的是过时部分）。

| 原表述（§3） | 今天实况 | 证据 |
|---|---|---|
| 「GroupUpdated 事件驱动刷新与 PeerUpdated 一起做」 | **已接**：桥接层在 `GroupUpdated` 分支 `emit(EVENT_GROUPS, ..)`，前端 `subscribeEvents.onGroupUpdated` → `refreshGroups()` | `desktop/src-tauri/src/engine_bridge.rs:812-819`；`desktop/src/lib/ipc.ts:154`；`desktop/src/lib/useAudioLink.ts:243-248`、`:422-428`（与 `docs/31-m3-group-panel.md` §4 是同一批更正）|
| 「多会话（并行推 ≥ 8 台）｜M3 交付物 1 未开始 —— 引擎里最后一块结构性工作」 | **两句都已过时**：共享采集枢纽（1 发喂 N 台的结构部分）在 `docs/33` 那一轮就落地了，8 台回环用例也已补上。**仍挂账的只有真机**：8 台设备的资源占用/稳定性与听感 | `docs/33-m3-multi-session.md` §2（CaptureHub / 每会话编码）；`core/crates/audiolink-engine/tests/engine/multi_session.rs:215`（8 台用例）、`:216`（`COUNT = 8`）|

**三行仍成立、不必再核**：「质量变化时的高亮/提示」（面板只按分级上色，没有「刚刚变差」的提示 —— 本轮复查 `GroupPanel.tsx` 仍无该逻辑）、「真机三台同时出声」、「面板观感人工复核」。
