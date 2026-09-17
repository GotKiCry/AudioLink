# M2 断链重连（FR-27）：现状、设计与判据

> 写作日期：2026-09-17 ｜ **行号基准：git HEAD 407bc59**（`runtime.rs` 4182 行 / `network_outage.rs` 798 行 / `session.rs` 520 行）
> 上游：`docs/48-m2-outage-boundary.md`（§2.4 根因、§4 判据陷阱、§5.1 对照实验）· `docs/01-requirements.md` FR-26/FR-27 · `docs/05-roadmap.md` M2「自愈」验收点

## 0. 结论先行

| # | 结论 | 依据 |
|---|---|---|
| 1 | 断链后运行时**没有任何重建路径**：`report_peer_gone` 把会话钉死在 `Failed`，插回网线也救不回来 | `runtime.rs:3590-3597`、`DEFAULT_IDLE_TIMEOUT` 文档 `runtime.rs:135-136` |
| 2 | 拔网 10 s（< `idle_timeout` 30 s）**不会触发任何错误**，会话全程 `Streaming`；慢的是数据面：上行静默约 4 s 后一次性放出积压 | `network_outage.rs:458-533`、`docs/48` §2.4 |
| 3 | 所以「接上 FR-27 重连」只解决「连接被判死」那一半。**M2 的 10 s 场景要达标，必须再加一条数据面静默看门狗**，把「传输层挂死」升级成 `LinkLost` | 本文 §2.6、§3.3 判据 ④ |
| 4 | 第 75 轮**曾把重连真正接上**：拔网 10 s 的恢复延迟 **4783 → 444 ms（达标）**；但接收侧留下**僵尸会话**把真会话从 `peers()` 顶掉，产品代码已回退 | 工作区快照 `network_outage.rs:832` 的 `#[ignore]` 理由（该理由指向本文）|

### 0.1 基线约定（读本文前先看这一段）

- **未标注来源的行号一律指 `git HEAD 407bc59`**；任何人可用 `git show HEAD:<path>` 复核。
- **`runtime.rs` 的 FR-27 接线当前不在工作区里**：09:49 曾存在一版候选实现（带 `reconnect_once` / `spawn_reconnect_supervisor` / `DEFAULT_RECONNECT_BUDGET`），09:54 复核时已被**撤回**（`runtime.rs` 回到 HEAD，4182 行，上述符号全部不存在）。本文 §2.9 只按**形状与函数名**记录那版候选实现的教训，**不引用它的行号** —— 代码已不在，行号引用会烂。
- 唯一仍未提交的改动是验收测试：工作区快照 `network_outage.rs`（09:54 为 910 行；HEAD 该文件只有 798 行），含 `ten_second_outage_recovers_within_budget`（第 834 行，带 `#[ignore]` 于第 832 行）与新增的会话表诊断行（第 541 行）。凡引用这些行号都写明「工作区快照」，且**它们会随未提交改动漂移**，以函数名与属性名为准。
- 本文不对撤回/未提交改动的质量背书；本文只负责「现状 + 设计 + 判据」。

---

## 1. 现状：断链后运行时实际发生什么

### 1.1 分叉点只有一个：断网时长 vs `idle_timeout`

| 断网时长 | 连接是否被判死 | 观察到的现象 | 出处 |
|---|---|---|---|
| 2 s / 4 s（< 30 s） | 否，QUIC 连接仍活 | 自愈，恢复 0.6~1.1 s | `network_outage.rs:561` / `:618` |
| **10 s（< 30 s，M2 场景）** | 否 | **能自愈，但恢复 4.2~4.8 s**；上行静默约 4 s 后一次放出 502 个包 | `network_outage.rs:687`、`docs/48` §2.4 |
| ≥ 30 s（> `idle_timeout`） | 是 | 会话进 `Failed`、表项被摘除、**永久静音** | `runtime.rs:3590-3597`、`docs/48` §2.1 |

`idle_timeout`（`runtime.rs:142`）是**唯一**会把连接判死的机制：它落到 QUIC 的 `max_idle_timeout`（`core/crates/audiolink-net/src/endpoint.rs:210-221`）。除此之外没有任何「链路沉默」检测。

### 1.2 情形 A：连接被判死 → 会话终局（谁把会话停住）

调用链（全部为 HEAD 行号，函数名 + 行号双锚点）：

```text
runtime.rs:1591  report_peer_gone(..)   握手被拒（HandshakeEvent::Rejected）
runtime.rs:1855  report_peer_gone(..)   控制流读错误  → 立刻 break（1856）
runtime.rs:2034  report_peer_gone(..)   数据报读错误  → 立刻 break（2035）
        │
        ▼
runtime.rs:3590  fn report_peer_gone(inner, session, error)
  ├─ 3591  session.apply(SessionEvent::LinkLost)
  │        → 迁移表 session.rs:163（Streaming）/ :167（Degraded）→ Reconnecting
  ├─ 3592  session.set_state(SessionState::Failed)   ← 同一函数内**立刻覆盖**
  └─ 3593  broadcast EngineEvent::PeerDisconnected
        │
        ▼  break 之后（2197-2199）
  stop_capture(2197) → stop_playout(2198) → drop_session(2199)
        └─ runtime.rs:2213 fn drop_session
             ├─ 2214-2222  从 peers 表摘掉自己（peers() 从此看不到这个对端）
             └─ 2224       再广播一条 PeerDisconnected（reason = "session closed"）
```

三条本轮核实出来的细节：

1. **状态分裂**：`apply(LinkLost)` 让 `SessionMachine` 进入 `Reconnecting`，下一行 `set_state(Failed)` 又把**快照**写成 `failed`。而 `PeerSession::snapshot()`（`runtime.rs:515-537`）读的是 `state` 字段，所以 `peers()`（`runtime.rs:1094`）报 `failed`，机器内部却停在 `reconnecting`。
2. **两条 `PeerDisconnected`**：`3593` 一条（reason 形如 `"1009: ..."`），`drop_session` 的 `2224` 又一条（reason `"session closed"`）。UI 侧必须去重，否则一次断链画两次「已断开」。
3. `SessionMachine::reconnects()`（`session.rs:246`）**全仓库没有第二个引用**（只在 `session.rs:329` 的单测里被读）——「重连了几次」目前没有任何出口，遥测/UI 都读不到。
   > **第 105 轮更正（已修复）**：这条的前提比写的时候更糟 —— 当时不是「有计数、只是没出口」，而是**整个状态机从未被驱动**：全 crate 没有任何地方 apply `ConnectRequested` / `AcceptedInbound`，`SessionMachine` 一直停在 `Idle`。所以 §1.2 第 1 条的「状态分裂」与这里的「计数没出口」**根子是同一个**：`mark_streaming()` 的 `HandshakeOk` 从 `Idle` 出发是**非法迁移**，`handshakes` / `reconnects` / `degradations` **三个计数全是死的**，状态机与 `state` 快照字段早已脱钩（这也解释了它为什么长期无人发现：`peers()` 读的是快照字段，跟状态机早就没关系了）。
   > 已于第 105 轮修复：**`8726537`** 让 `run_session` 起始按 Role 驱动起点事件（`Initiator → ConnectRequested` / `Responder → AcceptedInbound`），使 `HandshakeOk` / `LinkLost` / `ReconnectOk` 全部成为合法迁移，并把回执 apply 到表里的**当前会话**、补发 `PeerUpdated`；**`f601570`** 把 `PeerStatus.reconnects` 经 `peer_view()` 接进 `PeerView`（UI 契约）与 `types.ts`。红测试 `reconnect_receipt::reconnect_ok_is_visible_through_peers` 修前报「状态 Streaming，reconnects 0」、修后「reconnects 1」。
   > ⚠️ **副作用**：`handshakes` / `degradations` 两个计数**从现在起才会真的增长**（此前恒为 0）。任何基于「它们一直是 0」的判断或表述都要复查 —— 这也正是本轮留下这条更正的原因。

### 1.3 情形 B：连接没死 → 上行静默约 4 s（M2 的 10 s 场景走的就是这条）

`docs/48` §2.4 的两行诊断读数（由 `network_outage.rs:514-536` 的 `[outage-diag]` 打印产出）：

```text
[outage-diag] 拔网 10000 ms：插回后回程首个包 196~326 ms · 上行音频数据报 4086~4273 ms · 声音恢复 4546~4820 ms
[outage-diag] 插回后每 500 ms 的上行包数：[0, 0, 0, 0, 0, 0, 0, 0, 502, 27, 30, 25]
```

三条读数各自回答什么：

1. **回程首个包 196~326 ms** —— 对端只有收到上行包才会回包，所以「链路 0.3 s 就通了」，不是路径不可用。
2. **上行速率 12 格里前 8 格为 0（≈4 s）** —— 插回后整整 4 s，上行方向**一个包都没有**（含控制帧与保活）。
3. **一格 502，然后回落到 27/30/25**（≈50~60 pps，正是音频帧率）—— 那是拔网期间积压的全部音频**被一次性放出**（502 ≈ 10 s × 50 pps）。声音恢复 4.5 s ≈ 上行恢复 4.1 s + 抖动缓冲重新填满 0.4 s。

**为什么静默 4 s —— 证据能说到哪一步：**

- 不是「应用层没投喂」：会话循环照旧每帧调 `transmit_audio`（`runtime.rs:2059` → `send_audio_frame` `runtime.rs:2933`），控制面也在 1 Hz 发 `STREAM_STATS`（`runtime.rs:2188`），时钟探测照排（`runtime.rs:2088-2112`）。`docs/48` §5.1 的对照实验里，应用层 200 ms 投喂 49 次**全部被引擎接受**，静默期上行包数仍是 0。
- 所以结论是：**数据交给了 QUIC 的发送队列，传输层一个包都没往外发**，直到某个退避定时器到期才整批吐出（`docs/48` §2.4）。
- 机制层面（发送窗口被填满 / PTO 定时器到期才重试）目前是**推断**，本仓库没有直接验证 —— 要钉死它得在 quinn 侧加探针。这条列为未决（§4.7）。
- 引擎视角的后果：整个断网期间 `state` 保持 `streaming`（反证：未提交的预算测试 `ten_second_outage_recovers_within_budget` 断言恢复后两侧仍是 `Streaming` —— 工作区快照；若断网期间真的迁移过 `failed`，这条断言恢复后就不该成立），接收端只有欠载补静音并计数（`runtime.rs:3993`）。**这不是「断链」，是「哑」**：违反 `docs/02-architecture.md` §4「绝不静默停止」的纪律，但现行检测手段（只有读写返回错误才算断链）看不见它。

### 1.4 「会话最终停住」的两种含义

| 情形 | 谁把它停住 | 恢复手段 |
|---|---|---|
| A（≥ `idle_timeout`） | `report_peer_gone` 判失败 + `drop_session` 摘表项 + `stop_capture` 停采集 | **只能人工**：`Engine::connect`（`runtime.rs:1219`）或桌面端 `connect` command（`docs/11-m1-contract.md:381`） |
| B（< `idle_timeout`） | 没人停：传输层挂死约 4 s，播放侧只被欠载（静音） | 链路自愈，4.5 s 后回来（超预算） |

### 1.5 迁移表 vs 运行时：哪些边被接线了

| 迁移边 | 迁移表 | 谁驱动 | 现状 |
|---|---|---|---|
| `Streaming`/`Degraded` → `Reconnecting` | `session.rs:163` / `:167` | `report_peer_gone`（`3591`）、`Bye` 分支（`3511-3514`）| **只有状态、没有行为**：拨号不存在 |
| `Reconnecting` → `Streaming` | `session.rs:170` | `ReconnectOk` | **运行时从未发出**（仅 `session.rs:316-317` 单测用过）|
| `Reconnecting` → `Failed` | `session.rs:171` | `ReconnectFailed` | 运行时从未发出 |
| `Failed` → `Handshaking` | `session.rs:155` | `ConnectRequested` / `AcceptedInbound` | 有：`connect_inner`（`1230`）/ accept loop（`1171`）|

`docs/48` §2.1 那句「`SessionState::Reconnecting` 只在收到对端 `Bye` 帧时进入」在代码上确认成立：`handle_control` 的 `ControlRequest::Bye` 分支（`runtime.rs:3511-3514`）是唯一会主动写 `Reconnecting` 的地方，而它不拨号、也不重开流。

---

## 2. 设计：接上 FR-27 需要动哪些点

### 2.1 目标与不可让步的不变量

- 目标（FR-27，`docs/01-requirements.md:122`）：**网络中断恢复后 ≤ 3 s 自动重连并恢复播放；重连期间 UI 明示**（FR-26 `:121` 要求状态机含「重连中」）。
- 不变量：§5 握手与 PIN/信任判定、48 kHz 全链路、`Failed` 可重启、任何失败都有明确去向（`docs/02-architecture.md` §11、架构 §4）。

### 2.2 谁发起重建：内核自己发起，且**只有最初拨号的那一侧**负责拨号

决策：内核对「发起方」角色（`Role::Initiator`）的会话启动一条「退避重拨」路径；响应方保持被动等待对端连进来。

理由（三条，都有代码依据）：

1. **`Engine::start` 不自动监听**（`runtime.rs:636-637` 的文档明写「调用方需显式 `spawn_accept_loop`」，见 `1171`）。M2 测试里只有接收端起了 accept loop（`network_outage.rs:338`）——所以「让对端拨回来」在当前拓扑下必然落空。
2. `run_session` 已经拿到 `role: Role`（`runtime.rs:1397-1404`，类型见 `handshake.rs:43`），但**没有落到会话上**：`PeerSession`（`runtime.rs:440-466`）没有 `role` 字段。要按角色仲裁，就得补这个字段。
3. 外壳不知道「流还在不在推、采集还开着没有」（`resume_send` 的上下文），把这些判断推给桌面/Android 会破坏会话内聚。

> ⚠️ **已经踩过一次的坑**：09:49 那版（随后被撤回的）候选实现用 `stream_id.is_some()` 同时当「该不该重拨」和「要不要接回流」的判据 —— 给 `report_peer_gone` 加了一个 `resume_send` 形参，由会话循环的读错误出口传 `stream_id.is_some()`。但**接收侧也会置 `stream_id`**：`handle_control` 的 `OpenStream` 分支在 `runtime.rs:3381` 写 `*stream_id = Some(id)`。于是响应方也会拨号 —— 在 M2 拓扑里那次拨号一定落空（对方没在监听），白烧一个退避窗口；在真实设备上则可能造出双向连接（见 §4.2）。教训：**判据要用「本会话是不是由本端发起」（`Role`），不要用「本端有没有流」。**

### 2.3 用哪个 API：复用 `connect_inner`，不要新开捷径

- `connect_inner`（`runtime.rs:1230`）是唯一的拨号入口，**必须**走完整 §5 握手 + 配对：`Handshake::new`（`1499-1502`）→ `handshake.on_control`（`1551`），信任判定在 `1258` 的 `is_trusted`。任何「重连只连 QUIC 不握手」的捷径都等于绕过信任库（§3.4）。
- 它对**同地址**的第二条连接会回 `1009 BUSY`（`runtime.rs:1242-1256`）——这条查重直接决定了重连实现必须处理「旧表项」问题（§2.8）。
- `Engine::connect`（`1219`）是它的薄包装，**保留给用户手动重连**，不要拿它当自动重连入口（它会给调用方一个 15 s 的 oneshot 结果，语义是「用户点了连接」）。

### 2.4 状态机怎么走（事件 → 迁移 → 归谁 apply）

| 步 | 事件 | 迁移（迁移表行号） | 谁 apply | 副作用（必须一起做） |
|---|---|---|---|---|
| 1 | `LinkLost` | `Streaming`/`Degraded` → `Reconnecting`（`session.rs:163` / `:167`）| 看门狗或读写错误（等价于现在的 `report_peer_gone`）| 广播「重连中」快照；**保留**表项（§2.8）；入队重连请求 |
| 2a | `ReconnectOk` | `Reconnecting` → `Streaming`（`session.rs:170`）| 重拨成功**且身份一致** | `reconnects += 1`、`handshakes += 1`（`session.rs:265-268`）；恢复流；`PeerUpdated` |
| 2b | `ReconnectFailed` | `Reconnecting` → `Failed`（`session.rs:171`）| 预算耗尽或明确放弃 | 明确终局 + `PeerDisconnected`；UI 提示人工重连 |
| 3 | 任何非法组合 | `SessionTransition::Invalid`，状态不变（`session.rs:256-276`）| —— | 见 §4.6：现行 `PeerSession::apply`（`runtime.rs:552-556`）用 `let _ =` 吞掉了它 |

两条纪律：

- **重连成功不得用 `HandshakeOk` 驱动**：迁移表刻意拒绝 `Reconnecting + HandshakeOk`（`session.rs:142-143` 的设计说明，单测 `session.rs:373-385` 钉住）。新连接内部的握手由新会话任务自己按 `Handshaking → Streaming` 走（`session.rs:158`）。
- 状态只有一个权威来源：`SessionMachine` 与 `PeerSession.state` 现在**是两份**（§1.2 细节 1），接线时必须让「`peers()` 读到的状态」与「机器状态」在每次迁移后一致，否则验收读数和真实状态会对不上。

### 2.5 重连期间旧会话 / 旧流怎么处置

| 对象 | 现状（HEAD） | 重连时期的期望处置 |
|---|---|---|
| `peers` 表项 | 出错后由 `drop_session` 摘掉（`2213-2222`）| **保留**，状态写 `reconnecting` —— 否则 FR-26/27 的「UI 明示」无从实现（§3.3 判据 ⑤）|
| 上行 `capture` | 会话循环退出时 `stop_capture`（`2197`）| 断链即停（别继续往死连接里编码）；重连成功后由 `start_send`（`1285`）→ `start_send_pipeline`（`2345`）重开 |
| 下行 `playout` | `stop_playout`（`2198`）| 同上；重连后由对端 `OPEN_STREAM` 重建（`3383` 的 `spawn_playout_thread`）|
| 采集枢纽 `capture_hub` | 引用计数归零即停（`2531` `release`；`2370` `acquire_capture_hub`）| 新旧会话交接顺序决定「复用枢纽（时间轴连续）」还是「重建枢纽（`seq`/`sample_index` 从新基准起）」——两种都合法，但**必须能读到是哪一种**（加读数，§4.4）|
| 混音器源号 | `acquire_playout_mixer`（HEAD 3618）每会话一路 | 每次重连都会新开一路：`docs/37-m4-mixer.md:16` 记过「重连几次撞 8 路上限」→ 重连压测必须数源号 |
| 遥测 `telemetry` | 每 `PeerSession` 一份（`445`）| 保留表项即可保留基线；换对象 = 清零 |
| §7 排播 `pending_schedule` / 能力 `capabilities` | 每会话一份（`465` / `458`）| 换对象即丢失：组内排播与共同基准在重连后必须由上层重新下发（§4.5）|

### 2.6 ⚠️ 缺的关键一块：数据面静默看门狗

**这是本文最重要的一条设计判断。** 先把一件容易想当然的事说清楚：**在 HEAD 现有的检测集下**，光把 FR-27 状态机接上，M2 的 10 s 场景自己**不会**变 —— 因为没有任何机制会把这段沉默判成「断链」：

```text
断网 10 s < idle_timeout 30 s
  → QUIC 连接不被判死
  → control.recv()（1851）与 read_datagram_into（1875）都不返回 Err
  → report_peer_gone 永不调用
  → Reconnecting 永不进入
  → 重连永不启动，恢复仍是旧连接的 4.5 s 积压爆发
```

**第 75 轮的反证要说清楚**：那一版重连把恢复延迟从 4783 ms 压到 444 ms（工作区快照 `network_outage.rs:832`），说明确实有某条路径把它触发了；但该实现已回退、**触发路径没有留文档**。所以在重新实现时，「什么时候判 `LinkLost`」必须是一条**显式的、可读数的**判据，不能靠「某处大概会报错」。在 HEAD 现有的检测集（读写返回 `Err` + `idle_timeout` 判死）之外，至少要补上「沉默即断链」：

- **判据（发送侧）**：本端有流在推（`stream_id.is_some()`）**且连续 T 秒没有收到对端任何帧**（控制帧或数据报）→ 判 `LinkLost`。
- **现成的锚点**：对端 1 Hz 的 `STREAM_STATS` 是**双方互发**的（发送端 `runtime.rs:2188`，接收端处理在 `3516-3525` 并写 `session.peer_stats`）。给 `PeerSession` 加一个「最近一次收到对端帧的时刻」（`440-466` 各字段同形的 `AtomicU64`/`Mutex<Instant>`），再在会话循环里加一条 `select!` 分支（形如 `runtime.rs:1693` 的 1 Hz ticker）即可。
- **阈值 T**：必须 > `keep_alive`（`152`，1 s），否则健康会话会被自己的保活节奏误判；建议 1.5~2 s 起步 + 连续两次超时才判（消抖）。
- **反面方案（不要做）**：把 `idle_timeout` 压到 ≤ 1.5 s 让 QUIC 自己判死 —— 那会把「拔网 2 s 就判死」，与 M2「拔网 10 s 仍能自愈」冲突（取值论证见 `docs/48` §2.1、`runtime.rs:130-141`）。
- 合成后的完整路径（这才是达标路径）：
  ```text
  检测（≤2 s，看门狗）→ 断网期间按退避持续重拨（每次失败）→
  插回后 ≤ 1 个退避周期内建立新连接 → 握手 → 恢复流 → 抖动缓冲回填 0.4 s
  ```

### 2.7 参数与 3 s 预算反推

现役链路参数：`idle_timeout` = 30 s（`runtime.rs:142`）、`keep_alive` = 1 s（`152`）。
撤回的那版候选实现取的参数是：总预算 `DEFAULT_RECONNECT_BUDGET` = 20 s、单次尝试 `RECONNECT_ATTEMPT_TIMEOUT` = 1.5 s、退避 `RECONNECT_BACKOFF_MIN` 200 ms → `RECONNECT_BACKOFF_MAX` 1000 ms。下表**按这组取值**做预算反推 —— 结论是这组取值不够。

预算算式（从「插回网线」到「重新出声」）：

```text
恢复延迟 ≈ 相位损失(0 ~ attempt) + backoff + 握手(~0.05 s) + 流恢复(~0.1 s) + 抖动缓冲回填(0.4 s)
```

（0.4 s 来自 `docs/48` §2.4 的实测；握手与流恢复两项是量级估算，需在重连压测里标定。）

| 单次尝试 | 退避上限 | 最坏值 | 评价 |
|---|---|---|---|
| 1.5 s | 1.0 s | ≈ 3.4 s | ❌ 越过 3 s 验收阈值 |
| 0.8 s | 0.5 s | ≈ 2.0 s | ✅ 留 ≥ 1 s 余量 |
| 1.5 s | 1.0 s（但看门狗 2 s 内已开始重拨）| 仍然 ≈ 3.4 s | ❌ 看门狗救不了参数 |

建议：`RECONNECT_ATTEMPT_TIMEOUT` ≤ 800 ms、退避上限 ≤ 500 ms，总预算保持 ≥ 20 s（覆盖 10 s 拔网 + 移动网络切换的多次尝试）。理由：断网期间每次拨号都会**挂到超时**，attempt 太长会让退避循环退化成「只试一两次」，插回时正好卡在尝试中间（候选实现自己的注释里也是这么写的）。

### 2.8 建议的实现形状（与撤回那版的三点差异）

1. **不摘表项**：`connect_inner` 的按地址查重（`1242-1256`）是「先摘再拨」的直接原因。两条可行改法：(a) 重连路径跳过该查重；(b) 让重连**复用同一个 `PeerSession` 对象**（表项与状态历史都留着，只把 `Connection` 与 `run_session` 换成新的）。推荐 (b)：`Reconnecting → ReconnectOk → Streaming` 才落在 UI 看得见的对象上。
2. **`ReconnectOk` 必须 apply 到可见会话上**：撤回那版把 `ReconnectOk` apply 给了**刚从表里摘掉的旧 `PeerSession`**，而 `peers()` 读到的是 `create_session`（`2243`）新建的另一个对象（它的 `SessionMachine` 是全新的，`session.rs:220-228`）——于是 `reconnects()`/`handshakes()` 计数在 `peers()` 里读不到，历史断掉（§1.2 细节 3 的同一个问题）。
3. **按角色仲裁拨号方**（§2.2），不要用 `stream_id`。

### 2.9 一份已撤回的候选实现：三个必须避开的坑

09:49 的工作区里曾有一版 FR-27 接线（`reconnect_once` + `spawn_reconnect_supervisor` + `ReconnectQueue` 形状的请求通道），09:54 已被撤回。它的**形状本身值得记下来**，因为最容易踩的三个坑都在里面：

| # | 坑 | 为什么不成立 | 正确做法 |
|---|---|---|---|
| 1 | 用 `stream_id.is_some()` 当「该不该重拨、要不要接回流」的判据 | **接收侧也有 `stream_id`**（`runtime.rs:3381`）→ 响应方也拨号，在只有一方监听的拓扑里必然落空 | 用 `Role`（§2.2）：`Role::Initiator` 才拨，`resume_send` 由发起方按「本会话是否在推流」决定 |
| 2 | 拨号前先把旧会话从 `peers` 表里摘掉（被 `connect_inner` 的按地址查重逼出来的）| 整段断网期间 `peers()` 里没有这个对端 → FR-26/27 的「重连中 UI 明示」做不到；`ReconnectOk` 还落在被摘掉的旧对象上，计数丢失 | 保留表项 / 复用同一个 `PeerSession`（§2.8）|
| 3 | 退避参数取 1.5 s + 1.0 s，只看总预算 20 s | 插回后最坏要等「剩余尝试 + 一个退避周期」≈ 2.5 s，再叠加恢复流与抖动缓冲 0.4 s → 越过 3 s 验收阈值 | 参数按 3 s 反推（§2.7）：单次 ≤ 800 ms、退避上限 ≤ 500 ms |

另外两处（同属那版，需一并避免）：监督任务对每条请求 `spawn` 一条新任务、且用裸 `tokio::spawn` 绕过 `TaskTracker`（§4.1）；重连成功后 `start_send` 的错误被 `let _ =` 丢掉 → 「状态是 `Streaming` 但流没接回来」的哑状态同样没人报（与 §1.3 的「哑」是同一类病）。

---

## 3. 判据与验收

### 3.1 复用哪两个读数（都在 `network_outage.rs` 里现成）

| 读数 | 实现位置 |
|---|---|
| **上行速率曲线**：`fwd_up` 计数器（`105-106`，自增在 `162-166`）+ 每 500 ms 采样 12 格（`462-471`）+ 打印（`533`）| `network_outage.rs` |
| **声音恢复延迟**：`next_sound_after`（`237-257`）取「晚于插回时刻的非静音回调」，`recovery_ms` 在 `494-495` 算出 | 同上 |
| （可选）回程首包 `fwd_down`（`107-108` / `206-208`）、上行音频数据报 `fwd_big_up`（`109-111` / `211-213`）| 同上 |

### 3.2 为什么不能只看声音

三条，全部是踩过的坑（`docs/48` §4、`network_outage.rs` 文件头 `4-10`）：

1. **积压会伪装成恢复**：播放回调走无界通道，拔网期间积压的样本在插回后 **1.1 µs** 就被读出；502 个积压包一次性吐出也同样让「声音回来了」。
2. **声音是结果读数，不是机制读数**：它区分不了「新建连接把流接回来了」和「旧连接终于把积压放出来了」。只有**上行速率曲线**能区分 —— 重连成功 ⇒ 插回后第一格就有 25~30 包/0.5 s（≈50 pps）、**没有** 500 包量级的爆发柱；旧连接吐积压 ⇒ 前面 8 格 0、然后一柱 502。
3. **状态可观测性也需要断言，而且要看整张表**：未提交的预算测试只断言「恢复后两侧是 `Streaming`」，**断网期间**是什么状态没被断言 —— 而 FR-27 恰恰要求「重连期间 UI 明示」。更关键的是：它的两个状态读数都是「按 `id` 找对端」（`peers().iter().find(|peer| peer.id == ..)`），**看不见「表里有别的身份」这件事** —— 第 75 轮的僵尸会话缺陷正是从这里漏过去的（§4.2）。工作区快照新增的 `[outage-diag] 会话表（重连后）` 行（第 541 行）就是为了把整张表打出来。

### 3.3 建议的判据矩阵（编号可直接进测试断言）

| # | 读数 | 期望 | 出处 / 现状 |
|---|---|---|---|
| ① | `noise_while_cut` | `== 0`（网真的断干净） | `444` / `594-597` ✅ 已有 |
| ② | `recovery_ms` | `≤ 3000` | 未提交测试的断言；HEAD 实测 4.2~4.8 s ❌，第 75 轮重连版 444 ms ✅（但被回退）|
| ③ | `sustained_writes` | 恢复后 2 s 内 `≥ 20` 个非静音回调 | `500-503`（计数实现）+ 工作区快照 `:872-876`（断言）✅ 已有 |
| ④ | **上行速率曲线** | 插回后第 1 格即 `≥ 20`，且**不出现** `≥ 200` 的爆发柱 | 读数已有（`458-471`/`533`），断言**待加** |
| ⑤ | 断网期间 `peers()` | 该对端仍在，`state == reconnecting`（FR-26/27 UI 明示） | 现在对端直接消失（§1.2 / §4.3）❌ |
| ⑥ | 恢复后**整张会话表** | 两侧各只有一条、`id` 都等于预期对端、**无僵尸/重复会话** | 第 75 轮就栽在这里（§4.2）；`docs/37-m4-mixer.md:16` 的 8 路上限是同类教训 |
| ⑦ | 重连节拍 | `[reconnect] attempt=n backoff_ms=…` | 读数**待加**（现在没有），判据 ④ 的「谁在重拨」靠它 |

表内**裸行号一律指 `network_outage.rs` 的 HEAD 版本**（①~④ 的读数都在现有测试里现成）；写明「工作区快照」的才是未提交版本的行号。

### 3.4 安全性要求：重连不得绕过 PIN / 信任库

六条硬要求，逐条给锚点（重连是**最容易把安全口径磨薄**的地方：图快就会想「跳过握手」「复用 trusted 标记」）：

1. **重拨必须走完整握手**：`connect_inner`（`1230`）→ `is_trusted(peer_id)`（`1258`，实现 `2291-2297`：**只按 `NodeId` = 证书指纹**查信任库）→ `create_session(..., trusted)`（`2243`）→ `Handshake::new(..., trusted)`（`1499`）。不得为「更快」给 `Handshake` 传 `trusted = true`、或跳过 `AUTH_RESPONSE` 校验。
2. **身份必须一致**：新连接的对端 `peer_id` 必须等于原会话的 `NodeId`，否则不得进 `Streaming`。撤回的候选实现有这条匹配（把重拨结果与请求里记的 `peer` 比对），方向正确，且它是在完整握手**之后**才判的、不构成绕过 —— 这条要求必须保留。
3. **未信任的对端不得自动重拨**：`trusted=false`（PIN 未完成 / 被拒）时会话不应自动重拨 —— 否则等于「自动重试陌生设备」，每次尝试都会重新弹 `DisplayPin`/`PinNeeded`（`1595-1624`）刷屏，也顺手给地址劫持者提供重试便利。判据：重连入口检查 `session.trusted`（`451`）或 `is_trusted`。
4. **PIN 语义不得被重连放宽**：配对窗口权威在 `PinGate`（60 s / 5 次 / 锁 5 min），引擎只顺延握手死线（`DEFAULT_PIN_WAIT_TIMEOUT` 文档 `114-128`；`arm_pin_wait` `3224`，调用点 `1607`/`1623`）。重连不得顺延死线、不得重算尝试次数。
5. **信任库只在配对成功时写**：`session.trusted.store(true)` 与 `remember_peer` 只在 `Established{persist}` 分支（`1573-1578`）；信任库写入口只在 `remember_peer`（`2299`）。重连路径不得直接改信任库。
6. **身份不匹配的尝试不得污染会话表**：`create_session`（`2243`）会以**对端自己的 `NodeId`** 建表项（`2270-2274`），所以一次「连到别的设备」的尝试会让陌生设备短暂出现在 `peers()` 里（可能还带一次 PIN 弹窗）。验收应断言「身份不匹配的尝试不会留下陌生会话」。

---

## 4. 未决问题与风险

### 4.1 重连风暴

- 现状缺口（按 §2.9 那版候选实现的形状）：监督任务只保证「串行收请求」，但每条请求都再 `spawn` 一条独立任务 → **同一个 peer 收到两次请求就会有两套重连任务并发**（两套都会「先摘表项再拨号」，互相摘对方的会话）。请求来源不止一个：三个错误出口（HEAD `1591` / `1855` / `2034`），加上 §2.6 的看门狗 —— 看门狗判死与 QUIC 判死可能同时到达（§4.6）。
- 需要一道门：按 `NodeId` 做「单飞（in-flight）」判定，或让重连开始前先原子占位（复用 §2.8 的保留表项即可）。
- 边界：`Inner::spawn`（`581-590`）会在引擎停止后拒绝新任务（`586-588`），所以重连任务**必须走 `inner.spawn`**；撤回那版是在监督任务里直接用裸 `tokio::spawn`，**绕过了 `TaskTracker`**，`Engine::shutdown`（`1313`）不会等它 → 停止后仍可能有拨号在跑。

### 4.2 重复会话

- **这不是假想：第 75 轮已经发生过一次**（工作区快照 `network_outage.rs:832`）：重连版把拔网 10 s 的恢复延迟从 4783 ms 压到 **444 ms**（验收达标），但**接收侧留下僵尸会话、把真会话从 `peers()` 顶掉** → 产品代码回退。也就是说：**恢复时间达标不等于正确**，表项与身份归属必须自己成为断言（判据 ⑥）。
- 双向拨号（§2.2）+ 两侧 accept loop 同时工作 ⇒ 同一身份可能出现两条 QUIC 连接。`create_session` 的处理是「覆盖表项 + 给上一条发 `Shutdown`」（`2270-2279`），即**后到的踢掉先到的**；两端各自执行这一套，可能互相踢（flap），表现为「接上了又断」。
- `connect_inner` 的查重只按**地址**（`1245-1250`），抓不到「同身份、不同地址」的第二条连接。
- 已有护栏要保留并在文档里点名：旧任务退出时必须 `Arc::ptr_eq` 命中自己才摘表项（`2215-2221`，缘由见 `docs/14-pairing-state.md:18`）。

### 4.3 UI 明示与「重连占位」

- 若沿用「先摘表项再拨号」，`peers()`（`1094`）在整段断网期间**没有这个对端**；UI 只能靠 `PeerDisconnected`（`3593`）画一张「已断开」的卡片，而 FR-26/FR-27 要的是「重连中」（`docs/08-ui-spec.md:31` 定义的琥珀色态）。
- 需要决策：表项保留（本文推荐）还是新增一条「内核正在重连」的事件/快照字段。该决定要与桌面/Android 契约一起做 —— `docs/11-m1-contract.md:392` 的 `PeerView.state` 已经含 `"reconnecting"` 字面量，UI 侧不用改也能接。

### 4.4 旧流拆解 vs 新流建立的竞态

- 重连成功后 `start_send`（`1285`）→ `start_send_pipeline`（`2345`）→ `acquire_capture_hub`（`2370`）：若旧会话的 `stop_capture`（`2197`）还没跑完，共享枢纽的引用计数（`2531` `release`）决定「复用还是重建」，而重建意味着 `seq`/`sample_index` 从新基准起（时间轴复位）——这对 M4 的 `RECEIVER_EPOCH` 与组基准是**可见影响**，但目前没有任何读数能区分这两种结果。
- 播放侧：`playout_main` 退出时是否确实把自己的源从混音器摘掉 —— **未核实**（`docs/37-m4-mixer.md:16` 记过「重连几次撞 8 路上限」）。重连压测必须数源号。
- 建议读数（`tracing` 字段）：`attempt` / `backoff_ms` / `hub_rebuilt` / `resume_send_ok`。

### 4.5 对 M3/M4 的副作用（组场景）

- 换 `PeerSession` 对象会丢掉 `clock`（`447`）、`capabilities`（`458`）、`pending_schedule`（`465`）：组内排播与 M4 共同基准在重连后必须由上层重新下发。`docs/28-m3-scheduled-playout.md` / `docs/39-m4-common-time-base.md` 隐含的前提「会话一旦建立就稳定」需要补一条「重连后重新对齐」。
- 未决：重连后 `RECEIVER_EPOCH` 是否重发、由谁发；组里其他成员要不要知道「某成员重连过」（否则排播会静默错位）。

### 4.6 状态机的双重状态与静默 `Invalid`

- `machine`（`444`）与 `state`（`452`）是两份状态，现行 `report_peer_gone` 让它们互相矛盾（`3591` vs `3592`）。接上重连后这个矛盾会直接进入验收视线（`peers()` 读 `state`，计数在 `machine`）。
- `PeerSession::apply`（`552-556`）用 `let _ =` 丢弃 `Invalid`：任何「用错事件」的接线缺陷都会被静默吞掉。建议至少 `tracing::warn!` + 计数。
- 与心跳/超时的竞态：看门狗判死与 QUIC 判死（`idle_timeout`）可能**同时**到达 → 两次 `LinkLost` 入队（第二次的 `apply` 会因 `next_state(Reconnecting, LinkLost) == None` 变成 `Invalid`，但**请求已经入队**）→ 又回到 §4.1 的单飞问题。

### 4.7 未决问题清单

| 问题 | 现状 | 谁来定 |
|---|---|---|
| 看门狗阈值与具体判据（2 s？只认「收到对端帧」？）| 无实现 | 需实测：健康会话在 Android 侧是否稳定 1 Hz 上报；阈值须 > `keep_alive` |
| 传输层挂死的机制（发送窗口满 / PTO 退避）| 只有「投喂被接受但不出包」这条间接证据 | 需在 quinn 侧加探针 |
| 未信任对端是否自动重拨 | 无实现（候选版也没这条判据）| 安全侧建议：不拨（§3.4 第 3 条）|
| 重连期间是否保留表项 | 撤回那版是摘表项 | UI 契约（FR-26/27）|
| 是否允许两侧同时拨（地址可能变）| 现按原地址重拨 | 移动网络/NAT 场景要定：重拨 vs 连接迁移 |
| 断网 > `idle_timeout` 后多久放弃并提示用户 | 无实现（候选取值 20 s）| 与产品确认 |
| 混音器源号在重连后是否回收 | 未核实 | M4 混音压测 |
| 重连后 §7 组基准是否重发 | 未实现 | M3/M4 |

---

## 5. 验收状态

- [x] 拔网 2 s / 4 s：能自愈、恢复后持续出声（`network_outage.rs:561` / `:618`）
- [ ] 拔网 10 s 后 **≤ 3 s** 恢复（HEAD 实测 4.2~4.8 s；第 75 轮重连版 444 ms 但被回退，见 §0 结论 4 / §4.2）
- [ ] 恢复后 `peers()` 里**身份正确**、无僵尸/重复会话（第 75 轮的回退原因）
- [ ] 断网期间 UI 明示 `reconnecting`（当前对端直接从 `peers()` 消失）
- [ ] 数据面静默看门狗（§2.6）——**没有它，重连不会被触发**
- [ ] 重连参数按 3 s 预算反推（§2.7）
- [ ] 重连不改变 PIN/信任语义（§3.4 六条逐条补测试）
- [ ] 重连风暴 / 重复会话的单飞与去重（§4.1 / §4.2）

## 6. 复现

```text
cd core
cargo nextest run -p audiolink-engine --test engine -E "test(/outage/)" --success-output final
```

四条测试：2 s / 4 s / 10 s 拔网 + 应用层探活对照实验，均打印 `[outage]` 与 `[outage-diag]` 行。
未提交的预算测试 `ten_second_outage_recovers_within_budget`（工作区快照，第 834 行）打印 `[reconnect]` 行并把 3000 ms 钉成断言；它**当前带 `#[ignore]`**（第 832 行，理由：第 75 轮重连已把恢复时间做到 444 ms，但留下僵尸会话，产品代码回退），所以上面那条 `-E` 过滤器**不会**跑到它 —— 要验预算得显式 `--run-ignored`，或等 ignore 撤掉。它旁边新增的 `[outage-diag] 会话表（重连后）` 行（第 541 行）把两侧会话表整体打出来，专门抓「表里有对端但身份不对」。
## 7. 第 76 轮：一次更深的尝试，以及它止步的地方

第 76 轮按本文 §2.6 的建议补上了**静默看门狗 + 退避重拨**，并把拨号判据从 `stream_id.is_some()`
改成 `matches!(&role, Role::Initiator) && stream_id.is_some()`（即本文 §4.2 指出的双向拨号问题）。

- ✅ **双向拨号的问题确实修掉了**：实测两端都是 `发送侧 [Streaming] · 接收侧 [Streaming]`
  （第 75 轮是 `接收侧 []`）。这条改动方向经得起检验。
- ✅ 恢复时间一度做到 **444 ms**（第 75 轮实测），验收断言转绿。
- ❌ **但第 76 轮这一版恢复延迟仍是 None**（15 s 窗口内没有非静音回调），中继转发量也偏低
  （1864 / 4890 / 1990，基线约 867~1600）；把静默阈值从 1 s 提到 3 s 结论不变。

**结论：「重连成功」与「音频接得回来」是两件事**，当前实现只做到前一件。两件事的断点在哪、
最小修法是什么，见 `docs/51-fr27-reconnect-audit.md`（独立审计）。

过程材料：产品代码已按纪律回退（主干不留已知回归），patch 留档在
`target/evidence/fr27-reconnect-attempt.patch`，验收测试仍带 `#[ignore]` 并注明理由。

**又一条方法教训**：第 75 轮那个 444 ms 是在**错误的拨号判据**下测出来的 —— 当时两侧都在拨号，
接收侧那次注定失败的拨号反而掩盖了「音频接不回来」的问题。**换了判据之后才发现真正的断点**：
这说明「指标好看」有时只是错误路径的副作用，改判据要重测全部指标，不能只验修好的那一条。
