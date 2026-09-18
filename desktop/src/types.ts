/**
 * 契约 §6 的视图形状（`docs/11-m1-contract.md`）。
 *
 * 为什么手写而不用代码生成：M1 只有 8 个 command / 3 个事件，手写的成本低于引入
 * tauri-specta 之类工具的构建复杂度；**代价是必须与 `desktop/src-tauri/src/view.rs` 对齐**，
 * 那一侧有序列化形状的单元测试钉死字段名，两侧一起改才不会漂。
 *
 * 命名细则（照抄契约，不要"顺手改成 camelCase"）：
 * - `PeerView` / `TelemetryView` / `LocalStatus` 是 camelCase；
 * - `start_send` 的入参是 `{ id_short }`、返回是 `{ stream_id }` —— snake_case。
 */
import { t, type MessageKey } from "./i18n";

/** 会话状态。与 Rust 侧 `view::PeerState` 一一对应（契约 §6 的联合类型）。 */
export type PeerState = "idle" | "handshaking" | "streaming" | "degraded" | "failed";

/** `local_status` 的返回。 */
export interface LocalStatus {
  idShort: string;
  name: string;
  addr: string;
  platform: string;
}

/** Windows 活动输出端点。id 是完整实例 ID，不能用展示名代替。 */
export interface CaptureDeviceView {
  id: string;
  name: string;
  isDefault: boolean;
  isVirtual: boolean;
  sampleRate: number;
  channels: number;
  unavailableReason: string | null;
}

/** `list_peers` 的元素 / `connect` 的返回 / `audiolink://peer` 的元素。 */
/** §13 能力协商结果（界面用的形式：已经是人话，不是位图）。 */
export interface PeerCapabilitiesView {
  /** 本端能做什么。 */
  local: string;
  /** 对端能做什么。 */
  peer: string;
  /** 双方交集 —— 这一对到底能一起做什么。 */
  agreed: string;
  /** 本端有、对端没有的能力（界面据此说明「为什么这个功能用不了」）。 */
  missingOnPeer: string[];
  /** 交集里实际可用的能力键（机器可读，界面用它判断该不该置灰）。 */
  agreedKeys: string[];
  /** 本端有、对端没有的能力键。 */
  missingKeys: string[];
}

export interface PeerView {
  idShort: string;
  name: string;
  addr: string;
  state: PeerState;
  trusted: boolean;
  /** 重连成功次数（0 = 从未重连）：界面据此显示回执。 */
  /** 重连成功次数（Rust 侧总是序列化它；此处声明为可选，因为测试夹具用 Partial<PeerView> 展开构造）。 */
  reconnects?: number;
  /** §13 能力协商结果；null = 还没走完能力交换（握手中就是 null）。 */
  capabilities: PeerCapabilitiesView | null;
}

/** `telemetry` 的返回 / `audiolink://telemetry` 的载荷。单位：µs / bps / 百分数。 */
export interface TelemetryView {
  peers: number;
  rttUs: number;
  jitterUs: number;
  lossPct: number;
  bitrateBps: number;
  bufferLevelUs: number;
  underruns: number;
  e2eLatencyUs: number;
  e2eP50Us: number;
  e2eP95Us: number;
}

/** `audiolink://pair-required` 的载荷。 */
export interface PairRequiredPayload {
  idShort: string;
  name: string;
  pin: string;
}

/** `start_send` 的返回（注意 snake_case）。 */
export interface StartSendResult {
  stream_id: number;
}

/** `submit_pin` 的返回：**总是成功返回**，失败信息在 `reason`。 */
export interface SubmitPinResult {
  ok: boolean;
  reason: string;
}

/** 命令失败的 rejection 形状（Rust 侧 `error::CommandError`）。 */
export interface CommandError {
  code: number;
  message: string;
  context: string;
}

/** 把任意 rejection 归一成 `CommandError`，保证 UI 永远有可显示的原因（不允许t("err.unknown")）。 */
export function toCommandError(raw: unknown): CommandError {
  if (typeof raw === "object" && raw !== null) {
    const candidate = raw as Partial<CommandError>;
    if (typeof candidate.message === "string") {
      return {
        code: typeof candidate.code === "number" ? candidate.code : 0,
        message: candidate.message,
        context: typeof candidate.context === "string" ? candidate.context : "",
      };
    }
  }
  return {
    code: 0,
    message: t("err.ipc"),
    context: typeof raw === "string" ? raw : String(raw),
  };
}

const PIN_PATTERN = /^\d{6}$/;

/** 配对码是否形如 6 位数字（**输入前置校验**，不是安全校验；真正的校验在内核里）。 */
export function isPinWellFormed(pin: string): boolean {
  return PIN_PATTERN.test(pin);
}

/**
 * 状态文案（`docs/08-ui-spec.md` §1 的状态色语义表）。
 *
 * **写成函数而不是常量**：语言可以在运行期切换，而模块级常量会把文案固定在**加载那一刻**的语言上 ——
 * 切换语言后整个界面都变了、只有这几个字不动，这类 bug 一眼看不出来。
 */
export function peerStateLabel(state: PeerState): string {
  const key: Record<PeerState, MessageKey> = {
    idle: "state.idle",
    handshaking: "state.handshaking",
    streaming: "state.streaming",
    degraded: "state.degraded",
    failed: "state.failed",
  };
  return t(key[state]);
}

/** 状态样式：颜色 + 图标（**不允许只靠颜色传达信息**，见 UI 规格 §5 视觉验收清单）。 */
export const PEER_STATE_STYLE: Record<PeerState, { dot: string; text: string; icon: string }> = {
  idle: { dot: "bg-idle", text: "text-text-tertiary", icon: "○" },
  handshaking: { dot: "bg-accent animate-pulse", text: "text-text-secondary", icon: "◌" },
  streaming: { dot: "bg-success", text: "text-success", icon: "▶" },
  degraded: { dot: "bg-caution", text: "text-caution", icon: "!" },
  failed: { dot: "bg-critical", text: "text-critical", icon: "×" },
};

/** µs → ms 显示（1 位小数）。契约里所有时延都是 µs，展示层统一换算。 */
export function usToMs(us: number): string {
  return (us / 1000).toFixed(1);
}

/** bps → kbps 显示（取整）。 */
export function bpsToKbps(bps: number): string {
  return Math.round(bps / 1000).toString();
}

/**
 * 导出用的遥测样本（与 Rust 侧 `view::TelemetryRow` 一一对应）。
 *
 * 为什么不让后端导出 `TelemetryView`：CSV 是**跨版本持久化**的东西，
 * 列名一旦写进用户的文件就不该再跟着界面字段漂 —— 所以这里多一个 `atUnixMs`，
 * 并且 `telemetryRowOf` 的字段与 `TelemetryView` 严格同形（漏一个 TS 就报错）。
 */
/** 同步组的一个成员（M3）：短码与 PeerView.idShort 同一口径。 */
export interface GroupMemberView {
  idShort: string;
  /** §6.5 的时钟质量分级：good / fair / poor。 */
  quality: string;
  /** 时钟偏移估计（对端 − 本机，µs）；还没有估计时为 null。 */
  offsetUs: number | null;
}

/**
 * 临时同步组（list_groups 的元素）。
 *
 * epochId 是**字符串**：内核侧是 u64，而 JS 的安全整数只有 53 位 ——
 * 直接传数字会在前端悄悄丢精度，组基准对不上是最难查的那类 bug。
 */
export interface GroupView {
  groupId: number;
  epochId: string;
  leadMs: number;
  members: GroupMemberView[];
}

/** 对齐结论（M4）：unknown = 读数不足两路；aligned = 跨度在一帧内；drifting = 超过一帧。 */
export type AlignmentVerdict = "unknown" | "aligned" | "drifting";

/** 一路流的「最近一帧编号 ↔ 到达时刻」（M4 多源对齐观测）。 */
export interface StreamAxisView {
  peerShort: string;
  /** 最近一帧的样本编号；null = 本会话还没收到音频。 */
  sampleIndex: number | null;
  /** 那一帧到达本端的时刻（本端单调时钟，毫秒）。 */
  atMs: number;
  /** 用快照的「现在」推算出的当前编号。 */
  indexNow: number | null;
}

/**
 * 多源对齐快照（`alignment` 的返回）。
 *
 * 判据只有一条：用**同一个「现在」**推算各路编号，跨度就是时间轴错位量。
 * 一帧（960 样本 / 20 ms）以内算对齐 —— 这正是 M4 多源混音要保证的东西。
 */
export interface AlignmentView {
  axes: StreamAxisView[];
  /** 各路当前编号的跨度（样本）；null = 有效读数不足两路。 */
  spreadSamples: number | null;
  /** 跨度换算成毫秒。 */
  spreadMs: number | null;
  verdict: AlignmentVerdict;
}

/**
 * 第三方组件声明（`third_party_notices` 的返回）。
 *
 * `available = false` 时 `text` 不是空字符串，而是「怎么生成」的提示 ——
 * 空面板只会让人以为软件坏了。
 */
export interface NoticesView {
  available: boolean;
  /** 从哪里读到的（出问题时一眼看出读的是哪份）。 */
  source: string;
  bytes: number;
  text: string;
}

/**
 * 自动重连设置（`auto_connect_state` 的返回，M5 / FR-31）。
 *
 * `lastPeer` 是上一次**成功连接**的地址；失败的不记 —— 否则开机自动重连会去连一个
 * 已知连不上的地址。
 */
export interface AutoConnectPolicy {
  enabled: boolean;
  lastPeer: string | null;
}

/**
 * 移除设备（`revoke_trust`，FR-18）的结果。
 *
 * 为什么不是 `null`：这是一次**不可逆**的隐私操作，界面必须能如实说出「做了什么」——
 * 尤其「本来就不在信任库里」与「顺手清掉了上次设备记录」是两件不同的事，
 * 返回空值等于让用户无法判断是否生效。
 */
/**
 * 一条**已配对设备**（`list_trusted_peers` 的元素）。
 *
 * 与 `PeerView` 的区别在数据来源：`PeerView` 来自会话表（界面上的对端卡片），
 * 这份来自**信任库**（白名单）—— 换机后残留的旧记录、很久没连过的设备只在这一侧出现，
 * 而它们正是「取消配对」要覆盖的对象。
 * `idShort` 与 `PeerView.idShort` 同口径，所以两处共用同一个移除入口。
 */
export interface TrustedPeerView {
  /** 指纹短码（与 `PeerView.idShort` 同一口径）。 */
  idShort: string;
  /** 配对时记下的展示名（对端自报，仅展示）。 */
  name: string;
  /** `windows` / `android` / `unknown`。 */
  platform: string;
  /** 首次配对成功的 Unix 秒。 */
  pairedAtUnix: number;
}

export interface RevokeTrustResult {
  /** 是否真的从信任库里删掉了（`false` = 本来就不在，幂等）。 */
  removed: boolean;
  /** 是否顺手清掉了「上次设备」记录（它原本指向这台设备）。 */
  forgotLastPeer: boolean;
}

/**
 * `check_update` 的返回（M5 自动更新）。
 *
 * 只有两种**正常**结果：`version === null` = 已是最新；有版本号 = 发现新版本。
 * 第三种「出错」不走这里 —— 它是 `CommandError` rejection（与其它命令同一条路），
 * 失败原因由后端错误层翻译成人话，前端只负责显示，不拼文案。
 */
export interface UpdateCheckView {
  /** 当前运行版本（与 `version` 命令同源）。 */
  currentVersion: string;
  /** 新版本号；null = 已是最新。 */
  version: string | null;
  /** 新版本的发布说明（可能多行）；没有就是 null。 */
  notes: string | null;
  /** 新版本发布日期（YYYY-MM-DD）；没有就是 null。 */
  pubDate: string | null;
}

export interface TelemetryRow {
  /** 采样时刻（Unix 毫秒）。 */
  atUnixMs: number;
  peers: number;
  rttUs: number;
  jitterUs: number;
  lossPct: number;
  bitrateBps: number;
  bufferLevelUs: number;
  underruns: number;
  e2eLatencyUs: number;
  e2eP50Us: number;
  e2eP95Us: number;
}

/** 按界面快照造一行导出样本。 */
export function telemetryRowOf(view: TelemetryView, atUnixMs: number): TelemetryRow {
  return { atUnixMs, ...view };
}

/**
 * 应用背景（z0 壁纸）—— 与 Rust 侧 `settings.rs::BackgroundConfig` 一字对齐。
 *
 * 存在 settings.json 的 `background` 键里（与 locale / autostart 同一条路，不用 localStorage：
 * 背景和语言的持久化在用户眼里没有区别）。`c3` 允许空串 —— 那是「第三个颜色留空」的明确
 * 语义（三色按双色处理），不是缺数据。
 */
export type BackgroundMode = "mono" | "duo" | "tri";
export type BackgroundDir = "diag" | "h" | "v";

export interface BackgroundConfig {
  mode: BackgroundMode;
  /** 第一个颜色（单色模式下就是唯一的颜色）。 */
  c1: string;
  c2: string;
  /** 第三个颜色；空串 = 留空。 */
  c3: string;
  dir: BackgroundDir;
}
