# 72 · 移除匹配码（PIN 配对）与信任库：交付记录

> 状态：**已实施**
> 决策来源：用户直接指令「完整去掉匹配码机制。Desktop 和 Android 都要完整去除这个认证机制，因为在局域网使用不存在安全问题」。
> 设计契约（冻结、只读）：[`docs/71-remove-pairing.md`](71-remove-pairing.md)。本文是**交付记录**：背景 → 做法 → 取舍 → 实测 → 未验清单。

---

## 1. 背景

改之前，两个节点要建立会话，除了 QUIC/TLS 握手，还要求：

1. 对端证书指纹命中本机**信任库**（`trust.json` / Android 私有目录同名文件）→ 直连；
2. 否则进入 **6 位配对码**流程：主机亮码、接收端输码，成功后双方写入信任库；
3. 握手后另有一次 `AUTH_CHALLENGE` / `AUTH_RESPONSE` 挑战应答（证明「私钥持有者 = 证书主体」）。

用户判定：**局域网内不存在这个安全需求**。于是本轮把这三件事一起删掉，两端同步升级、**不留旧版兼容**。

---

## 2. 做法（按 crate / 组件）

| 层 | 改动 |
|---|---|
| `audiolink-identity` | 删除 `pin.rs`、`trust.rs` 与全部再导出；`identity.rs` 删除 `sign_challenge` / `verify_challenge` / `challenge_message`；`IdentityError` 删除 `AuthFailed` / `TrustStore` 变体 |
| `audiolink-types` | `OpCode` 删除 `0x03`–`0x07`（AUTH_CHALLENGE / AUTH_RESPONSE / PAIR_REQUIRED / PAIR_SUBMIT / PAIR_RESULT）；`1002 NOT_PAIRED` **保号改名** `NO_PEER`，删除 `1003 PAIR_REJECTED` 与 `1004 AUTH_FAILED` |
| `audiolink-proto` | `DiscoveryRecord.paired` 字段与 TXT 编解码分支删除（它已无数据来源） |
| `audiolink-net` | 只改 `src/error.rs`：握手 / 对端身份错误的码从 `1004` 收敛到 `1008 BAD_REQUEST`；**TLS 配置与证书代码一行未动** |
| `audiolink-engine` | 握手阶段只保留 `Idle → AwaitHelloAck → AwaitPeer → Established`（+ `Failed`）；删除 `PinGate`、`submit_pin` / `displayed_pin` / `pending_pin_peer` / `trusted_peers` / `revoke_trust`、`EngineEvent::{DisplayPin, PinNeeded, PairCompleted}`、`EngineConfig.trust_store_path`、`PeerStatus.trusted` 与 `create_session` 的 `trusted` 参数 |
| `audiolink-ffi` | 导出面删除 `submitPin` / `displayedPin`；`kotlin_guard.rs` 的 `REQUIRED_SYMBOLS` 同步；Kotlin 绑定**用仓库既有命令重新生成**，不是手改 |
| 桌面端 | 删除 `PairDialog` 与其测试、`EVENT_PAIR_REQUIRED`、`api.submitPin` / `listTrustedPeers` / `revokeTrust`、设置页「已配对设备」区、`CODE_NOT_PAIRED` 提示分支与配对相关 i18n 键 |
| Android | 删除配对对话框 / 面板与 PIN 文案；`PairingUiState.kt` **改名保留**为 `PeerUiState.kt`（它同时承载 `PeerUi` / 状态映射，不是纯配对文件），删掉其中的 `PairingUiState`、`pinIsStale`、`normalizePin` |
| 文档 | 协议（`03`）、需求（`01`）、架构（`02`）、UI 规格（`08`）、契约（`11`）、路线图（`05`）、概览（`00`）、迁移说明（`07`）、发现（`63`）、重连（`50`）与全部用户手册 / 隐私说明 / README / PRODUCT / DESIGN 同步；`docs/14-pairing-state.md` **删除**；历史交付文档保留事实并加「该机制已移除」注记 |

---

## 3. 取舍（含用户可见的行为变化）

**保留的东西**：QUIC + TLS1.3 通道加密与完整性；双向出示自签证书（证明对端就是它出示的那张证书、且持有对应私钥）；
节点身份 `NodeId = SHA-256(证书 DER)`（用于区分设备与会话路由）；能力协商（`caps` 位图）；
`1002` 这个号码本身（它被大量挪用于「指定的对端不存在」，因此**保号改名 `NO_PEER`** 而不是删除）。

**代价与变化**：

1. **任何能连上本机 QUIC 端口（默认 UDP `58290`）的设备，接入后都会被当作正常对端接受。**
   而「接入之后会不会自动开始推流」，**两端当前不一样**，如实分端记账：
   - **桌面端：成立。** 外壳有一层状态翻译（`desktop/src-tauri/src/engine_bridge.rs:15-19`：引擎 `Streaming` + 未推流 → 视图 `idle`），
     而「有人接入时自动开始推流」开关默认开 —— 接入那一刻，本机正在播放的声音就可能被送出去。
   - **Android 端：不会触发。** `SenderUiState.shouldAutoStartSend` 的判据要求先观测到 `idle`，而引擎在活会话里从不报 `idle`
     （`android/app/src/main/kotlin/com/gotkicry/audiolink/service/SenderUiState.kt:330-337`）——这是**既有的跨端映射不一致**，
     **非本轮引入**，本轮有意不改逻辑，见 §5 第 8 条。
   - 两端共同的限制手段：关掉「设备连接后自动推流」，或不要让该端口暴露在你不信任的网络上。
   - 以上已同时写进两份用户手册（醒目提示，按端分述）与 `docs/privacy.md`。
2. **旧版本不再能连上这台机器**：`0x03`–`0x07` 五个命令与四个握手阶段直接删除，不留降级路径 ——
   带 PIN 的旧版与本版**两端必须一起升级**。
3. **`trust.json` 变成历史遗留文件**：程序不再读、不再写、不再删除它（尊重用户磁盘上的既有数据）；
   介意可手工删除，路径见用户手册 §配置与数据放在哪。
4. **「移除设备 / 取消配对」这个操作不再存在**：没有信任列表，也就没有可移除的对象。
   原先它在桌面端由 FR-18 兑现，本轮连入口一起删除。
5. **TLS 仍在，但没有人再回答「这张证书值不值得信任」**：握手只能证明「对端持有它出示的私钥」，
   不能证明「它是你想连的那台设备」。同一局域网内的第三方若先应答，同样能完成握手。
   这是「局域网无认证」这个决定的**直接代价**，如实记账，不美化。
6. **删除冗余不等于削弱加密**：被删的 `AUTH_CHALLENGE/RESPONSE` 是对 **TLS 已经证明过的事实**的第二次证明
   （「私钥持有者 = 证书主体」），删掉它不改变链路加密强度；变的只是「连接前是否还有人做准入裁决」。

---

## 4. 实测

**全部工作流收口后，由 Lead 跑的最终门禁**（原始日志：`target/evidence/remove-pairing/gate.log`、`android-gate.log`；脚本 `run-gate.ps1`、`run-android-gate.ps1`）：

| 命令 | 结果 |
|---|---|
| `cargo fmt --all --check` | **exit 0** |
| `cargo clippy --workspace --exclude audiolink-desktop --all-targets --all-features -- -D warnings` | **exit 0**，零警告 |
| `cargo nextest run --workspace --exclude audiolink-desktop --lib --tests --no-fail-fast --test-threads 2` | **542 tests run: 542 passed, 2 skipped**（255 s） |
| `cargo clippy -p audiolink-desktop --all-targets --all-features -- -D warnings` | **exit 0** |
| `cargo test -p audiolink-desktop` | 单测 40 passed / 1 ignored（既有：需真声卡）+ `engine_seam` 1 passed + updater 1 + 8 passed |
| `cd desktop; pnpm build` | **exit 0**（tsc 严格 + i18n 一致性 zh=264 / en=264 + vite） |
| `cd desktop; pnpm test` | 18 files / **162 tests passed** |
| `pwsh android/scripts/build-rust.ps1` | **exit 0**（双 ABI `.so`） |
| `pwsh tools/gradlew.ps1 -JavaHome <JDK17> assembleDebug testDebugUnitTest` | **BUILD SUCCESSFUL**，**245 tests / 0 failures**（30 suites） |
| `pwsh core/crates/audiolink-ffi/bindings/check-so-symbols.ps1` | **四环同源**：① 绑定与当前源码逐字节一致 ② jniLibs 20/20 ③ target 产物 20/20 ④ APK 内嵌 `.so` 20/20 |

`engine_seam` 是本轮最有价值的一条接缝测试：两个**真实 `Engine`** 经 127.0.0.1 真 QUIC，`connect()` **一次调用直接成功**（没有 PIN 提交、没有挑战应答）→ 开流 → 接收端收到非零码率遥测。

**为什么 `nextest` 加 `--test-threads 2`**：engine 的 `group_sync*` / `group_join` / `multi_session` / `playout_tests` 等时间敏感用例，在默认并发且其它 crate 同时编译的窗口下会抖红 —— 三次默认并发全量的失败集合各不相同（5 / 1 / 2 条），过滤单跑全绿，低并发全绿。这是**既有负载敏感性**（同 `docs/65-wifi-playout-continuity.md` 第 33 条的记账），本轮**未改阈值、未改断言**（改阈值等于偷偷放宽验收线）。

**改写测试时暴露并修掉的一处真实前提缺陷**（`reconnect_receipt::reconnect_ok_is_visible_through_peers` 报「状态 Streaming、reconnects 0」）：根因不是回执缺陷，而是 FR-27 静默看门狗判据 `silent_for_ms()` 在 `last_rx_us == 0`（一次都没听到过对端）时恒返回 0。原测试 `wire_up()` 里那段「等 PIN → 提交 PIN」的人工等待顺带让发起方先听到过对端；换成一次 `connect()` 之后 `cut()` 发生得太早，3 s 阈值永不成立，于是「重连没发生」被误读成回执缺陷。修法是**只动测试**：`start_send` 之后等到 `clock_probe_stats().received > 0`，把「静默判据从听到过起算」这个前提显式写进测试。修后单跑与全量均绿。

**删除带来的覆盖损失（如实记账，不做替代）**：`revoke_tests` 的「无会话设备也能被移除 / 移除一台不影响其它对端 / 同一 data_dir 是同一身份」三条断言随功能删除而消失；`pairing_tests` 的「会话替换后 `drop_session` 不误摘新会话」一条同样不再有断言（它与 PIN 快照耦合太深，未搬移）。

**真机**：**双向免配对直连已在真机上跑通**，做法与证据见 §5 第 1 条。

---

## 5. 未验清单（真实待验项，不声称已验）

1. **真机免配对直连 —— 已通过 ✅**（2026-09-22 16:21–16:24；真机 PJZ110 / Android 16 / SDK 36 / arm64-v8a）。
   做法与证据：
   - 最终 Debug APK（`android/app/build/outputs/apk/debug/app-arm64-v8a-debug.apk`，mtime 16:16:42）`adb install -r` → **Success**；
   - PC 侧启动真实桌面端 `target/x86_64-pc-windows-msvc/debug/audiolink-desktop.exe`，确认它监听 UDP `58290`；
   - **方向一（手机当接收端 → PC 当主机）**：手机「接收声音」页点「连接主机」（发现列表里的 `GOTKICRY_WORK 192.168.3.200:58290`）
     → **直接进入「✓ 接收中」，全程没有出现任何配对码输入界面**；RTT 20.4 ms（8 s 后）→ 22.4 ms（13 s 后），连接稳定。
     证据：`target/evidence/remove-pairing/phone-5.png`、`phone-6.png`，控件树 `ui2.xml`；
   - **方向二（PC 当接收端 → 手机当主机）**：手机切到「发送声音」页（界面显示「服务运行中」，监听 `192.168.3.59:58290`），
     PC 侧 `device-link run --peer 192.168.3.59 --capture synth` → **一次 `connect()` 直接成功，退出码 0**；
     手机侧确实收到了音频：链路码率均值 **328.3 kbps**、丢包 **0.00%**、PLC 0。证据：`target/evidence/device-tool/device-1790065308.json`；
   - **结论：两端都不再要求配对码，双向直连均通过。**
   - 附记（排除一个常见怀疑）：16:18 / 16:19 两次 `device-link` 是 **25 s 连接超时**，原因是当时手机处于锁屏、App 引擎未启动；
     解锁并在界面上启动服务后连接立即成功 —— 因此可以排除「端口被防火墙挡住」。
2. **旧版兼容性的反证**：带 PIN 的旧版客户端连本版（预期在握手/命令层失败）——**未实测**，只给出协议层推断（§3.2）。
3. **「接入即自动推流」的现场表现**：多人/不可信网络下误接入是否会造成声音外泄、以及关掉开关后的行为，
   只做了单测与代码级确认，**未做现场验证**。
4. **声学端到端延迟（P50/P95）**：与 `docs/12` §10 同一条阻塞 —— 本机无物理麦克风，`tools/acoustic-latency.ps1`
   按纪律以退出码 4 退出，不给数字。本轮**没有**任何新的声学读数。
5. ~~**Android 侧构建产物**~~ → **已验证**：双 ABI `.so` + `assembleDebug` + `testDebugUnitTest`（245 tests / 0 failures）全绿，
   最终 APK：`app-arm64-v8a-debug.apk` 21.2 MB、`app-armeabi-v7a-debug.apk` 18.4 MB（mtime 16:16:42）。见 §4 门禁表。
6. ~~**Kotlin 绑定同源自检**~~ → **已验证**：`check-so-symbols.ps1` 四环同源（① 绑定 ↔ 源码逐字节一致 ②③④ 各 20/20），见 §4。
   遗留（既有工具缺陷，非本轮引入）：该脚本第 ④ 环按 mtime 从新到旧遍历 APK，**最新包缺某 ABI 的 `.so` 时会静默回退到更早的包并报 OK**
   （读码确认，见 ffi 工作流报告）。建议加固为「先断言最新包内存在该 ABI 的 `.so`，再查符号」——本轮未改该脚本。
7. **`tools/acoustic-latency.ps1` 的命令行示例**：已按「`device-link` 不再有 `--pin-file`」同步，
   但**未实际跑过**（该脚本需要物理麦克风）。若 `device-link` 最终参数名有出入，以该工具为准。
8. **Android「接入即推」的既有跨端不一致**（**非本轮引入，建议单独立项**）：
   引擎语义是「握手成功即 `Streaming`」，活会话从不报 `idle`；**桌面**外壳有状态翻译层
   （`desktop/src-tauri/src/engine_bridge.rs:15-19`：引擎 `Streaming` + 未推流 → 视图 `idle`），所以桌面端「接入即自动推流」成立。
   **Android 缺这层翻译**：`SenderUiState.shouldAutoStartSend` 的 `peerState != idle → false` 判据
   （`android/app/src/main/kotlin/com/gotkicry/audiolink/service/SenderUiState.kt:330-337`）在活会话里**恒不成立** ⇒
   Android 的自动推流从不触发；另外 `ReceiverDeck` 只要连上就显示「接收中」（此刻可能并没有音频）。
   本轮**有意不改逻辑**（改它会真的打开 Android 自动推流，属超出用户指令的新增行为）；文档侧已按端如实标注（§3 第 1 条与两份用户手册）。

---

## 6. 文档处置备注

- **契约**：`docs/71-remove-pairing.md` 是本轮的唯一权威，未改动。
- **历史交付文档**（`docs/09`、`12`、`15`、`16`、`21`、`22`、`26`、`27`、`29`、`30`、`33`、`34`、`36`–`39`、`41`、`42`、`46`、`47`、`49`、`50`、`52`–`58`、`60`–`62`、`64`、`69`、`android-ui-refactor`、`10-handoff` 等）：
  保留当时的验收数字与结论，只在文首加一条「该认证机制已在本轮移除」的说明 —— 历史记账不应被改写。
- **`tools/sync-board.ps1`**：历史看板条目按契约**一字未动**（那是既往交付的记账）。
- **`docs/compliance/`**、`docs/design/**`：其中的「pin / pair / 白名单」是第三方依赖名、色彩配对与形状术语，与本机制无关，未动。
- 用户可见的「配对」字样现在只剩下两类：**历史记账**（上述文档，均带注记）与 **色彩/形状术语**（DESIGN.md 的「配对法则」「pill 白名单」）。
