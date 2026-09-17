# FR-27 重连独立审计：为什么「两侧 Streaming」却没有声音

> 审计日期 **2026-09-17** ｜ 审计对象：`target/evidence/fr27-reconnect-attempt.patch`（第 76 轮那版候选实现，已 `git checkout` 回退）
> 行号基准：**HEAD `cd4d6dc`**，工作区干净；`core/crates/audiolink-engine/src/runtime.rs` **4182 行**、`tests/engine/network_outage.rs` **912 行**。
> patch 自带的行号是它当时那一版的偏移量（形如 `@@ -3579,6 +3699,92 @@`），与 HEAD 相差约 120 行 —— 本文引用 patch 时一律写「patch 第 N 行」并按函数名定位，**不与 HEAD 行号混用**。
> 上游：`docs/50-m2-reconnect.md`（§2.6 看门狗 / §2.8 形状 / §2.9 三坑 / §4.1–§4.4 风险 / §7 第 76 轮记录）· `docs/48-m2-outage-boundary.md`（§2.4 根因 / §5.1 否证）· `docs/37-m4-mixer.md`（§1 owner 模式）
> 性质：**静态审计**。本次没有运行 patch（产品代码已回退），所有关于代码的断言都是「函数名 + 行号」可复核的；无法静态判定的地方一律标注「未验证」并给出验证方法（§5）。

## 0. 结论先行

| # | 结论 | 一句话依据 |
|---|---|---|
| 1 | **断点在接收侧第二个会话建立播放管线那一步**：`acquire_playout_mixer` 把「混音器槽里已经有东西」当成「我不是 owner」，而引擎级混音器槽**从第一次建起就永不置空**。重连后的接收会话因此 `is_owner=false` → 不建 sink → 每帧只把 PCM `push` 进混音器，**从不写播放设备**。 | `runtime.rs:3618-3656`（判定）、`577`/`683`/`3652`（槽的写点，全文件没有 `= None`）、`3719-3723`（factory 判定）、`3783-3793`（sink=None）、`3678-3680`（非 owner 直接 `return true`）。强度：**确定性** |
| 2 | 这不是重连专有缺陷：**任何**「接收侧第一条会话结束、第二条建立」的场景都必然哑。它同时是 M4 owner 模式的**生命周期缺口**（owner 退出后无人接管）。 | 同上；第 75 轮「旧连接自愈所以仍有声音」恰好绕开了它 |
| 3 | 「两侧 Streaming」与有没有声音无关：状态在**握手完成**时就写成 `Streaming`，播放管线是后一步（收到 `OPEN_STREAM`）才建，且建立失败与否**不影响**状态。 | `mark_streaming` `2283-2289`；`handle_control` 的 `OpenStream` `3363-3389` |
| 4 | 最小修法：给引擎级混音器补 **owner 接管**（owner 退出即让位，新会话 CAS 抢 owner 并重新打开设备，混音器对象继续复用）。 | §3.1 |
| 5 | 第 75 轮两个附带问题：①裸 `tokio::spawn` **已改走 `inner.spawn`**；②`start_send` 错误**不再被 `let _ =` 吞，但仍是「只发一条事件就完」**，对「没声音」依旧静默。 | §4；patch 第 118/124/130-132 行、第 275-283 行 |

证据强度分级：**确定性** = 代码静态可判定、无时序依赖；**推断** = 由代码结构推出但未运行验证；**未验证** = 需要读数或实验才能定论。

---

## 1. 素材、拓扑与复现

| 素材 | 说明 |
|---|---|
| `target/evidence/fr27-reconnect-attempt.patch`（356 行） | 第 76 轮产品代码改动的**唯一**证据。HEAD `cd4d6dc` 的 `runtime.rs` 里没有 `reconnect_once` / `spawn_reconnect_supervisor` / `DEFAULT_RECONNECT_BUDGET`。 |
| `tests/engine/network_outage.rs:836` | 验收测试 `ten_second_outage_recovers_within_budget`，`#[ignore]` 在 `832`；诊断打印 `[outage-diag] 会话表（重连后）`（`541`）与 `[reconnect]`（`860-869`）。 |
| `wire_up`（`307-376`） | 拓扑决定「谁拨号」：**发送侧是 Initiator**（`346` 的 `sender.connect`），接收侧只 `spawn_accept_loop`（`338`）；接收侧配 `playout` factory（`330-335`，`RecordingSink` 记录每次 `write`），发送侧配 `capture` factory（`322-324`）。 |

```text
cd core
cargo nextest run -p audiolink-engine --test engine -E "test(/outage/)" --success-output final
# 预算测试带 #[ignore]，要显式：--run-ignored
```

「有没有声音」在本测试里的唯一口径：`RecordingSink::write`（`network_outage.rs:79-83`）被调用并写进记录通道。`next_sound_after`（`237-257`）只在「**晚于插回时刻**的非静音回调」上认恢复（`494-495`）。**因此本文的断点必须解释：为什么 `write` 一次都不发生。**

---

## 2. A：断点在哪一步

### 2.1 一次重连的完整时序（发送侧 = Initiator 拨号，接收侧 = Responder 只接受）

| 步 | 动作 | 代码锚点 | 结果 |
|---|---|---|---|
| 1 | 拔网；发送侧 `last_rx_us` 停止刷新 | patch 第 174-177 行（只在 `read_datagram_into` 的 `Ok(len)` 分支写） | 静默计时开始 |
| 2 | 1 Hz ticker 触发看门狗 → 投重连请求 | patch 第 199-215 行（ticker 分支，条件 `stream_id.is_some() && matches!(role, Role::Initiator)`） | 会话快照写 `Reconnecting`（patch 344-350）；**旧会话任务不退出、旧连接不关** |
| 3 | 监督任务收请求 → `spawn(reconnect_once)` | patch 118-133 | 无单飞门（§2.6-②） |
| 4 | 每轮先 `peers.remove(&peer)` | patch 252-261 | 旧 `PeerSession` 被移出表；**没有 `Shutdown`**，旧任务继续跑（§2.6-①） |
| 5 | `connect_inner(addr)`（800 ms 超时包着） | patch 263-268；HEAD `1230-1281` | 按地址查重（HEAD `1245-1256`）因第 4 步已摘表而放行；同一 endpoint 建**新 QUIC 连接**（源端口不变） |
| 6 | 接收侧 accept → `create_session` 插表，并把**上一条同身份会话 `Shutdown`** | HEAD `1171-1197`（accept 分支无查重）· `2243-2281`（`2270-2279` previous → `Shutdown`） | **接收侧旧会话（owner）开始退出** ← 关键触发 |
| 7 | 接收侧旧会话 `break` → 收尾 | HEAD `1847`（`Shutdown => break`）→ `2197-2199`；`stop_playout` `2754-2766` | 旧播放线程退出、`remove_source`（`2758-2762`）、**sink 被 drop，播放设备释放** |
| 8 | 发送侧新会话握手完成 → ready → `ReconnectOk` + `Streaming` | patch 269-274；HEAD `1261-1280` | 发送侧表里是**新对象**、状态 `Streaming` |
| 9 | `start_send(request.peer)` → 新会话的 `StartSend` | patch 第 275 行；HEAD `1285-1292` → `1712-1747` | 新会话 `capture` 为 `None`（`1684`），走 `start_send_pipeline`（`2345-2367`） |
| 10 | `acquire_capture_hub` 复用旧枢纽 | HEAD `2370-2394`（`2375-2379` 未停则复用） | 旧发送会话未退出 ⇒ 枢纽未 `release`（`2531-2539`）⇒ **复用**，不重建、不报错（见 §2.4） |
| 11 | 发 `OPEN_STREAM` | HEAD `1731-1734` | 经新连接到达接收侧新会话 |
| 12 | 接收侧新会话 `OpenStream` → `spawn_playout_thread` | HEAD `3363-3389` → `3685-3764` | **走进去了，但拿到 `is_owner=false`** |
| 13 | `acquire_playout_mixer`：槽非空 ⇒ 非 owner | HEAD `3618-3656`，命中 `3632-3639` | `factory=None`（`3719-3723`）→ 播放线程 `sink=None`（`3783-3793`） |
| 14 | 播放线程照常跑、照常消费帧、`push` 进混音器、**不写设备** | HEAD `3662-3683`（`3669-3671` / `3678-3680`）· 调用点 `3953` / `3985` | **没有任何 `sink.write`** ⇒ `RecordingSink::write` 一次都不触发 |
| 15 | 验收读数 | `network_outage.rs:494-495`（15 s 窗口） | `recovery_ms = None`；两侧 `peers()` 都是 `Streaming`（`898-907` 的断言反而会先绿） |

**断点在第 13 / 14 步。**

### 2.2 三处代码依据（把「确定性」钉住）

1. **槽永不清空。** `playout_mixer: Mutex<Option<Arc<Mutex<PcmMixer>>>>`（`runtime.rs:577`）的写点只有：构造初始化 `683`、`acquire_playout_mixer` 的新建分支 `3652`。整个 crate 里 `playout_mixer` 只出现 6 次（`577` / `683` / `870` / `3618` / `3623` / `3716`），**没有任何 `= None`**（`870` 是 `Engine::mixer_stats` 的只读访问）。⇒ 只要引擎活着、只要接收侧建过一次播放管线，后续 `acquire_playout_mixer` 必然走 `Some(mixer)` 分支（`3632-3639`）⇒ `is_owner = false`。
2. **非 owner 不建设备，且不会报错。** `spawn_playout_thread` 里 `let factory = if is_owner { Some(..) } else { None };`（`3719-3723`）；`playout_main` 把 `factory=None` 映射成 `sink=None`（`3783-3793`），并**照常在 `3804` 发 `ready_tx.send(Ok(()))`**；`write_frame` 在 `sink` 为 `None` 时**返回 `true`**（无 mixer 直通分支 `3669-3671`；有 mixer 时 `3678-3680` 的 `return true; // 非 owner：混进去就完事`）。返回 `true` 意味着播放线程既不退出也不计数欠载 —— 它是**活着的哑巴**。
3. **整条链上没有任何一环失败。** `spawn_playout_thread` 返回 `Ok` → `handle_control` 正常回 `OPEN_STREAM_ACK`（`3402-3409`）→ 发送侧把 `stream_id` 填上（`3412-3418`）→ 发送路径照常发音频（`2050-2052` 的 `let Some(id) = stream_id else { continue }`）。这就是「重连看似成功」的机制解释。

### 2.3 嫌疑① 复核：接收侧旧会话被 `Shutdown` 后播放线程会不会停？新会话能不能重新拿到设备？

| 问 | 答 | 依据 |
|---|---|---|
| 旧会话会被 `Shutdown` 吗？ | **会，而且是必然**。`create_session` 在 `peers.insert` 拿到 `previous` 后 `spawn` 一个小任务给旧会话发 `SessionCommand::Shutdown`（`2275-2279`）；接收侧走 accept 分支（`1188-1197`），**没有**按地址查重，所以新连接一定 `create_session`，而表里那条对端就是发送侧 ⇒ 一定命中 `previous`。 | `2243-2281`、`1171-1197` |
| 播放线程会停吗？ | **会**。会话阶段 `SessionCommand::Shutdown => break`（`1847`）→ 循环退出 → `2197-2199` 依次 `stop_capture` / `stop_playout` / `drop_session`。`stop_playout`（`2754-2766`）先置 `stop`、再 `remove_source`（`2758-2762`）、最后 `drop(frames)` + `join_audio_thread`（`2763-2764`）⇒ **sink 被 drop，设备释放**。 | `1847`、`2197-2199`、`2754-2766` |
| 新会话能重新拿到设备吗？ | **不能**。由 §2.2 第 1、2 条，`is_owner=false` ⇒ 不建 sink。 | `3618-3656`、`3719-3723` |

补充：**即使旧会话还没退干净，结论也不变** —— 槽永不清空，`is_owner` 恒为 `false`。所以这不是竞态，是**确定性**。

### 2.4 嫌疑② 复核：`start_send` 会不会因为 `stream_id` / `capture_hub` 失败？

逐条看 `StartSend` 分支（`1712-1747`）：

| 检查 | 判定 |
|---|---|
| `ready.is_closed()`（`1713`） | 否 —— `Engine::start_send` 刚建了 oneshot（`1286`） |
| `capture.is_some()` → `Busy("capture is already running")`（`1716-1721`） | 否 —— **新会话**的 `capture` 是 `None`（`1684`）。这条只在**同一会话**重复 `StartSend` 时才炸 |
| `start_send_pipeline`（`1722` → `2345-2367`） | 本场景下成功：见下 |
| `stream_id` | **不是失败源**。`stream_id = Some(1)` 是本分支自己写的（`1727`），不读旧值；新会话从 `None` 开始没有任何影响。全文件读 `stream_id` 的只有 Nack 回包（`1919`）与发帧门（`2052`），都在本分支之后才成立 |
| `send_control(OpenStream)`（`1731-1734`） | 走新连接；若失败会 `stop_capture` + `stream_id=None` + `report_error`，并把 `Err` 交回 `ready` |

`acquire_capture_hub`（`2370-2394`）在**本场景下不会失败**：旧发送会话被 `peers.remove` 摘表，但**没有人给它发 `Shutdown`**（patch 252-261 只摘表），它的 `CaptureHandle` 还攥着 ⇒ `hub.subscribers ≥ 1`（`2531-2539`）、`is_stopped() == false`（`2553-2555`）⇒ 新会话**复用**枢纽（`2375-2379`），既不重建、也不会落到 `cap_unsupported`（`2383-2385`）。

结论：**嫌疑② 不成立**（至少不是主因）。但这条「复用」有代价，记两个隐患：

- 新旧两条发送会话会**同时订阅同一个 hub 广播**（`2400-2426` 各 `hub.frames.subscribe()` 一次），同一串帧被编码两次、发到两条连接上（旧的发向已死连接）。若旧会话在 `idle_timeout`（`DEFAULT_IDLE_TIMEOUT` = 30 s，`142`）之后才退出，`release` 才会把枢纽停掉（`2531-2539`），此后新会话走重建分支（`2380-2392`）——`is_stopped()` 正是为这个场景写的，所以那时 `start_send` 依然成立。**10 s 拔网走的是复用分支。**
- 上面那条「双份发音频」是**推断**，它可以解释中继转发量在不同 run 之间的巨大差异（1864 / 4890 / 1990），但本文没有证据把它钉死（§5 第 3 条）。

### 2.5 为什么「两侧都是 Streaming」

- 发送侧：新会话握手成功（`1261-1280` 的 ready）→ patch 269-274 写 `Streaming`。注意它 apply 在**已摘表的旧对象**上（§2.6-④）。
- 接收侧：新会话是**全新的 `PeerSession`**（`2243-2268`），握手完成时 `mark_streaming`（`2283-2289`）写 `Streaming` —— 这发生在打开播放设备**之前**（`OPEN_STREAM` 才建播放管线，`3363-3389`）。
- 所以「两侧 Streaming」只证明**握手与表项**没问题，完全不证明音频路径通了。这是 docs/50 §3.2 第 3 条「状态可观测性也需要断言」的第二个实证：**当前读数集里没有一个量能回答「播放设备有没有被打开」**。

### 2.6 顺带查出的四处「让故障更难定位」的缺陷（都在这版 patch 里）

**① 旧发送会话成了僵尸。** `reconnect_once` 每轮 `peers.remove`（patch 252-261）只摘表，**从不发 `Shutdown`**；因此 `2197-2199` 的收尾路径对这条会话**永不执行**，它带着 `CaptureHandle`、旧 `Connection` 和自己的 `stream_id` 一直跑到 QUIC 判死（默认 30 s）。后果：(a) 采集枢纽永不重建（§2.4）；(b) 它后续一旦读/写报错，还会**再投一次重连请求** —— 因为错误出口（patch 162-166、186-191）**不检查** `reconnect_started`（那个守卫只在看门狗分支，patch 203）。

**② 重连请求没有单飞。** 监督任务对每条请求再 spawn 一条任务（patch 124-133），而请求来源至少三个（两个错误出口 + 看门狗）。两套 `reconnect_once` 并发时都执行 `peers.remove` ⇒ **互相摘对方刚插进表里的会话**，表现为「接上又断」。docs/50 §4.1 预言过这个风险，patch 没有加门。

**③ 800 ms 超时会留下孤儿会话。** `tokio::time::timeout(RECONNECT_ATTEMPT_TIMEOUT, engine.connect_inner(..))`（patch 263-268）只丢弃**外层 future**；而 `connect_inner` 在此之前已经 `create_session`（插表，`1259`）并 `inner.spawn(run_session(..))`（`1262`）。超时后那个会话任务照旧握手、照旧 `mark_streaming` ⇒ 「本轮判失败」与「表里多了一条 Streaming」同时成立；下一轮 `peers.remove` 又把它摘掉（仍不 Shutdown）⇒ 僵尸 +1。若它占过一路混音源号，反复重连还会逼近 FR-12 的 8 路上限（`MIXER_FULL` 常量在 `3611`；docs/37 §1 记过「重连几次撞 8 路上限」）。

**④ `ReconnectOk` 落在看不见的对象上。** patch 269-274 apply 给 `old_session`（已从表里摘掉的那一个），而 `peers()` 读的是 `create_session` 新建的另一个对象（`2243-2268`，全新 `SessionMachine`）⇒ `reconnects()` / `handshakes()` 在 UI 侧读不到，历史断掉。docs/50 §2.8 第 2 条已指出，patch 未改。

---

## 3. B：最小修法

### 3.1 方案 1（推荐）：给引擎级混音器补 owner 接管

**改哪**：`runtime.rs` 的 `Inner.playout_mixer` 字段（`577`）、`acquire_playout_mixer`（`3618-3656`）、`spawn_playout_thread`（`3685-3764`）、`stop_playout`（`2754-2766`）、`playout_main`（`3767-3804` 一带）。

**为什么这样就能接回来**：断点的本质是「owner 是一个**身份**，却被记成了「槽里有没有东西」这种**一次性事实**」。把 owner 变成**可让位、可接管**的状态，重连后的新会话就会重新打开设备并写混音输出；`PcmMixer` 对象继续复用，M4 的多路混音语义**不变**（其余非 owner 会话的 `push` 仍进同一个 mixer，`write_frame` `3662-3683`）。

```rust
// ① 槽位带上 owner 生命周期（runtime.rs:577 附近）
struct PlayoutMixSlot {
    mixer: PlayoutMix,               // Arc<Mutex<PcmMixer>>
    /// owner（真正持有播放设备的那一路）是否健在。任何退出路径都必须先置 false。
    owner_alive: AtomicBool,
}
// Inner.playout_mixer: Mutex<Option<Arc<PlayoutMixSlot>>>   （683 的初始化仍是 None，语义不变）

// ② 取用：先 add_source，再用 CAS 抢 owner（3618-3656）
//    槽已存在 → add_source(new_source) 之后：
let is_owner = slot.owner_alive
    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
    .is_ok();
//    槽不存在 → 新建，owner_alive = true（is_owner = true）

// ③ spawn_playout_thread 的 factory 判定（3719-3723）不动，它会自动跟着 is_owner 变正确。
//    但 owner 要把「让位」的钥匙交给线程，覆盖「设备自己断了」的情况：
//    owner 线程持 Arc<AtomicBool>，任何退出路径都先 store(false)：
//      - sink 打开失败 return（3783-3793）
//      - break 'playout（DueFrame::Disconnected，3966）
//      - write_frame 返回 false（3953 / 3985 / 3987）

// ④ stop_playout 也要让位（2754-2766；它可能在 owner 线程仍活时被调）
//    PlayoutHandle 增加 is_owner: bool 与 owner_flag: Arc<AtomicBool>；
//    stop_playout(inner, handle) 里 if handle.is_owner { handle.owner_flag.store(false, ..) }
//    签名要加 inner：调用点两处 —— 2198（会话收尾）、3507（CloseStream）
```

**代价与影响面**：只碰 `runtime.rs`（5 处）；`audiolink-audio` 的 `PcmMixer` **不必改**；不碰协议、不碰 UI。CAS 保证并发接管只有一个赢家。

**顺带修好的**：M4 里「接收端 owner 会话先断、其他路还在」的静音缺陷；以及设备被拔掉导致 `playout_main` 自行退出后的静音。

### 3.2 方案 2（最小代价，只求 FR-27 收口）：孤儿判定

`acquire_playout_mixer` 的 `Some(mixer)` 分支加一条：**若本会话是引擎里唯一活跃的会话**（`inner.peers` 长度 ≤ 1），就当作接管（`is_owner = true`），仍然复用同一个 mixer。

- **优点**：约 5 行，不动类型、不动 `stop_playout`。重连场景下接收侧表里只剩新会话一条（旧表项已被覆盖摘除，`2270-2274`、`drop_session` 的 `Arc::ptr_eq` 护栏 `2216-2221`），条件成立。
- **缺点**：判定是「引擎里只剩我」而不是「旧 owner 真退了」，存在**短暂重叠**窗口（旧 sink 还没关、新 sink 已开）⇒ 同一台设备被两条线程写，可能爆音/双份音频；而且它在「多路 + owner 先退」时依然哑。
- **定位**：过渡方案。若只求让 `ten_second_outage_recovers_within_budget` 转绿、且 M4 多路尚未上真机，可以用它换时间；不要当最终形态。

### 3.3 方案 3（不推荐）：`stop_playout` 时把槽置空

在 owner 会话停止时 `*inner.playout_mixer.lock() = None`。

- 为什么不够：M4 多路下，其余非 owner 会话仍持有旧 mixer 的 `Arc` 并继续 `push`（`3675`），而下一个新会话会**新建另一个 mixer** ⇒ 多路混音被拆成两摊，旧的这一摊**没有人写设备**。等于用一个静音换另一个静音。
- 只在「引擎同时只服务一个接收会话」这个**没有写进契约**的假设下成立。

### 3.4 必须同批改的三处（否则「接回来了」仍会不稳）

| # | 改什么 | 为什么 |
|---|---|---|
| 1 | `reconnect_once` 摘表**之前**给旧会话发 `SessionCommand::Shutdown`（patch 252-261 那一处；`old_session` 变量已经在手边） | 消灭僵尸会话：采集枢纽交接变成确定行为（复用还是重建可判定，docs/50 §4.4）、旧会话不再二次投递重连请求、不再双份发音频 |
| 2 | 重连请求单飞：`PeerSession` 上加 `reconnect_in_flight: AtomicBool`，`report_peer_gone` 里 CAS 成功才 `send`，`reconnect_once` 终局时清位 | 三个请求来源 + 看门狗，两套 `reconnect_once` 并发会互相摘表（§2.6-②） |
| 3 | 800 ms 尝试超时的处置：把超时下沉进 `connect_inner`（它已有 15 s 的 ready 兜底，`1273-1280`），或超时后把**那一轮**已建的会话摘除 + `Shutdown` | 否则每轮超时都留一个孤儿会话（§2.6-③） |

### 3.5 修完的验收判据（在 docs/50 §3.3 的 ①–⑦ 之上再加两条）

| # | 读数 | 期望 | 怎么读 |
|---|---|---|---|
| ⑧ | 播放设备被重新打开 | 重连后 playout factory 的调用计数 **+1** | 测试侧给 factory 加一个 `AtomicU64` 自增：factory 只在 owner 分支被调用（`3719-3723`） |
| ⑨ | 混音源号不泄漏 | 重连 n 次后 `Engine::mixer_stats()`（`870`）的 `sources` 不单调增长、且 ≠ 8 | `mixer_stats().sources`（`audiolink-audio/src/mixer.rs:173` 的 `snapshot`） |

另外**必须补的一条 M4 回归**：接收端两个发送会话，让**先建的那个（owner）**断开，断言另一路仍然出声。现有 `tests/engine/mixer_convergence.rs` 只测了 `set_peer_gain(0)` 静音（`135-144`），**没有**覆盖 owner 退出 —— 也就是说 §2.2 那个缺口目前没有任何测试守着。

---

## 4. C：第 75 轮两个附带问题的复核

| 问题 | patch 里的状态 | 依据 | 判定 |
|---|---|---|---|
| 裸 `tokio::spawn` 绕过 `TaskTracker`（docs/50 §2.9 末段） | **已改**：监督任务用 `self.inner.spawn(..)`（patch 118、124），每条请求用 `engine.inner.spawn(reconnect_once(..))`（patch 130-132）；**patch 全文没有 `tokio::spawn`**。`Inner::spawn` 在引擎停止后拒绝新任务（`581-590`，`586-588`），所以这是有效修复 | patch 118-133；`runtime.rs:581-590` | ✅ 已修 |
| `start_send` 的错误被 `let _ =` 吞（docs/50 §2.9 末段 / §1.3「哑」） | **部分修**：改成 `inner.events.send(EngineEvent::Error { code, context: "reconnect succeeded but resuming the stream failed: .." })`（patch 275-283），不再丢弃。但它仍是 fire-and-forget：**不发 peer id**、**不改会话状态**（新会话已是 `Streaming`）、**不重试**、**不打日志**，而验收测试也不订阅该事件 ⇒ 对「没声音」依旧是静默失败 | patch 275-283；`network_outage.rs` 中没有任何 `EngineEvent::Error` 的订阅/断言 | ⚠️ 半个 |

关于「不发 peer id」的依据：`EngineEvent::Error` 只有 `code` 与 `context` 两个字段（调用点 `3583-3588`），会话相关的富信息要靠 `PeerUpdated` 携带 —— 这与 `3516-3520` 对「事件不带对端 id 是已知局限」的说明同源。

**建议的最小补强**（与 §3.4 同批）：`start_send` 失败时至少 (a) `tracing::warn!` + `session.set_state(SessionState::Failed)`，或重试一次；(b) 事件带上 `peer`。

---

## 5. 未验证项与下一步验证清单

| # | 待验证 | 为什么还没定论 | 怎么验（最小代价） |
|---|---|---|---|
| 1 | `start_send` 在实测里到底成功没有 | 静态看**应该**成功（§2.4），但 patch 的错误出口只发一条事件、测试不订阅 | 在 `run_outage`（`395-572`）里 `receiver.subscribe()` 并断言整场无 `EngineEvent::Error`（或把事件打出来）；也可临时在 patch 第 275 行加一条 `tracing::warn!` 跑一次 |
| 2 | 「非 owner 哑巴」这条链子的运行时证据 | 本文全部由静态代码推出（强度：确定性，但没有跑过 patch） | 判据 ⑧ + ⑨：若 `mixer_stats().sources` 涨到 2 而 factory 计数停在 1，就同时坐实「帧进了混音器」与「设备没被打开」 |
| 3 | 中继转发量 1864 / 4890 / 1990 与「基线 867~1600」的可比性 | 对照口径（整场累计 vs 窗口、含不含 QUIC 重传、是否双会话同发）在测试里**没有定义**；§2.4 的「双份发音频」只是推断 | 用 `[outage-diag] 插回后每 500 ms 的上行包数`（`533`）看曲线形状：重连成功应是第 1 格就 ~25-30 包、**无** 500 量级爆发柱（docs/50 §3.2） |
| 4 | 第 75 轮那 444 ms 的机制 | 第 75 轮代码不在手边（没有 patch 留档），无法复核它是「新会话出声」还是「旧连接自愈」 | 不必为它单独复现；判据 ⑧⑨ 补上后，下一轮的读数会直接回答「新会话有没有出声」 |
| 5 | `last_rx_us` 的刷新率在真机上是否稳定 > 阈值 | 静态看健康期双方各按 1 Hz 互发 `ClockProbe`（数据报；`2088-2113`、`clock.rs:20-25`、`STEADY_INTERVAL_MS` = 1 s），发送侧每 1 s 至少收一个数据报 ⇒ 3 s 阈值有余量；但 Android 侧是否照排**未实测** | 给看门狗分支加 tracing 计数，断言拔网前 10 s 健康期内**不产生**重连请求 |

**一条方法上的提醒**（与 docs/50 §7 同源）：本次故障真正的可观测性缺口是「**播放设备有没有被打开**」这件事**没有任何读数**。它比「状态是不是 Streaming」重要一个数量级 —— 建议把判据 ⑧ 作为 FR-27 的**常驻**读数，而不是临时调试打印。

---

## 6. 附：锚点索引

### 6.1 HEAD `cd4d6dc` · `runtime.rs`（4182 行）

| 行号 | 内容 |
|---|---|
| `142` | `DEFAULT_IDLE_TIMEOUT` = 30 s（文档 `130-141`） |
| `577` / `683` | `playout_mixer` 字段 / 初始化 |
| `581-590` | `Inner::spawn`（停止后拒绝新任务） |
| `870` | `Engine::mixer_stats` |
| `1171-1197` | accept loop（`1179` `remote_addr`；无按地址查重） |
| `1230-1281` | `connect_inner`（`1245-1256` 查重；`1259` 建会话；`1262` spawn；`1273-1280` 15 s ready 兜底） |
| `1285-1292` | `Engine::start_send` |
| `1681-1687` | 会话阶段可变状态（`playback` / `capture` / `stream_id` / `next_stream_id`） |
| `1712-1747` | `StartSend` 分支（`1716` Busy 检查；`1722` 管线；`1727` `stream_id=Some(1)`；`1731` OpenStream） |
| `1847` | `SessionCommand::Shutdown => break` |
| `1891-1897` | `ClockProbe` → `respond_clock_probe`（数据报应答） |
| `2050-2052` | 发送门：`let Some(id) = stream_id else { continue }` |
| `2088-2113` | 时钟探测发送（无角色/无流条件，稳态 1 Hz） |
| `2197-2199` | `stop_capture` / `stop_playout` / `drop_session` |
| `2213-2228` | `drop_session`（`2216-2221` `Arc::ptr_eq` 护栏） |
| `2243-2281` | `create_session`（`2270-2279` previous → `Shutdown`） |
| `2283-2289` | `mark_streaming` |
| `2345-2367` | `start_send_pipeline` |
| `2370-2394` | `acquire_capture_hub`（`2375-2379` 复用 / `2380-2392` 重建） |
| `2400-2442` | `spawn_session_encoder`（`2407`/`2429` 5 s ready 兜底） |
| `2514-2556` | `CaptureHub`（`2531-2539` `release`；`2553-2555` `is_stopped`） |
| `2741-2750` | `stop_capture`（`2746-2748` `hub.release`） |
| `2754-2766` | `stop_playout`（`2758-2762` `remove_source`） |
| `3363-3389` | `OpenStream` → `spawn_playout_thread` |
| `3412-3418` | `OpenStreamAck` → 发送侧 `stream_id` |
| `3618-3656` | `acquire_playout_mixer`（`3632-3639` 非 owner / `3640-3654` owner） |
| `3662-3683` | `write_frame`（`3669-3671` 直通 / `3678-3680` 非 owner） |
| `3685-3764` | `spawn_playout_thread`（`3716` 取 mixer；`3719-3723` factory 判定） |
| `3783-3804` | `playout_main` 建 sink（或 `None`）与 ready |
| `3953` / `3966` / `3985` | `write_frame` 的写入调用与 `break 'playout` 退出点 |

### 6.2 patch（`target/evidence/fr27-reconnect-attempt.patch`，356 行）

| patch 行 | 内容 |
|---|---|
| `13-25` | `DEFAULT_RECONNECT_BUDGET` 20 s / `RECONNECT_ATTEMPT_TIMEOUT` 800 ms / 退避 150 → 400 ms |
| `34` | `LINK_STALL_THRESHOLD_MS` = 3 000 |
| `51-55` / `65-72` | `last_rx_us` 字段 / `silent_for_ms()`（0 = 还没听到过） |
| `118-133` | `spawn_reconnect_supervisor`（走 `inner.spawn`；每条请求再 spawn） |
| `151-152` | `reconnect_started` 守卫（只守卫看门狗分支） |
| `162-166` / `186-191` | 两个错误出口的 `allow_reconnect = matches!(role, Role::Initiator) && stream_id.is_some()` |
| `174-177` | `last_rx_us` 只在数据报 `Ok` 分支刷新（控制帧不刷新） |
| `199-215` | 1 Hz ticker 里的静默看门狗（`report_peer_gone(.., true)`，**不 break**） |
| `241-315` | `reconnect_once`（`252-261` 每轮摘表；`263-268` 800 ms 包 `connect_inner`；`269-284` 成功分支 + `start_send`；`301-314` 预算耗尽） |
| `275-283` | `start_send` 失败 → 发 `EngineEvent::Error`（不再 `let _ =`） |
| `324-354` | `report_peer_gone` 新签名（`344-350` 投递请求；`351-353` 否则 `Failed`） |

