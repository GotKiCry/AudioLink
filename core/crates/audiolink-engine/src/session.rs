//! 会话状态机（`docs/02-architecture.md` §11）
//!
//! ```text
//! Idle ──connect──► Handshaking ──ok──► Streaming ──loss/latency──► Degraded
//!                      │                   │                          │
//!                   失败│                断链│                       恶化│
//!                      ▼                   ▼                          ▼
//!                   Failed ◄─────────Reconnecting ──ok──► Streaming(Rebuilt)
//! ```
//!
//! # 为什么单独成模块
//!
//! 「异常即永久静音」是旧版最大的病根（`docs/02-architecture.md` §4）。根治手段是让**每一处失败都有
//! 明确的去向**：要么是本模块里一条合法的迁移边，要么是一次显式的 [`SessionTransition::Invalid`] 报错。
//! 把迁移表写成数据而不是散落在各处的 `if`，是为了让「哪些状态组合根本不该出现」变成可测试的事实。

use std::fmt;

use audiolink_types::AudioLinkError;

/// 会话状态（`docs/02-architecture.md` §11）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SessionState {
    /// 未开始（初始态，也是关闭后的归宿）。
    #[default]
    Idle,
    /// QUIC 已连上，正在跑 §5 握手（`HELLO` / `HELLO_ACK`）。
    Handshaking,
    /// 正在推流（音视频数据报在流动）。
    Streaming,
    /// 链路质量下降（丢包 / 延迟劣化），**仍在出声**。
    Degraded,
    /// 链路断开，正在指数退避重连（FR-27）。
    Reconnecting,
    /// 终态：本轮会话失败（不可自动恢复，需要用户或上层重新发起）。
    Failed,
}

impl SessionState {
    /// 全部状态（遍历测试用）。
    pub const ALL: [SessionState; 6] = [
        Self::Idle,
        Self::Handshaking,
        Self::Streaming,
        Self::Degraded,
        Self::Reconnecting,
        Self::Failed,
    ];

    /// 快照名（日志 / 遥测 / UI 契约用，**不要改名**，桌面端 TS 侧按这些字面量匹配）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Handshaking => "handshaking",
            Self::Streaming => "streaming",
            Self::Degraded => "degraded",
            Self::Reconnecting => "reconnecting",
            Self::Failed => "failed",
        }
    }

    /// 本状态下音频数据报是否**应当**在流动。
    ///
    /// `Degraded` 为 `true` 是刻意的：劣化只是加深缓冲与降码率的信号，**不是停播理由**。
    pub const fn is_streaming(self) -> bool {
        matches!(self, Self::Streaming | Self::Degraded)
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 驱动状态机的事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionEvent {
    /// 本机发起连接（出站）。
    ConnectRequested,
    /// 收到对端入站连接（进站）。
    AcceptedInbound,
    /// §5 握手完成。
    HandshakeOk,
    /// §5 握手失败（版本不匹配 / 能力协商失败 / 对端拒绝）。
    HandshakeFailed,
    /// 链路质量下降（丢包或延迟越过阈值）—— 仍可出声。
    LinkDegraded,
    /// 链路质量恢复。
    LinkRecovered,
    /// 链路断开（连接丢失 / 超时）。
    LinkLost,
    /// 重连成功。
    ReconnectOk,
    /// 重连失败（已达退避上限）。
    ReconnectFailed,
    /// 主动关闭（任何状态都合法）。
    Shutdown,
}

impl SessionEvent {
    /// 全部事件（遍历测试用）。
    pub const ALL: [SessionEvent; 10] = [
        Self::ConnectRequested,
        Self::AcceptedInbound,
        Self::HandshakeOk,
        Self::HandshakeFailed,
        Self::LinkDegraded,
        Self::LinkRecovered,
        Self::LinkLost,
        Self::ReconnectOk,
        Self::ReconnectFailed,
        Self::Shutdown,
    ];

    /// 快照名（日志用）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::ConnectRequested => "connect-requested",
            Self::AcceptedInbound => "accepted-inbound",
            Self::HandshakeOk => "handshake-ok",
            Self::HandshakeFailed => "handshake-failed",
            Self::LinkDegraded => "link-degraded",
            Self::LinkRecovered => "link-recovered",
            Self::LinkLost => "link-lost",
            Self::ReconnectOk => "reconnect-ok",
            Self::ReconnectFailed => "reconnect-failed",
            Self::Shutdown => "shutdown",
        }
    }
}

/// 状态迁移表（`docs/02-architecture.md` §11 的图，逐条落成数据）。
///
/// # 这张表里几条刻意的决定
///
/// 1. **`Degraded` 不是失败**：它仍属 `is_streaming()`，所以不提供「从 Degraded 直接进 Failed」的边 ——
///    只有 `LinkLost`（真的断了）或显式 `Shutdown` 才离开。这是「不许静默停止」的直接体现。
/// 2. **`Failed` 可以重启**：`ConnectRequested` / `AcceptedInbound` 都能把它拉回 `Handshaking`，
///    否则一次失败会让会话永久卡死，必须重启进程 —— 那正是旧版的病。
/// 3. **`Shutdown` 在任何状态都合法**，且一律回到 `Idle`（幂等：重复关闭不报错）。
/// 4. **`Reconnecting` 不接受 `HandshakeOk`**：重连成功走 `ReconnectOk`，
///    否则「首次握手」与「重连成功」两条路径的日志会混在一起，排障时无法区分。
pub const fn next_state(from: SessionState, event: SessionEvent) -> Option<SessionState> {
    use SessionEvent as E;
    use SessionState as S;

    // 主动关闭优先：任何状态都能关，且幂等。
    if matches!(event, E::Shutdown) {
        return Some(S::Idle);
    }

    match (from, event) {
        // 启动：空转与失败态都能重新发起（Failed 可重启是关键，见上文第 2 条）。
        (S::Idle | S::Failed, E::ConnectRequested | E::AcceptedInbound) => Some(S::Handshaking),

        // 握手阶段
        (S::Handshaking, E::HandshakeOk) => Some(S::Streaming),
        (S::Handshaking, E::HandshakeFailed | E::LinkLost) => Some(S::Failed),

        // 流中：劣化与断链
        (S::Streaming, E::LinkDegraded) => Some(S::Degraded),
        (S::Streaming, E::LinkLost) => Some(S::Reconnecting),

        // 劣化中：恢复，或进一步断链
        (S::Degraded, E::LinkRecovered) => Some(S::Streaming),
        (S::Degraded, E::LinkLost) => Some(S::Reconnecting),

        // 重连中
        (S::Reconnecting, E::ReconnectOk) => Some(S::Streaming),
        (S::Reconnecting, E::ReconnectFailed) => Some(S::Failed),

        // 其余组合都不该出现 —— 由 `Invalid` 显式暴露，而不是悄悄忽略
        _ => None,
    }
}

/// 一次迁移尝试的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionTransition {
    /// 合法迁移。
    Applied {
        /// 迁移前状态。
        from: SessionState,
        /// 迁移后状态。
        to: SessionState,
        /// 触发事件。
        event: SessionEvent,
    },
    /// 非法迁移：`(状态, 事件)` 这一组合不在迁移表里。
    ///
    /// 这是**缺陷信号**而不是用户错误：说明调用方在用错误的事件驱动状态机。
    /// 上报它比忽略它更安全 —— 忽略会让状态机悄悄偏离真实链路状态。
    Invalid {
        /// 当前状态。
        from: SessionState,
        /// 被拒绝的事件。
        event: SessionEvent,
    },
}

/// 当前会话状态 + 迁移历史计数。
#[derive(Debug, Clone)]
pub struct SessionMachine {
    state: SessionState,
    handshakes: u64,
    reconnects: u64,
    degradations: u64,
    last_event: Option<SessionEvent>,
}

impl Default for SessionMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionMachine {
    /// 从 [`SessionState::Idle`] 开始。
    pub const fn new() -> Self {
        Self {
            state: SessionState::Idle,
            handshakes: 0,
            reconnects: 0,
            degradations: 0,
            last_event: None,
        }
    }

    /// 当前状态。
    pub const fn state(&self) -> SessionState {
        self.state
    }

    /// 最近一次被接受的事件（`None` = 从未迁移过）。
    pub const fn last_event(&self) -> Option<SessionEvent> {
        self.last_event
    }

    /// 完成过的握手次数（含重连后的重建）。
    pub const fn handshakes(&self) -> u64 {
        self.handshakes
    }

    /// 成功重连次数。
    pub const fn reconnects(&self) -> u64 {
        self.reconnects
    }

    /// 进入 `Degraded` 的次数（劣化频率是链路体检的核心指标）。
    pub const fn degradations(&self) -> u64 {
        self.degradations
    }

    /// 接受一个事件；非法组合返回 [`SessionTransition::Invalid`] 且**状态不变**。
    pub fn apply(&mut self, event: SessionEvent) -> SessionTransition {
        let from = self.state;
        let Some(to) = next_state(from, event) else {
            return SessionTransition::Invalid { from, event };
        };

        // 计数放在迁移成功之后：被拒绝的事件不该污染统计。
        match (from, event) {
            (SessionState::Handshaking, SessionEvent::HandshakeOk) => self.handshakes += 1,
            (SessionState::Reconnecting, SessionEvent::ReconnectOk) => {
                self.handshakes += 1;
                self.reconnects += 1;
            }
            (SessionState::Streaming, SessionEvent::LinkDegraded) => self.degradations += 1,
            _ => {}
        }

        self.state = to;
        self.last_event = Some(event);
        SessionTransition::Applied { from, to, event }
    }

    /// 把一个错误按「该不该断流」映射成事件（`docs/02-architecture.md` §11 的处置纪律）。
    ///
    /// 判据**只看** [`AudioLinkError::is_statistical`]，不看具体错误码 —— 这样以后新增统计类
    /// 错误码时，这里不需要跟着改，也就不会漏。
    pub fn event_for_error(error: &AudioLinkError) -> Option<SessionEvent> {
        if error.is_statistical() {
            // 统计类错误：链路还在跑，最多算劣化信号，绝不迁移到 Failed。
            Some(SessionEvent::LinkDegraded)
        } else {
            // 契约级失败（1001–1009）：握手阶段是握手失败，其余情况按断链处理。
            None
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use audiolink_types::AudioLinkError;

    #[test]
    fn fresh_machine_starts_idle() {
        let machine = SessionMachine::new();
        assert_eq!(machine.state(), SessionState::Idle);
        assert_eq!(machine.last_event(), None);
        assert_eq!(machine.handshakes(), 0);
    }

    #[test]
    fn happy_path_walks_the_documented_diagram() {
        let mut machine = SessionMachine::new();

        for (event, expected) in [
            (SessionEvent::ConnectRequested, SessionState::Handshaking),
            (SessionEvent::HandshakeOk, SessionState::Streaming),
            (SessionEvent::LinkDegraded, SessionState::Degraded),
            (SessionEvent::LinkRecovered, SessionState::Streaming),
            (SessionEvent::LinkLost, SessionState::Reconnecting),
            (SessionEvent::ReconnectOk, SessionState::Streaming),
            (SessionEvent::Shutdown, SessionState::Idle),
        ] {
            let transition = machine.apply(event);
            assert!(
                matches!(transition, SessionTransition::Applied { to, .. } if to == expected),
                "事件 {event:?} 应迁移到 {expected:?}，实际 {transition:?}"
            );
            assert_eq!(machine.state(), expected);
        }

        assert_eq!(machine.handshakes(), 2, "首次握手 + 重连各计一次");
        assert_eq!(machine.reconnects(), 1);
        assert_eq!(machine.degradations(), 1);
    }

    #[test]
    fn handshake_failure_lands_in_failed_and_failed_can_restart() {
        let mut machine = SessionMachine::new();
        machine.apply(SessionEvent::AcceptedInbound);
        assert!(matches!(
            machine.apply(SessionEvent::HandshakeFailed),
            SessionTransition::Applied {
                to: SessionState::Failed,
                ..
            }
        ));

        // Failed 必须是「可重启」的，否则一次失败就会把会话永久卡死（旧版病根之一）。
        assert!(matches!(
            machine.apply(SessionEvent::ConnectRequested),
            SessionTransition::Applied {
                to: SessionState::Handshaking,
                ..
            }
        ));
    }

    #[test]
    fn invalid_transition_is_reported_and_leaves_state_untouched() {
        let mut machine = SessionMachine::new();

        // Idle 下没有握手可完成。
        let transition = machine.apply(SessionEvent::HandshakeOk);
        assert_eq!(
            transition,
            SessionTransition::Invalid {
                from: SessionState::Idle,
                event: SessionEvent::HandshakeOk,
            }
        );
        assert_eq!(machine.state(), SessionState::Idle, "非法事件不得改变状态");
        assert_eq!(machine.handshakes(), 0, "非法事件不得污染统计");
    }

    #[test]
    fn reconnecting_rejects_handshake_ok_to_keep_the_two_paths_distinct() {
        let mut machine = SessionMachine::new();
        machine.apply(SessionEvent::ConnectRequested);
        machine.apply(SessionEvent::HandshakeOk);
        machine.apply(SessionEvent::LinkLost);

        assert_eq!(machine.state(), SessionState::Reconnecting);
        assert!(matches!(
            machine.apply(SessionEvent::HandshakeOk),
            SessionTransition::Invalid { .. }
        ));
        assert_eq!(machine.state(), SessionState::Reconnecting);
    }

    #[test]
    fn shutdown_is_idempotent_from_every_state() {
        for state in SessionState::ALL {
            // 把机器推到目标状态（能推到就推，推不到的用 Idle 代表）。
            let mut machine = SessionMachine::new();
            let _ = drive_to(&mut machine, state);

            assert!(matches!(
                machine.apply(SessionEvent::Shutdown),
                SessionTransition::Applied {
                    to: SessionState::Idle,
                    ..
                }
            ));
            // 再关一次仍然合法（幂等）。
            assert!(matches!(
                machine.apply(SessionEvent::Shutdown),
                SessionTransition::Applied {
                    to: SessionState::Idle,
                    ..
                }
            ));
        }
    }

    #[test]
    fn every_state_event_pair_is_covered_by_the_table() {
        // 迁移表是「无遗漏」的：每个组合要么有边，要么明确 Invalid（不会 panic、不会 nullptr）。
        for state in SessionState::ALL {
            for event in SessionEvent::ALL {
                let mut machine = SessionMachine::new();
                let _ = drive_to(&mut machine, state);
                if machine.state() != state {
                    continue; // 该状态在当前表下不可达（如 Reconnecting 无法直接构造），跳过
                }
                let before = machine.state();
                let transition = machine.apply(event);
                match transition {
                    SessionTransition::Applied { from, to, .. } => {
                        assert_eq!(from, before);
                        assert_eq!(machine.state(), to);
                        // `Idle + Shutdown -> Idle` 是刻意的幂等自环；其余自环没有意义。
                        if !(before == SessionState::Idle && event == SessionEvent::Shutdown) {
                            assert_ne!(to, before, "除幂等关闭外，自环迁移没有意义");
                        }
                    }
                    SessionTransition::Invalid { from, .. } => {
                        assert_eq!(from, before);
                        assert_eq!(machine.state(), before, "非法迁移不得改变状态");
                    }
                }
            }
        }
    }

    #[test]
    fn streaming_is_true_for_both_streaming_and_degraded() {
        assert!(SessionState::Streaming.is_streaming());
        assert!(
            SessionState::Degraded.is_streaming(),
            "劣化只是降级信号，不是停播理由"
        );
        assert!(!SessionState::Idle.is_streaming());
        assert!(!SessionState::Handshaking.is_streaming());
        assert!(!SessionState::Reconnecting.is_streaming());
        assert!(!SessionState::Failed.is_streaming());
    }

    #[test]
    fn statistical_errors_map_to_degraded_not_failure() {
        // 统计类（2001/2002/2003/3001）：链路还在跑 → 劣化信号，绝不 Failed。
        for error in [
            AudioLinkError::playout_underrun("sink starved"),
            AudioLinkError::sink_rebuild("watchdog"),
            AudioLinkError::capture_lost("device unplugged"),
            AudioLinkError::rate_limited("too many probes"),
        ] {
            assert!(error.is_statistical(), "{error} 应当是统计类");
            assert_eq!(
                SessionMachine::event_for_error(&error),
                Some(SessionEvent::LinkDegraded),
                "{error} 应映射为劣化而非失败"
            );
        }

        // 契约级（1001–1009）：没有自动迁移边，必须由调用方显式决定去向。
        for error in [
            AudioLinkError::version_mismatch("v3 peer"),
            AudioLinkError::cap_unsupported("peer lacks opus"),
            AudioLinkError::bad_request("handshake failed"),
        ] {
            assert!(error.is_fatal(), "{error} 应当是契约级失败");
            assert_eq!(SessionMachine::event_for_error(&error), None);
        }
    }

    #[test]
    fn state_names_match_the_desktop_ts_contract() {
        // 桌面端 TS 侧按这些字面量匹配（docs/11-m1-contract.md §6 的 PeerView.state）。
        assert_eq!(SessionState::Idle.name(), "idle");
        assert_eq!(SessionState::Handshaking.name(), "handshaking");
        assert_eq!(SessionState::Streaming.name(), "streaming");
        assert_eq!(SessionState::Degraded.name(), "degraded");
        assert_eq!(SessionState::Reconnecting.name(), "reconnecting");
        assert_eq!(SessionState::Failed.name(), "failed");
    }

    /// 把状态机推到 `target`（可达则返回 `Ok`，不可达返回当前状态）。
    fn drive_to(machine: &mut SessionMachine, target: SessionState) -> SessionState {
        let path: &[SessionEvent] = match target {
            SessionState::Idle => &[],
            SessionState::Handshaking => &[SessionEvent::ConnectRequested],
            SessionState::Streaming => &[SessionEvent::ConnectRequested, SessionEvent::HandshakeOk],
            SessionState::Degraded => &[
                SessionEvent::ConnectRequested,
                SessionEvent::HandshakeOk,
                SessionEvent::LinkDegraded,
            ],
            SessionState::Reconnecting => &[
                SessionEvent::ConnectRequested,
                SessionEvent::HandshakeOk,
                SessionEvent::LinkLost,
            ],
            SessionState::Failed => &[
                SessionEvent::ConnectRequested,
                SessionEvent::HandshakeFailed,
            ],
        };
        for event in path {
            machine.apply(*event);
        }
        machine.state()
    }
}
