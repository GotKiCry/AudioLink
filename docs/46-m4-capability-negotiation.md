# M4 · 能力协商（握手期交换能力位图）

> 看板：`[M4] 能力协商（内核侧）`。日期 **2026-09-17**。
> 一句话：把「连得上但没声音」这类失败**从开流时刻提前到握手时刻**，并给出看得懂的原因。

> ## ⚠️ 口径更新（task-43，2026-09-17 补记，**读本文前必读**）
>
> 本文 §2 / §7 / §8.3.4 / §8.3.5 里「Android 内录尚未实现 / `CURRENT` 不声明内录」是**第一轮的事实**，
> **已被后续实现取代**，不要当现状缺口来读：
>
> - **Android 内录与麦克风已实现**：`service/CaptureWiring.kt:38,41`（`SYSTEM_LOOPBACK = 16u` /
>   `MICROPHONE = 32u`）、`:45`（内录需 SDK ≥ 29）、`:72-73`（按 SDK 与 `RECORD_AUDIO` 门控取并集）；
>   采集已接进服务生命周期（`service/CaptureController.kt`、`service/AudioLinkService.kt`）。
> - **平台能力注入已接线**：`core/crates/audiolink-ffi/src/engine_bridge.rs:88`（`EngineStartConfig.capabilities`）、
>   `:381`、`:393`（`engine_config.capabilities = Capabilities::CURRENT | config.capabilities`）——
>   §8.3.5 说的那个「口子」已经用上了。
> - 落地提交：`a0158a2`（Android 向对端声明内录/麦克风能力位，内核侧取并集）、`4c2d21a`（采集接进服务生命周期）。
> - **仍然成立**（别连带改掉）：§8.3.4 第一条「无损档 `PCM16`、混音路数界面上还没有对应控件」；
>   §8.4 第一条「真正的置灰」目前只在 §8.3.2 建组勾选这一处落地。

---

## 1. 为什么需要它

**连得上 ≠ 能互相说话。** 假如一端只有 Opus、另一端只有 PCM，握手会成功，链路也会建起来，
但一开流必然失败 —— 用户看到的是「连上了却没声音」，日志里只有一条编解码错误，而真正的
原因（两端没有共同能力）要在几十行日志里翻。

把能力在**握手期**就换清楚，有三个好处：失败提前、原因人话化、以及给「降级」留出决策点。

---

## 2. 设计：位图 + 交集

位图的语义是「本端**能**做什么」，协商结果是双方的交集。

| 位 | 含义 | 必需？ |
|---|---|---|
| `OPUS` | 能收 Opus（默认与唯一的常规音频档） | **必需** |
| `PCM16` | 能收 PCM16 无损档 | 可选 |
| `CAPTURE` / `PLAYOUT` | 能作为发送端采集 / 接收端播放 | 可选 |
| `SYSTEM_LOOPBACK` / `MICROPHONE` | 系统内录 / 麦克风 | 可选 |
| `MIXER` / `GROUP_EPOCH` | 多路混音 / 同步组预约播放 | 可选 |

三条判断：

1. **只有不可替代的能力才标必需**（现在只有 Opus）。标多了会把「能用但功能少」的组合误判成
   「连不上」—— 那比不做协商更糟：用户本来只是想少个功能，结果整个用不了。
2. **可选位缺失不拒绝**，交给调用方降级（把入口置灰 + 说明原因）。
3. **`Capabilities::CURRENT` 只声明内核真做到的**：内录与麦克风要平台侧真的接上才算
   （Windows WASAPI loopback / Android AudioPlaybackCapture）。~~现在不声明~~ →
   **口径更新**：两侧现在都声明了（Android 依据 `service/CaptureWiring.kt:38-73`，提交 `a0158a2`）；
   原则不变：宁可少声明，也不要声明一件做不到的事。

---

## 3. 协议变化（向后不兼容，v1 冻结前）

`HELLO`（`0x01`）加 `caps: u32`；`HELLO_ACK`（`0x02`）加 `caps: u32` + `agreed_caps: u32`。

拒绝路径**沿用既有范式**（版本不匹配就是这么做的）：`accepted = false` + `reason` + 
事件里的错误码 —— 用的是既有的 **`1005 CAP_UNSUPPORTED`**，没有新增错误码。

拒绝原因长这样：

```text
能力协商失败：缺少 Opus 编码（本端 Opus 编码、音频采集、音频播放、多路混音、同步组预约播放；对端 音频采集）
```

双方各自有什么、缺哪一项，一行看完 —— 这条字符串就是给用户和排障人看的。

> 载荷用 postcard 编码（非自描述），加字段**不向后兼容**。本项目 `v0.1.0` 尚未发布、没有外部旧客户端，
> 且 `HELLO` 里本来就有 `proto_version` 校验（不一致直接拒绝），所以直接加字段是安全的。

---

## 4. 双向复核（一个刻意的冗余）

发起方收到 `HELLO_ACK` 后会**自己再算一遍交集**，与对端报来的 `agreed_caps` 比对，一致才继续。

两侧算法一致时这是恒等检查，看起来多余；但一旦将来只改了一侧（比如新增一种能力只在一边生效），
这里会立刻失败 —— 比「双方带着不一样的理解继续跑，然后在某个奇怪的地方出问题」便宜得多。

---

## 5. 实测

| 层 | 内容 |
|---|---|
| 位图单测（`core/crates/audiolink-types/tests/capabilities.rs`，5 项） | 交集、必需/可选位判定、`describe` 顺序稳定性、未知位、`CURRENT` 自洽性（不声明未知位、满足自己声明的必需位） |
| 握手单测（`audiolink-engine` 的 `handshake::tests`，+2 项） | ① 缺必需位 → 拒绝性 `HELLO_ACK` + `CAP_UNSUPPORTED` + 原因点名「Opus 编码」+ 阶段落 `Failed`；② 能力不同但兼容 → 双方协商出**同一个**交集 |
| 回归 | 引擎 146 项单测全过（原 144 + 2）；clippy 零警告；fmt 通过 |

单测里刻意用了两种「不一致」：一边声明 `OPUS|CAPTURE|MIXER`、另一边 `OPUS|PLAYOUT|GROUP_EPOCH` ——
交集只有 Opus，但仍然可以连接（可选位缺失不拒绝），这正是「降级而不是拒绝」的那条线。

### 5.1 一个被测试抓出来的小问题

`Capabilities::describe` 最初在「位图里全是未知位」时返回「无」，结果是：对端比本端新、声明了一个
本端看不懂的位时，界面会显示「对端没有能力」。已改为这种情况输出「未知能力」——
「没有能力」与「有本端看不懂的能力」是完全不同的两件事。

---

## 6. 与编解码协商的关系（别混起来）

| 层 | 何时 | 协商什么 |
|---|---|---|
| **能力协商**（本文） | **握手期**（`HELLO`/`HELLO_ACK`） | 双方能不能互相说话（位图交集） |
| 编解码档协商（`docs/03` §8） | 开流期（`OPEN_STREAM`/`OPEN_STREAM_ACK`） | 这一条流用哪个编解码与参数（`codec_prefs` → `codec_chosen`） |

能力协商是**门**（能不能进），编解码协商是**选**（进来后用哪档）。

---


---

## 8. 界面侧：把协商结果送到用户眼前（第二轮）

内核侧落地后，协商结果还躺在 `Handshake::agreed_caps()` 里 —— 界面看不到，用户于是仍然不知道
「为什么同步组对这一台用不了」。这一轮把它接通：

| 层 | 变化 |
|---|---|
| 握手事件 | `HandshakeEvent::Established` 带上 `local_caps` / `peer_caps` / `agreed_caps` —— **自包含**：会话层不用再回头去问握手对象 |
| 会话状态 | `PeerSession` 存 `Option<PeerCapabilities>`，`PeerStatus` 暴露给引擎调用方 |
| 桥接层 | `PeerView` 增加 `capabilities`；位图在 **Rust 侧**就翻成人话（`PeerCapabilitiesView`：`local` / `peer` / `agreed` / `missingOnPeer[]`） |
| 界面 | 对端卡片多一行「可用：…」；本端有而对端没有的能力非空时，再补一句「对端不支持：…」（琥珀色） |

**为什么翻译放在 Rust 侧**：位图是协议用的，界面要回答的是「这台对端能做什么、缺什么」。
把 `1 << 3` 丢给前端，等于让每个前端各实现一遍解释，迟早解释不一致。

**`missingOnPeer` 怎么算**：`local & !peer`，再按 `Capabilities::ALL_KNOWN` 的固定顺序翻成名字 ——
它就是「置灰」那一半的直接输入（M4 交付物 4 的降级 UI 靠它）。

### 8.1 一个被契约测试抓到的形状变化

`view.rs` 的 `peer_and_local_json_shape_match_contract` 立刻变红：`PeerView` 多了 `capabilities` 字段。
这正是那条测试存在的理由（字段名一旦漂移，前端会静默拿到 `undefined`，什么都不报）。
已顺手把它扩成两个形状：**未协商**（`null`）与**已协商**（含嵌套对象与数组）—— 后者钉住了
`missingOnPeer` 这个名字，前端就是按它决定置灰的。

### 8.2 这一环的验证

| 层 | 证据 |
|---|---|
| 引擎 | 170 项单测与集成测试全过（能力协商 +2 项在内） |
| 桥接 | `cargo test -p audiolink-desktop` 27 项通过，含扩写后的契约形状测试 |
| 前端 | `pnpm build` 通过（tsc 严格 + i18n 键与占位符一致性 + vite） |
| 双语文案 | 新增两个键（`peer.caps_agreed` / `peer.caps_missing`）中英齐备，护栏会检查占位符 |


---

### 8.3 降级：把「不能用」变成「选不了」（第三轮）

§8 让界面**说明**了对端缺什么；这一轮把说明变成**行为** —— 依赖该能力的入口直接不可选。

#### 8.3.1 一条必须坚持的边界：判断不解析文案

界面需要回答的是「这个功能能不能用」。最省事的写法是拿 `agreed` 那串中文去 `includes("同步组预约播放")` ——
**这是错的**：文案会随措辞调整，改一次文案就断一次判断，而且断得**静默**（界面照常显示，只是不再置灰）。

所以位图现在出**两条**：`agreed`（人话，给人看）与 `agreedKeys`（键，给判断用，如 `group_epoch`）。
`Capabilities::key()` 与 `name()` 并列存在，并有测试钉住「每个已知位都有独立且非 `unknown` 的键」与「键不重复」。

#### 8.3.2 第一处落地：建组勾选

建组依赖「组内排播」（`GROUP_EPOCH`）。现在的行为：

| 对端状态 | 复选框 | 旁边写什么 |
|---|---|---|
| 交集含 `group_epoch` | 可选 | — |
| 交集不含 | **禁用** | 琥珀色「对端不支持组内排播」 |
| 还没协商完（`capabilities === null`） | **禁用** | 琥珀色「等待能力协商…」 |

最后一行的区别很重要：**「等一下」与「别等了」是两件事**。把它们混成一句，用户在握手那几百毫秒里
会以为设备不兼容 —— 而实际上再等一下就好。

#### 8.3.3 验证

| 层 | 证据 |
|---|---|
| 键 | `capability_keys_are_stable_machine_readable_ids`（含「每个已知位都有独立键」「键不重复」两条断言） |
| 桥接 | 契约形状测试扩到 `agreedKeys` / `missingKeys`；`cargo test -p audiolink-desktop` 27 项通过 |
| 前端 | `pnpm build` 通过（tsc 严格 + i18n 键与占位符护栏 + vite）；新增两个键中英齐备 |
| 回归 | types + engine 171 项测试全过；fmt / clippy 零警告 |

#### 8.3.4 仍未做

- 其余依赖能力的入口（无损档 `PCM16`、混音路数）**界面上目前还没有对应控件**，等有了再按同一模式处理；
- ~~平台能力注入仍是缺口（Windows loopback 已有、Android 内录未实现）—— `with_capabilities` 的口子已经开好。~~
  → **已不是缺口**（见顶部口径块）：Android 侧按运行时门控声明 `SYSTEM_LOOPBACK | MICROPHONE`
  （`service/CaptureWiring.kt:38-73`），FFI 侧 `engine_bridge.rs:88,393` 把它并进 `EngineConfig.capabilities`。
#### 8.3.5 平台能力注入：把「内核能做什么」与「这台机器能做什么」分开

`Capabilities::CURRENT` 说的是**内核**能做到什么；而真实设备还有平台差异 —— Windows 有 WASAPI loopback
（系统内录），~~Android 的 `AudioPlaybackCapture` 尚未实现~~（**后已实现**，见本节表末与顶部口径块）。
在这之前，那个常量被写死在握手构造函数里，
于是「所有设备都声称自己一样」—— 能力协商也就只剩形式。

现在 `EngineConfig.capabilities`（默认仍是 `CURRENT`）由**平台侧**声明，握手直接用它：

| 侧 | 声明 | 依据 |
|---|---|---|
| 桌面端（Tauri） | `CURRENT \| SYSTEM_LOOPBACK` | WASAPI loopback 采集已实现（`docs/13`） |
| Android（FFI） | ~~`CURRENT`（不含内录）~~ → 现为 `CURRENT \| SYSTEM_LOOPBACK \| MICROPHONE`（按运行时门控取并集） | ~~`AudioPlaybackCapture` 尚未实现 —— 保持默认就是此刻的实话~~ **已实现**：`service/CaptureWiring.kt:38-73`（`SYSTEM_LOOPBACK=16u` / `MICROPHONE=32u`，SDK ≥ 29 且声明 `RECORD_AUDIO`），提交 `a0158a2` |

**端到端证据**：`core/crates/audiolink-engine/tests/engine/capability_negotiation.rs` 起两个真实 Engine
（真 QUIC + PIN 配对），一端声明内录、另一端不声明，断言**对端** `peers()` 里读到的那份能力：
`peer` 位含内录、`local` 位不含、`agreed` 交集不含 —— 也就是界面按 `missingKeys` 置灰用的那个输入。
这条测试回答的是「注入的位真的出网了吗」，而不是「字段赋值对不对」。
### 8.4 仍未做（第一轮时记下的）

- **真正的「置灰」**：现在卡片上**说明**了缺什么，但依赖该能力的入口（比如建组时勾选对端）还没有
  真的禁用 —— 那一步要等「哪些入口依赖哪些能力」在界面上说清楚（`GROUP_EPOCH` 已是明确的一例）；
- ~~**平台能力注入**：`Capabilities::CURRENT` 仍是内核能力；接上平台（Windows loopback / Android 内录）
  之后要能把真实设备能力传进握手 —— 口子（`Handshake::with_capabilities`）已经开好并在测试里用过。~~
  → **已完成**（顶部口径块）：`EngineStartConfig.capabilities` + `engine_bridge.rs:393` 的并集；
  §8.3.5 的端到端测试就是这一条的验收。
---

## 7. 未做（第一轮时记下的，部分已在 §8 完成）

- **引擎层可观测**：`Handshake::agreed_caps()` 已经有了，但还没接到 `Engine` 的查询 API 与事件上 ——
  所以桌面端现在看不到「协商出了什么」；
- **界面降级**：内录/麦克风不可用时把入口置灰并说明（M4 交付物 4 的另一半，等平台能力接上再做）；
  → **口径更新**：平台能力已接上，置灰已在 §8.3.2（建组勾选）落地；其余入口仍未做（见 §8.3.4）。
- ~~**平台能力接线**：`Capabilities::CURRENT` 是内核能力；真实设备能力（Windows 有 loopback、Android 看 API 等级）
  还没注入 —— `Handshake::with_capabilities` 就是为这一步留的口子（测试已在用）。~~
  → **已注入**（顶部口径块）。
