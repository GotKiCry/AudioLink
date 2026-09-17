# M3 · 桌面端同步组面板（勾选成组 / 动态加入退出）

> 日期 **2026-09-16**。引擎侧的组账本、四个组控制帧、排播都就绪了，但用户看不见也点不到。
> 这一轮把它接成界面：**勾选设备 → 建组 → 看成员 → 让成员加入或退出**。

---

## 1. 交付物

| 层 | 内容 | 位置 |
|---|---|---|
| 命令 | list_groups / create_group(id_shorts, lead_ms) / join_group(id_short, group_id) / leave_group(id_short, group_id) | desktop/src-tauri/src/lib.rs |
| 桥接 | 视图转换（组成员短码、**epoch_id 字符串化**）+ 复用 resolve_peer（短码 → 完整指纹） | desktop/src-tauri/src/engine_bridge.rs |
| 视图形状 | GroupView / GroupMemberView（camelCase，契约风格） | desktop/src-tauri/src/view.rs |
| 前端 | 类型 + 4 个 IPC + hook 状态与动作（groups / groupBusy / createGroup / joinGroup / leaveGroup / refreshGroups） | desktop/src/types.ts、lib/ipc.ts、lib/useAudioLink.ts |
| 面板 | 勾选列表（可加入的对端）+ 提前量输入 + 建组按钮 + 组卡片（成员、epoch、逐成员退出、未入组成员的一键加入） | desktop/src/components/GroupPanel.tsx |
| 挂载 | 主界面在遥测面板上方渲染 | desktop/src/App.tsx |

**为什么 epoch_id 走字符串**：内核侧是 u64，而 JS 的安全整数只有 53 位。直接传数字会在前端
悄悄丢精度 —— 组基准恰好对不上是最难查的那类 bug，所以出网即字符串（单测钉住）。

---

## 2. 验证

| 层 | 内容 |
|---|---|
| Rust 单测 | 视图转换：组 ID / 提前量原样、**epoch 以 16 位 hex 字符串出网**、成员短码与 PeerView 同口径（NodeId::short） |
| 前端 | tsc --noEmit（严格模式，含 noUncheckedIndexedAccess）+ vite build 通过 |
| 质量门 | 桌面 clippy（-D warnings）、桌面测试、内核全量、fmt |

---

## 3. 未做（记账，不假装）

| 项 | 说明 |
|---|---|
| 成员同步质量展示 | 需要每成员的时钟估计质量（Poor → UI 明示「该设备同步质量差」），~~尚未接线~~ → **已接线，见 §4** |
| 面板自动刷新 | 目前是打开/操作后刷新；事件驱动（GroupUpdated）刷新~~留待与 PeerUpdated 一起做~~ → **已接，见 §4** |
| 真机三台同时出声 | 组内 ±10 ms P95 的真机验收仍需设备 |
| 多会话（并行推 ≥ 8 台） | M3 交付物 1 ~~未开始~~ → **回环 8 台已覆盖、真机压测仍挂账，见 §4** |
| 界面观感与人工复核 | 无头环境点不了；曲线/面板视觉与 docs/13、docs/25 同一口径 |

---

## 4. 更正（第 96 轮复核，2026-09-17）

上面 §3 的三条「未做」在写下时（2026-09-16）是真的，**此后已被做掉**。原文保留在表里（划掉的是过时部分），
这里按「原表述 / 今天实况 / 证据」补齐 —— 免得后来者再核一遍。

| 原表述（§3） | 今天实况 | 证据 |
|---|---|---|
| 「成员同步质量展示…尚未接线」 | **已接线**：引擎把每个成员的 §6.5 分级随成员表出网，桌面按分级上色（poor 红字「同步质量差」/ fair 琥珀 / good 绿）+ 悬停显示偏移 µs | 引擎：`core/crates/audiolink-engine/src/runtime.rs:686`（`GroupMember`）、`:690`（`quality: ClockQuality`）、`:1001`（`groups()`）、`:1038`（逐成员填分级）；桌面：`desktop/src-tauri/src/view.rs:64`（`quality: String`）、`desktop/src/components/GroupPanel.tsx:145-157`；该轮记录：`docs/32-m3-member-quality.md` |
| 「事件驱动（GroupUpdated）刷新留待与 PeerUpdated 一起做」 | **已接**：引擎事件 → 桥接层 `emit(EVENT_GROUPS, ..)` → 前端 `listen` → hook 里 `refreshGroups()`（事件只当「变了」的信号，明细仍走 `list_groups`） | `desktop/src-tauri/src/engine_bridge.rs:812-819`（`:819` 是 `app.emit(EVENT_GROUPS, ..)`）、`:82`（事件名常量）；`desktop/src/lib/ipc.ts:33`（TS 侧同名常量）、`:154`（`listen(EVENT_GROUPS, .. → onGroupUpdated)`）；`desktop/src/lib/useAudioLink.ts:243-248`（订阅）、`:422-428`（`refreshGroups` → `api.listGroups()`） |
| 「多会话（并行推 ≥ 8 台）｜M3 交付物 1 未开始」 | **回环侧已覆盖，真机侧仍挂账**：1 发 + 8 收（8 台同组、`start_send_many`）跑通，连测 4 次逐台样本数完全一致 | `core/crates/audiolink-engine/tests/engine/multi_session.rs:215`（`one_capture_feeds_eight_receivers`）、`:216`（`COUNT = 8`）、`:291`（`create_group`）、`:292`（`start_send_many`）；该用例自己的注释 `:207` 就写着「M3 交付物 1 的账面上写着…未开始」；实测数字见 `docs/33-m3-multi-session.md` §9 |

**两行仍成立、不必再核**：「真机三台同时出声」（要三台设备 + 同期录音）、「界面观感与人工复核」（无头环境点不了）。
