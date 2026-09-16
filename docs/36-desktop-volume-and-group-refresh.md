# 桌面端 · 音量滑块 + 同步组事件驱动刷新

> 日期 **2026-09-16**。这一轮把两处「引擎能力已经有了、界面还没接」补齐：
> §4.1 的 SET_GAIN 变成对端卡片上的滑块，§7 的 GroupUpdated 变成面板的自动刷新。

---

## 1. 音量滑块（对端卡片）

| 层 | 内容 |
|---|---|
| UI | 对端卡片在**推流中**显示 range 滑块（0–200%，step 0.05）；aria-label 与 title 都写明「拖动时按 200 ms 渐变生效」 |
| 链路 | onGain → api.setPeerGain(idShort, gain, 200) → 桥接 resolve_peer（短码 → 完整指纹）→ Engine::set_peer_gain |
| 反馈 | 成功用 notice 报「音量已设为 N%」；非法值由**引擎边界**拒绝并人话化（不会到线上） |

**设计取舍：刻意没动原来那个三元结构。** 对端卡片的操作区把「停止推流 / 开始推流 / 等待配对」
放在一个三元表达式里；要让滑块与按钮并列，得把三元包进 fragment。上一轮我就是在这里停手的
（不为一个滑块改 JSX 结构），本轮换成**独立的 streaming 条件块渲染在操作区之上** ——
结构不动，功能到位。卡片顶部那句「不做：音量滑块」的注释也同步删掉了。

---

## 2. 同步组面板：事件驱动刷新

| 层 | 内容 |
|---|---|
| 桥接 | 新增 EVENT_GROUPS（audiolink://groups）；EngineEvent::GroupUpdated 分支除了日志还 emit 一条轻量信号（group_id / epoch_id / members） |
| 前端 | ipc 增加 EVENT_GROUPS 常量与 subscribeEvents 的 onGroupUpdated handler |
| hook | 收到信号 → refreshGroups() 拉一次最新列表（明细始终以 list_groups 为准，信号只表达「变了」） |

**为什么不轮询**：建组 / 成员加入 / 成员退出都是低频**人工动作**；事件驱动既省 IPC，也不会漏。

---

## 3. 验证

| 层 | 内容 |
|---|---|
| 前端 | tsc --noEmit（严格模式，含 noUncheckedIndexedAccess）+ vite build |
| 桌面 | clippy（-D warnings）、全部测试 |
| 内核 | 全量 clippy / 测试 / fmt（确认没有回归） |

---

## 4. 未做（记账，不假装）

| 项 | 说明 |
|---|---|
| 真机听感验证 | RMS 与滑块都只是「发出了正确的指令」；顺不顺、有没有咔哒要耳朵 |
| 音量回读 | 现在只下发不回报（UI 想显示「当前音量」需要一条回读路径） |
| 界面的视觉与交互复核 | 无头环境点不了，与 docs/13、docs/25 同一口径 |
| 真机三台同时出声 | M3 的验收项，仍需设备 |
