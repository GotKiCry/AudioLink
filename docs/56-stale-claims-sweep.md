# docs/56 · 过时口径扫描与分类（task-43 步骤 1：只报不改）

> 日期 **2026-09-17**。扫描范围：`docs/**` 与全仓代码注释；目标不是"文档好不好看"，
> 而是**找出会被当现状读的历史表述**，避免后续每一轮基于错误前提判断（`docs/51` 被当现任缺口读、
> App.tsx 页脚、MainActivity 注释、`docs/12` 未修前的 30 min 对勾，都是已发生的样本）。
>
> **本文件只做分类，不改任何正文。** 等 Lead 确认 (a) 类范围后，再按"划掉不删 + 就地指向 + 说明哪一轮改的"落地。
> 代码注释**只列不改**（注释改动要过门禁，由 Lead 另派）。

---

## 1. 扫描方法与词表

| 项 | 内容 |
|---|---|
| 脚本 | `target/evidence/audit/task43/scan.py`（Python + PyYAML，按文件逐行扫描，记录 文件:行:原文:命中词） |
| docs 词表 | 尚未 / 未做 / 未开始 / 未接线 / 未实现 / 暂不 / 目前不支持 / 计划中 / TODO / FIXME / 还没有 / 没做 / 不做 / pending / not yet / not implemented |
| 额外标注 | 该行是否已在代码围栏内、是否已有"已过时/已被取代/历史"类标记 |
| 排除 | `target/**`、`node_modules/**`、`.git/**`；二进制与 lock 文件不扫 |

**命中总数**：docs **223 行 / 52 个文件**；代码注释 **148 行 / 79 个文件**。
（按词计 211 处词次，按行计 223 行：有 12 行同时命中两个词，例如"尚未…也没做"。）

按词的分布（docs）：`未做` 62、`不做` 44、`尚未` 33、`未实现` 22、`没做` 16、`未开始` 10、
`还没有` 9、`pending` 5、`TODO` 4、`未接线` 3、`暂不` 1、`not yet` 1、`not implemented` 1。

命中最多的 docs：`docs/10-handoff.md` 53（逐轮日志，天然是 (c)）、`docs/46` 10、`docs/33` 9、
`docs/53` 9、`docs/30` 8、`docs/42` 8、`docs/28` 7、`docs/privacy.md` 7、`docs/11` 6、
`docs/29` 6、`docs/31` 6、`docs/32` 6、`docs/41` 6、`docs/26` 5、`docs/54` 5、`docs/55` 5。

---

## 2. 三类统计

判定口径（关键）：**一个绝对词只有在当前 HEAD 能证伪时才算过时**。

| 类 | 含义 | 行数 | 占比 |
|---|---|---|---|
| (a) | 明确过时、可定点修（已找到反证） | **10 条**（跨 8 个文件） | 约 4% |
| (b) | 仍然成立（登记在册，避免下一轮重复核查） | **约 120 行** | 约 54% |
| (c) | 历史快照 / 逐轮日志（只加标注，不动正文） | **约 93 行** | 约 42% |

- 179 行属"高风险"（无任何过时标记、也不在代码围栏内），是 (a) 的候选池；
- 另 44 行已带标记或位于围栏内 —— 其中大部分是 `docs/41` §10 / `docs/42` §13 /
  `docs/31·32·33·27·28·29·30·34` 的 §4 修正段，即**此前 task-16/17 已修**，本轮不重复动；
- (a) 类只占 4%，说明"过时表述"不是普遍现象，而集中在**少数几处 M2/M3/M4 能力位**上 —— 这也解释了为什么它们会被反复误读：
  它们恰好都写在"能力矩阵 / 缺口清单"这种最容易被当现状引用的位置。

---

## 3. (a) 类：(会误导现状的过时表述) —— 待 Lead 确认后修

### A-1 · Android 内录在用户手册里被写成"尚未实现"

| 项 | 内容 |
|---|---|
| 位置 | `docs/manual/troubleshooting.zh-CN.md:62` |
| 原文 | `想让手机**采集**系统声音 | 尚未实现（见用户手册 §13），当前 Android 以接收播放为主` |
| 反证 | `docs/manual/user-guide.en-US.md:164` 同一条写的是 Android **system audio capture … implemented**；实现见 `android/app/src/main/kotlin/com/gotkicry/audiolink/capture/CaptureController.kt:85-89`、`capture/CaptureWiring.kt:70-73`，提交 `1c4ac55` / `4c2d21a` / `a0158a2` |
| 影响 | 中英两份手册**自相矛盾**，中文用户按此认为"手机不能内录" |
| 建议改法 | 划掉"尚未实现"，就地指向 `CaptureWiring.kt`（SDK≥29 + `RECORD_AUDIO` 门控）与 en-US 同条目，注明"由 M4 能力位轮次（`a0158a2`）落地" |

### A-2 · 同一族口径在中文用户手册里也过时

| 项 | 内容 |
|---|---|
| 位置 | `docs/manual/user-guide.zh-CN.md:152` |
| 原文 | 内录/系统声音采集归入"未实现"一栏 |
| 反证 | 同 A-1；`docs/privacy.md:128` 此前已识别过这条 |
| 影响 | 与 en-US 孪生文件不一致；两处中文手册同错，会互相"佐证" |
| 建议改法 | 同 A-1，两处一并改，保持中英一致 |

### A-3 · "真正的 8 h 长跑尚未执行"

| 项 | 内容 |
|---|---|
| 位置 | `docs/22-m2-soak-runner.md:134` |
| 原文 | `**真正的 8 h 长跑**尚未执行（工具就绪；跑一次 8 h 是环境时间成本，不是代码问题）` |
| 反证 | 8 h 长跑**此刻正在跑**：`soak-runner.exe.locked-8h`（PID 60896）启动于 2026-09-17 13:05:48，日志 `target/evidence/soak/soak-8h-head.log` 已到 `t=19905s`（28800 s 的约 69%），零异常计数 |
| 影响 | 高。这条正是"把已启动的验收当成未开始"的典型，会让下一轮重复排期 |
| 建议改法 | 划掉，就地标注"2026-09-17 已启动并运行中，日志与计数器见 `target/evidence/soak/soak-8h-head.log`；`target/` 被 gitignore，故结论以本文件为准" |

### A-4 · 能力协商文档把 Android 内录写成"CURRENT 不声明"

| 项 | 内容 |
|---|---|
| 位置 | `docs/46-m4-capability-negotiation.md` L27 / L35-36 / L177 / L181 / L189 / L207 |
| 原文 | 大意："Android 的内录尚未实现""CURRENT 不声明内录" |
| 反证 | `android/.../service/AudioLinkService.kt:338-339` 与 `capture/CaptureWiring.kt:38-73`：`SYSTEM_LOOPBACK=16u`、`MICROPHONE=32u`，按 SDK≥29 与 `RECORD_AUDIO` 门控取并集；FFI 侧 `core/crates/audiolink-ffi/src/engine_bridge.rs:76-88`、`:381-382` 已有 `EngineStartConfig.capabilities` |
| 影响 | 高。这是"能力矩阵"类表格，最容易被当现状引用，并直接导致"内录没做"的连锁误判 |
| 建议改法 | 逐处划掉，就地指向 `CaptureWiring.kt` 与 `a0158a2`，注明"由 M4 能力位轮次落地：Android 向对端声明内录/麦克风并取并集" |

### A-5 · `docs/55` §5.2 / §6.4 "手机 → PC 不可执行"（**我自己造成的过时，需明确认领**）

| 项 | 内容 |
|---|---|
| 位置 | `docs/55-device-acceptance-runbook.md` §5.2、§6.4 |
| 原文 | 大意："Android 侧无 `connect` / `start_send` 调用点 ⇒ 双向回环不可执行" |
| 反证 | 该结论在我提交 `c82ed2f`（2026-09-17 17:48）时**成立**；其后 `6f04cbf`（同一日后段，`c82ed2f` 的后代）"feat(android): 发送入口接线"落了发送方向：`MainActivity.kt:59-63`（`onConnect` / `onStartSend`）、`AudioLinkService.kt:34,39`、新增 `service/SenderUiState.kt`、`PlaybackScreen.kt:232-239` |
| 影响 | 高，且**性质最差**：它是我自己刚写下的、最"新鲜"的验收手册；下一轮按它执行会直接跳过双向回环这条验收 |
| 建议改法 | 由 Lead 派单（或授权我改）：把"不可执行"改为"**入口已就绪**（`6f04cbf`），待真机执行双向回环"；同时保留"仍属未验"的诚实标注 |

### A-6 · `lib.rs` 的 TODO(M5) 计数与注释

| 项 | 内容 |
|---|---|
| 位置 | `docs/43-m5-desktop-shell.md:3`；代码注释 `desktop/src-tauri/src/lib.rs:359` |
| 原文 | 文档："`lib.rs` 里挂着三条 `TODO(M5)`，这一轮做掉两条"；注释：`// TODO(M5)：启动后自动连接上次设备（FR-31，可关）` |
| 反证 | 当前全仓 `TODO(M5` 只剩 **1 处**（就是 `lib.rs:359` 自己）；而它所指的 FR-31 自动连接**已实现**：命令 `try_auto_connect` / `auto_connect_state`（`lib.rs:211,240,348-350`）由前端挂载后调用（`desktop/src/lib/useAudioLink.ts:220`），设置持久化与策略在 `engine_bridge.rs:564,584` |
| 影响 | 中。文档那句是"本轮做了两条"的记账，但读起来像现状；注释则是**真的遗留**（功能已做，注释没删），会让 grep 的人以为还没做 |
| 建议改法 | 文档：划掉"三条"，标注"现仅剩 1 处 TODO(M5)，且其功能已实现（见下）"。注释：`lib.rs:359` 只有**报**的份，改与不改由 Lead 定（属代码改动，需过门禁） |

### A-7 · 公共时基文档说"接收端读不到"

| 项 | 内容 |
|---|---|
| 位置 | `docs/39-m4-common-time-base.md:69-71` |
| 原文 | 大意："现在读不到：接收端没有暴露…" |
| 反证 | 同文档 §6 已给出读取路径；`core/crates/audiolink-engine/src/runtime.rs:1221` 有 `Engine::stream_axes()`，接收轴 `rx_axis` 见 `runtime.rs:505,550` |
| 影响 | 中。读"读不到"会以为要新增 API，实际已存在，只是文档没回填 |
| 建议改法 | 划掉，就地指向 `stream_axes()` 与 `rx_axis` 的行号 |

### A-8 · 桌面页脚"M5 不做"类旧口径（抽查已修，补一条确认）

| 项 | 内容 |
|---|---|
| 位置 | `desktop/src/App.tsx:9`（前端文案，非 docs） |
| 现状 | 已改为"**当时不做**（M1 范围）：托盘、开机自启、双语…"——**已是正确写法**，本轮无需再动 |
| 归类 | 列在此处只为登记"已修"，避免下一轮重复排查（属 (b)） |

### A-9 · 已发布口径里的能力矩阵（需 Lead 判断是否算过时）

| 项 | 内容 |
|---|---|
| 位置 | `docs/42-m4-*` §2.4 一带 |
| 原文 | 大意："CI 里只有 `assembleDebug`" |
| 反证 | `.github/workflows/release.yml` 现含 `assembleRelease` + keystore secrets |
| 注意 | `docs/42` §13 已有评审修正段；**本条可能已被 §13 覆盖**。若不覆盖则属 (a)，若已覆盖则归 (b) |
| 建议改法 | Lead 确认后，若未覆盖则加一行"已过时"指向 `release.yml` |

### A-10 · 无法判定但值得标注的一条

| 项 | 内容 |
|---|---|
| 位置 | `docs/50-m2-reconnect.md:320` |
| 原文 | `重连后 §7 组基准是否重发 | 未实现 | M3/M4` |
| 核查 | `reconnect_once`（`runtime.rs:4015`）重建会话走 `connect_inner`，**未找到**重发组基准的代码路径；但也没有"已实现"的证据 |
| 归类 | 暂按**仍成立**入 (b)；建议下一轮 M3/M4 复核时顺手定论，不要留在"未实现"这种模糊措辞里 |

---

## 4. (b) 类：(仍成立) —— 登记在册，下一轮不必重查

这一类的价值是**省掉重复核查**：它们是绝对词，但当前 HEAD 支持它们。

| 表述 | 位置 | 为什么仍成立 |
|---|---|---|
| 无代码签名证书 | `docs/42` §2.2 | 仓库内无签名配置；发布说明仍写"未签名" |
| 按偏差自动补偿未实现 | `docs/27-m3-*.md:112` | 只有度量与展示，无补偿执行路径 |
| §M3 偏差滑杆（±50 ms）未实现 | `docs/05` 一带 | `docs/55` §（第 309 行附近）同口径；UI 无该控件 |
| 多个发送端之间**采样级**对齐未实现 | `docs/38:32` | 公共时基（`stream_axes`）已有，但逐源对齐确实没有 —— 容易与 A-7 混淆，须分开读 |
| 跨子网真机验收阻塞 | `docs/39` | 受设备/网络环境限制，非代码问题 |
| 节点自动发现列表属 M2+ | `docs/46` §8.4（L197/L207 一带） | 未发现广播/发现实现 |
| 无损档 PCM16、混音路数无 UI 控件 | `docs/46`（L176 一带） | 界面上确实没有对应控件 |
| 遥测面板仍待做 | `docs/22:137` | 工具只产出报告文件 |
| 真实弱网不在 soak 工具内 | `docs/22:135-136` | 工具跑干净回环，弱网需 netem 注入 |
| 未信任对端不自动重拨 | `docs/50:315` | 安全侧设计决定，非缺口 |
| "未做（记账，不假装）"类小节标题 | 多份 `docs/2x-3x` | 本身就是**记账语义**，不是现状声明 |

---

## 5. (c) 类：历史快照 —— 只加标注，不动正文

| 位置 | 规模 | 处理 |
|---|---|---|
| `docs/10-handoff.md` | 53 行（最多） | 逐轮日志，**天然是历史**。建议在文首统一声明"本文件按轮次追加，早期轮次的能力缺口不代表当前 HEAD" |
| `docs/51-fr27-reconnect-audit.md` | 6 行 | 顶部已有"修复前快照"块（模板）。**细节补充**：文中提到预算测试被 `#[ignore]`（第 41 行一带），该 `#[ignore]` 现已移除（`network_outage.rs` 内 `ten_second_outage_recovers_within_budget`（约 L845）可直接跑，全文件无 `#[ignore]`）——属快照块里值得补一句"该忽略已撤销" |
| `docs/41` §10、`docs/42` §13 | 已修 | 评审修正段，本轮不动 |
| `docs/31·32·33·27·28·29·30·34` 的 §4/§4.1/§5.1 | 44 行中的主体 | task-16/17 已修，本轮不动 |
| `docs/05·12·16·50·53·54·55` | — | 近期刚改或属快照，本轮不动（例外见 A-5） |

---

## 6. 代码注释命中（**只报不改**）

**148 行 / 79 个文件**。筛掉"注释里记历史/解释为什么"这类正常叙述后，**41 条**值得 Lead 关注。
分布靠前的文件：`runtime.rs` 21、`handshake.rs` 6、`PlayoutLoop.kt` 6、`format.rs` 4、
`audiolink-ffi/src/engine_bridge.rs` 4、`PcmConversion.kt` 4、`AudioLinkService.kt` 4、`tools/sync-board.ps1` 4。

### 明确可疑（与 (a) 类同源，建议与 A-4/A-6 一并处置）

| 位置 | 注释 | 问题 |
|---|---|---|
| `core/crates/audiolink-engine/src/runtime.rs:228` | "Android 的内录尚未实现" | 与 A-4 同一条错误前提，落在**核心引擎文档注释**里，杀伤面比 doc 更大 |
| `core/crates/audiolink-engine/src/runtime.rs:21` | "…尚未落地的 M2/M3 能力" | M2/M3 均已落地（`docs/28·29·30·33·49` 有验收），措辞需复核 |
| `desktop/src-tauri/src/lib.rs:359` | `TODO(M5)：启动后自动连接上次设备（FR-31，可关）` | 功能已实现（见 A-6），注释是遗留 |
| `tools/sync-board.ps1:92` | "仍未做：8h" | 8 h 长跑正在跑（见 A-3） |
| `tools/sync-board.ps1:118` | 一条列出 `docs/31·32·33` 过时项的说明 | 那些过时项已被 task-16/17 修掉，说明本身过期 |

### 需人工过一眼（疑点未定）

`core/crates/audiolink-types/src/lib.rs:510`、`core/crates/audiolink-engine/src/epoch.rs:19`、
`core/crates/audiolink-engine/src/mixer_alignment.rs:9`、
`core/crates/audiolink-engine/src/capability_negotiation.rs:42`、
`android/.../AudioLinkApp.kt:14-15`、`android/.../ui/theme/Theme.kt:22`。

**边界说明**：注释改动要过 fmt/clippy/测试门禁，且 8 h 长跑占着机器，本轮**不动**。建议由 Lead 在长跑结束后单独排一轮"注释与能力位对齐"。

---

## 7. 扫描产物与复现

| 文件 | 内容 |
|---|---|
| `target/evidence/audit/task43/scan.py` | 扫描脚本（词表、围栏与标记识别） |
| `target/evidence/audit/task43/docs-hits.json` | docs 全量命中（223 行） |
| `target/evidence/audit/task43/high-risk.md` | 179 条高风险明细（51 个文件） |
| `target/evidence/audit/task43/already-marked.md` | 44 条已带标记/围栏内 |
| `target/evidence/audit/task43/code-hits.json` + `code-candidates.md` | 代码注释 148 行，其中 41 条候选 |
| `target/evidence/audit/task43/verify-lines.txt`、`lib-rs.txt`、`reconnect-once.txt` | 判定时逐条取回的原文片段（便于复核，不靠记忆） |

> `target/` 被 `.gitignore` 覆盖（`/target/`），所以**产物不入库**；结论以本文件为准，原件可按需重跑。

---

## 8. 待 Lead 裁决

1. **(a) 类范围**：10 条是否全修？我建议优先级为 **A-5 > A-1/A-2 > A-3 > A-4 > A-7 > A-6 > A-9 > A-10**：
   A-5 是我造成的、且指向"最容易被执行的验收手册"；A-1/A-2 是**中英自相矛盾**；A-3 是**把运行中的验收写成未开始**。
2. **A-5 归属**：`docs/55` 不在我的写范围（`docs/56`）。请明确由谁改，或授权我改这一处。
3. **(c) 类标注**：`docs/10-handoff.md` 加一句文首声明是否可接受（1 行改动，收益是终结 53 行的"像现状"问题）。
4. **代码注释**：41 条候选里，`runtime.rs:21/228` 与 `lib.rs:359` 建议与 A-4/A-6 同批；是否排单？
5. **修后交付**：确认范围后我按"划掉不删 + 就地指向 + 注明哪一轮/何证据"落地，只动 `docs/**`，
   回填本节统计与 commit hash（当前：**尚未修改任何 doc**，工作树 docs 部分与 `c82ed2f` 之后无新增改动）。
