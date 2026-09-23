//! L2 分发层：把控制帧的**不透明载荷**变成有类型的命令，并落实 §1.1 的处置纪律。
//!
//! `docs/03-protocol.md` §1.1 把非法帧的处置明确分成两层：
//!
//! | 层 | 职责 |
//! |---|---|
//! | **L1**（`audiolink-proto`） | 严格解码，任何非法字节流 → `1008 BAD_REQUEST`，**绝不 panic** |
//! | **L2**（本模块） | 「未知类型忽略并计数、不断流」 |
//!
//! 也就是说：L1 只管「这堆字节是不是合法帧」，L2 才管「这条合法帧我认不认识、要不要理它」。
//! 两层分工混淆会立刻产生两种事故：
//!
//! - 把 L1 的 `BadRequest` 直接升级成断连 → 对方发一条本版本不认识的命令，本端就掉线（**过度反应**）；
//! - 把 L2 的「忽略」下沉到 L1 → 连帧长都不校验就跳过，流边界错位（**静默数据损坏**）。
//!
//! 本模块严格站在 L2：**能重新同步就忽略并计数，不能重新同步才报错**。

use audiolink_types::{AudioLinkError, OpCode, StreamStats};

use crate::payload::{
    ByePayload, ClockResultPayload, CloseStreamPayload, ErrorPayload, GroupCreatePayload,
    GroupEpochPayload, GroupJoinPayload, GroupLeavePayload, HelloAckPayload, HelloPayload,
    OpenStreamAckPayload, OpenStreamPayload, PingPayload, SetGainPayload, SetMutePayload,
    decode_payload, encode_payload,
};

/// 分发结论 —— 调用方据此决定「继续处理」还是「只是记账」。
///
/// 四个分支刻意分开统计：它们的**诊断含义完全不同**，混在一起就查不出问题。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// 已识别且载荷合法，`ControlRequest` 已产出。
    Handled,
    /// **命令码不认识**（`op == None`）：对端版本比本端新，或帧被损坏。
    /// 忽略并计数，**连接继续**（§1.1）。
    IgnoredUnknownOp,
    /// **命令码认识但本里程碑没有语义**（如 M2 的 `GROUP_*`）：
    /// 对端功能比本端全，属正常的版本差，忽略并计数，**连接继续**。
    IgnoredUnimplemented,
    /// **载荷非法**（postcard 解不出 / 有尾随字节）：§1.1 的「记日志并忽略该帧」。
    /// 注意与「信封非法」区分 —— 信封坏了连帧长都不可信，那时 `recv` 返回 `Err`，轮不到本函数。
    IgnoredBadPayload,
}

impl DispatchOutcome {
    /// 是否被本层忽略（三种 Ignored* 之一）。
    pub const fn is_ignored(self) -> bool {
        !matches!(self, Self::Handled)
    }

    /// 快照名（日志 / 测试断言用）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Handled => "handled",
            Self::IgnoredUnknownOp => "ignored-unknown-op",
            Self::IgnoredUnimplemented => "ignored-unimplemented",
            Self::IgnoredBadPayload => "ignored-bad-payload",
        }
    }
}

/// L2 分发统计（§1.1 的行为契约落成可观测数字）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DispatchStats {
    /// 收到的控制帧总数（**含**被忽略的那些）。
    pub received: u64,
    /// 未知命令码被忽略的次数（§1.1：忽略并计数，不断流）。
    pub ignored_unknown_op: u64,
    /// 本里程碑未实现语义的命令码被忽略的次数。
    pub ignored_unimplemented: u64,
    /// 载荷 postcard 解码失败被忽略的次数（§1.1：同上）。
    pub ignored_bad_payload: u64,
    /// 成功解码并交给调用方的次数。
    pub handled: u64,
}

impl DispatchStats {
    /// 被忽略的总次数。
    pub const fn ignored(&self) -> u64 {
        self.ignored_unknown_op + self.ignored_unimplemented + self.ignored_bad_payload
    }

    /// 忽略率（百分比 ×100，避免浮点；无收帧时为 0）。
    ///
    /// 「忽略率长期偏高」说明两端版本漂移严重，是升级提示的依据，所以值得进遥测。
    pub fn ignored_pct_x100(&self) -> u16 {
        if self.received == 0 {
            return 0;
        }
        let scaled = self.ignored().saturating_mul(10_000) / self.received;
        u16::try_from(scaled.min(10_000)).unwrap_or(10_000)
    }
}

/// 一条已解码的控制命令（M1 需要的子集，`docs/03-protocol.md` §4.1）。
///
/// 未列入的 `TELEMETRY_PUSH` / `SET_VOLUME_LOCK` 仍走
/// [`DispatchOutcome::IgnoredUnimplemented`] 路径 —— 这正好是 §13「未知值可忽略」的实践。
#[derive(Debug, Clone, PartialEq)]
pub enum ControlRequest {
    /// `0x01` 接收端 → 主机：连接请求。
    Hello(HelloPayload),
    /// `0x02` 主机 → 接收端：连接应答。
    HelloAck(HelloAckPayload),
    /// `0x10` 主机 → 接收端：开流协商。
    OpenStream(OpenStreamPayload),
    /// `0x11` 接收端 → 主机：开流应答。
    OpenStreamAck(OpenStreamAckPayload),
    /// `0x12` 双方：关流。
    CloseStream(CloseStreamPayload),
    /// `0x13` 双方：1 Hz 遥测。
    StreamStats(StreamStats),
    /// `0x20` 双方：设置增益。
    SetGain(SetGainPayload),
    /// `0x21` 双方：设置静音。
    SetMute(SetMutePayload),
    /// `0x30` 主机 → 接收端：时钟同步结果（对端诊断）。
    ClockResult(ClockResultPayload),
    /// `0x40` 主机 → 接收端：创建临时同步组（§7）。
    GroupCreate(GroupCreatePayload),
    /// `0x41` 主机 → 接收端：成员动态加入（§7）。
    GroupJoin(GroupJoinPayload),
    /// `0x42` 主机 → 接收端：成员退出（§7）。
    GroupLeave(GroupLeavePayload),
    /// `0x43` 主机 → 接收端：同步组公共时间基准（§7 预约播放）。
    GroupEpoch(GroupEpochPayload),
    /// `0x44` 接收端 → 主机：接收端作为**基准来源**广播共同时间基准（M4 多源混音对齐）。
    ReceiverEpoch(GroupEpochPayload),
    /// `0x60` 双方：可靠流 ping。
    Ping(PingPayload),
    /// `0x61` 双方：可靠流 pong。
    Pong(PingPayload),
    /// `0x70` 双方：错误通知。
    Error(ErrorPayload),
    /// `0x80` 双方：断开。
    Bye(ByePayload),
}

impl ControlRequest {
    /// 对应的命令码。
    pub const fn op(&self) -> OpCode {
        match self {
            Self::Hello(_) => OpCode::Hello,
            Self::HelloAck(_) => OpCode::HelloAck,
            Self::OpenStream(_) => OpCode::OpenStream,
            Self::OpenStreamAck(_) => OpCode::OpenStreamAck,
            Self::CloseStream(_) => OpCode::CloseStream,
            Self::StreamStats(_) => OpCode::StreamStats,
            Self::SetGain(_) => OpCode::SetGain,
            Self::SetMute(_) => OpCode::SetMute,
            Self::ClockResult(_) => OpCode::ClockResult,
            Self::GroupCreate(_) => OpCode::GroupCreate,
            Self::GroupJoin(_) => OpCode::GroupJoin,
            Self::GroupLeave(_) => OpCode::GroupLeave,
            Self::GroupEpoch(_) => OpCode::GroupEpoch,
            Self::ReceiverEpoch(_) => OpCode::ReceiverEpoch,
            Self::Ping(_) => OpCode::Ping,
            Self::Pong(_) => OpCode::Pong,
            Self::Error(_) => OpCode::Error,
            Self::Bye(_) => OpCode::Bye,
        }
    }

    /// 把本命令的命令体序列化为 postcard 字节（§4）。
    ///
    /// 与 [`decode_control`] 严格互逆 —— 有一个「命令 → 字节」的唯一出口，
    /// 才能保证「编码端」与「解码端」不会各自漂移。
    pub fn encode_body(&self) -> Result<Vec<u8>, AudioLinkError> {
        match self {
            Self::Hello(v) => encode_payload(v),
            Self::HelloAck(v) => encode_payload(v),
            Self::OpenStream(v) => encode_payload(v),
            Self::OpenStreamAck(v) => encode_payload(v),
            Self::CloseStream(v) => encode_payload(v),
            Self::StreamStats(v) => encode_payload(v),
            Self::SetGain(v) => encode_payload(v),
            Self::SetMute(v) => encode_payload(v),
            Self::ClockResult(v) => encode_payload(v),
            Self::GroupCreate(v) => encode_payload(v),
            Self::GroupJoin(v) => encode_payload(v),
            Self::GroupLeave(v) => encode_payload(v),
            Self::GroupEpoch(v) => encode_payload(v),
            Self::ReceiverEpoch(v) => encode_payload(v),
            Self::Ping(v) => encode_payload(v),
            Self::Pong(v) => encode_payload(v),
            Self::Error(v) => encode_payload(v),
            Self::Bye(v) => encode_payload(v),
        }
    }

    /// 按 [`Self::encode_body`] 组一条**完整控制帧**（8 B 帧头 + `request_id` + 载荷）。
    ///
    /// 组帧本身由 `audiolink-proto::ControlFrame` 负责（L1 只有一份实现），这里只做命令码 → 帧的粘合。
    /// 用 `ControlFrame` 而不是 `encode_with_payload`，是因为载荷**已经**是 postcard 字节：
    /// 再走一次 `Serialize` 会套第二层长度前缀，对端必然解不出。
    pub fn encode_frame(&self, request_id: u32) -> Result<Vec<u8>, AudioLinkError> {
        let body = self.encode_body()?;
        audiolink_proto::ControlFrame {
            op: self.op(),
            request_id,
            payload: &body,
        }
        .encode_to_vec()
    }
}

/// 分发一条控制帧。
///
/// `op` 来自 `audiolink_net::ControlMessage::op`：`None` 表示「信封合法但命令码未知」。
/// 这个区分正是 §1.1 能落地的前提（见 `docs/11-m1-contract.md` §3 的说明）。
pub fn dispatch(
    op: Option<OpCode>,
    payload: &[u8],
    stats: &mut DispatchStats,
) -> Option<ControlRequest> {
    dispatch_into(op, payload, stats).0
}

/// 与 [`dispatch`] 同义，但同时返回处置结论，便于调用方记账与测试断言。
///
/// 统计**无论走哪条分支都会 `received += 1`**：忽略率本身是有价值的遥测（版本漂移信号）。
pub fn dispatch_into(
    op: Option<OpCode>,
    payload: &[u8],
    stats: &mut DispatchStats,
) -> (Option<ControlRequest>, DispatchOutcome) {
    stats.received += 1;

    let Some(op) = op else {
        stats.ignored_unknown_op += 1;
        return (None, DispatchOutcome::IgnoredUnknownOp);
    };

    match decode_control(op, payload) {
        Ok(request) => {
            stats.handled += 1;
            (Some(request), DispatchOutcome::Handled)
        }
        Err(reason) => {
            // §1.1：载荷非法的处置是「记日志并忽略该帧」，**不是**断连。
            match reason {
                DecodeFailure::Unimplemented => {
                    stats.ignored_unimplemented += 1;
                    (None, DispatchOutcome::IgnoredUnimplemented)
                }
                DecodeFailure::BadPayload => {
                    stats.ignored_bad_payload += 1;
                    (None, DispatchOutcome::IgnoredBadPayload)
                }
            }
        }
    }
}

/// 解码失败的两种性质（决定计入哪个计数器）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecodeFailure {
    /// 命令码在 §4.1 里有定义，但本里程碑没有语义（M2/M3 的命令）。
    Unimplemented,
    /// 载荷本身不合法（postcard 解不出 / 尾随字节）。
    BadPayload,
}

/// 按命令码解码载荷。
fn decode_control(op: OpCode, payload: &[u8]) -> Result<ControlRequest, DecodeFailure> {
    fn bad<T>(result: Result<T, AudioLinkError>) -> Result<T, DecodeFailure> {
        result.map_err(|_| DecodeFailure::BadPayload)
    }

    match op {
        OpCode::Hello => bad(decode_payload(payload)).map(ControlRequest::Hello),
        OpCode::HelloAck => bad(decode_payload(payload)).map(ControlRequest::HelloAck),
        OpCode::OpenStream => bad(decode_payload(payload)).map(ControlRequest::OpenStream),
        OpCode::OpenStreamAck => bad(decode_payload(payload)).map(ControlRequest::OpenStreamAck),
        OpCode::CloseStream => bad(decode_payload(payload)).map(ControlRequest::CloseStream),
        OpCode::StreamStats => bad(decode_payload(payload)).map(ControlRequest::StreamStats),
        OpCode::SetGain => bad(decode_payload(payload)).map(ControlRequest::SetGain),
        OpCode::SetMute => bad(decode_payload(payload)).map(ControlRequest::SetMute),
        OpCode::ClockResult => bad(decode_payload(payload)).map(ControlRequest::ClockResult),
        OpCode::GroupCreate => bad(decode_payload(payload)).map(ControlRequest::GroupCreate),
        OpCode::GroupJoin => bad(decode_payload(payload)).map(ControlRequest::GroupJoin),
        OpCode::GroupLeave => bad(decode_payload(payload)).map(ControlRequest::GroupLeave),
        OpCode::GroupEpoch => bad(decode_payload(payload)).map(ControlRequest::GroupEpoch),
        OpCode::ReceiverEpoch => bad(decode_payload(payload)).map(ControlRequest::ReceiverEpoch),
        OpCode::Ping => bad(decode_payload(payload)).map(ControlRequest::Ping),
        OpCode::Pong => bad(decode_payload(payload)).map(ControlRequest::Pong),
        OpCode::Error => bad(decode_payload(payload)).map(ControlRequest::Error),
        OpCode::Bye => bad(decode_payload(payload)).map(ControlRequest::Bye),

        // M2/M3 的命令码（§4.1 里有定义，但本里程碑不实现）。
        OpCode::SetVolumeLock | OpCode::TelemetryPush => Err(DecodeFailure::Unimplemented),
    }
}

/// 本模块用到的载荷类型（对外转发，避免调用点写两条 `use`）。
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use audiolink_types::Capabilities;

    use super::*;
    use crate::payload::{CodecPref, SourceKind};
    use audiolink_types::{
        Caps, ClockQuality, NodeId, NodeInfo, PROTO_VERSION, Platform, StreamStats,
    };

    fn sample_node() -> NodeInfo {
        NodeInfo::new(
            NodeId::from_bytes([7u8; 32]),
            "unit-test-node",
            Platform::Windows,
            Caps::CAN_SEND | Caps::CAN_RECEIVE,
        )
    }

    fn opus_pref() -> CodecPref {
        CodecPref::Opus {
            frame_ms: 20,
            bitrate_kbps: 160,
            fec: false,
            vbr: true,
        }
    }

    /// 全部 M1 命令各造一条样本，供往返与分发测试复用。
    fn m1_samples() -> Vec<ControlRequest> {
        let node = sample_node();
        vec![
            ControlRequest::Hello(HelloPayload {
                proto_version: PROTO_VERSION,
                node: node.clone(),
                nonce: vec![1u8; 16],
                caps: Capabilities::CURRENT,
            }),
            ControlRequest::HelloAck(HelloAckPayload {
                proto_version: PROTO_VERSION,
                node: node.clone(),
                accepted: true,
                reason: String::new(),
                caps: Capabilities::CURRENT,
                agreed_caps: 0,
            }),
            ControlRequest::OpenStream(OpenStreamPayload {
                session_id: 42,
                source: SourceKind::SystemLoopback,
                codec_prefs: vec![opus_pref(), CodecPref::Pcm16],
                target_rate_kbps: 160,
                channels: 2,
                group: None,
            }),
            ControlRequest::OpenStreamAck(OpenStreamAckPayload {
                session_id: 42,
                stream_id: 1,
                codec_chosen: opus_pref(),
                epoch_id: 0x1122_3344_5566_7788,
                epoch_local_us: 987_654,
            }),
            ControlRequest::CloseStream(CloseStreamPayload {
                stream_id: 1,
                reason: "user stop".to_string(),
            }),
            ControlRequest::StreamStats(StreamStats {
                stream_id: 1,
                rtt_us: 4_321,
                e2e_latency_us: 39_700,
                ..StreamStats::default()
            }),
            ControlRequest::SetGain(SetGainPayload {
                stream_id: crate::payload::STREAM_ID_ALL,
                gain: 1.0,
                ramp_ms: 20,
            }),
            ControlRequest::SetMute(SetMutePayload {
                stream_id: 1,
                mute: false,
            }),
            ControlRequest::ClockResult(ClockResultPayload {
                offset_us: -1_234,
                drift_ppm: 12,
                quality: ClockQuality::Good,
            }),
            ControlRequest::GroupCreate(GroupCreatePayload {
                group_id: 7,
                epoch_id: 0x1122_3344_5566_7788,
                epoch_local_us: 1_234_567,
                lead_ms: 30,
                members: vec![NodeId::from_bytes([9u8; 32])],
            }),
            ControlRequest::GroupJoin(GroupJoinPayload {
                group_id: 7,
                member: NodeId::from_bytes([9u8; 32]),
            }),
            ControlRequest::GroupLeave(GroupLeavePayload {
                group_id: 7,
                member: NodeId::from_bytes([9u8; 32]),
            }),
            ControlRequest::GroupEpoch(GroupEpochPayload {
                epoch_id: 0x1122_3344_5566_7788,
                epoch_local_us: 1_234_567,
                lead_ms: 30,
            }),
            // M4：同一个载荷的反方向用法（接收端 → 发送端广播共同基准）。
            ControlRequest::ReceiverEpoch(GroupEpochPayload {
                epoch_id: 0x9988_7766_5544_3322,
                epoch_local_us: 7_654_321,
                lead_ms: 200,
            }),
            ControlRequest::Ping(PingPayload { t1: 1, t2: 2 }),
            ControlRequest::Pong(PingPayload { t1: 1, t2: 2 }),
            ControlRequest::Error(ErrorPayload {
                code: 1008,
                message: "bad frame".to_string(),
                context: Some("truncated".to_string()),
            }),
            ControlRequest::Bye(ByePayload {
                reason: "shutdown".to_string(),
            }),
        ]
    }

    #[test]
    fn every_m1_payload_round_trips_through_postcard() {
        for request in m1_samples() {
            let body = request
                .encode_body()
                .unwrap_or_else(|error| panic!("编码 {request:?} 失败：{error}"));
            let (decoded, outcome) =
                dispatch_into(Some(request.op()), &body, &mut DispatchStats::default());

            assert_eq!(outcome, DispatchOutcome::Handled, "{request:?}");
            assert_eq!(
                decoded.as_ref(),
                Some(&request),
                "postcard 往返必须逐字段一致"
            );
        }
    }

    #[test]
    fn m1_samples_cover_every_implemented_opcode() {
        // 防止将来加了命令却忘了写测试样本（样本表是「覆盖度」的唯一来源）。
        let samples = m1_samples();
        let covered: Vec<OpCode> = samples.iter().map(ControlRequest::op).collect();

        for op in OpCode::ALL {
            let implemented = !matches!(op, OpCode::SetVolumeLock | OpCode::TelemetryPush);
            assert_eq!(
                covered.contains(&op),
                implemented,
                "{op:?} 的样本覆盖与「是否实现」不一致"
            );
        }
    }

    #[test]
    fn frames_round_trip_through_the_l1_codec() {
        // encode_frame 必须产出真的合法 L1 帧（否则对端根本解不出来）。
        for request in m1_samples() {
            let frame = request.encode_frame(7).unwrap();
            let decoded = audiolink_proto::ControlFrame::decode(&frame).unwrap();
            assert_eq!(decoded.op, request.op(), "{request:?}");
            assert_eq!(decoded.request_id, 7);
            assert_eq!(
                decoded.payload,
                request.encode_body().unwrap(),
                "{request:?}"
            );
        }
    }

    #[test]
    fn unknown_opcode_is_ignored_and_counted_not_fatal() {
        // §1.1 的行为契约：信封合法、命令码不认识 → 忽略并计数，**不断流**。
        let mut stats = DispatchStats::default();

        let (request, outcome) = dispatch_into(None, &[1, 2, 3], &mut stats);

        assert_eq!(outcome, DispatchOutcome::IgnoredUnknownOp);
        assert!(request.is_none());
        assert_eq!(stats.received, 1, "被忽略的帧也要计入收帧总数");
        assert_eq!(stats.ignored_unknown_op, 1);
        assert_eq!(stats.handled, 0);
    }

    #[test]
    fn bad_payload_is_ignored_and_counted_not_fatal() {
        let mut stats = DispatchStats::default();

        // 声称是 HELLO，但载荷是垃圾字节 —— 帧边界完好，postcard 解不出来。
        let (request, outcome) =
            dispatch_into(Some(OpCode::Hello), &[0xFF, 0xFF, 0xFF, 0xFF], &mut stats);

        assert_eq!(outcome, DispatchOutcome::IgnoredBadPayload);
        assert!(request.is_none());
        assert_eq!(stats.ignored_bad_payload, 1);
        assert_eq!(stats.ignored_pct_x100(), 10_000, "1 收 1 忽略 = 100.00%");
    }

    #[test]
    fn unimplemented_opcodes_are_distinguished_from_bad_payloads() {
        // M2/M3 的命令码是「认识但本里程碑不做」——与「载荷烂了」是两件事，
        // 必须分开计数：前者说明对端功能更新，后者说明链路或对端有 bug。
        for op in [OpCode::TelemetryPush, OpCode::SetVolumeLock] {
            let mut stats = DispatchStats::default();
            let (request, outcome) = dispatch_into(Some(op), &[], &mut stats);

            assert_eq!(outcome, DispatchOutcome::IgnoredUnimplemented, "{op:?}");
            assert!(request.is_none(), "{op:?}");
            assert_eq!(stats.ignored_unimplemented, 1, "{op:?}");
            assert_eq!(stats.ignored_bad_payload, 0, "{op:?} 不应计成坏载荷");
        }
    }

    #[test]
    fn trailing_bytes_are_rejected_by_the_l1_rule() {
        // §4：postcard 载荷必须恰好消费全部字节 —— 尾随字节必须判非法（防 schema 漂移被静默接受）。
        let mut stats = DispatchStats::default();
        let mut body = ControlRequest::Ping(PingPayload { t1: 1, t2: 2 })
            .encode_body()
            .unwrap();
        body.push(0xAB);

        let (request, outcome) = dispatch_into(Some(OpCode::Ping), &body, &mut stats);

        assert_eq!(outcome, DispatchOutcome::IgnoredBadPayload);
        assert!(request.is_none());
    }

    #[test]
    fn op_is_preserved_across_the_round_trip() {
        let body = ControlRequest::Ping(PingPayload { t1: 11, t2: 22 })
            .encode_body()
            .unwrap();
        let request = dispatch(Some(OpCode::Ping), &body, &mut DispatchStats::default()).unwrap();
        assert_eq!(request.op(), OpCode::Ping);
    }

    #[test]
    fn stats_arithmetic_is_sane() {
        let mut stats = DispatchStats::default();
        assert_eq!(stats.ignored_pct_x100(), 0, "无收帧时不得除零");

        dispatch_into(None, &[], &mut stats); // 未知命令码
        dispatch_into(Some(OpCode::Hello), &[0xFF; 4], &mut stats); // 非法载荷
        dispatch_into(
            Some(OpCode::Ping),
            &ControlRequest::Ping(PingPayload { t1: 1, t2: 2 })
                .encode_body()
                .unwrap(),
            &mut stats,
        ); // 正常

        assert_eq!(stats.received, 3);
        assert_eq!(stats.handled, 1);
        assert_eq!(stats.ignored(), 2);
        assert_eq!(stats.ignored_pct_x100(), 6_666);
    }

    #[test]
    fn outcome_names_are_stable_for_logs() {
        assert_eq!(DispatchOutcome::Handled.name(), "handled");
        assert_eq!(
            DispatchOutcome::IgnoredUnknownOp.name(),
            "ignored-unknown-op"
        );
        assert_eq!(
            DispatchOutcome::IgnoredUnimplemented.name(),
            "ignored-unimplemented"
        );
        assert_eq!(
            DispatchOutcome::IgnoredBadPayload.name(),
            "ignored-bad-payload"
        );
        assert!(DispatchOutcome::IgnoredUnknownOp.is_ignored());
        assert!(!DispatchOutcome::Handled.is_ignored());
    }
}
