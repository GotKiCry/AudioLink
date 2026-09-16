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
export interface PeerView {
  idShort: string;
  name: string;
  addr: string;
  state: PeerState;
  trusted: boolean;
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

/** 把任意 rejection 归一成 `CommandError`，保证 UI 永远有可显示的原因（不允许"未知错误"）。 */
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
    message: "界面与内核通信失败，请重试；若持续出现请查看日志",
    context: typeof raw === "string" ? raw : String(raw),
  };
}

const PIN_PATTERN = /^\d{6}$/;

/** 配对码是否形如 6 位数字（**输入前置校验**，不是安全校验；真正的校验在内核里）。 */
export function isPinWellFormed(pin: string): boolean {
  return PIN_PATTERN.test(pin);
}

/** 状态文案（`docs/08-ui-spec.md` §1 的状态色语义表；M1 不做双语，只出中文）。 */
export const PEER_STATE_LABEL: Record<PeerState, string> = {
  idle: "空闲",
  handshaking: "连接中",
  streaming: "推送中",
  degraded: "网络不稳",
  failed: "已断开",
};

/** 状态样式：颜色 + 图标（**不允许只靠颜色传达信息**，见 UI 规格 §5 视觉验收清单）。 */
export const PEER_STATE_STYLE: Record<PeerState, { dot: string; text: string; icon: string }> = {
  idle: { dot: "bg-slate-400", text: "text-slate-500", icon: "○" },
  handshaking: { dot: "bg-indigo-500 animate-pulse", text: "text-indigo-600", icon: "◌" },
  streaming: { dot: "bg-emerald-500", text: "text-emerald-600", icon: "▶" },
  degraded: { dot: "bg-amber-500", text: "text-amber-600", icon: "!" },
  failed: { dot: "bg-red-500", text: "text-red-600", icon: "×" },
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
