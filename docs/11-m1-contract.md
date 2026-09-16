# M1 接口契约（并行开发冻结版）

> **性质**：M1 并行开发的**唯一对齐依据**。冻结时间 2026-09-14。
> **谁改**：只有 Lead 能改本文；任何执行方发现契约不可实现，**先报 Lead**，不要自行改形状。
> 上游规格：`docs/03-protocol.md`（wire 契约）、`docs/02-architecture.md`（分层与线程模型）。
> 本文只冻结**跨 crate / 跨语言的接口形状**；内部实现与私有类型由各执行方自定。

---

## 0. 本轮范围（对应看板 M1 未完成项）

| 看板条目 | 归属流 | 本地可验证性 |
|---|---|---|
| `[M1] QUIC 通道（quinn：控制流 #0 + 音频数据报）` | **net** | ✅ 完整（127.0.0.1 真 QUIC） |
| `[M1] PIN 配对最小可用（白名单落盘）` | **identity** | ✅ 完整（单元 + wire） |
| `[M1] Android 播放（AudioTrack 低延迟 + JNI 环缓冲 + 欠载检测）` | **android** | ⚠️ 仅编译 + JVM 单测（**本机无真机**） |
| `[M1] 最小 UI（桌面手工 IP 连接；Android 服务启停）` | **desktop** / **android** | ⚠️ 仅构建（无头环境点不了 UI） |
| `[M1] 验收：P50 ≤ 110 ms / P95 ≤ 150 ms、零重采样、低延迟模式` | **lead**（`tools`） | ⚠️ PC↔PC 段可实测；Android 段需真机 |
| `[护栏] 跨端一致性夹具：golden vectors 在 Rust 与 FFI 双跑` | **ffi** | ✅ 完整 |
| 引擎编排（看板未单列，M1 主线的接缝） | **lead** | ✅ 完整 |

**明确不做**（属 M2）：抖动缓冲自适应、FEC/双发、NACK 重传、码率自适应、看门狗自愈、混音器。
本轮接收侧只做**固定深度播放环 + 迟到丢弃 + 欠载计数**，把指标口径先跑通。

---

## 1. 写域（严格隔离，禁止越界）

**一个文件只能有一个所有者。** 越界写入会立刻造成集成失败，比不写更糟。

| 流 | 所有者 | 可写路径（其余一律只读） |
|---|---|---|
| 契约 / 根 | **lead** | 根 `Cargo.toml`、`Cargo.lock`、`docs/**`、`core/crates/audiolink-types/**` |
| 引擎 | **lead** | `core/crates/audiolink-engine/**`、`core/crates/audiolink-tools/**` |
| net | **net-quic** | `core/crates/audiolink-net/**`（含其 `Cargo.toml`） |
| identity | **identity-pin** | `core/crates/audiolink-identity/**`（含其 `Cargo.toml`） |
| ffi | **ffi-bridge** | `core/crates/audiolink-ffi/**`（含其 `Cargo.toml`） |
| android | **android-playback** | `android/**` |
| desktop | **desktop-ui** | `desktop/**` |

**共享文件纪律**：

- 根 `Cargo.toml` 的 `[workspace.dependencies]` 已预置全部本轮所需依赖（`quinn` / `rustls` / `rcgen` / `bytes` / `futures` / `sha2` / `rand` / `p256` / `tempfile` …）。队友**只在自己的 crate `Cargo.toml` 里写 `{ workspace = true }`**，不要动根清单。
- `Cargo.lock` 由 cargo 自动维护；多个执行方并发编译时会**互相等待 target 锁**，这是正常的，不要 kill 也不要绕过。
- `audiolink-types` 已由 Lead 冻结（错误变体 / `NodeId` / `NodeInfo` / `ClockQuality`）。如缺类型，**报 Lead 加，不要自己加**。

---

## 2. 分层与依赖方向（单向，禁止反向）

```
types ──► proto ──► { audio , identity } ──► net ──► engine ──► ffi ──► { desktop/src-tauri , android }
```

三条硬规则：

1. **net 不依赖 identity**：net 只接收 `cert_der` / `key_der_pkcs8` 原始字节，自己不管证书从哪来。
2. **net 只做传输，不懂协议语义**：net `send/recv` 的是**不透明字节**；`§4` 控制帧载荷结构体与握手状态机属 **engine**（`audiolink-proto::lib.rs` 已言明 L2 分发层是 engine 的职责）。
3. **错误类型**：各 crate 用自己的 `XxxError`（`thiserror`，风格对齐 `audiolink-audio::AudioError`：`code() -> ErrorCode` + `context() -> &str` + `is_statistical()`）；**在 engine / ffi 边界收敛为 `audiolink_types::AudioLinkError`**。

---

## 3. `audiolink-net` 冻结 API（所有者：net-quic）

规格依据：`03-protocol.md` §2（传输映射）、§3（数据报 ≤ 1200 B）、§6（时钟同步）。
**参考实现**：`core/crates/audiolink-tools/src/bin/latency_probe.rs` —— 仓库内**已验证可编译可运行**的 quinn 0.11 + rustls 0.23(ring) + rcgen 0.14 用法，直接照抄其配置方式。

```rust
// ---- 端点 ----
pub struct EndpointConfig {
    pub bind: SocketAddr,
    /// 本机自签证书 DER（来自 identity，net 不解析）。
    pub cert_der: Vec<u8>,
    /// 本机私钥 PKCS#8 DER。
    pub key_der_pkcs8: Vec<u8>,
    /// 空闲超时（ms）；默认 10_000。
    pub idle_timeout_ms: u32,
    /// keep-alive 间隔（ms）；默认 3_000。
    pub keep_alive_ms: u32,
}

pub struct AudioLinkEndpoint { /* ... */ }

impl AudioLinkEndpoint {
    pub async fn bind(cfg: EndpointConfig) -> Result<Self, NetError>;
    pub fn local_addr(&self) -> Result<SocketAddr, NetError>;
    /// 接受下一个入站连接（循环调用）。
    pub async fn accept(&self) -> Result<Connection, NetError>;
    /// 主动连接。`server_name` 用固定值 `"audiolink"`（自签证书，SNI 不参与信任判定）。
    pub async fn connect(&self, addr: SocketAddr, server_name: &str) -> Result<Connection, NetError>;
    pub fn close(&self, code: u32, reason: &str);
}

// ---- 连接 ----
pub struct Connection { /* ... */ }

impl Connection {
    pub fn remote_addr(&self) -> SocketAddr;
    /// 对端证书指纹 = SHA-256(对端证书 DER) → `NodeId`。**信任判定的唯一依据**。
    pub fn peer_id(&self) -> Result<NodeId, NetError>;
    /// 对端 TLS 证书链的叶子证书 DER，供 §5 的 `AUTH_RESPONSE` 验签取公钥。
    ///
    /// 仅有 `peer_id()`（一个 SHA-256 摘要）还原不出公钥，所以验签必须靠这个。
    /// 取不到时返回 `Err` —— **不要**回退成空 `Vec`，否则验签会静默失败，把安全缺陷伪装成「认证不通过」。
    pub fn peer_cert_der(&self) -> Result<Vec<u8>, NetError>;
    /// QUIC 给出的数据报上限（本机实测 1162 B）。
    pub fn max_datagram_size(&self) -> Option<usize>;
    /// 音频数据报的**可用载荷上限** = `min(DATAGRAM_MAX_LEN, max_datagram_size) - DATAGRAM_HEADER_LEN`。
    /// 发送侧必须按此值分片/截断，不得直接按 1200 写。
    pub fn max_audio_payload(&self) -> usize;
    pub async fn send_datagram(&self, bytes: &[u8]) -> Result<(), NetError>;
    /// 读一个数据报**到调用方缓冲**（实时路径零分配）；返回写入长度；
    /// 缓冲不足时返回 `NetError`（不是截断静默成功）。
    pub async fn read_datagram_into(&self, buf: &mut [u8]) -> Result<usize, NetError>;
    /// 打开/接管控制流 #0。
    pub async fn open_control(&self) -> Result<ControlChannel, NetError>;
    /// QUIC 平滑 RTT（µs）。
    pub fn rtt_us(&self) -> u64;
    pub fn close(&self, code: u32, reason: &str);
}

// ---- 控制流 #0 ----
/// 收到的控制帧。
///
/// **为什么 `op` 是 `Option`**：`audiolink-proto` 是 L1 严格解码层，遇到**未知命令码**会返回
/// `BadRequest`；但 §1.1 规定「未知类型忽略并计数、不断流」是 **L2（engine）** 的职责。
/// 若这里只能给出 `OpCode`，net 就只能整体报错，把「命令码不认识」误升级成断连 —— 所以
/// 「信封合法但命令码未知」必须以 `op == None` **正常返回**，由 engine 决定忽略并计数。
pub struct ControlMessage {
    /// 已识别的命令码；`None` = 未知类型（§1.1：交给 L2 忽略并计数，不得断流）。
    pub op: Option<OpCode>,
    /// 线上原始 `type` 字节（诊断 / 计数用；`op` 为 `Some(v)` 时 `v.as_u8() == raw_type`）。
    pub raw_type: u8,
    pub flags: u16,
    pub request_id: u32,
    pub payload: Vec<u8>,   // postcard 原始字节，net 不解释
}

pub struct ControlChannel { /* ... */ }

impl ControlChannel {
    pub async fn send(&mut self, op: OpCode, request_id: u32, payload: &[u8]) -> Result<(), NetError>;
    /// 读一帧（内部缓冲自动增长；单帧上限 `CONTROL_MAX_PAYLOAD`）。
    pub async fn recv(&mut self) -> Result<ControlMessage, NetError>;
}

// ---- 时钟估计（§6 算法）----
pub struct ClockEstimate {
    pub offset_us: i64,
    pub drift_ppm: i32,
    pub rtt_us: u64,
    pub quality: ClockQuality,
    pub samples: usize,
}

pub struct ClockEstimator { /* 200 样本窗口 */ }

impl ClockEstimator {
    pub fn new() -> Self;
    /// 记录一次四时间戳样本（§6：`t1`/`t4` 本机单调 µs，`t2`/`t3` 对端单调 µs）。
    pub fn record(&mut self, probe_seq: u32, t1: u64, t2: u64, t3: u64, t4: u64) -> ClockSample;
    /// 当前估计：RTT 最小的 8 个样本取 `offset` 中位数；漂移 = 窗口内线性回归斜率。
    /// 样本 < 8 时返回 `None`（**不得**用退化值假装收敛）。
    pub fn estimate(&self) -> Option<ClockEstimate>;
    pub fn samples(&self) -> usize;
    pub fn clear(&mut self);
}
```

**验收（net-quic 必须自己跑出来）**：

- `cargo test -p audiolink-net` 全绿，至少要覆盖：
  1. 两个端点经 127.0.0.1 真 QUIC 握手成对，`peer_id()` 双方互相一致且等于各自证书的 SHA-256；
  2. 数据报往返：发 1162 B 成功、发 `max_audio_payload()+1` 被拒（返回 Err 而非 panic）；
  3. `read_datagram_into` 缓冲不足 → Err；
  4. 控制帧往返：24 个 `OpCode` 全扫一遍，`request_id` 与载荷字节一致；
  5. 控制帧载荷超过 `CONTROL_MAX_PAYLOAD` → 发送侧 Err；
  6. `ClockEstimator`：合成样本（固定 offset、无漂移）估计误差 ≤ 1 ms；注入一个 500 ms 的异常样本后估计**不被带偏**（中位数抗差）；样本 < 8 返回 `None`。
- 编译纪律：`cargo clippy -p audiolink-net --all-targets -- -D warnings` 干净；`unwrap`/`expect`/`panic` 禁止（workspace lint 已 deny）。

---

## 4. `audiolink-identity` 冻结 API（所有者：identity-pin）

规格依据：`03-protocol.md` §5（连接与认证时序、REST 强度说明、PIN 安全）、§9（信任库）。
实现提示：证书用 `rcgen 0.14`（**ECDSA P-256**，`signing_key.serialize_der()` 即 PKCS#8 DER）；签名/验签用 `p256`（纯 Rust，零 C 依赖，符合「不装 CMake」）。

```rust
pub struct NodeIdentity { /* ... */ }

impl NodeIdentity {
    /// 从 `dir` 加载 `cert.pem` + `key.pem`；不存在则生成并**原子落盘**（临时文件 + rename，见架构 §9）。
    pub fn load_or_create(dir: impl AsRef<Path>, node_name: &str) -> Result<Self, IdentityError>;
    pub fn from_pem(cert_pem: &str, key_pem: &str, node_name: &str) -> Result<Self, IdentityError>;
    pub fn id(&self) -> NodeId;                 // SHA-256(cert DER)
    pub fn cert_der(&self) -> &[u8];
    pub fn key_der_pkcs8(&self) -> &[u8];
    pub fn node_name(&self) -> &str;
    pub fn cert_pem(&self) -> &str;
    /// §5 `AUTH_RESPONSE`：`ECDSA-P256(privkey, nonce ‖ fp_local ‖ fp_peer)`，返回 DER 签名。
    pub fn sign_challenge(&self, nonce: &[u8; 32], peer: NodeId) -> Result<Vec<u8>, IdentityError>;
    /// 用对端证书验签。失败 → `AuthFailed`（1004，可能是中间人）。
    pub fn verify_challenge(
        peer_cert_der: &[u8],
        nonce: &[u8; 32],
        local: NodeId,
        peer: NodeId,
        signature: &[u8],
    ) -> Result<(), IdentityError>;
}

pub struct TrustEntry {
    pub id: NodeId,
    pub name: String,
    pub platform: Platform,
    pub paired_at_unix: u64,
}

pub struct TrustStore { /* ... */ }

impl TrustStore {
    /// 不存在 → 空库（不是错误）。文件损坏 → `Err`（**不静默重置**，否则信任边界被悄悄清空）。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, IdentityError>;
    pub fn is_trusted(&self, id: NodeId) -> bool;
    /// 写入并**原子落盘**；同一 `NodeId` 重复写入为更新。
    pub fn trust(&mut self, entry: TrustEntry) -> Result<(), IdentityError>;
    /// 撤销信任（FR-18）；返回是否确实删掉了。
    pub fn revoke(&mut self, id: NodeId) -> Result<bool, IdentityError>;
    pub fn entries(&self) -> &[TrustEntry];
}

/// PIN 门禁（§5「PIN 安全」：6 位数字 + 60 s 有效 + 失败 5 次锁定 5 分钟）。
pub struct PinGate { /* ... */ }

impl PinGate {
    pub fn new(now: Instant) -> Self;            // 生成并持有 6 位 PIN
    pub fn pin(&self) -> &str;                   // 接收端展示用
    pub fn expires_at(&self) -> Instant;
    pub fn remaining_attempts(&self) -> u8;
    /// 提交校验。成功**消耗**该 PIN（不可重放）。
    pub fn verify(&mut self, submitted: &str, now: Instant) -> Result<(), PairRejection>;
    /// 锁定中则返回剩余时长（`None` = 未锁定）。
    pub fn lockout_remaining(&self, now: Instant) -> Option<Duration>;
}

pub enum PairRejection {
    Expired,
    WrongPin { remaining: u8 },
    Locked { retry_after: Duration },
}
```

**验收（identity-pin 必须自己跑出来）**：

- `cargo test -p audiolink-identity` 全绿，至少覆盖：
  1. `load_or_create` 两次得到**同一 `NodeId`**（持久化生效）；`cert.pem`/`key.pem` 落盘存在；
  2. `NodeId` 等于对 `cert_der` 独立算出的 SHA-256（用 `sha2` 现算，不调被测代码）；
  3. 签名往返：A 签 B 验通过；换一个 `nonce` / 换 `peer` / 篡改签名 1 bit → 全部失败；
  4. `TrustStore`：落盘→重载后 `is_trusted` 保持；`revoke` 后变 false；**损坏 JSON → Err**；
  5. `PinGate`：正确 PIN 通过且**不可二次通过**（消耗）；错误 PIN 递减剩余次数；第 5 次失败后进入锁定且 `lockout_remaining` 正确；过期 PIN 返回 `Expired`；
  6. 落盘原子性：目录中途只读/路径不存在等错误路径**返回 Err 且不 panic**。

---

## 5. `audiolink-engine` 冻结 API（所有者：lead）

engine 是本轮的**集成核心**：把 `audio`（采集/编码/解码/播放）与 `net`（QUIC）拼成会话，并生成 `StreamStats`。
消费方：ffi / android / desktop / tools。

```rust
pub struct EngineConfig {
    pub node_name: String,
    pub identity_dir: PathBuf,
    pub trust_store_path: PathBuf,
    pub listen: SocketAddr,
    /// 编码参数（默认 20 ms / 160 kbps / VBR / 48 kHz）。
    pub codec: CodecConfig,
    /// 播放环深度（ms）；M1 固定值，默认 60。
    pub buffer_ms: u32,
}

pub struct Engine { /* ... */ }

impl Engine {
    pub async fn start(cfg: EngineConfig) -> Result<Arc<Self>, AudioLinkError>;
    pub fn info(&self) -> NodeInfo;
    pub fn local_addr(&self) -> SocketAddr;
    /// 接受入站连接的后台任务（返回句柄，便于测试等待）。
    pub fn spawn_accept_loop(self: &Arc<Self>) -> JoinHandle<()>;
    /// 主动连接（FR-17 手工 IP）：QUIC → §5 握手 →（必要时）PIN 配对；成功返回对端 `NodeId`。
    ///
    /// **对端要求 PIN 时返回 `1002 NOT_PAIRED`，但会话与命令通道仍然活着** ——
    /// UI 收到 [`EngineEvent::PinNeeded`] 后调 `submit_pin` 即可接着走同一条连接。
    /// 把「需要 PIN」当成连接失败会让 UI 只能整条重连，白白丢掉已完成的 QUIC 握手与 HELLO 交换。
    pub async fn connect(self: &Arc<Self>, addr: SocketAddr) -> Result<NodeId, AudioLinkError>;
    /// 开始向对端推流（发送方向）。
    /// 等本地采集/编码初始化和 OPEN_STREAM 写出，失败直接返回；尚不代表远端已开始播放。
    pub async fn start_send(&self, peer: NodeId) -> Result<(), AudioLinkError>;
    /// 停止推流（保留连接与信任）。
    pub async fn stop_send(&self, peer: NodeId) -> Result<(), AudioLinkError>;
    /// 提交对端显示的 PIN。**必须带 `peer`** —— PIN 本身没有归属信息，引擎无法从 6 位数字
    /// 反推是哪条会话（这决定了桌面端 command 是 `{ id_short, pin }` 而不是 `{ pin }`）。
    pub async fn submit_pin(&self, peer: NodeId, pin: &str) -> Result<(), AudioLinkError>;
    /// 已连接对端列表 + 每个对端的遥测。
    pub fn peers(&self) -> Vec<PeerStatus>;
    pub fn telemetry(&self, peer: NodeId) -> Option<StreamStats>;
    /// 事件订阅（UI 用；按 500 ms 批量，见架构 §4）。
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<EngineEvent>;
    pub async fn shutdown(&self);
}

pub struct PeerStatus {
    pub id: NodeId,
    pub name: String,
    pub addr: SocketAddr,
    /// 会话状态。**六态，不是五态** —— `Reconnecting` 是一个真实存在的状态
    /// （链路断了、正在退避重连），把它折叠进 `Degraded` 会让 UI 无法区分
    /// 「还能出声但质量差」与「已经断了」。见 [`SessionState`]。
    pub state: SessionState,       // Idle/Handshaking/Streaming/Degraded/Reconnecting/Failed
    pub trusted: bool,
    pub stats: StreamStats,
}

pub enum EngineEvent {
    /// 对端状态变化（连接 / 状态迁移）。
    PeerUpdated(Box<PeerStatus>),
    PeerDisconnected { id: NodeId, reason: String },
    /// 本机是接收端：把 PIN 显示给用户（对端要照着念）。
    DisplayPin { from: NodeId, name: String, pin: String, remaining_attempts: u8 },
    /// 本机是发起端：需要用户输入对端屏幕上显示的 PIN。
    PinNeeded { id: NodeId, name: String },
    PairCompleted { id: NodeId, ok: bool, reason: String },
    /// 遥测快照。
    ///
    /// ⚠️ **已知局限**：本事件不带对端 id —— 它对「本机 1 Hz 采样」与「对端 `STREAM_STATS`
    /// 透传」是同一个载荷。M1 单路会话够用；**多对端会串**，M3 落地多会话时必须给它加上 `peer: NodeId`。
    Telemetry(Box<StreamStats>),
    Error { code: u16, context: String },
}

/// 会话状态机（`docs/02-architecture.md` §11）。
pub enum SessionState { Idle, Handshaking, Streaming, Degraded, Reconnecting, Failed }
```

**`Streaming` 的语义（容易误读，桌面端踩过一次）**：`SessionState::Streaming` 表示**握手已完成、
会话可用**，不代表「正在推流」。引擎不区分「已连接未推流」与「正在推流」——
外壳若要区分，自己在 `start_send` 成功前后加标记（桌面端就是这么做的：
只有 `start_send` 之后才把卡片呈现为 `streaming`，否则呈现 `idle`）。
若不加这个映射，UI 会在刚连上时就显示「推送中」，按钮还会错误地变成「停止推流」。

**`e2e_latency_us` 的分位数不在 `StreamStats` 里**：该结构只有单值字段（§10 冻结）。
需要 P50/P95 的消费方（如桌面遥测面板）自己按对端维护滑动窗口。

**「零重采样」断言口径（验收硬指标，engine 负责实现）**：采集线程与播放线程在**启动时**校验
`device_format()` 为 `48_000 Hz / 2ch / F32`，不达标即拒绝启动（返回 `1005 CAP_UNSUPPORTED`），
**绝不**顺手做 SRC。断言在音频线程内、建立设备之后执行，失败会通过 ready 信号同步回报给调用方，
因此「推流已开始」这件事不会在失败时被谎报。

**`STREAM_STATS` 是双方互发的**（§10）：引擎 1 Hz 把快照同时（a）以控制帧发给对端、
（b）作为本地事件推给 UI。漏掉 (a) 的后果很具体：推流端面板永远看不到 e2e 延迟、播放环水位、
欠载次数 —— 而那三个量**只有接收侧才量得到**。

---

## 6. 桌面端契约（所有者：desktop-ui）

`desktop/src-tauri` 对前端暴露以下 **Tauri command**（既有名称/字段保持兼容；2026-09-16 为看板采集端点选择器补充可选入参与两个查询）：

| command | 入参 | 返回（JSON） |
|---|---|---|
| `version` | — | `String`（已有） |
| `local_status` | — | `{ id_short: string, name: string, addr: string, platform: string }` |
| `list_peers` | — | `PeerView[]` |
| `connect` | `{ addr: string }` | `PeerView` |
| `list_capture_devices` | — | `CaptureDeviceView[]`（活动的 Windows 输出端点） |
| `active_capture_device` | — | `CaptureDeviceView \| null`（实际正在采集的端点） |
| `start_send` | `{ id_short: string, capture_device_id?: string \| null }` | `{ stream_id: number }` |
| `stop_send` | — | `null` |
| `submit_pin` | `{ id_short: string, pin: string }` | `{ ok: boolean, reason: string }` |
| `telemetry` | — | `TelemetryView` |

```ts
type PeerView = {
  idShort: string; name: string; addr: string;
  state: "idle" | "handshaking" | "streaming" | "degraded" | "reconnecting" | "failed";
  trusted: boolean;
};
type TelemetryView = {
  peers: number; rttUs: number; jitterUs: number; lossPct: number;
  bitrateBps: number; bufferLevelUs: number; underruns: number;
  e2eLatencyUs: number; e2eP50Us: number; e2eP95Us: number;
};
type CaptureDeviceView = {
  id: string; name: string; isDefault: boolean; isVirtual: boolean;
  sampleRate: number; channels: number; unavailableReason: string | null;
};
```

采集选择语义：省略或 `null` 在**每次开流**时解析系统默认输出；显式 ID 精确匹配活动端点，
不存在或格式不支持则拒绝，不回退默认设备。采集工厂在自己的线程里打开 WASAPI 并记录实际端点名字、ID、格式。
`start_send` 等待设备/编码初始化与 OPEN_STREAM 写出，失败不会把桌面卡片标成推流中；成功仍不代表远端已开始播放。
开始/停止操作在桌面桥中串行化；更换端点前需停止推流。选择保留在当前界面会话中。
`active_capture_device` 的快照不会随 Windows 默认设备变化而改变；停止或断连后返回 `null`。

**事件**（后端 → 前端，`listen` 订阅名冻结）：`audiolink://peer`（`PeerView[]`）、
`audiolink://telemetry`（`TelemetryView`，**500 ms** 节流）、`audiolink://pair-required`（`{ idShort, name, pin }`）。

**UI 范围（M1 最小版）**：手工输入 IP → 连接 → 显示对端卡片与状态 → 开始/停止推流 → 遥测数字面板 +
连接失败原因；补充本机采集端点选择、刷新、不可用原因及实际端点展示。**不做**：托盘 / 自启 / 双语 / 局域网自动发现设备列表 / 曲线图（属 M5/M2）。

**验收**：`pnpm build`（TS 严格 + Vite 构建）通过；`cargo check -p audiolink-desktop` 通过。
engine 尚未就绪时，Rust 侧 command 可先返回**明确的 mock**，但必须留 `TODO(M1)` 并在引擎落地后替换 —— 不允许把 mock 当成交付。

---

## 7. Android 契约（所有者：android-playback）

**架构纪律（架构 §1 第 3 条，不可反转）**：`AudioTrack` 的生命周期与线程亲和性**必须在 Kotlin 侧**，
内核只输出「PCM + 目标播放时刻 + 期望速率」，不做「内核托管播放线程」。

Kotlin 侧冻结接口（Kotlin 文件路径由实现者自定，接口形状冻结）：

```kotlin
/** 低延迟播放器：持有 AudioTrack，消费 PCM。 */
interface PcmSource { fun readInto(dst: FloatArray): Int }

class LowLatencyPlayer(
    sampleRate: Int = 48_000,          // 必须 48000（零重采样）
    channelCount: Int = 2,
    bufferFrames: Int,                 // 请求缓冲（帧）
) {
    /** 必须断言 AudioTrack.getPerformanceMode() == PERFORMANCE_MODE_LOW_LATENCY，否则抛/上报。 */
    fun start(): PlaybackReport
    fun write(samples: FloatArray, frames: Int): Unit
    fun stop()
    fun stats(): PlaybackStats           // underruns / framesWritten / writeErrors / actualBufferFrames / performanceMode
}

data class PlaybackReport(val lowLatency: Boolean, val actualBufferFrames: Int,
                          val sampleRate: Int, val channelCount: Int, val performanceMode: Int)
```

**验收（本机无真机，只能做到这些，必须如实报告）**：

1. `pwsh tools/gradlew.ps1 -JavaHome 'C:\Users\liuzh\scoop\apps\corretto17-jdk\current' assembleDebug` 成功（**注意 `-JavaHome` 是位置参数**，直接传 `assembleDebug` 会被当成 JDK 路径而报错）；
2. **JVM 单元测试**（`app/src/test/**`，`./gradlew testDebugUnitTest`）覆盖**不依赖 Android 框架**的纯逻辑：环缓冲的读写/溢出/欠载计数、PCM 交错与 `FloatArray` 索引换算、`PlaybackStats` 累计口径；
3. 所有 `TODO(M1)` 要么落地要么保留并写清阻塞原因；服务里 `startForeground` 的前台类型分级逻辑**保持现状**（已按 API 等级精确分级）。
4. **明确报告**：真机指标（`getPerformanceMode()` 实测值、出声延迟、30 min 无断流）**本轮无法验证**，需真机接入后再跑。

---

## 8. `audiolink-ffi` 契约（所有者：ffi-bridge）

- UniFFI 0.29，导出给 Kotlin；**唯一允许 `unsafe` 的 crate**，需在文件顶部 `#[allow(unsafe_code)]` 并写明理由。
- 导出面（M1 最小集）：`engineStart(config)` / `engineStop()` / `connect(addr)` / `startSend()` / `stopSend()` /
  `submitPin(pin)` / `localStatus()` / `peers()` / `telemetry()` / `displayedPin()`，
  加一个 `protocolSelfTest()` → `String`，内部跑 `audiolink-proto` 的 golden vectors 并返回摘要
  （这就是看板 `[护栏] 跨端一致性夹具`：**同一组向量在 Rust 与 FFI 两侧双跑，结果必须一致**）。
- **验收**：`cargo test -p audiolink-ffi` 通过（`protocolSelfTest` 的 Rust 侧断言）；
  `cargo check -p audiolink-ffi --target aarch64-linux-android` 通过（证明 FFI 表面能交叉编译）；
  若 UniFFI 代码生成可用，把生成的 Kotlin 放到 android 侧并让 `assembleDebug` 通过。

---

## 9. 验收口径与命令（所有人提交前自跑）

```powershell
# 1) 内核：格式 + lint + 测试（工作区级，排除桌面外壳——它依赖 Tauri 的构建期代码生成）
cargo fmt --all
cargo clippy --workspace --exclude audiolink-desktop --all-targets --all-features -- -D warnings
cargo test --workspace --exclude audiolink-desktop

# 2) Android 交叉编译内核
cargo check -p audiolink-ffi --target aarch64-linux-android

# 3) 桌面外壳
cargo check -p audiolink-desktop
cd desktop; pnpm build

# 4) Android APK（-JavaHome 必须显式给，否则位置参数会被吃掉）
pwsh tools/gradlew.ps1 -JavaHome 'C:\Users\liuzh\scoop\apps\corretto17-jdk\current' assembleDebug
```

**报告纪律**：每条结论必须附**原始命令 + 关键输出行**。没有跑过的，写「未验证」，**不许推断**。

---

## 10. 已知坑（别重复踩）

1. 本机 `JAVA_HOME` 指向 **JDK 11**，Gradle 9 需要 17+ → 必须显式 `-JavaHome`；
2. `tools/gradlew.ps1` 的 `-JavaHome` 是**第一个位置参数**，`pwsh tools/gradlew.ps1 assembleDebug` 会把它当 JDK 路径；
3. rustls 必须用 **ring** 后端（`builder_with_provider` + `QuicServerConfig::try_from`），默认 aws-lc-rs 要 CMake；
4. quinn 0.11 的读接口叫 **`read_datagram()`**（不是 `recv_datagram`）；
5. `AudioLinkError` 现已是 §11 全量变体，`bad_request()` 的签名**保持不变**（M0 用例依赖）；
6. `CaptureSource` / `PlayoutSink` **不是 `Send`**（WASAPI COM 线程绑定）→ 谁用谁建，线程只传数据；
7. 共享模式 WASAPI 缓冲下限 **22 ms**、引擎周期 **10 ms**（别再按「10–20 ms 缓冲」立指标）；
8. Gradle 的 `GRADLE_USER_HOME` 已隔离到 `.gradle-home`，别动全局配置。
