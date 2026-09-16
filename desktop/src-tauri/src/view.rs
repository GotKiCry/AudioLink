//! 桌面端 ⇄ 前端 的**冻结视图形状**（`docs/11-m1-contract.md` §6）。
//!
//! 为什么单独成模块：这些 JSON 字段名是契约（前端按它写、Lead 按它对接引擎），
//! 只在这里定义一次，改字段时不可能"改了一处忘了另一处"。
//! mock 与真实引擎**都必须**产出这些类型 —— 前端不为 mock 定制。
//!
//! 命名注意：`PeerView` / `TelemetryView` / `LocalStatus` 用 camelCase（契约原文如此），
//! 而 `start_send` 的返回 `stream_id` 是 **snake_case**（契约原文如此，不是笔误）。

use serde::Serialize;

/// Windows 活动输出端点；完整 ID 用于选择，同名设备仍各自独立。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDeviceView {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub is_virtual: bool,
    pub sample_rate: u32,
    pub channels: u16,
    pub unavailable_reason: Option<String>,
}

/// `local_status` 的返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalStatus {
    /// 本机指纹短码（`NodeId` 前 8 个 hex，UI 上显示为 `fp:xxxxxxxx`）。
    pub id_short: String,
    /// 本机节点名（用户可见，仅用于展示）。
    pub name: String,
    /// 本机 QUIC 端点 `ip:port`。
    pub addr: String,
    /// `windows` / `android`（枚举字符串，M1 桌面外壳只可能是 `windows`）。
    pub platform: String,
}

/// 会话状态。取值与契约 §6 的 TS 联合类型一一对应（全小写单词，故用 `lowercase`）。
///
/// 与 `audiolink_engine::SessionState`（契约 §5）的映射：
/// `Idle→Idle`、`Handshaking→Handshaking`、`Streaming→Streaming`、
/// `Degraded→Degraded`、`Failed→Failed`（重连中在引擎侧仍是 `Degraded`/`Failed` 的过渡态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PeerState {
    Idle,
    Handshaking,
    Streaming,
    /// 契约 §6 冻结的取值之一：链路质量下降但仍在出声（引擎 `SessionState::Degraded`）。
    Degraded,
    /// 契约 §6 冻结的取值之一：会话失败（引擎 `SessionState::Failed`）。
    Failed,
}

/// 对端卡片 + `list_peers` 的元素。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerView {
    /// 对端指纹短码（8 hex）；也是 `start_send` 的入参 `id_short`。
    pub id_short: String,
    /// 对端自报节点名（配对前可能是占位名）。
    pub name: String,
    /// 对端 QUIC 端点 `ip:port`。
    pub addr: String,
    pub state: PeerState,
    /// 是否已在信任库（`docs/02-architecture.md` §8：白名单命中 = 免交互直连）。
    pub trusted: bool,
}

/// 遥测数字面板的数据（契约 §6；单位统一 µs / bps，由前端换算成 ms / kbps 展示）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryView {
    /// 已登记对端数（与 `list_peers().len()` 一致，UI 两处数字不会自相矛盾）。
    pub peers: u32,
    /// QUIC 平滑 RTT（µs）。
    pub rtt_us: u64,
    /// 到达间隔抖动（µs）。
    pub jitter_us: u64,
    /// 丢包率（百分数，`0.10` = 0.1%）。
    pub loss_pct: f64,
    /// 实际编码码率（bps）。
    pub bitrate_bps: u64,
    /// 播放环水位（µs）。
    pub buffer_level_us: u64,
    /// 播放欠载次数（累计）。
    pub underruns: u64,
    /// 当前端到端延迟估计（µs）—— 验收主指标。
    pub e2e_latency_us: u64,
    /// 端到端延迟 P50（µs）。
    pub e2e_p50_us: u64,
    /// 端到端延迟 P95（µs）。
    pub e2e_p95_us: u64,
}

impl TelemetryView {
    /// 无会话时的全零快照（**不是**"没数据"的妥协：契约字段是固定数字，UI 才好写）。
    pub const fn zeros(peers: u32) -> Self {
        Self {
            peers,
            rtt_us: 0,
            jitter_us: 0,
            loss_pct: 0.0,
            bitrate_bps: 0,
            buffer_level_us: 0,
            underruns: 0,
            e2e_latency_us: 0,
            e2e_p50_us: 0,
            e2e_p95_us: 0,
        }
    }
}

/// `audiolink://pair-required` 的载荷（契约 §6）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequiredPayload {
    /// 发起配对的对端短指纹。
    pub id_short: String,
    /// 对端名字（展示用，便于用户确认"是和谁配对"）。
    pub name: String,
    /// 需要用户核对/输入的 6 位数字码。
    pub pin: String,
}

/// `start_send` 的返回（契约 §6：字段名就是 `stream_id`，snake_case）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StartSendResult {
    pub stream_id: u32,
}

/// `submit_pin` 的返回（契约 §6：**始终成功返回**，配对失败走 `ok: false` + 人话 `reason`，
/// 这样 UI 能直接显示"还可尝试 N 次"而不用解析错误码）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubmitPinResult {
    pub ok: bool,
    /// 成功时为 `""`；失败时是给用户看的整句原因（不允许"未知错误"）。
    pub reason: String,
}

#[cfg(test)]
// 测试里断言失败就该炸：显式放行 panic 系列 lint（工作区默认 deny，见根 Cargo.toml `[workspace.lints]`）。
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    /// 契约字段名一旦漂移，前端会静默拿到 `undefined`；这里用序列化结果钉死形状。
    #[test]
    fn telemetry_json_shape_matches_contract() {
        let json = serde_json::to_string(&TelemetryView::zeros(2)).expect("serialize");
        assert_eq!(
            json,
            r#"{"peers":2,"rttUs":0,"jitterUs":0,"lossPct":0.0,"bitrateBps":0,"bufferLevelUs":0,"underruns":0,"e2eLatencyUs":0,"e2eP50Us":0,"e2eP95Us":0}"#
        );
    }

    #[test]
    fn peer_and_local_json_shape_match_contract() {
        let peer = PeerView {
            id_short: "3f9a1c0b".into(),
            name: "客厅 R1".into(),
            addr: "192.168.1.23:58290".into(),
            state: PeerState::Handshaking,
            trusted: false,
        };
        assert_eq!(
            serde_json::to_string(&peer).expect("serialize"),
            r#"{"idShort":"3f9a1c0b","name":"客厅 R1","addr":"192.168.1.23:58290","state":"handshaking","trusted":false}"#
        );

        let local = LocalStatus {
            id_short: "00112233".into(),
            name: "书房 PC".into(),
            addr: "0.0.0.0:58290".into(),
            platform: "windows".into(),
        };
        assert_eq!(
            serde_json::to_string(&local).expect("serialize"),
            r#"{"idShort":"00112233","name":"书房 PC","addr":"0.0.0.0:58290","platform":"windows"}"#
        );

        // 唯一的 snake_case 例外：契约就是 `stream_id`。
        assert_eq!(
            serde_json::to_string(&StartSendResult { stream_id: 7 }).expect("serialize"),
            r#"{"stream_id":7}"#
        );
        assert_eq!(
            serde_json::to_string(&PairRequiredPayload {
                id_short: "3f9a1c0b".into(),
                name: "客厅 R1".into(),
                pin: "482913".into(),
            })
            .expect("serialize"),
            r#"{"idShort":"3f9a1c0b","name":"客厅 R1","pin":"482913"}"#
        );
    }
}
