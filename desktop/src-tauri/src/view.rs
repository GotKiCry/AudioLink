//! 桌面端 ⇄ 前端 的**冻结视图形状**（`docs/11-m1-contract.md` §6）。
//!
//! 为什么单独成模块：这些 JSON 字段名是契约（前端按它写、Lead 按它对接引擎），
//! 只在这里定义一次，改字段时不可能"改了一处忘了另一处"。
//! mock 与真实引擎**都必须**产出这些类型 —— 前端不为 mock 定制。
//!
//! 命名注意：`PeerView` / `TelemetryView` / `LocalStatus` 用 camelCase（契约原文如此），
//! 而 `start_send` 的返回 `stream_id` 是 **snake_case**（契约原文如此，不是笔误）。

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

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

/// 同步组的一个成员（M3 交付物 4）：短码与 PeerView 同一口径（指纹前 8 字节 hex）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupMemberView {
    pub id_short: String,
    /// §6.5 的质量分级（`good` / `fair` / `poor`）：UI 据此明示「该设备同步质量差」。
    pub quality: String,
    /// 时钟偏移估计（对端 − 本机，µs）；还没有估计时为 null。
    pub offset_us: Option<i64>,
}

/// 临时同步组（list_groups 的元素）。
///
/// epoch_id 用**十六进制字符串**传给前端：它是 u64，而 JS 的安全整数只有 53 位 ——
/// 直接传数字会在前端悄悄丢精度，组基准恰好对不上时是最难查的那类 bug。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    pub group_id: u32,
    pub epoch_id: String,
    pub lead_ms: u32,
    pub members: Vec<GroupMemberView>,
}

/// 对端卡片 + list_peers 的元素。
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
    /// §13 能力协商结果；`None` = 还没走完能力交换（握手阶段的对端就是 `None`）。
    pub capabilities: Option<PeerCapabilitiesView>,
    /// 重连成功次数（FR-27 回执；0 表示从未重连）。界面据此显示已重连几次。
    pub reconnects: u64,
}

/// §13 能力协商结果（给界面看的形式）。
///
/// 为什么在这里把位图**翻成人话**：位图是协议用的，界面要回答的是「这台对端能做什么、缺什么」。
/// 把 `1 << 3` 丢给前端，等于让每个前端各实现一遍解释，迟早解释不一致。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerCapabilitiesView {
    /// 本端能力（人话列表）。
    pub local: String,
    /// 对端能力（人话列表）。
    pub peer: String,
    /// 双方交集 —— 也就这一对到底能一起做什么。
    pub agreed: String,
    /// 本端有、对端没有的能力（逐项人话；界面据此说明原因）。
    pub missing_on_peer: Vec<String>,
    /// 交集里实际可用的能力键（**机器可读**，界面用它判断该不该置灰）。
    pub agreed_keys: Vec<String>,
    /// 本端有、对端没有的能力键（同样是机器可读）。
    pub missing_keys: Vec<String>,
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

/// 导出用的遥测样本：前端累积的**历史**，一行一个采样点。
///
/// 为什么不直接复用 [`TelemetryView`]：导出的 CSV 是**跨版本持久化**的东西，
/// 列名一旦写进用户的文件就不该再跟着界面字段漂 —— 所以这里把必要字段平铺一份，
/// 并在 [`TelemetryRow::from_view`] 里做唯一的映射。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryRow {
    /// 采样时刻（Unix 毫秒）。
    pub at_unix_ms: u64,
    /// 已登记对端数。
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
    /// 当前端到端延迟（µs）。
    pub e2e_latency_us: u64,
    /// 端到端延迟 P50（µs）。
    pub e2e_p50_us: u64,
    /// 端到端延迟 P95（µs）。
    pub e2e_p95_us: u64,
}

impl TelemetryRow {
    /// 由界面快照 + 采样时刻构造。
    ///
    /// 前端在 TS 侧做同一个映射（它才是调用方）；这里是那份映射的 **Rust 镜像**，
    /// 只在测试里用来钉住「导出列与界面字段一一对应」——字段漏一个，编译期就红。
    #[cfg(test)]
    pub fn from_view(at_unix_ms: u64, view: &TelemetryView) -> Self {
        Self {
            at_unix_ms,
            peers: view.peers,
            rtt_us: view.rtt_us,
            jitter_us: view.jitter_us,
            loss_pct: view.loss_pct,
            bitrate_bps: view.bitrate_bps,
            buffer_level_us: view.buffer_level_us,
            underruns: view.underruns,
            e2e_latency_us: view.e2e_latency_us,
            e2e_p50_us: view.e2e_p50_us,
            e2e_p95_us: view.e2e_p95_us,
        }
    }
}

/// CSV 表头（**列名即契约**：用户在表格软件里看到的就是这一行）。
pub const TELEMETRY_CSV_HEADER: &str = "at_unix_ms,peers,rtt_us,jitter_us,loss_pct,bitrate_bps,buffer_level_us,underruns,e2e_latency_us,e2e_p50_us,e2e_p95_us";

/// 把遥测历史渲染成 CSV。
///
/// **纯函数**：命令层只负责挑路径与落盘，于是「导出内容对不对」这一半可以完整单测。
/// 字段全是数字，不需要引号转义（真要加文本列时再补，而不是提前写一套没人用的转义）。
pub fn render_telemetry_csv(rows: &[TelemetryRow]) -> String {
    let mut out = String::with_capacity(TELEMETRY_CSV_HEADER.len() + rows.len() * 96 + 1);
    out.push_str(TELEMETRY_CSV_HEADER);
    out.push('\n');
    for row in rows {
        let _ = writeln!(
            out,
            "{},{},{},{},{:.2},{},{},{},{},{},{}",
            row.at_unix_ms,
            row.peers,
            row.rtt_us,
            row.jitter_us,
            row.loss_pct,
            row.bitrate_bps,
            row.buffer_level_us,
            row.underruns,
            row.e2e_latency_us,
            row.e2e_p50_us,
            row.e2e_p95_us,
        );
    }
    out
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
            reconnects: 0,
            name: "客厅 R1".into(),
            addr: "192.168.1.23:58290".into(),
            state: PeerState::Handshaking,
            trusted: false,
            capabilities: None,
        };
        assert_eq!(
            serde_json::to_string(&peer).expect("serialize"),
            r#"{"idShort":"3f9a1c0b","name":"客厅 R1","addr":"192.168.1.23:58290","state":"handshaking","trusted":false,"capabilities":null,"reconnects":0}"#
        );

        // 协商完成后的形状：字段名与嵌套名都要钉住 —— 前端按 `agreedKeys` / `missingKeys`
        // 决定置灰，按 `missingOnPeer` 写说明；这几个名字漂一个，界面就静默失效。
        let negotiated = PeerView {
            capabilities: Some(PeerCapabilitiesView {
                local: "Opus 编码、音频采集".into(),
                peer: "Opus 编码、音频播放".into(),
                agreed: "Opus 编码".into(),
                missing_on_peer: vec!["音频采集".into()],
                agreed_keys: vec!["opus".into()],
                missing_keys: vec!["capture".into()],
            }),
            ..peer
        };
        assert_eq!(
            serde_json::to_string(&negotiated).expect("serialize"),
            r#"{"idShort":"3f9a1c0b","name":"客厅 R1","addr":"192.168.1.23:58290","state":"handshaking","trusted":false,"capabilities":{"local":"Opus 编码、音频采集","peer":"Opus 编码、音频播放","agreed":"Opus 编码","missingOnPeer":["音频采集"],"agreedKeys":["opus"],"missingKeys":["capture"]},"reconnects":0}"#
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

    fn sample_view() -> TelemetryView {
        TelemetryView {
            peers: 2,
            rtt_us: 1_500,
            jitter_us: 250,
            loss_pct: 0.25,
            bitrate_bps: 320_000,
            buffer_level_us: 40_000,
            underruns: 3,
            e2e_latency_us: 88_000,
            e2e_p50_us: 85_000,
            e2e_p95_us: 120_000,
        }
    }

    /// 导出的 CSV：表头固定、一行一个采样点、小数位固定、以换行收尾。
    #[test]
    fn telemetry_csv_renders_header_and_rows() {
        let rows = [TelemetryRow::from_view(1_700_000_000_000, &sample_view())];
        let csv = render_telemetry_csv(&rows);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 2, "表头 + 1 行");
        assert_eq!(lines[0], TELEMETRY_CSV_HEADER);
        assert_eq!(
            lines[1],
            // 丢包列固定两位小数（0.25% → `0.25`）；**不要**用 0.105 这种值，
            // 二进制浮点的 `{:.2}` 不保证按十进制"四舍五入"（实测得到 0.10）。
            "1700000000000,2,1500,250,0.25,320000,40000,3,88000,85000,120000"
        );
        assert!(
            csv.ends_with('\n'),
            "文件以换行收尾，表格软件才不会吃掉最后一行"
        );
    }

    #[test]
    fn telemetry_csv_without_samples_is_header_only() {
        assert_eq!(
            render_telemetry_csv(&[]),
            format!("{TELEMETRY_CSV_HEADER}\n")
        );
    }

    /// 前端发来的行形状（camelCase）必须能被反序列化 —— 字段名漂了这里就红。
    #[test]
    fn telemetry_row_round_trips_through_frontend_json() {
        let raw = r#"{"atUnixMs":7,"peers":2,"rttUs":1500,"jitterUs":250,"lossPct":0.5,"bitrateBps":160000,"bufferLevelUs":40000,"underruns":1,"e2eLatencyUs":88000,"e2eP50Us":85000,"e2eP95Us":120000}"#;
        let row: TelemetryRow = serde_json::from_str(raw).expect("deserialize");
        assert_eq!(row.at_unix_ms, 7);
        assert_eq!(row.peers, 2);
        assert_eq!(row.e2e_p95_us, 120_000);
        assert_eq!(
            serde_json::to_string(&TelemetryRow::from_view(42, &TelemetryView::zeros(1)))
                .expect("serialize"),
            r#"{"atUnixMs":42,"peers":1,"rttUs":0,"jitterUs":0,"lossPct":0.0,"bitrateBps":0,"bufferLevelUs":0,"underruns":0,"e2eLatencyUs":0,"e2eP50Us":0,"e2eP95Us":0}"#
        );
    }
}

// ---------------------------------------------------------------------------
// M4：多源对齐观测（docs/39）
// ---------------------------------------------------------------------------

use audiolink_engine::StreamAxis;

/// 一帧的样本数（20 ms @ 48 kHz）。对齐判据的分母。
pub const ALIGNMENT_FRAME_SAMPLES: u32 = 960;

/// 一路流的「最近一帧编号 ↔ 到达时刻」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamAxisView {
    /// 对端短码。
    pub peer_short: String,
    /// 最近一帧的样本编号；`None` = 本会话还没收到音频。
    pub sample_index: Option<u32>,
    /// 那一帧到达本端的时刻（本端单调时钟，毫秒）。
    pub at_ms: u32,
    /// 用快照的「现在」推算出的当前编号。
    pub index_now: Option<u32>,
}

/// 对齐结论（与前端联合类型一一对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AlignmentVerdict {
    /// 有效读数不足两路。
    Unknown,
    /// 两路跨度在一帧以内 —— 采样级对齐达标。
    Aligned,
    /// 跨度超过一帧：要么没广播过共同基准，要么基准没生效。
    Drifting,
}

/// 多源对齐快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlignmentView {
    /// 各路读数。
    pub axes: Vec<StreamAxisView>,
    /// 各路当前编号的跨度（样本）；`None` = 有效读数不足两路。
    pub spread_samples: Option<u32>,
    /// 跨度换算成毫秒。
    pub spread_ms: Option<u32>,
    /// 结论。
    pub verdict: AlignmentVerdict,
}

/// 纯函数：把引擎给的各路读数折成界面要的形状。
///
/// 判据只有一条 —— **用同一个「现在」推算各路编号，跨度就是时间轴错位量**。
/// 一帧以内算对齐（采样级对齐的目标就是一帧内），超过就是错位（多半是没广播过共同基准）。
pub fn alignment_view(axes: &[StreamAxis], now_ms: u32) -> AlignmentView {
    let views: Vec<StreamAxisView> = axes
        .iter()
        .map(|axis| StreamAxisView {
            peer_short: axis.peer.short(),
            sample_index: axis.sample_index,
            at_ms: axis.at_ms,
            index_now: axis.index_at(now_ms),
        })
        .collect();

    let mut indices: Vec<u32> = views.iter().filter_map(|view| view.index_now).collect();
    indices.sort_unstable();
    let spread_samples = if indices.len() < 2 {
        None
    } else {
        let min = indices.first().copied().unwrap_or(0);
        let max = indices.last().copied().unwrap_or(0);
        Some(max.wrapping_sub(min))
    };
    let verdict = match spread_samples {
        None => AlignmentVerdict::Unknown,
        Some(samples) if samples <= ALIGNMENT_FRAME_SAMPLES => AlignmentVerdict::Aligned,
        Some(_) => AlignmentVerdict::Drifting,
    };
    AlignmentView {
        axes: views,
        spread_samples,
        spread_ms: spread_samples.map(|samples| samples / 48),
        verdict,
    }
}

#[cfg(test)]
mod alignment_tests {
    use super::*;
    use audiolink_types::NodeId;

    fn axis(seed: u8, sample_index: Option<u32>, at_ms: u32) -> StreamAxis {
        StreamAxis {
            peer: NodeId::from_bytes([seed; 32]),
            sample_index,
            at_ms,
        }
    }

    #[test]
    fn one_stream_cannot_be_judged() {
        let view = alignment_view(&[axis(1, Some(48_000), 1_000)], 1_050);
        assert_eq!(view.axes.len(), 1);
        assert_eq!(view.axes[0].index_now, Some(50_400), "50 ms = 2400 样本");
        assert_eq!(view.spread_samples, None);
        assert_eq!(view.verdict, AlignmentVerdict::Unknown);
    }

    #[test]
    fn aligned_streams_land_inside_one_frame() {
        // 两路最近一帧相差 144 样本（3 ms）—— 正是 M4 实测对齐后的样子。
        let view = alignment_view(
            &[axis(1, Some(48_000), 1_000), axis(2, Some(48_144), 1_000)],
            1_000,
        );
        assert_eq!(view.spread_samples, Some(144));
        assert_eq!(view.spread_ms, Some(3));
        assert_eq!(view.verdict, AlignmentVerdict::Aligned);
    }

    #[test]
    fn streams_far_apart_are_reported_as_drifting() {
        // 两路各自从启流瞬间编号、相差 1.5 s → 72048 样本。
        let view = alignment_view(
            &[axis(1, Some(0), 1_000), axis(2, Some(72_048), 1_000)],
            1_000,
        );
        assert_eq!(view.spread_samples, Some(72_048));
        assert_eq!(view.spread_ms, Some(1_501));
        assert_eq!(view.verdict, AlignmentVerdict::Drifting);
    }

    #[test]
    fn streams_without_audio_are_unknown() {
        let view = alignment_view(&[axis(1, None, 0), axis(2, None, 0)], 1_000);
        assert_eq!(view.spread_samples, None);
        assert_eq!(view.verdict, AlignmentVerdict::Unknown);
    }

    #[test]
    fn one_silent_stream_still_cannot_be_judged() {
        let view = alignment_view(&[axis(1, Some(48_000), 1_000), axis(2, None, 0)], 1_000);
        assert_eq!(view.spread_samples, None);
        assert_eq!(view.verdict, AlignmentVerdict::Unknown);
    }
}

// ---------------------------------------------------------------------------
// M5：第三方声明的投放（用户看得见，而不是只躺在仓库里）
// ---------------------------------------------------------------------------

/// 找不到声明时给用户的提示 —— 必须包含**怎么生成**，而不是一句「文件不存在」。
pub const NOTICES_MISSING_HINT: &str =
    "第三方组件声明尚未生成。在仓库根执行：pwsh tools/license-audit.ps1 -Notices";

/// 第三方组件声明的可读状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticesView {
    /// 是否拿到了声明内容。
    pub available: bool,
    /// 从哪里读到的（展示给用户，出问题时能一眼看出读的是哪份）。
    pub source: String,
    /// 字节数（用户据此判断要不要展开）。
    pub bytes: usize,
    /// 全文；`available = false` 时是给用户的提示语。
    pub text: String,
}

/// 纯函数：把「找到的内容」折成界面要的形状。
///
/// `found` 是 `(来源标签, 全文)`；`None` 表示两处都没找到 —— 这时**不返回空字符串**，
/// 而是返回「怎么生成」的提示：空面板只会让人以为软件坏了。
#[must_use]
pub fn notices_view(found: Option<(String, String)>) -> NoticesView {
    match found {
        Some((source, text)) => NoticesView {
            available: true,
            source,
            bytes: text.len(),
            text,
        },
        None => NoticesView {
            available: false,
            source: String::new(),
            bytes: 0,
            text: NOTICES_MISSING_HINT.to_string(),
        },
    }
}

#[cfg(test)]
mod notices_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn found_notices_report_source_and_size() {
        let view = notices_view(Some((
            "资源目录".to_string(),
            "# 声明\n\n组件清单".to_string(),
        )));
        assert!(view.available);
        assert_eq!(view.source, "资源目录");
        assert_eq!(view.bytes, "# 声明\n\n组件清单".len());
        assert!(view.text.starts_with("# 声明"));
    }

    #[test]
    fn missing_notices_explain_how_to_generate() {
        // 空面板只会让人以为软件坏了：找不到时必须给出**怎么生成**。
        let view = notices_view(None);
        assert!(!view.available);
        assert_eq!(view.bytes, 0);
        assert!(view.text.contains("license-audit.ps1 -Notices"));
        assert_eq!(view.text, NOTICES_MISSING_HINT);
    }
}
