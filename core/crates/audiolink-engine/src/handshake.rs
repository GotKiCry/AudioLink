//! §5 握手的**纯状态机**（`docs/03-protocol.md` §5）
//!
//! ```text
//! 发起方 A                                    响应方 B
//!   │ ── HELLO(proto_version, node_info, nonce) ──► │
//!   │ ◄──────────── HELLO_ACK(accepted) ──────────  │
//!   │        ← 到此握手已 Established：没有 AUTH_*、没有 PAIR_* 分支
//! ```
//!
//! # 为什么把「发什么帧」和「做什么 I/O」分开
//!
//! 本模块**不做任何 I/O、不碰网络、不读文件**：它只吃「收到了一条已解码的控制命令」，
//! 吐出「该回哪些帧 + 该通知上层什么」。这样握手逻辑可以在毫秒内被测完（含各种攻击路径），
//! 而不需要起 UDP 端口、不依赖真机、不依赖单调时钟的真实流逝。
//!
//! # 三条不能被简化掉的纪律
//!
//! 1. **对端身份只认 TLS 证书指纹**。`HELLO.node_info.id` 是**对端自报**的字段，
//!    任何人都能填任意值；信了它等于让任何人都能冒充别人的身份。本模块一律用 `peer_id`
//!    （由 `audiolink-net::Connection::peer_id()` 从对端证书现算）覆盖自报值 —— 见 [`Handshake::on_control`]。
//! 2. **过程序外帧一律忽略并计数，不断流**（§1.1）。握手期间收到 `PING`、未知命令码、载荷非法的帧，
//!    都是「忽略」而不是「拒绝连接」—— 否则对端一个版本差异就能把连接踢掉。
//! 3. **`request_id` 必须校验**。§4 规定响应方要回填请求方的 `request_id`。
//!    不校验的话，一条**上一轮握手的迟到响应**会被当成本轮的合法响应接受 —— 这是可被重放利用的洞。

use audiolink_types::{AudioLinkError, Capabilities, ErrorCode, NodeId, NodeInfo, PROTO_VERSION};

use crate::dispatch::ControlRequest;
use crate::payload::{HelloAckPayload, HelloPayload};

/// 握手角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// 主动发起连接的一方（§5 的节点 A）。
    Initiator,
    /// 接受连接的一方（§5 的节点 B，负责应答 `HELLO`）。
    Responder,
}

/// 握手阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakePhase {
    /// 未开始。
    Idle,
    /// 发起方已发 `HELLO`，等 `HELLO_ACK`。
    AwaitHelloAck,
    /// 已收到对端的 `HELLO`/`HELLO_ACK`，正在收尾。
    ///
    /// 删除认证与配对分支后没有任何迁移会进入本阶段（`HELLO` 交换成功即 `Established`）；
    /// 保留它是按 `docs/71-remove-pairing.md` §2.2 的冻结范围执行 —— 那里只点名删掉
    /// `AwaitAuthResponse` / `AwaitPairSubmit` / `AwaitPairResult` / `AwaitAuthOk`。
    AwaitPeer,
    /// 握手完成（可以进入会话协商）。
    Established,
    /// 握手失败（终态）。
    Failed,
}

impl HandshakePhase {
    /// 快照名（日志 / 测试断言用）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::AwaitHelloAck => "await-hello-ack",
            Self::AwaitPeer => "await-peer",
            Self::Established => "established",
            Self::Failed => "failed",
        }
    }
}

/// 一条待发出的控制命令。
#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    /// 命令本体。
    pub request: ControlRequest,
    /// `request_id`：**非 0 = 请求**（对端必须回填同值），**0 = 单向通知**（§4）。
    pub request_id: u32,
}

impl Outgoing {
    /// 单向通知（`request_id = 0`）。
    pub fn notice(request: ControlRequest) -> Self {
        Self {
            request,
            request_id: 0,
        }
    }

    /// 请求（`request_id` 由 [`Handshake`] 分配）。
    pub const fn request(request: ControlRequest, request_id: u32) -> Self {
        Self {
            request,
            request_id,
        }
    }
}

/// 除「发帧」之外要通知上层的事。
#[derive(Debug, Clone, PartialEq)]
pub enum HandshakeEvent {
    /// 什么都不用做（帧已放进 `send`，或本帧被忽略）。
    None,
    /// 握手完成。
    Established {
        /// 对端节点信息（`id` 已用证书指纹覆盖）。
        peer: NodeInfo,
        /// §13 能力协商：本端能力位图。
        local_caps: u32,
        /// §13 能力协商：对端声明的能力位图。
        peer_caps: u32,
        /// §13 能力协商：双方交集。
        agreed_caps: u32,
    },
    /// 握手失败（终态）。
    Rejected {
        /// 失败原因（已映射为 §11 错误码）。
        error: AudioLinkError,
    },
    /// 本阶段无关的帧：已忽略并计数（§1.1），**连接继续**。
    Ignored,
}

/// 一次状态推进的完整产物。
#[derive(Debug, Clone, PartialEq)]
pub struct HandshakeStep {
    /// 需要**按顺序**发出的命令（可能为空）。
    pub send: Vec<Outgoing>,
    /// 除发帧外的通知。
    pub event: HandshakeEvent,
}

impl HandshakeStep {
    /// 什么都不做。
    pub fn nothing() -> Self {
        Self {
            send: Vec::new(),
            event: HandshakeEvent::None,
        }
    }

    /// 只发帧。
    pub fn sending(send: Vec<Outgoing>) -> Self {
        Self {
            send,
            event: HandshakeEvent::None,
        }
    }

    /// 被忽略的一帧（不产帧、不改状态）。
    pub fn ignored() -> Self {
        Self {
            send: Vec::new(),
            event: HandshakeEvent::Ignored,
        }
    }

    /// 附上一个事件。
    pub fn with_event(mut self, event: HandshakeEvent) -> Self {
        self.event = event;
        self
    }
}

/// 握手状态机。
#[derive(Debug)]
pub struct Handshake {
    role: Role,
    phase: HandshakePhase,
    local: NodeInfo,
    peer_id: NodeId,
    peer: Option<NodeInfo>,
    /// `HELLO` 里带的本机随机数（把这次 `HELLO` 与本次连接绑定，防重放）。
    local_nonce: [u8; 32],
    /// 本机最近分配出去的 `request_id`（0 = 尚未分配）。
    next_request_id: u32,
    /// 等待对端回填的 `request_id`（0 = 当前不期待响应）。
    pending_request_id: u32,
    /// 被忽略的帧数（§1.1 的可观测面）。
    ignored_frames: u64,
    /// 本端能力位图（§13）；默认 [`Capabilities::CURRENT`]，可用 `with_capabilities` 覆盖。
    local_caps: u32,
    /// 协商结果（双方能力交集）；握手未走到 `HELLO`/`HELLO_ACK` 交换时为 0。
    agreed_caps: u32,
    /// 对端声明的能力位图（`HELLO` 或 `HELLO_ACK` 里带来）。
    peer_caps: u32,
}

impl Handshake {
    /// 新建握手。
    ///
    /// `peer_id` 必须来自 **TLS 证书指纹**（`audiolink_net::Connection::peer_id()`），
    /// 不能来自对端自报的 `node_info.id`。
    ///
    pub fn new(role: Role, local: NodeInfo, peer_id: NodeId) -> Self {
        Self {
            role,
            phase: HandshakePhase::Idle,
            local,
            peer_id,
            peer: None,
            local_nonce: random_nonce(),
            next_request_id: 0,
            pending_request_id: 0,
            ignored_frames: 0,
            local_caps: Capabilities::CURRENT,
            agreed_caps: 0,
            peer_caps: 0,
        }
    }

    /// 注入本端能力位图（默认 [`Capabilities::CURRENT`]）。
    ///
    /// 存在的理由主要是**测试与将来的平台接线**：内录能力取决于平台（Windows 有 WASAPI loopback、
    /// Android 要看 API 等级），接上之后要能声明不同能力集，而不是被一个写死的常量锁住。
    #[must_use]
    pub const fn with_capabilities(mut self, caps: u32) -> Self {
        self.local_caps = caps;
        self
    }

    /// 协商结果（双方能力交集）；尚未交换 `HELLO` 时为 0。
    pub const fn agreed_caps(&self) -> u32 {
        self.agreed_caps
    }
}

impl Handshake {
    /// 当前阶段。
    pub const fn phase(&self) -> HandshakePhase {
        self.phase
    }

    /// 握手角色。
    pub const fn role(&self) -> Role {
        self.role
    }

    /// 对端节点信息（`HELLO` 交换后可用，`id` 已被证书指纹覆盖）。
    pub const fn peer(&self) -> Option<&NodeInfo> {
        self.peer.as_ref()
    }

    /// 对端指纹（来自证书，从一开始就可用）。
    pub const fn peer_id(&self) -> NodeId {
        self.peer_id
    }

    /// 是否已完成握手。
    pub const fn is_established(&self) -> bool {
        matches!(self.phase, HandshakePhase::Established)
    }

    /// 被忽略的帧数（§1.1；长期偏高说明两端版本漂移）。
    pub const fn ignored_frames(&self) -> u64 {
        self.ignored_frames
    }

    /// 开局第一步：发起方发 `HELLO`；响应方静候对端。
    pub fn start(&mut self) -> HandshakeStep {
        if self.role != Role::Initiator || self.phase != HandshakePhase::Idle {
            return HandshakeStep::nothing();
        }

        let nonce = self.local_nonce.to_vec();
        let request = ControlRequest::Hello(HelloPayload {
            proto_version: PROTO_VERSION,
            node: self.local.clone(),
            nonce: nonce.clone(),
            caps: self.local_caps,
        });

        self.phase = HandshakePhase::AwaitHelloAck;
        let request_id = self.issue_request_id();

        // HELLO 里的 nonce 只用于把 HELLO 与本次连接绑定（防重放），因此这里不设 pending：
        // HELLO_ACK 是**响应**而非**请求**，`request_id` 由对端选择。
        self.pending_request_id = 0;

        HandshakeStep::sending(vec![Outgoing::request(request, request_id)])
    }

    /// 处理一条已解码的对端命令。
    ///
    /// 删除认证与配对分支后，握手只剩 `HELLO` / `HELLO_ACK` 两个分支需要处理；
    /// 其余帧一律走 §1.1 的「忽略并计数」。
    ///
    /// 不再需要本机身份材料与对端证书：签名 / 验签是挑战应答专用的，已随认证一起删除。
    pub fn on_control(&mut self, request: &ControlRequest) -> HandshakeStep {
        match request {
            ControlRequest::Hello(payload) => self.on_hello(payload),

            ControlRequest::HelloAck(payload) => self.on_hello_ack(payload),

            // 对端明确报错
            ControlRequest::Error(payload) => {
                let error = AudioLinkError::owned(
                    audiolink_types::ErrorCode::from_u16(payload.code)
                        .unwrap_or(audiolink_types::ErrorCode::BadRequest),
                    format!("{}: {}", payload.code, payload.message),
                );
                self.fail(error)
            }

            // 对端说要走了
            ControlRequest::Bye(payload) => {
                let error = AudioLinkError::bad_request_owned(format!(
                    "peer said bye during handshake: {}",
                    payload.reason
                ));
                self.fail(error)
            }

            // 握手期间其余一切（PING、遥测、还没实现的 M2/M3 命令…）都只是「忽略并计数」。
            // 这是 §1.1 的直接落实：**不认识的帧不构成断连理由**。
            _ => self.ignore(),
        }
    }

    // -----------------------------------------------------------------
    // 各分支
    // -----------------------------------------------------------------

    fn on_hello(&mut self, payload: &HelloPayload) -> HandshakeStep {
        // HELLO 只对响应方有语义；发起方收到 HELLO 说明两端同时发起（都以为自己是 A）。
        // 这不是攻击，但确实是状态错误 —— 报 Busy 而不是静默接受，避免两端都以为自己是对端。
        if self.role != Role::Responder || self.phase != HandshakePhase::Idle {
            if self.role == Role::Initiator && self.phase == HandshakePhase::AwaitHelloAck {
                return self.fail(AudioLinkError::busy(
                    "simultaneous connect: both sides sent HELLO",
                ));
            }
            return self.ignore();
        }

        self.peer = Some(self.certified_peer_info(&payload.node));

        // §13：协议主版本不兼容 → 明确拒绝，不能让两端跑在半懂不懂的状态里。
        if payload.proto_version != PROTO_VERSION {
            let reason = format!(
                "proto version mismatch: peer=0x{:04X} local=0x{:04X}",
                payload.proto_version, PROTO_VERSION
            );
            let ack = ControlRequest::HelloAck(HelloAckPayload {
                proto_version: PROTO_VERSION,
                node: self.local.clone(),
                accepted: false,
                reason: reason.clone(),
                caps: Capabilities::CURRENT,
                agreed_caps: 0,
            });
            // 阶段必须落到 `Failed`：只回一条拒绝帧而不改状态，会让本端停在
            // 「等对端说话」上看不出已经结束 —— 上层会一直等一个不会来的响应。
            self.phase = HandshakePhase::Failed;
            return HandshakeStep::sending(vec![Outgoing::notice(ack)]).with_event(
                HandshakeEvent::Rejected {
                    error: AudioLinkError::owned(ErrorCode::VersionMismatch, reason),
                },
            );
        }

        // §13 能力协商：交集缺了必需位就当场拒绝。不这么做的话，链路会「连上但没声音」——
        // 失败被推迟到开流那一刻，用户看到的只是一条编解码错误，排查成本高得多。
        let agreed = Capabilities::intersect(self.local_caps, payload.caps);
        let missing = Capabilities::missing_required(agreed);
        if missing != 0 {
            let reason = format!(
                "能力协商失败：缺少 {}（本端 {}；对端 {}）",
                Capabilities::describe(missing),
                Capabilities::describe(self.local_caps),
                Capabilities::describe(payload.caps)
            );
            let ack = ControlRequest::HelloAck(HelloAckPayload {
                proto_version: PROTO_VERSION,
                node: self.local.clone(),
                accepted: false,
                reason: reason.clone(),
                caps: self.local_caps,
                agreed_caps: 0,
            });
            self.phase = HandshakePhase::Failed;
            return HandshakeStep::sending(vec![Outgoing::notice(ack)]).with_event(
                HandshakeEvent::Rejected {
                    error: AudioLinkError::owned(ErrorCode::CapUnsupported, reason),
                },
            );
        }
        self.agreed_caps = agreed;
        self.peer_caps = payload.caps;

        let ack = ControlRequest::HelloAck(HelloAckPayload {
            proto_version: PROTO_VERSION,
            node: self.local.clone(),
            accepted: true,
            reason: String::new(),
            caps: self.local_caps,
            agreed_caps: agreed,
        });

        // `HELLO_ACK` 一发出，双方对「身份、协议版本、能力」的理解就一致了 —— 握手到此完成。
        // 无认证之后没有后续的挑战应答或配对分支，直接进入会话协商（§5）。
        self.phase = HandshakePhase::Established;
        HandshakeStep::sending(vec![Outgoing::notice(ack)]).with_event(
            HandshakeEvent::Established {
                peer: self.peer_info_or_stub(),
                local_caps: self.local_caps,
                peer_caps: self.peer_caps,
                agreed_caps: self.agreed_caps,
            },
        )
    }

    fn on_hello_ack(&mut self, payload: &HelloAckPayload) -> HandshakeStep {
        if self.role != Role::Initiator || self.phase != HandshakePhase::AwaitHelloAck {
            return self.ignore();
        }

        self.peer = Some(self.certified_peer_info(&payload.node));

        if !payload.accepted {
            return self.fail(AudioLinkError::bad_request_owned(format!(
                "peer rejected the connection: {}",
                payload.reason
            )));
        }

        // 版本先于一切校验：先确认双方对协议的理解一致，再谈别的。
        if payload.proto_version != PROTO_VERSION {
            return self.fail(AudioLinkError::bad_request_owned(format!(
                "proto version mismatch: peer=0x{:04X} local=0x{:04X}",
                payload.proto_version, PROTO_VERSION
            )));
        }

        // §13 能力协商的**复核**：本端自己算一遍交集，与对端报来的结果比对。
        // 两侧算法一致时这是恒等检查；一旦将来只改了一侧，这里会立刻失败 ——
        // 比「双方带着不一样的理解继续跑」便宜得多。
        let agreed = Capabilities::intersect(self.local_caps, payload.caps);
        if agreed != payload.agreed_caps {
            return self.fail(AudioLinkError::bad_request_owned(format!(
                "capability negotiation mismatch: local 0x{agreed:04X} peer 0x{:04X}",
                payload.agreed_caps
            )));
        }
        if Capabilities::missing_required(agreed) != 0 {
            return self.fail(AudioLinkError::owned(
                ErrorCode::CapUnsupported,
                format!(
                    "能力协商失败：缺少 {}",
                    Capabilities::describe(Capabilities::missing_required(agreed))
                ),
            ));
        }
        self.agreed_caps = agreed;
        self.peer_caps = payload.caps;

        // `HELLO_ACK` 校验通过 = 握手完成：无认证之后没有「等对端选分支」这一步。
        self.phase = HandshakePhase::Established;
        HandshakeStep::nothing().with_event(HandshakeEvent::Established {
            peer: self.peer_info_or_stub(),
            local_caps: self.local_caps,
            peer_caps: self.peer_caps,
            agreed_caps: self.agreed_caps,
        })
    }

    // -----------------------------------------------------------------
    // 内部工具
    // -----------------------------------------------------------------

    fn ignore(&mut self) -> HandshakeStep {
        self.ignored_frames = self.ignored_frames.saturating_add(1);
        HandshakeStep::ignored()
    }

    fn fail(&mut self, error: AudioLinkError) -> HandshakeStep {
        self.phase = HandshakePhase::Failed;
        HandshakeStep::nothing().with_event(HandshakeEvent::Rejected { error })
    }

    fn issue_request_id(&mut self) -> u32 {
        // `request_id` 从 1 开始：§4 规定 0 表示「单向通知，无需响应」。
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        self.next_request_id
    }

    /// 用**证书指纹**覆盖对端自报的 `id`（见本模块文档第 1 条纪律）。
    fn certified_peer_info(&self, reported: &NodeInfo) -> NodeInfo {
        let mut info = reported.clone();
        info.id = self.peer_id;
        info.caps = info.caps.known_only();
        // 名字只用于展示，允许对端自报；但长度要设上限，避免 UI 被超长字符串撑爆。
        if info.name.chars().count() > 64 {
            info.name = info.name.chars().take(64).collect();
        }
        info.platform = reported.platform;
        info.proto_version = reported.proto_version;
        info
    }

    fn peer_info_or_stub(&self) -> NodeInfo {
        self.peer.clone().unwrap_or_else(|| {
            NodeInfo::new(
                self.peer_id,
                "unknown",
                audiolink_types::Platform::Unknown,
                audiolink_types::Caps::NONE,
            )
        })
    }
}

/// 生成 32 B 随机数（§5：`HELLO.nonce(32 B)`，把这次 `HELLO` 与本次连接绑定）。
fn random_nonce() -> [u8; 32] {
    let mut nonce = [0u8; 32];
    rand::Rng::fill(&mut rand::rng(), &mut nonce);
    nonce
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::payload::{ByePayload, ErrorPayload, PingPayload, SetMutePayload};
    use audiolink_identity::NodeIdentity;
    use audiolink_types::{Caps, OpCode, Platform};
    use tempfile::TempDir;

    /// 一个测试用节点：真证书、真密钥（落在临时目录里，测试结束即删）。
    ///
    /// 刻意**不用编造的指纹**：本模块第一条纪律是「对端身份只认证书指纹」，
    /// 用假 id 测，等于把最该验证的部分（自报 id 必须被证书覆盖）测成了空。
    struct Node {
        identity: NodeIdentity,
        _dir: TempDir,
    }

    impl Node {
        fn new(name: &str) -> Self {
            let dir = TempDir::new().unwrap();
            let identity = NodeIdentity::load_or_create(dir.path(), name).unwrap();
            Self {
                identity,
                _dir: dir,
            }
        }

        fn info(&self, display_name: &str) -> NodeInfo {
            NodeInfo::new(
                self.identity.id(),
                display_name,
                Platform::Windows,
                Caps::CAN_SEND | Caps::CAN_RECEIVE,
            )
        }
    }

    /// 把 `step` 里的帧按顺序投递给 `target`，返回**最后一次**产生的 step。
    fn pump(target: &mut Handshake, step: &HandshakeStep) -> HandshakeStep {
        let mut last = HandshakeStep::nothing();
        for outgoing in &step.send {
            last = target.on_control(&outgoing.request);
        }
        last
    }

    fn established_of(step: &HandshakeStep) -> Option<&NodeInfo> {
        match &step.event {
            HandshakeEvent::Established { peer, .. } => Some(peer),
            _ => None,
        }
    }

    #[test]
    fn handshake_completes_in_one_round_trip_on_both_sides() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");

        let mut a = Handshake::new(Role::Initiator, a_node.info("A"), b_node.identity.id());
        let mut b = Handshake::new(Role::Responder, b_node.info("B"), a_node.identity.id());

        // A 发起：HELLO 是**请求**，request_id 必须非 0（§4：0 表示无需响应的单向通知）。
        let s1 = a.start();
        assert_eq!(s1.send.len(), 1);
        assert_eq!(s1.send[0].request.op(), OpCode::Hello);
        assert!(s1.send[0].request_id >= 1, "HELLO 必须带非 0 request_id");
        assert_eq!(a.phase(), HandshakePhase::AwaitHelloAck);

        // B 收 HELLO → 回 HELLO_ACK，并当场完成（没有 AUTH_*、没有 PAIR_*）。
        let s2 = pump(&mut b, &s1);
        assert_eq!(s2.send.len(), 1, "无认证后 HELLO_ACK 是唯一要发的帧");
        assert_eq!(s2.send[0].request.op(), OpCode::HelloAck);
        assert_eq!(
            s2.send[0].request_id, 0,
            "ACK 是响应，不带自己的 request_id"
        );
        assert_eq!(b.phase(), HandshakePhase::Established);
        let peer_seen_by_b = established_of(&s2).expect("B 应当完成握手");
        assert_eq!(peer_seen_by_b.id, a_node.identity.id(), "对端指纹来自证书");
        assert!(b.is_established());

        // A 收 HELLO_ACK → 完成。
        let s3 = pump(&mut a, &s2);
        let peer_seen_by_a = established_of(&s3).expect("A 应当完成握手");
        assert_eq!(peer_seen_by_a.id, b_node.identity.id(), "对端指纹来自证书");
        assert!(a.is_established());
    }

    #[test]
    fn established_peer_info_carries_the_certificate_identity() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");

        let mut a = Handshake::new(Role::Initiator, a_node.info("A"), b_node.identity.id());
        let mut b = Handshake::new(Role::Responder, b_node.info("B"), a_node.identity.id());

        let s1 = a.start();
        let s2 = pump(&mut b, &s1);
        let s3 = pump(&mut a, &s2);

        let peer = established_of(&s3).unwrap();
        assert_eq!(peer.id, b_node.identity.id());
        assert_eq!(peer.name, "B");
        assert_eq!(peer.platform, Platform::Windows);
        assert!(peer.caps.contains(Caps::CAN_SEND));
    }

    #[test]
    fn self_reported_node_id_is_overridden_by_the_certificate_fingerprint() {
        // 核心纪律：HELLO 里的 node_info.id 是**对端自报**的，任何人都能填成别人的指纹。
        // 真实数据流：`peer_id` 来自 `audiolink_net::Connection::peer_id()`（对端证书的 SHA-256），
        // 在 QUIC 握手完成时就有了 —— 而 `HELLO` 是之后才到的。所以这里给 b 传**攻击者证书**的指纹。
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");
        let attacker = Node::new("attacker");

        let mut b = Handshake::new(Role::Responder, b_node.info("B"), attacker.identity.id());

        // 攻击者用自己的连接，但把 node_info.id 填成 A 的指纹
        let mut forged = attacker.info("A（伪装）");
        forged.id = a_node.identity.id();
        assert_eq!(forged.id, a_node.identity.id());

        let hello = ControlRequest::Hello(HelloPayload {
            proto_version: PROTO_VERSION,
            node: forged,
            nonce: vec![0u8; 16],
            caps: Capabilities::CURRENT,
        });
        let step = b.on_control(&hello);

        assert!(
            matches!(step.event, HandshakeEvent::Established { .. }),
            "无认证：HELLO 校验通过即完成握手"
        );
        let seen = b.peer().expect("应当记下对端信息");
        assert_eq!(
            seen.id,
            attacker.identity.id(),
            "对端身份必须来自证书指纹，而不是 HELLO 自报值"
        );
        assert_ne!(seen.id, a_node.identity.id(), "伪造的指纹必须被丢弃");
        assert_eq!(
            b.peer_id(),
            attacker.identity.id(),
            "握手自身的 peer_id 也必须是证书那个"
        );
    }

    #[test]
    fn capability_mismatch_is_rejected_with_a_readable_reason() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");

        // 响应方只有 Opus + 播放；发起方只声明「采集」（没有 Opus）→ 交集缺必需位。
        let mut b = Handshake::new(Role::Responder, b_node.info("B"), a_node.identity.id())
            .with_capabilities(Capabilities::OPUS | Capabilities::PLAYOUT);

        let hello = ControlRequest::Hello(HelloPayload {
            proto_version: PROTO_VERSION,
            node: a_node.info("A"),
            nonce: vec![0u8; 16],
            caps: Capabilities::CAPTURE,
        });
        let step = b.on_control(&hello);

        assert_eq!(step.send.len(), 1);
        match &step.send[0].request {
            ControlRequest::HelloAck(payload) => {
                assert!(!payload.accepted, "缺少必需能力必须拒绝，而不是连上再说");
                assert_eq!(payload.agreed_caps, 0, "拒绝时协商结果为 0");
                assert!(
                    payload.reason.contains("Opus 编码"),
                    "拒绝原因要点名缺哪一项：{}",
                    payload.reason
                );
            }
            other => panic!("应当是拒绝性 HELLO_ACK，实际 {other:?}"),
        }
        match &step.event {
            HandshakeEvent::Rejected { error } => {
                assert_eq!(error.code(), ErrorCode::CapUnsupported);
            }
            other => panic!("应当拒绝，实际 {other:?}"),
        }
        assert_eq!(b.phase(), HandshakePhase::Failed);
    }

    #[test]
    fn capabilities_negotiate_to_the_intersection_on_both_sides() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");

        let a_caps = Capabilities::OPUS | Capabilities::CAPTURE | Capabilities::MIXER;
        let b_caps = Capabilities::OPUS | Capabilities::PLAYOUT | Capabilities::GROUP_EPOCH;
        let expected = Capabilities::intersect(a_caps, b_caps);
        assert_eq!(
            expected,
            Capabilities::OPUS,
            "这个例子里唯一的共同项是 Opus"
        );

        let mut a = Handshake::new(Role::Initiator, a_node.info("A"), b_node.identity.id())
            .with_capabilities(a_caps);
        let mut b = Handshake::new(Role::Responder, b_node.info("B"), a_node.identity.id())
            .with_capabilities(b_caps);

        let s1 = a.start();
        let ControlRequest::Hello(hello) = &s1.send[0].request else {
            panic!("第一步应当是 HELLO");
        };
        assert_eq!(hello.caps, a_caps, "HELLO 必须带上本端能力");

        let s2 = b.on_control(&s1.send[0].request);
        let ControlRequest::HelloAck(ack) = &s2.send[0].request else {
            panic!("第二步应当是 HELLO_ACK");
        };
        assert!(ack.accepted);
        assert_eq!(ack.caps, b_caps);
        assert_eq!(ack.agreed_caps, expected);

        // 发起方复核对端报来的交集，两端对「能一起做什么」的理解必须一致。
        let _s3 = a.on_control(&s2.send[0].request);
        assert_eq!(a.agreed_caps(), expected);
        assert_eq!(b.agreed_caps(), expected);
    }

    #[test]
    fn version_mismatch_is_rejected_without_hanging() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");

        let mut b = Handshake::new(Role::Responder, b_node.info("B"), a_node.identity.id());

        let hello = ControlRequest::Hello(HelloPayload {
            proto_version: PROTO_VERSION.wrapping_add(1), // 装成未来的版本
            node: a_node.info("A"),
            nonce: vec![0u8; 16],
            caps: Capabilities::CURRENT,
        });
        let step = b.on_control(&hello);

        // 必须明确回一条拒绝性 ACK，再判失败 —— 不能只是静默关闭，否则对端只看到连接没了。
        assert_eq!(step.send.len(), 1);
        match &step.send[0].request {
            ControlRequest::HelloAck(payload) => {
                assert!(!payload.accepted);
                assert!(payload.reason.contains("version"));
            }
            other => panic!("应当是拒绝性 HELLO_ACK，实际 {other:?}"),
        }
        match &step.event {
            HandshakeEvent::Rejected { error } => {
                assert_eq!(error.code(), ErrorCode::VersionMismatch);
                assert!(error.context().contains("version"));
            }
            other => panic!("应当拒绝，实际 {other:?}"),
        }
        assert_eq!(
            b.phase(),
            HandshakePhase::Failed,
            "拒绝之后阶段必须落到 Failed —— 否则上层会一直等一个不会来的响应"
        );
        assert!(!b.is_established());
    }

    #[test]
    fn out_of_phase_frames_are_ignored_and_counted_never_fatal() {
        // §1.1：握手期间收到无关帧 → 忽略并计数，**不断连**。
        // 这是「对端版本比本端新」时还能接通的关键：多出来的命令不该把连接踢掉。
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");

        let mut b = Handshake::new(Role::Responder, b_node.info("B"), a_node.identity.id());

        let noise = [
            ControlRequest::Ping(PingPayload { t1: 1, t2: 2 }),
            ControlRequest::StreamStats(audiolink_types::StreamStats::default()),
            // 旧协议的 PONG 是「认证 OK」的载体；现在它只是诊断帧，握手期收到当噪声忽略。
            ControlRequest::Pong(PingPayload { t1: 0, t2: 0 }),
            ControlRequest::SetMute(SetMutePayload {
                stream_id: 1,
                mute: false,
            }),
        ];

        for request in &noise {
            let step = b.on_control(request);
            assert!(
                matches!(step.event, HandshakeEvent::Ignored),
                "{request:?} 在未开始握手时应当被忽略"
            );
            assert!(step.send.is_empty(), "{request:?} 不应触发任何发送");
            assert_eq!(b.phase(), HandshakePhase::Idle, "{request:?} 不得改变阶段");
        }

        assert_eq!(b.ignored_frames(), noise.len() as u64, "忽略必须被计数");
        assert!(!b.is_established());
    }

    #[test]
    fn peer_bye_during_handshake_is_a_clean_rejection() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");

        let mut a = Handshake::new(Role::Initiator, a_node.info("A"), b_node.identity.id());
        let _ = a.start();

        let step = a.on_control(&ControlRequest::Bye(ByePayload {
            reason: "user cancelled".to_string(),
        }));

        assert!(matches!(step.event, HandshakeEvent::Rejected { .. }));
        assert_eq!(a.phase(), HandshakePhase::Failed);
    }

    #[test]
    fn peer_error_frame_maps_its_code_into_the_rejection() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");

        let mut a = Handshake::new(Role::Initiator, a_node.info("A"), b_node.identity.id());
        let _ = a.start();

        let step = a.on_control(&ControlRequest::Error(ErrorPayload {
            code: 1009,
            message: "busy".to_string(),
            context: None,
        }));

        match &step.event {
            HandshakeEvent::Rejected { error } => {
                assert_eq!(error.code(), audiolink_types::ErrorCode::Busy);
                assert!(error.context().contains("busy"));
            }
            other => panic!("应当拒绝，实际 {other:?}"),
        }
    }

    #[test]
    fn simultaneous_connect_is_reported_as_busy_not_silently_accepted() {
        // 两端同时发起：两边都以为自己是 A。如果都静默接受 HELLO_ACK 语义会乱，
        // 明确报 Busy 让上层退避重试，比「碰运气谁先到」可控。
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");

        let mut a = Handshake::new(Role::Initiator, a_node.info("A"), b_node.identity.id());
        let _ = a.start();

        let step = a.on_control(&ControlRequest::Hello(HelloPayload {
            proto_version: PROTO_VERSION,
            node: b_node.info("B"),
            nonce: vec![0u8; 16],
            caps: Capabilities::CURRENT,
        }));

        match &step.event {
            HandshakeEvent::Rejected { error } => {
                assert_eq!(error.code(), audiolink_types::ErrorCode::Busy);
            }
            other => panic!("应当报 Busy，实际 {other:?}"),
        }
    }

    #[test]
    fn request_ids_start_at_one_and_never_repeat_zero() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");

        let mut a = Handshake::new(Role::Initiator, a_node.info("A"), b_node.identity.id());
        let step = a.start();
        assert_eq!(step.send[0].request_id, 1, "§4：非 0 值，从 1 开始");
    }

    #[test]
    fn responder_never_starts_on_its_own() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");

        let mut b = Handshake::new(Role::Responder, b_node.info("B"), a_node.identity.id());
        let step = b.start();

        assert!(step.send.is_empty(), "响应方等对端先说话");
        assert_eq!(b.phase(), HandshakePhase::Idle);
    }
}
