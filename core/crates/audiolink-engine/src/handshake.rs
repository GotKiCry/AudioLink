//! §5 握手与配对的**纯状态机**（`docs/03-protocol.md` §5）
//!
//! ```text
//! 发起方 A                                    响应方 B
//!   │ ── HELLO(proto_version, node_info, nonce) ──► │
//!   │ ◄──────────── HELLO_ACK(accepted) ──────────  │
//!   │                                              │
//!   │ 已配对分支                                     │ 未配对分支
//!   │ ◄────────── AUTH_CHALLENGE(nonce) ──────────  │ ── PAIR_REQUIRED ──► │
//!   │ ─── AUTH_RESPONSE(ECDSA 签名) ──────────────► │ ◄──── PAIR_SUBMIT(pin) ─── │
//!   │ ◄────────── PONG（OK，进入会话协商）─────────  │ ── PAIR_RESULT(ok) ─► │
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
//!    任何人都能填任意值；把它当身份就等于取消了身份认证。本模块一律用 `peer_id`
//!    （由 `audiolink-net::Connection::peer_id()` 从对端证书现算）覆盖自报值 —— 见 [`Handshake::on_control`]。
//! 2. **过程序外帧一律忽略并计数，不断流**（§1.1）。握手期间收到 `PING`、未知命令码、载荷非法的帧，
//!    都是「忽略」而不是「拒绝连接」—— 否则对端一个版本差异就能把连接踢掉。
//! 3. **`request_id` 必须校验**。§4 规定响应方要回填请求方的 `request_id`。
//!    不校验的话，一条**上一轮握手的迟到响应**会被当成本轮的合法响应接受 —— 这是可被重放利用的洞。

use std::time::Instant;

use audiolink_identity::{IdentityError, NodeIdentity, PairRejection, PinGate};
use audiolink_types::{AudioLinkError, Capabilities, ErrorCode, NodeId, NodeInfo, PROTO_VERSION};

use crate::dispatch::ControlRequest;
use crate::payload::{
    AuthChallengePayload, AuthResponsePayload, HelloAckPayload, HelloPayload, PairRequiredPayload,
    PairResultPayload, PairSubmitPayload, PingPayload,
};

/// 握手角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// 主动发起连接的一方（§5 的节点 A）。
    Initiator,
    /// 接受连接的一方（§5 的节点 B，负责展示 PIN）。
    Responder,
}

/// 握手阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakePhase {
    /// 未开始。
    Idle,
    /// 发起方已发 `HELLO`，等 `HELLO_ACK`。
    AwaitHelloAck,
    /// 双方已交换 `HELLO`/`HELLO_ACK`，等认证或配对分支的下一帧。
    AwaitPeer,
    /// 响应方已发 `AUTH_CHALLENGE`，等 `AUTH_RESPONSE`。
    AwaitAuthResponse,
    /// 响应方已发 `PAIR_REQUIRED`，等 `PAIR_SUBMIT`。
    AwaitPairSubmit,
    /// 发起方已提交 PIN，等 `PAIR_RESULT`。
    AwaitPairResult,
    /// 发起方已回 `AUTH_RESPONSE`，等响应方的 OK。
    AwaitAuthOk,
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
            Self::AwaitAuthResponse => "await-auth-response",
            Self::AwaitPairSubmit => "await-pair-submit",
            Self::AwaitPairResult => "await-pair-result",
            Self::AwaitAuthOk => "await-auth-ok",
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
    /// 响应方：把 PIN 展示给用户（桌面弹窗 / Android 通知）。
    DisplayPin {
        /// 6 位码。
        pin: String,
        /// 剩余尝试次数。
        remaining_attempts: u8,
    },
    /// 发起方：需要用户输入 PIN。
    NeedPin {
        /// 对端节点信息（UI 里展示「正在与 XX 配对」）。
        peer: NodeInfo,
    },
    /// 发起方：PIN 被拒，可重试（UI 重新提示）。
    PinRejected {
        /// 失败原因（含剩余次数）。
        reason: String,
    },
    /// 握手完成。
    Established {
        /// 对端节点信息（`id` 已用证书指纹覆盖）。
        peer: NodeInfo,
        /// 是否应当把对端写入信任库（PIN 刚配对成功 → `true`；走已配对分支 → `false`）。
        persist: bool,
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
    peer_trusted: bool,
    /// 本机发出的挑战随机数（响应方在验签时需要，发起方在签名时需要）。
    local_nonce: [u8; 32],
    /// 对端发来的挑战随机数。
    peer_nonce: Option<[u8; 32]>,
    pin_gate: Option<PinGate>,
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
}

impl Handshake {
    /// 新建握手。
    ///
    /// `peer_id` 必须来自 **TLS 证书指纹**（`audiolink_net::Connection::peer_id()`），
    /// 不能来自对端自报的 `node_info.id`。
    ///
    /// `peer_trusted` 在**握手开始前**就已确定：指纹在本地信任库里就是已配对，直接走
    /// `AUTH_CHALLENGE/RESPONSE` 分支；不在就走 PIN 配对分支（§5）。
    pub fn new(role: Role, local: NodeInfo, peer_id: NodeId, peer_trusted: bool) -> Self {
        Self {
            role,
            phase: HandshakePhase::Idle,
            local,
            peer_id,
            peer: None,
            peer_trusted,
            local_nonce: random_nonce(),
            peer_nonce: None,
            pin_gate: None,
            next_request_id: 0,
            pending_request_id: 0,
            ignored_frames: 0,
            local_caps: Capabilities::CURRENT,
            agreed_caps: 0,
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

    /// 响应方当前有效的 PIN；配对结束或到期后不再展示。
    pub fn displayed_pin(&self) -> Option<&str> {
        self.display_pin_state()
            .filter(|(_, expires_at)| Instant::now() < *expires_at)
            .map(|(pin, _)| pin)
    }

    /// 供运行时同步发布快照；失效时刻直接取自门禁，错误重试不能续期。
    pub(crate) fn display_pin_state(&self) -> Option<(&str, Instant)> {
        if self.phase != HandshakePhase::AwaitPairSubmit {
            return None;
        }
        let gate = self.pin_gate.as_ref()?;
        Some((gate.pin(), gate.expires_at()))
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

        // 本机发出的 nonce 同时也是「对端在 AUTH_RESPONSE 里要签」的挑战值吗？
        // 不是 —— §5 的 AUTH_CHALLENGE 由**响应方**发起，签名对象是响应方的 nonce。
        // HELLO 里的 nonce 只用于把 HELLO 与本次连接绑定（防重放），因此这里不设 pending：
        // HELLO_ACK 是**响应**而非**请求**，`request_id` 由对端选择。
        self.pending_request_id = 0;

        HandshakeStep::sending(vec![Outgoing::request(request, request_id)])
    }

    /// 处理一条已解码的对端命令。
    ///
    /// `crypto` 是本机身份材料（签名 / 验签）；`peer_cert_der` 由 `audiolink-net` 提供；
    /// `now` 是单调时钟（PIN 过期判定用，**禁止**用 `SystemTime`，见架构 §4）。
    pub fn on_control(
        &mut self,
        request: &ControlRequest,
        crypto: &NodeIdentity,
        peer_cert_der: &[u8],
        now: Instant,
    ) -> HandshakeStep {
        match request {
            ControlRequest::Hello(payload) => self.on_hello(payload, now),

            ControlRequest::HelloAck(payload) => self.on_hello_ack(payload, now),

            ControlRequest::AuthChallenge(payload) => self.on_auth_challenge(payload, crypto),

            ControlRequest::AuthResponse(payload) => {
                self.on_auth_response(payload, crypto, peer_cert_der)
            }

            ControlRequest::PairRequired(_) => {
                if self.role != Role::Initiator || self.phase != HandshakePhase::AwaitPeer {
                    return self.ignore();
                }
                self.phase = HandshakePhase::AwaitPeer;
                HandshakeStep::nothing().with_event(HandshakeEvent::NeedPin {
                    peer: self.peer_info_or_stub(),
                })
            }

            ControlRequest::PairSubmit(payload) => self.on_pair_submit(payload, now),

            ControlRequest::PairResult(payload) => self.on_pair_result(payload),

            // §5 用 `PONG` 作为「已配对分支」的 OK 帧（§4.1 没有专门的 AUTH_OK）。
            // 只在「本机刚发完 AUTH_RESPONSE」这个阶段才把它当 OK —— 否则会把诊断用的
            // PING/PONG 误判成认证完成，用一个状态依赖的判断换掉一个协议漏洞。
            ControlRequest::Pong(_) => {
                if self.role == Role::Initiator && self.phase == HandshakePhase::AwaitAuthOk {
                    self.phase = HandshakePhase::Established;
                    HandshakeStep::nothing().with_event(HandshakeEvent::Established {
                        peer: self.peer_info_or_stub(),
                        persist: false,
                    })
                } else {
                    self.ignore()
                }
            }

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

    /// 发起方收到用户输入的 PIN。
    pub fn on_pin_input(&mut self, pin: &str) -> HandshakeStep {
        if self.role != Role::Initiator || self.phase != HandshakePhase::AwaitPeer {
            return HandshakeStep::nothing();
        }

        let request = ControlRequest::PairSubmit(PairSubmitPayload {
            pin: normalize_pin(pin),
        });
        self.phase = HandshakePhase::AwaitPairResult;
        HandshakeStep::sending(vec![Outgoing::notice(request)])
    }

    // -----------------------------------------------------------------
    // 各分支
    // -----------------------------------------------------------------

    fn on_hello(&mut self, payload: &HelloPayload, now: Instant) -> HandshakeStep {
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

        self.peer = Some(self.trusted_peer_info(&payload.node));

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

        let ack = ControlRequest::HelloAck(HelloAckPayload {
            proto_version: PROTO_VERSION,
            node: self.local.clone(),
            accepted: true,
            reason: String::new(),
            caps: self.local_caps,
            agreed_caps: agreed,
        });

        if self.peer_trusted {
            // 已配对分支：发挑战，等 AUTH_RESPONSE
            let challenge = ControlRequest::AuthChallenge(AuthChallengePayload {
                nonce: self.local_nonce.to_vec(),
            });
            self.phase = HandshakePhase::AwaitAuthResponse;
            let request_id = self.issue_request_id();
            self.pending_request_id = request_id;
            return HandshakeStep::sending(vec![
                Outgoing::notice(ack),
                Outgoing::request(challenge, request_id),
            ]);
        }

        // 未配对分支：建立 PIN 门禁并展示（§5：60 s 有效、最多 5 次）
        let gate = PinGate::new(now);
        let pin = gate.pin().to_string();
        let remaining = gate.remaining_attempts();
        self.pin_gate = Some(gate);
        self.phase = HandshakePhase::AwaitPairSubmit;

        let required = ControlRequest::PairRequired(PairRequiredPayload { pin_display: true });
        HandshakeStep::sending(vec![Outgoing::notice(ack), Outgoing::notice(required)]).with_event(
            HandshakeEvent::DisplayPin {
                pin,
                remaining_attempts: remaining,
            },
        )
    }

    fn on_hello_ack(&mut self, payload: &HelloAckPayload, _now: Instant) -> HandshakeStep {
        if self.role != Role::Initiator || self.phase != HandshakePhase::AwaitHelloAck {
            return self.ignore();
        }

        self.peer = Some(self.trusted_peer_info(&payload.node));

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

        // 接下来等对端开分支：已配对 → AUTH_CHALLENGE；未配对 → PAIR_REQUIRED。
        // 两者都由对端选择，本端不预设 —— 预设就会在「本地以为已配对、对端其实没记录」
        // 这种信任库不一致的情况下死等。
        self.phase = HandshakePhase::AwaitPeer;
        HandshakeStep::nothing()
    }

    fn on_auth_challenge(
        &mut self,
        payload: &AuthChallengePayload,
        crypto: &NodeIdentity,
    ) -> HandshakeStep {
        // AUTH_CHALLENGE 只对发起方有意义（响应方发起挑战，见 §5 时序）。
        if self.role != Role::Initiator || self.phase != HandshakePhase::AwaitPeer {
            return self.ignore();
        }

        let Ok(nonce) = nonce_from_slice(&payload.nonce) else {
            return self.ignore();
        };
        self.peer_nonce = Some(nonce);

        let signature = match crypto.sign_challenge(&nonce, self.peer_id) {
            Ok(signature) => signature,
            Err(error) => return self.fail(identity_to_link_error(&error)),
        };

        let response = ControlRequest::AuthResponse(AuthResponsePayload { signature });
        self.phase = HandshakePhase::AwaitAuthOk;
        HandshakeStep::sending(vec![Outgoing::notice(response)])
    }

    fn on_auth_response(
        &mut self,
        payload: &AuthResponsePayload,
        _crypto: &NodeIdentity,
        peer_cert_der: &[u8],
    ) -> HandshakeStep {
        if self.role != Role::Responder || self.phase != HandshakePhase::AwaitAuthResponse {
            return self.ignore();
        }

        // 空签名是「对端压根没实现这一步」的典型特征，单独判掉能省一次无意义的验签。
        if payload.signature.is_empty() {
            return self.fail(AudioLinkError::auth_failed("empty auth response signature"));
        }

        // §5「认证强度」：用对端证书里的公钥验证「私钥持有者 = 证书主体」。
        match NodeIdentity::verify_challenge(
            peer_cert_der,
            &self.local_nonce,
            self.local.id,
            self.peer_id,
            &payload.signature,
        ) {
            Ok(()) => {
                self.phase = HandshakePhase::Established;
                // §5 的 OK 帧：用 PONG 回填（§4.1 没有专门的 AUTH_OK）。
                let ok = ControlRequest::Pong(PingPayload { t1: 0, t2: 0 });
                HandshakeStep::sending(vec![Outgoing::notice(ok)]).with_event(
                    HandshakeEvent::Established {
                        peer: self.peer_info_or_stub(),
                        persist: false,
                    },
                )
            }
            Err(error) => self.fail(identity_to_link_error(&error)),
        }
    }

    fn on_pair_submit(&mut self, payload: &PairSubmitPayload, now: Instant) -> HandshakeStep {
        if self.role != Role::Responder || self.phase != HandshakePhase::AwaitPairSubmit {
            return self.ignore();
        }

        let Some(gate) = self.pin_gate.as_mut() else {
            // 状态说在等 PIN，但没有门禁对象 —— 内部不一致，明确失败而不是假装通过。
            return self.fail(AudioLinkError::bad_request(
                "pair submit without an active pin gate",
            ));
        };

        match gate.verify(&payload.pin, now) {
            Ok(()) => {
                self.phase = HandshakePhase::Established;
                let result = ControlRequest::PairResult(PairResultPayload {
                    ok: true,
                    reason: String::new(),
                    // §5：配对成功后**双方**写入信任库 → 让上层落盘。
                    persist: true,
                });
                HandshakeStep::sending(vec![Outgoing::notice(result)]).with_event(
                    HandshakeEvent::Established {
                        peer: self.peer_info_or_stub(),
                        persist: true,
                    },
                )
            }
            Err(rejection) => {
                let reason = describe_rejection(&rejection);
                let result = ControlRequest::PairResult(PairResultPayload {
                    ok: false,
                    reason: reason.clone(),
                    persist: false,
                });

                // 锁定 / 过期 → 本轮配对彻底结束；仅「PIN 错」还留在等提交阶段让用户重试。
                let step = HandshakeStep::sending(vec![Outgoing::notice(result)]);
                match rejection {
                    PairRejection::WrongPin { remaining } => {
                        step.with_event(HandshakeEvent::DisplayPin {
                            pin: gate.pin().to_string(),
                            remaining_attempts: remaining,
                        })
                    }
                    PairRejection::Expired | PairRejection::Locked { .. } => {
                        self.phase = HandshakePhase::Failed;
                        step.with_event(HandshakeEvent::Rejected {
                            error: AudioLinkError::bad_request_owned(reason),
                        })
                    }
                }
            }
        }
    }

    fn on_pair_result(&mut self, payload: &PairResultPayload) -> HandshakeStep {
        if self.role != Role::Initiator || self.phase != HandshakePhase::AwaitPairResult {
            return self.ignore();
        }

        if payload.ok {
            self.phase = HandshakePhase::Established;
            HandshakeStep::nothing().with_event(HandshakeEvent::Established {
                peer: self.peer_info_or_stub(),
                persist: payload.persist,
            })
        } else {
            // 回到「等用户重新输入」：PIN 错不该把连接踢掉，用户还有剩余次数。
            self.phase = HandshakePhase::AwaitPeer;
            HandshakeStep::nothing().with_event(HandshakeEvent::PinRejected {
                reason: payload.reason.clone(),
            })
        }
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
    fn trusted_peer_info(&self, reported: &NodeInfo) -> NodeInfo {
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

/// 生成 32 B 随机挑战值（§5：`nonce(32 B)`）。
fn random_nonce() -> [u8; 32] {
    let mut nonce = [0u8; 32];
    rand::Rng::fill(&mut rand::rng(), &mut nonce);
    nonce
}

/// 把载荷里的 `Vec<u8>` nonce 转成定长数组；长度不对 → `None`（当作非法帧忽略）。
fn nonce_from_slice(bytes: &[u8]) -> Result<[u8; 32], ()> {
    <[u8; 32]>::try_from(bytes).map_err(|_| ())
}

/// 归一化用户输入的 PIN：去掉空格与连字符，只留数字。
///
/// 用户在手机号键盘上输入 6 位码时，很容易带上分隔符（`123 456`）。
/// 在**协议边界之前**归一化，比让用户在「PIN 明明是 123456 却报错」里困惑要好。
fn normalize_pin(input: &str) -> String {
    input.chars().filter(char::is_ascii_digit).take(6).collect()
}

/// 把 `PairRejection` 变成给用户看的一句话（同时是 `PairResult.reason` 的内容）。
fn describe_rejection(rejection: &PairRejection) -> String {
    match rejection {
        PairRejection::WrongPin { remaining } => {
            format!("PIN 错误，剩余 {remaining} 次尝试")
        }
        PairRejection::Expired => "PIN 已过期（超过 60 s），请重新发起配对".to_string(),
        PairRejection::Locked { retry_after } => {
            format!("尝试次数过多，已锁定 {} 秒", retry_after.as_secs().max(1))
        }
    }
}

/// `IdentityError` → `AudioLinkError`（在 engine 边界收敛，见契约 §2.3）。
fn identity_to_link_error(error: &IdentityError) -> AudioLinkError {
    AudioLinkError::owned(error.code(), error.context().to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::payload::{ByePayload, ErrorPayload, SetMutePayload};
    use audiolink_types::{Caps, OpCode, Platform};
    use tempfile::TempDir;

    /// 一个测试用节点：真证书、真 ECDSA 密钥（落在临时目录里，测试结束即删）。
    ///
    /// 刻意**不用假签名**：握手模块的核心价值就是身份认证，用 `Ok(vec![0;1])` 这种假实现测，
    /// 等于把最该验证的部分（「篡改的签名必须被拒」）测成了空。
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

        fn cert(&self) -> &[u8] {
            self.identity.cert_der()
        }
    }

    /// 把 `step` 里的帧按顺序投递给 `target`，返回**最后一次**产生的 step。
    ///
    /// 「最后一次」是刻意的：一个 step 里可能同时带 `HELLO_ACK` 与 `AUTH_CHALLENGE`，
    /// 前者只推进阶段，后者的产物才是调用方要看的。
    fn pump(
        target: &mut Handshake,
        step: &HandshakeStep,
        crypto: &NodeIdentity,
        peer_cert: &[u8],
        now: Instant,
    ) -> HandshakeStep {
        let mut last = HandshakeStep::nothing();
        for outgoing in &step.send {
            last = target.on_control(&outgoing.request, crypto, peer_cert, now);
        }
        last
    }

    /// 造一个必定与 `correct` 不同的 6 位 PIN。
    fn wrong_pin(correct: &str) -> String {
        correct
            .chars()
            .map(|c| if c == '0' { '1' } else { '0' })
            .collect()
    }

    fn established_of(step: &HandshakeStep) -> Option<(&NodeInfo, bool)> {
        match &step.event {
            HandshakeEvent::Established { peer, persist } => Some((peer, *persist)),
            _ => None,
        }
    }

    #[test]
    fn already_paired_flow_establishes_on_both_sides_without_persisting() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");
        let now = Instant::now();

        let mut a = Handshake::new(
            Role::Initiator,
            a_node.info("A"),
            b_node.identity.id(),
            true,
        );
        let mut b = Handshake::new(
            Role::Responder,
            b_node.info("B"),
            a_node.identity.id(),
            true,
        );

        // A 发起：HELLO 是**请求**，request_id 必须非 0（§4：0 表示无需响应的单向通知）。
        let s1 = a.start();
        assert_eq!(s1.send.len(), 1);
        assert_eq!(s1.send[0].request.op(), OpCode::Hello);
        assert!(s1.send[0].request_id >= 1, "HELLO 必须带非 0 request_id");

        // B 收 HELLO → 回 HELLO_ACK + AUTH_CHALLENGE（已配对分支）
        let s2 = pump(&mut b, &s1, &b_node.identity, a_node.cert(), now);
        assert_eq!(s2.send.len(), 2, "应当先 ACK 再挑战");
        assert_eq!(s2.send[0].request.op(), OpCode::HelloAck);
        assert_eq!(s2.send[1].request.op(), OpCode::AuthChallenge);
        assert_eq!(
            s2.send[0].request_id, 0,
            "ACK 是响应，不带自己的 request_id"
        );
        assert!(s2.send[1].request_id >= 1, "AUTH_CHALLENGE 是请求");
        assert_eq!(b.phase(), HandshakePhase::AwaitAuthResponse);

        // A 收 HELLO_ACK + AUTH_CHALLENGE → 回签名
        let s3 = pump(&mut a, &s2, &a_node.identity, b_node.cert(), now);
        assert_eq!(s3.send.len(), 1);
        assert_eq!(s3.send[0].request.op(), OpCode::AuthResponse);
        match &s3.send[0].request {
            ControlRequest::AuthResponse(payload) => {
                assert!(!payload.signature.is_empty(), "签名不得为空");
            }
            other => panic!("应当是 AUTH_RESPONSE，实际 {other:?}"),
        }
        assert_eq!(a.phase(), HandshakePhase::AwaitAuthOk);

        // B 验签 → 发 OK（PONG）并完成
        let s4 = pump(&mut b, &s3, &b_node.identity, a_node.cert(), now);
        assert_eq!(s4.send.len(), 1);
        assert_eq!(s4.send[0].request.op(), OpCode::Pong, "§5：B→A 的 OK");
        let (peer_seen_by_b, persist_b) = established_of(&s4).expect("B 应当完成握手");
        assert!(!persist_b, "已配对分支不应重复写信任库");
        assert_eq!(peer_seen_by_b.id, a_node.identity.id());
        assert!(b.is_established());

        // A 收 OK → 完成
        let s5 = pump(&mut a, &s4, &a_node.identity, b_node.cert(), now);
        let (peer_seen_by_a, persist_a) = established_of(&s5).expect("A 应当完成握手");
        assert!(!persist_a);
        assert_eq!(peer_seen_by_a.id, b_node.identity.id(), "对端指纹来自证书");
        assert!(a.is_established());
    }

    #[test]
    fn pairing_flow_retries_on_wrong_pin_then_establishes_and_persists() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");
        let now = Instant::now();

        let mut a = Handshake::new(
            Role::Initiator,
            a_node.info("A"),
            b_node.identity.id(),
            false,
        );
        let mut b = Handshake::new(
            Role::Responder,
            b_node.info("B"),
            a_node.identity.id(),
            false,
        );

        let s1 = a.start();
        let s2 = pump(&mut b, &s1, &b_node.identity, a_node.cert(), now);

        // B 必须把 PIN 交给 UI 展示（用户要能念给对方）。
        let pin = match &s2.event {
            HandshakeEvent::DisplayPin {
                pin,
                remaining_attempts,
            } => {
                assert_eq!(pin.len(), 6, "PIN 是 6 位数字");
                assert!(pin.chars().all(|c| c.is_ascii_digit()));
                assert_eq!(*remaining_attempts, 5, "初始剩余 5 次（§5）");
                pin.clone()
            }
            other => panic!("应当展示 PIN，实际 {other:?}"),
        };
        assert_eq!(s2.send[1].request.op(), OpCode::PairRequired);
        assert_eq!(b.displayed_pin(), Some(pin.as_str()));

        // A 收 HELLO_ACK + PAIR_REQUIRED → 要求用户输入
        let s3 = pump(&mut a, &s2, &a_node.identity, b_node.cert(), now);
        assert!(matches!(s3.event, HandshakeEvent::NeedPin { .. }));
        assert_eq!(a.phase(), HandshakePhase::AwaitPeer);

        // ---- 第一轮：错误 PIN ----
        let bad = wrong_pin(&pin);
        let s4 = a.on_pin_input(&bad);
        assert_eq!(s4.send[0].request.op(), OpCode::PairSubmit);

        let s5 = pump(&mut b, &s4, &b_node.identity, a_node.cert(), now);
        match &s5.send[0].request {
            ControlRequest::PairResult(payload) => {
                assert!(!payload.ok, "错误 PIN 必须被拒");
                assert!(!payload.persist, "失败不得写信任库");
                assert!(
                    payload.reason.contains("剩余 4 次"),
                    "原因要带剩余次数：{}",
                    payload.reason
                );
            }
            other => panic!("应当是 PAIR_RESULT，实际 {other:?}"),
        }
        assert_eq!(
            b.phase(),
            HandshakePhase::AwaitPairSubmit,
            "PIN 错不是终态，用户还有次数可以重试"
        );
        assert!(!b.is_established());

        let s6 = pump(&mut a, &s5, &a_node.identity, b_node.cert(), now);
        assert!(
            matches!(s6.event, HandshakeEvent::PinRejected { .. }),
            "A 侧要收到「可重试」而不是「连接失败」"
        );
        assert_eq!(
            a.phase(),
            HandshakePhase::AwaitPeer,
            "A 必须回到可再次输入的状态"
        );

        // ---- 第二轮：正确 PIN ----
        let s7 = a.on_pin_input(&pin);
        let s8 = pump(&mut b, &s7, &b_node.identity, a_node.cert(), now);
        match &s8.send[0].request {
            ControlRequest::PairResult(payload) => {
                assert!(payload.ok);
                assert!(payload.persist, "§5：配对成功后双方写入信任库");
            }
            other => panic!("应当是 PAIR_RESULT，实际 {other:?}"),
        }
        let (peer_seen_by_b, persist_b) = established_of(&s8).expect("B 应当完成配对");
        assert!(persist_b);
        assert_eq!(peer_seen_by_b.id, a_node.identity.id());
        assert!(b.is_established());

        let s9 = pump(&mut a, &s8, &a_node.identity, b_node.cert(), now);
        let (_, persist_a) = established_of(&s9).expect("A 应当完成配对");
        assert!(persist_a, "发起方也要落盘");
        assert!(a.is_established());
    }

    #[test]
    fn capability_mismatch_is_rejected_with_a_readable_reason() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");
        let now = Instant::now();

        // 响应方只有 Opus + 播放；发起方只声明「采集」（没有 Opus）→ 交集缺必需位。
        let mut b = Handshake::new(
            Role::Responder,
            b_node.info("B"),
            a_node.identity.id(),
            false,
        )
        .with_capabilities(Capabilities::OPUS | Capabilities::PLAYOUT);

        let hello = ControlRequest::Hello(HelloPayload {
            proto_version: PROTO_VERSION,
            node: a_node.info("A"),
            nonce: vec![0u8; 16],
            caps: Capabilities::CAPTURE,
        });
        let step = b.on_control(&hello, &b_node.identity, a_node.cert(), now);

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
        let now = Instant::now();

        let a_caps = Capabilities::OPUS | Capabilities::CAPTURE | Capabilities::MIXER;
        let b_caps = Capabilities::OPUS | Capabilities::PLAYOUT | Capabilities::GROUP_EPOCH;
        let expected = Capabilities::intersect(a_caps, b_caps);
        assert_eq!(
            expected,
            Capabilities::OPUS,
            "这个例子里唯一的共同项是 Opus"
        );

        let mut a = Handshake::new(
            Role::Initiator,
            a_node.info("A"),
            b_node.identity.id(),
            false,
        )
        .with_capabilities(a_caps);
        let mut b = Handshake::new(
            Role::Responder,
            b_node.info("B"),
            a_node.identity.id(),
            false,
        )
        .with_capabilities(b_caps);

        let s1 = a.start();
        let ControlRequest::Hello(hello) = &s1.send[0].request else {
            panic!("第一步应当是 HELLO");
        };
        assert_eq!(hello.caps, a_caps, "HELLO 必须带上本端能力");

        let s2 = b.on_control(&s1.send[0].request, &b_node.identity, a_node.cert(), now);
        let ControlRequest::HelloAck(ack) = &s2.send[0].request else {
            panic!("第二步应当是 HELLO_ACK");
        };
        assert!(ack.accepted);
        assert_eq!(ack.caps, b_caps);
        assert_eq!(ack.agreed_caps, expected);

        // 发起方复核对端报来的交集，两端对「能一起做什么」的理解必须一致。
        let _s3 = a.on_control(&s2.send[0].request, &a_node.identity, b_node.cert(), now);
        assert_eq!(a.agreed_caps(), expected);
        assert_eq!(b.agreed_caps(), expected);
    }

    #[test]
    fn version_mismatch_is_rejected_without_hanging() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");
        let now = Instant::now();

        let mut b = Handshake::new(
            Role::Responder,
            b_node.info("B"),
            a_node.identity.id(),
            false,
        );

        let hello = ControlRequest::Hello(HelloPayload {
            proto_version: PROTO_VERSION.wrapping_add(1), // 装成未来的版本
            node: a_node.info("A"),
            nonce: vec![0u8; 16],
            caps: Capabilities::CURRENT,
        });
        let step = b.on_control(&hello, &b_node.identity, a_node.cert(), now);

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
    fn self_reported_node_id_is_overridden_by_the_certificate_fingerprint() {
        // 安全核心：HELLO 里的 node_info.id 是**对端自报**的，任何人都能填成别人的指纹。
        // 如果信了它，攻击者只要谎报自己是「已信任的那台设备」就能绕过整条信任链。
        //
        // 真实数据流：`peer_id` 来自 `audiolink_net::Connection::peer_id()`（对端证书的 SHA-256），
        // 在 QUIC 握手完成时就有了 —— 而 `HELLO` 是之后才到的。所以这里给 b 传**攻击者证书**的指纹。
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");
        let attacker = Node::new("attacker");
        let now = Instant::now();

        let mut b = Handshake::new(
            Role::Responder,
            b_node.info("B"),
            attacker.identity.id(),
            false,
        );

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
        let step = b.on_control(&hello, &b_node.identity, attacker.cert(), now);

        assert!(matches!(step.event, HandshakeEvent::DisplayPin { .. }));
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
    fn tampered_signature_fails_authentication() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");
        let now = Instant::now();

        let mut b = Handshake::new(
            Role::Responder,
            b_node.info("B"),
            a_node.identity.id(),
            true,
        );

        // 走一遍完整流程，但在最后一步把签名翻掉一位
        let mut a = Handshake::new(
            Role::Initiator,
            a_node.info("A"),
            b_node.identity.id(),
            true,
        );
        let s1 = a.start();
        let s2 = pump(&mut b, &s1, &b_node.identity, a_node.cert(), now);
        let s3 = pump(&mut a, &s2, &a_node.identity, b_node.cert(), now);

        let mut tampered = s3;
        match &mut tampered.send[0].request {
            ControlRequest::AuthResponse(payload) => {
                // 翻掉最后一个字节的一位
                if let Some(last) = payload.signature.last_mut() {
                    *last ^= 0x01;
                }
            }
            other => panic!("应当是 AUTH_RESPONSE，实际 {other:?}"),
        }

        let s4 = pump(&mut b, &tampered, &b_node.identity, a_node.cert(), now);
        match &s4.event {
            HandshakeEvent::Rejected { error } => {
                assert_eq!(error.code(), audiolink_types::ErrorCode::AuthFailed);
            }
            other => panic!("篡改签名必须认证失败，实际 {other:?}"),
        }
        assert_eq!(b.phase(), HandshakePhase::Failed);
        assert!(!b.is_established());
    }

    #[test]
    fn out_of_phase_frames_are_ignored_and_counted_never_fatal() {
        // §1.1：握手期间收到无关帧 → 忽略并计数，**不断连**。
        // 这是「对端版本比本端新」时还能接通的关键：多出来的命令不该把连接踢掉。
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");
        let now = Instant::now();

        let mut b = Handshake::new(
            Role::Responder,
            b_node.info("B"),
            a_node.identity.id(),
            true,
        );

        let noise = [
            ControlRequest::Ping(PingPayload { t1: 1, t2: 2 }),
            ControlRequest::StreamStats(audiolink_types::StreamStats::default()),
            // 已配对分支下收到 PAIR_SUBMIT：属于「对端状态不一致」，不是攻击，忽略即可
            ControlRequest::PairSubmit(PairSubmitPayload {
                pin: "000000".to_string(),
            }),
            ControlRequest::SetMute(SetMutePayload {
                stream_id: 1,
                mute: false,
            }),
        ];

        for request in &noise {
            let step = b.on_control(request, &b_node.identity, a_node.cert(), now);
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
        let now = Instant::now();

        let mut a = Handshake::new(
            Role::Initiator,
            a_node.info("A"),
            b_node.identity.id(),
            true,
        );
        let _ = a.start();

        let step = a.on_control(
            &ControlRequest::Bye(ByePayload {
                reason: "user cancelled".to_string(),
            }),
            &a_node.identity,
            b_node.cert(),
            now,
        );

        assert!(matches!(step.event, HandshakeEvent::Rejected { .. }));
        assert_eq!(a.phase(), HandshakePhase::Failed);
    }

    #[test]
    fn peer_error_frame_maps_its_code_into_the_rejection() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");
        let now = Instant::now();

        let mut a = Handshake::new(
            Role::Initiator,
            a_node.info("A"),
            b_node.identity.id(),
            true,
        );
        let _ = a.start();

        let step = a.on_control(
            &ControlRequest::Error(ErrorPayload {
                code: 1009,
                message: "busy".to_string(),
                context: None,
            }),
            &a_node.identity,
            b_node.cert(),
            now,
        );

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
        let now = Instant::now();

        let mut a = Handshake::new(
            Role::Initiator,
            a_node.info("A"),
            b_node.identity.id(),
            true,
        );
        let _ = a.start();

        let step = a.on_control(
            &ControlRequest::Hello(HelloPayload {
                proto_version: PROTO_VERSION,
                node: b_node.info("B"),
                nonce: vec![0u8; 16],
                caps: Capabilities::CURRENT,
            }),
            &a_node.identity,
            b_node.cert(),
            now,
        );

        match &step.event {
            HandshakeEvent::Rejected { error } => {
                assert_eq!(error.code(), audiolink_types::ErrorCode::Busy);
            }
            other => panic!("应当报 Busy，实际 {other:?}"),
        }
    }

    #[test]
    fn pin_input_tolerates_user_formatting() {
        // 手机号键盘上输入 6 位码很容易带上分隔符；归一化放在协议边界之前，
        // 免得用户在「PIN 明明是 123456 却报错」里困惑。
        assert_eq!(normalize_pin("123 456"), "123456");
        assert_eq!(normalize_pin("123-456"), "123456");
        assert_eq!(normalize_pin(" 123456 "), "123456");
        assert_eq!(normalize_pin("1234567890"), "123456", "多输的数字要截断");
        assert_eq!(normalize_pin("abc"), "", "没有数字就是空");
    }

    #[test]
    fn request_ids_start_at_one_and_never_repeat_zero() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");

        let mut a = Handshake::new(
            Role::Initiator,
            a_node.info("A"),
            b_node.identity.id(),
            true,
        );
        let step = a.start();
        assert_eq!(step.send[0].request_id, 1, "§4：非 0 值，从 1 开始");
    }

    #[test]
    fn responder_never_starts_on_its_own() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");

        let mut b = Handshake::new(
            Role::Responder,
            b_node.info("B"),
            a_node.identity.id(),
            false,
        );
        let step = b.start();

        assert!(step.send.is_empty(), "响应方等对端先说话");
        assert_eq!(b.phase(), HandshakePhase::Idle);
    }

    #[test]
    fn pairing_expiry_ends_the_round_instead_of_looping_forever() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");
        let now = Instant::now();

        let mut b = Handshake::new(
            Role::Responder,
            b_node.info("B"),
            a_node.identity.id(),
            false,
        );
        let hello = ControlRequest::Hello(HelloPayload {
            proto_version: PROTO_VERSION,
            node: a_node.info("A"),
            nonce: vec![0u8; 16],
            caps: Capabilities::CURRENT,
        });
        let _ = b.on_control(&hello, &b_node.identity, a_node.cert(), now);

        // 把时钟推到 PIN 有效期（60 s）之后
        let later = now + std::time::Duration::from_secs(61);
        let step = b.on_control(
            &ControlRequest::PairSubmit(PairSubmitPayload {
                pin: "000000".to_string(),
            }),
            &b_node.identity,
            a_node.cert(),
            later,
        );

        assert!(matches!(step.event, HandshakeEvent::Rejected { .. }));
        assert_eq!(b.phase(), HandshakePhase::Failed);
    }

    #[test]
    fn established_peer_info_carries_the_certificate_identity() {
        let a_node = Node::new("node-a");
        let b_node = Node::new("node-b");
        let now = Instant::now();

        let mut a = Handshake::new(
            Role::Initiator,
            a_node.info("A"),
            b_node.identity.id(),
            true,
        );
        let mut b = Handshake::new(
            Role::Responder,
            b_node.info("B"),
            a_node.identity.id(),
            true,
        );

        let s1 = a.start();
        let s2 = pump(&mut b, &s1, &b_node.identity, a_node.cert(), now);
        let s3 = pump(&mut a, &s2, &a_node.identity, b_node.cert(), now);
        let s4 = pump(&mut b, &s3, &b_node.identity, a_node.cert(), now);
        let s5 = pump(&mut a, &s4, &a_node.identity, b_node.cert(), now);

        let (peer, _) = established_of(&s5).unwrap();
        assert_eq!(peer.id, b_node.identity.id());
        assert_eq!(peer.name, "B");
        assert_eq!(peer.platform, Platform::Windows);
        assert!(peer.caps.contains(Caps::CAN_SEND));
    }
}
