# 71 · 彻底移除匹配码（PIN 配对）与信任库认证

> 状态：**设计冻结** —— 本轮任务开工前定稿（Lead 产出，全队只读输入，不要改本文件）
> 决策来源：用户直接指令「完整去掉匹配码机制。Desktop 和 Android 都要完整去除这个认证机制，因为在局域网使用不存在安全问题」。
> 两项已拍板的范围决策：**① 彻底无认证**（PIN + 信任库白名单 + AUTH_* 挑战应答全删，任何节点直连）；**② 两端同步升级**（协议命令 `0x03`–`0x07` 直接删除，不留旧版兼容路径）。

---

## 1. 为什么可以删干净（第一性原理）

被删的部分承担的每一条保证，都有别的层正在真实承担：

| 原本宣称的保证 | 真实承担者 | 处置 |
|---|---|---|
| 通道加密与完整性 | QUIC 的 TLS1.3（`core/crates/audiolink-net/src/tls.rs`，ring 后端） | **保留不动** |
| 「对端确实持有该证书私钥」 | TLS1.3 握手本身（服务端 `with_single_cert`、客户端 `with_client_auth_cert`，双向出示证书） | `AUTH_CHALLENGE/AUTH_RESPONSE` 是**同一件事的第二次**证明 → 冗余，删 |
| 「这个指纹值不值得信任」 | engine 层的 `TrustStore` 查表 + `PinGate` 6 位码 | 用户判定局域网内无此需求 → 删裁决本身 |
| 节点身份（区分设备、展示短码） | `NodeId = SHA-256(证书 DER)`，由 `Connection::peer_id()` 从对端证书现算 | 与信任裁决解耦 → **保留** |

删除后的准确语义：**TLS 仍然证明「对端就是它出示的那张证书」，只是不再有人问「这张证书值不值得信任」。**
这句话必须原样出现在两份用户手册与 `docs/privacy.md` 里 —— 否则用户会以为自己连加密也丢了。

TLS 层本来就不做信任裁决（`tls.rs` 的两个 `Tofu*Verifier` 恒返回 assertion，判定上移到 engine）。因此本次改动**完全不碰 `audiolink-net` 的加密配置**，只碰它一个错误码。

---

## 2. 目标行为

### 2.1 连接时序（替换 `docs/03-protocol.md` §5）

```text
发起方 A（接收端）                          响应方 B（主机）
  │ ── QUIC 握手：TLS1.3 + 自签证书（双方互验私钥持有）──► │
  │ ── HELLO(proto_version, node_info, nonce) ──────────► │
  │ ◄──────────────── HELLO_ACK(proto_version, node_info) │
  │        ← 到此握手已 Established，没有 AUTH_*、没有 PAIR_*
  │ ◄──────────────── OPEN_STREAM(...) ──────────────────  │
  │ ── OPEN_STREAM_ACK(stream_id, codec_chosen, epoch) ─► │
```

### 2.2 握手状态机（`handshake.rs`）

保留：`Idle → AwaitHelloAck → AwaitPeer → Established`（失败态 `Failed` 保留）。
删除：`AwaitAuthResponse`、`AwaitPairSubmit`、`AwaitPairResult`、`AwaitAuthOk` 四个阶段及其迁移。
保留并在文档中明确的纪律（不要顺手删）：**对端身份只认证书指纹**（`HELLO.node_info.id` 是对端自报，必须被 `peer_id` 覆盖）、**过程序外帧忽略并计数不断流**、**`request_id` 必须校验**。

能力协商（`caps` 位图、`agreed_caps`）**保留** —— 它回答「双方能不能互相说话」，与认证无关。

### 2.3 用户可见的变化

- 桌面端：主界面不再弹配对对话框；设置页不再有「已配对设备 / 移除设备」；对端卡片不再有「未配对」标记。
- Android：接收/发送页不再有配对码输入与展示卡片。
- 首次连接即通 —— 复制主机地址（或从发现列表点选）→ 直接开始推流。

---

## 3. 冻结的 crate 级接口

### 3.1 `audiolink-identity`

- 删除 `mod pin;`、`mod trust;` 与 `pin.rs`、`trust.rs` 两个文件；删除再导出 `PIN_DIGITS`、`PIN_LOCKOUT`、`PIN_MAX_ATTEMPTS`、`PIN_TTL`、`PairRejection`、`PinGate`、`TrustEntry`、`TrustStore`。
- `identity.rs` 删除 `sign_challenge` / `verify_challenge` / `challenge_message`（挑战应答专用）。
- **保留**：`NodeIdentity`（`load_or_create` / `from_pem` / `id` / `cert_der` / `key_der_pkcs8` / `node_name` / `cert_pem`）、`CERT_FILE_NAME`、`KEY_FILE_NAME`、`CERT_SUBJECT_ALT_NAME`、`IdentityError`。
- `IdentityError` 删除 `AuthFailed`、`TrustStore` 两个变体与 `auth_failed*` / `trust_store*` 构造器；其余错误码收敛到 `1008 BAD_REQUEST`（沿用既有「表里唯一中性的本端拒绝码」惯例）。

### 3.2 `audiolink-types`

- `OpCode` 删除 `AuthChallenge(0x03)`、`AuthResponse(0x04)`、`PairRequired(0x05)`、`PairSubmit(0x06)`、`PairResult(0x07)` 五个变体（含 `from_u8` / `name` / 列表常量里的对应项）。
- `ErrorCode`：见 §4。

### 3.3 `audiolink-proto`

- `discovery.rs` 删除 `DiscoveryRecord.paired` 字段及其 TXT 编解码分支（`paired` 不再有数据来源）。调用方随之改签名。
- `tests/roundtrip.rs`、`tests/rejects.rs` 里针对 `paired` 的用例同步删除。

### 3.4 `audiolink-net`

- 只改 `src/error.rs`：`Handshake` / `PeerIdentity` 的错误码从 `1004` 改为 `1008`（消息文案里的 `1004 AUTH_FAILED` 前缀同步改）。
- `src/tls.rs`、`src/connection.rs`、`src/endpoint/*` **不动**。

### 3.5 `audiolink-engine`

- `handshake.rs`：`HandshakeEvent` 删除 `DisplayPin` / `NeedPin` / `PinRejected`；`HandshakeEvent::Established` 删除 `persist` 字段；`Handshake::new` 签名去掉 `peer_trusted`；删除 `displayed_pin` / `on_pin_input` / `on_pair_submit` / `on_pair_result` / `describe_rejection` / `normalize_pin` 及 `pin_gate` 字段。
- `runtime.rs`：删除 `Engine::displayed_pin` / `displayed_pin_at` / `pending_pin_peer` / `submit_pin` / `trusted_peers` / `revoke_trust`；删除 `EngineEvent::DisplayPin` / `PinNeeded` / `PairCompleted`；删除 `EngineConfig.trust_store_path`；删除 session 上的 `pairing` 状态；`create_session` 去掉 `trusted` 参数；删除 `is_trusted` / `remember_peer` 辅助函数。
- `dispatch.rs` / `payload.rs`：删除 5 个配对/认证命令变体与 `PairRequiredPayload` / `PairSubmitPayload` / `PairResultPayload` / `AuthChallengePayload` / `AuthResponsePayload`。
- **保留**：握手死线、`1002`（改名后表示「无该对端」，见 §4）、`BUSY(1009)` 重复连接拦截、能力协商、全部音频链路逻辑。

### 3.6 `audiolink-ffi`

- 删除导出 `submit_pin`、`displayed_pin`；`connect` 的文档与 `FfiError` 文案不再提配对。
- `bindings/kotlin/**` 是 UniFFI **生成产物**，必须用仓库既有命令重新生成，不要手改（生成命令见该 crate 的 README/脚本或 `android/scripts/`）。

### 3.7 两端外壳

- 桌面端：删除 `PairDialog.tsx`（含测试）、`EVENT_PAIR_REQUIRED` 订阅、`api.submitPin` / `api.listTrustedPeers` / `api.revokeTrust`、`src-tauri` 上对应的 Tauri command 与事件常量、配对相关 i18n key（中英）、`useAudioLink.ts` 的 `CODE_NOT_PAIRED` 提示分支。
- Android：删除 `PairingUiState.kt`、`PairingPanel.kt`、`PairingDialog.kt`、`PairingStateMapperTest.kt` 及其在 `AudioLinkService` / `ReceiverEntryCard` / `SenderUiState` / `UiStrings` 中的全部引用与文案。

---

## 4. 错误码处置（冻结）

调研发现 `1002 NOT_PAIRED` 被大量**挪用**为「指定对端不存在」的运行时错误（`startSend`/`stopSend` 传了不存在的 peer、短码未命中、FFI 无对端时的断言）。因此不能简单删除它。冻结如下：

| 码 | 原名 | 处置 | 新名与含义 |
|---|---|---|---|
| `1002` | `NOT_PAIRED` | **改名保号** | `NoPeer` / 短名 `NO_PEER`：「指定的对端不存在」—— 与配对无关的既有复用语义，wire 值不变 |
| `1003` | `PAIR_REJECTED` | **删除** | —— |
| `1004` | `AUTH_FAILED` | **删除** | 相关的 net 握手/身份错误改为 `1008` |

- `AudioLinkError::not_paired()` → 改名 `no_peer()`；`pair_rejected()`、`auth_failed()` 构造器删除。
- `docs/03-protocol.md` §11 表同步：`1002` 改 `NO_PEER`，删两行，`1008` 的描述补「含握手失败与对端身份不可读」。
- 两端 UI 里凡是以 `1002` 判定「这是提示不是错误」的分支（desktop `useAudioLink.ts` 的 `CODE_NOT_PAIRED`、Android `SenderUiState.NOT_PAIRED_CODE`）**一并删除** —— 该区分的唯一目的就是引导输码。

---

## 5. 工作流划分与写入域（禁止越界写文件）

| 任务 | 负责范围 | 依赖 |
|---|---|---|
| `core-base` | `core/crates/audiolink-identity/**`、`audiolink-types/**`、`audiolink-proto/**`、`audiolink-net/src/error.rs` | 无 |
| `core-engine` | `core/crates/audiolink-engine/**`（含 `tests/**`、`src/runtime/*_tests.rs`） | core-base |
| `core-tools` | `core/crates/audiolink-tools/**` | core-engine |
| `ffi` | `core/crates/audiolink-ffi/**`（含重新生成的 Kotlin 绑定） | core-engine |
| `android` | `android/**` | ffi |
| `desktop-web` | `desktop/src/**` | 无（纯 TS/TSX，可先做） |
| `desktop-shell` | `desktop/src-tauri/**` | core-engine |
| `docs` | `docs/**`、`README.md`、`PRODUCT.md`、`DESIGN.md`、`tools/*.ps1` | 无（内容依据本文档） |

跨域改动一律通过共享任务板协调，不要直接改别人的文件。

---

## 6. 仓库当前状态（施工前提，务必先读）

1. **工作区是脏的**：有 78 个未提交改动（含 18 个未跟踪新文件，例如 `android/.../ui/screens/PairingDialog.kt`、`docs/65`–`docs/70`）。那是用户在途的工作，**本次改动叠加其上**。
2. **禁止** `git checkout` / `git reset` / `git stash` / `git clean`，**禁止** 提交（commit）—— 交付形态是「工作区里改好」，由用户自己决定何时提交。
3. 删除文件是允许的：配对专属文件（`pin.rs`、`trust.rs`、`PairDialog.tsx`、`PairingUiState.kt`、`PairingPanel.kt`、`PairingDialog.kt`、`PairingStateMapperTest.kt`、`pairing_tests.rs`、`revoke_tests.rs`、`pairing_lifecycle.rs`）整体删除。
4. 已存在的 `trust.json`（`%APPDATA%\AudioLink\trust.json`、Android 应用私有目录）**不读、不写、不主动删除**；在用户手册里说明可手工删除。

---

## 7. 验收门禁（全队共同标准）

```powershell
cargo fmt --all --check
cargo clippy --workspace --exclude audiolink-desktop --all-targets --all-features -- -D warnings
cargo nextest run --workspace --exclude audiolink-desktop --lib --tests
cargo clippy -p audiolink-desktop --all-targets --all-features -- -D warnings
cargo test -p audiolink-desktop
cd desktop; pnpm build          # tsc 严格 + i18n 一致性检查 + vite
```

Android 侧额外：`pwsh android/scripts/build-rust.ps1`（双 ABI）与 `pwsh tools/gradlew.ps1 -JavaHome C:\Users\liuzh\scoop\apps\corretto17-jdk\current assembleDebug testDebugUnitTest`。

**证据要求**：每个工作流在任务完成时贴出自己实际跑过的命令与关键输出（绿/红、用例数）；红的地方必须写明原因与是否已修。

---

## 8. 明确不做

- 不改 `NOTICE` / `LICENSE` / 上游归属（合规红线）。
- 不改 `tools/sync-board.ps1` 的历史看板条目（那是既往交付的记账，不是待删代码）。
- 不删除 `audiolink-net` 的 TLS/证书相关代码，不改 QUIC 配置。
- 不给旧版（带 PIN 的 0.1.0）留任何降级路径。
- 不重构与本机制无关的代码（只做删减与必要改名）。

---

## 9. 历史文档处置

- `docs/14-pairing-state.md` **删除**（它记录的功能已不存在），其遗留结论（QUIC 端口释放竞态）由 `docs/15-engine-restart.md` 覆盖。
- `docs/12-m1-device-acceptance.md`、`docs/55-device-acceptance-runbook.md`、`docs/manual/**` 里的配对步骤改写为「连上即用」。
- 本轮交付另写 `docs/72-remove-pairing.md`（背景 → 做法 → 取舍 → 实测 → 未验清单），沿用仓库「一个交付物一份文档」的约定。

---

## 10. 收尾裁决（Lead · 2026-09-22，逐条回应两端只读清点报告）

两端清点报告发现的事实与本契约有四处出入，现冻结裁决，各工作流据此施工。

### 10.1 `trusted` 字段族：全链删除

`PeerStatus.trusted`（engine runtime.rs:359）→ `PeerView.trusted`（FFI engine_bridge.rs:150 / 桌面 view.rs:99）→ `PeerUi.trusted`（Android PairingUiState.kt:32）这一族字段的唯一数据来源就是信任库查表。信任库删除后它只可能恒为 `true`；留一个恒真的「信任」标记比删掉更糟 —— 界面会继续向用户承诺一个已经不存在的机制。

**内核 `PeerStatus.trusted` 字段删除**（连带 `peers()` 快照、FFI 的 `PeerView.trusted`、Kotlin 绑定的同名字段）；两端的筛选与门禁一律改为「不筛选」：
- 桌面 `App.tsx` 的 broadcastTargets 过滤、`useAudioLink.ts:482` 自动推流筛选 → 去掉 trusted 条件
- Android `SenderUiState.peerTrusted` 与 `shouldAutoStartSend` 条件 3、`ReceiverDeck.awaitingPairing`、`DeviceDeck` 的「信任」行 → 删除

**行为变化必须如实记账**（写进 docs/72 与用户手册，不要埋在 diff 里）：此前「接入即自动推流」只对信任设备生效，现在**任何能连上本机 QUIC 端口的设备接入后都会自动开始推流**（自动推流开关本身仍是默认开）。这是「局域网无认证」的直接推论。

### 10.2 `revoke_trust` / 「移除设备」：整条删除

它兼任三件事：断会话 + 撤信任 + 清 last_peer。信任库没了，它只剩「断会话」一项，而那是 `Engine::disconnect` 的职责。

删除全链：Tauri command `revoke_trust`、`resolve_revocable`/`find_revocable`、`RevokeTrustResult`、`TrustedPeerView`、桌面 `PeerCard` 的移除入口与二次确认、`SettingsPanel` 的「已配对设备」区、`toast.revoked*` 文案。
**`AutoConnectPolicy.last_peer`（启动时自动连接上次设备）与信任无关 —— 保留**；只删「移除设备时顺便清 last_peer」那段逻辑。

### 10.3 Android `PairingUiState.kt` 不是纯配对文件：改名保留，不整删

它同时承载 `PeerUi`(26-69)、`PeerSnapshot`(77-112)、`PairingStateMapper.stateLabel`(171-179)/`displayName`(187-188)/`peers()`(191-206)，被 15+ 处复用。正确做法：
- 文件改名 `PeerUiState.kt`，`PairingStateMapper` 改名 `PeerStateMapper`，引用处同步改（`SenderUiState.kt:17/1042`、`AudioLinkService.kt:1042`、`PeerStateText.kt`、`PeerGainText.kt` KDoc）
- 删除 `PairingUiState`(121-140)、`pinIsStale`(139/148-156)、`normalizePin`(165-168)；`map()` 去掉 `pin`/`error` 形参
- `peers()`(191-206) 里的 `trusted = snapshot.trusted` 随 §10.1 删除
- **不得顺手删 `peers` 通路**：`AudioLinkService.applyNegotiatedFrameWatermark`(593) 等 6 处依赖它

同理 `PairingStateMapperTest.kt` 改名 `PeerStateMapperTest.kt`：删 9 个配对用例，**保留**其余 6 个非配对用例（遥测映射 20-28 / peer 字段 66-89 / 状态标签 101-115 / 未知状态 117-124 / displayName 回落 126-142 / peers 顺序 195-205）。`PairingDialogGateTest.kt` 整份删除。

### 10.4 护栏与目录事实

- `core/crates/audiolink-ffi/src/kotlin_guard.rs` 的 `REQUIRED_SYMBOLS`(258-283) 含 `submitPin`(271)、`displayedPin`(275)，**必须与 FFI 导出删除同批**，否则护栏用例（339-356）会红。
- `android/app/src/androidTest` **不存在**（实测只有 `main` 与 `test`），任务描述里的相关内容按防御性写法处理即可。
- `docs/01-requirements.md` 的 FR-18（撤销信任）等条目：**保留编号占位并标注「已移除（本轮）」**，不要删除编号导致后续 FR 位移。
- 绑定重新生成的既有命令见 `core/crates/audiolink-ffi/bindings/README.md:11-21`；同源自检 `bindings/check-so-symbols.ps1` 由 Lead 在收口阶段跑。

### 10.5 补充授权与并行化（Lead · 2026-09-22）

- **§3.4 的范围放宽**：`audiolink-net` 除 `src/error.rs` 的错误码外，还要求清理 `src/tls.rs` / `src/lib.rs` / `src/connection.rs` / `src/endpoint.rs` 里引用已删机制（TOFU / PIN / 信任库 / AUTH_RESPONSE 验签）的注释与文档字符串 —— 零行为改动，但必须改：留着会误导读者的过时说明，等于把已删机制留在代码里。
- **标识符改名**：`TofuServerVerifier` → `AcceptAnyServerVerifier`、`TofuClientVerifier` → `AcceptAnyClientVerifier`（2 个 struct + 使用点 + doc link）。理由：TOFU（首次使用即信任）这个概念随信任库一起消失，名字必须跟着走。
- **并行化**：task-3（`audiolink-tools`）的实际代码改动允许在 task-2 完成前开始（写入域不变），但 **claim 与验收必须在 task-2 完成、engine API 冻结之后** —— tools 的编译依赖 engine，而 engine 是本轮最重的一块。
- **保留历史引述**：`proto/tests/roundtrip.rs`、`net/tests/quic_pair.rs`、`identity/tests/m1_acceptance.rs` 里「删掉 0x03–0x07 后是 20 项」「1003/1004 现为未分配值」这类**说明删除本身**的注释予以保留（它们解释了计数为何变化，是有效信息）。
