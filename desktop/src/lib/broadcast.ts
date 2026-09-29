import type { MessageKey } from "../i18n";
import type { CommandError } from "../types";

export type BroadcastStatus = "loading" | "waiting" | "starting" | "streaming" | "pausing" | "paused" | "unavailable" | "error" | "manual";

export const broadcastLabels: Record<BroadcastStatus, MessageKey> = {
  loading: "share.loading", waiting: "share.waiting", starting: "share.starting",
  streaming: "share.streaming", pausing: "share.pausing", paused: "share.paused",
  unavailable: "share.unavailable", error: "share.error", manual: "share.manual",
};

export const broadcastHints: Record<BroadcastStatus, MessageKey> = {
  loading: "share.loading_hint", waiting: "share.waiting_hint", starting: "share.starting_hint",
  streaming: "share.streaming_hint", pausing: "share.pausing_hint", paused: "share.paused_hint",
  unavailable: "share.unavailable_hint", error: "share.error_hint", manual: "share.manual_hint",
};

export function broadcastLamp(status: BroadcastStatus): string {
  if (status === "streaming") return "on";
  if (status === "starting" || status === "pausing" || status === "loading") return "busy";
  if (status === "unavailable") return "warn";
  if (status === "error") return "live";
  return "off";
}

/**
 * 一次状态折算要吃的全部输入：`useAudioLink()` 暴露的**原始 flag**，一个都不预处理。
 *
 * 字段名与 `AudioLinkController` 一一对应，调用点可以逐字段直传，不需要中间变量。
 * 为什么除了 boolean 还收 `streamingCount`：同一个计数还要喂给 `broadcastHints` 的
 * `{count}` 占位符 —— 界面上一处算、两处用，才不会出现"计数说 2、状态说没在推"的自相矛盾。
 */
export interface BroadcastStatusInput {
  /** `useAudioLink().broadcastStopping`：停止已发起、对端回执还没到（界面显示"正在停止"）。 */
  broadcastStopping: boolean;
  /** 处于 `streaming` 或 `degraded` 的对端**数量**；`> 0` 即视为正在发送（0 或负值都不算）。 */
  streamingCount: number;
  /** `broadcastPaused`：用户主动暂停了整个共享会话；暂停意图不会被自动推流取消。 */
  broadcastPaused: boolean;
  /**
   * `autoBroadcastReady`：自动推流偏好是否已从设置文件读回。
   *
   * **`false` 表示"还没读完"，不是"开关关着"** —— 所以它落到 `loading` 而不是 `manual`。
   */
  autoBroadcastReady: boolean;
  /** `captureLoading`：采集设备正在枚举。 */
  captureLoading: boolean;
  /**
   * `canStartCapture`：当前选中的音源可用（设备枚举完了、没报错、不是 unavailable 设备）。
   *
   * **不含 `captureLocked`** —— 正在推流（锁定中）时它照样为真，界面显示 `streaming` 而不是 `unavailable`。
   */
  canStartCapture: boolean;
  /** `busyPeer`：正在 start/stop 的对端 id（`null` = 没有）；非 null 即"启动中"。 */
  busyPeer: string | null;
  /** `broadcastError`：最近一次推流动作的失败原因（`null` = 没有）。 */
  broadcastError: CommandError | null;
  /** `autoBroadcast`：「有人接入即自动推流」开关（关掉 = 完全手动模式）。 */
  autoBroadcast: boolean;
  /** `broadcastRequested`：已请求共享的**意图**；真正"在发送"只有对端 `streaming` 状态算数。 */
  broadcastRequested: boolean;
}

/**
 * 把原始 flag 折算成侧栏 / 工具条要显示的 `BroadcastStatus`（纯函数，无 IO、无时钟）。
 *
 * **判断顺序就是优先级，逐条不能重排**：
 * `pausing > streaming > paused > loading > unavailable > starting > error > waiting > manual`。
 *
 * 四处"读起来像 bug，其实是有意的"：
 *
 * 1. `broadcastStopping` 排在 `streamingCount` 前面 —— 停止过程中对端可能还报 `streaming`，
 *    这时必须显示"正在停止"；否则按钮退回"停止共享"，用户会以为没点上；
 * 2. `autoBroadcastReady === false` 落 `loading` —— 它表示**设置还没读回**，与"开关是关的"是两件事；
 * 3. `canStartCapture === false` 落 `unavailable` —— 它只表示**没有可用音源**，不含锁定态；
 * 4. `broadcastError !== null` 落在 `busyPeer` 之后 —— 启动中就算上一次报过错，也先显示"启动中"。
 *
 * 这段逻辑原先写在 `App.tsx:98-105` 的 9 分支三元链里，搬进来时逐分支对齐；
 * `broadcast.test.ts` 用**穷举等价**用例把两边钉死（搬迁前后结果必须逐一相同）。
 */
export function deriveBroadcastStatus(input: BroadcastStatusInput): BroadcastStatus {
  if (input.broadcastStopping) return "pausing";
  if (input.streamingCount > 0) return "streaming";
  if (input.broadcastPaused) return "paused";
  if (!input.autoBroadcastReady || input.captureLoading) return "loading";
  if (!input.canStartCapture) return "unavailable";
  if (input.busyPeer !== null) return "starting";
  if (input.broadcastError !== null) return "error";
  if (input.autoBroadcast || input.broadcastRequested) return "waiting";
  return "manual";
}

/** 侧栏主按钮此刻的样子：语义 + 可用性 + 文案键。 */
export interface BroadcastAffordances {
  /** 主按钮当前是"暂停/停止"语义（true）还是"开始/恢复"语义（false）—— 决定点下去干什么。 */
  canPause: boolean;
  /** 主按钮是否禁用。 */
  disabled: boolean;
  /**
   * 主按钮文案的 **i18n 键**（调用方 `t(label)` 取文案）。
   *
   * 为什么返回键而不是成品文案：这是个纯函数，不该依赖 i18n 的当前语言；
   * 返回键名也让测试不必模拟语言。切语言由组件重渲染负责（`useLocale()` 触发）。
   */
  label: MessageKey;
}

/**
 * 把 `status` 折算成侧栏主按钮的三项：`canPause` / `disabled` / `label`（纯函数，无 IO）。
 *
 * 这段逻辑原先在 `Sidebar.tsx:29-32` 里，和 `deriveBroadcastStatus` 合起来才是一个完整的
 * 广播状态机；拆在两处时，"按钮为什么是灰的"要跳两个文件才看得懂。
 *
 * 两个入参分工不同，**不要试图合并**：
 * - `status` 说的是"正在发生什么"（由 `deriveBroadcastStatus` 算出）；
 * - `canResume` 说的是"用户此刻有没有条件开" —— 即 `canStartCapture && !captureLocked`。
 *   它与 `status` 正交：同样是 `paused`，有音源就能恢复，没音源就只能停在灰按钮上。
 *
 * 规则（与搬迁前逐字一致，顺序即优先级）：
 * - `canPause`：`streaming` / `starting` / `waiting` 这三种状态下主按钮是"停止"语义；
 * - `disabled`：`loading` 与 `pausing` 一律禁用（正在过渡，此时按什么都是错的）；
 *   其余状态取决于能不能开 —— `!canPause && !canResume`；
 * - `label`：`pausing` → `peer.stopping`；能暂停 → `side.stop_broadcast`；
 *   `error` → `share.retry`；`manual` → `share.start`；
 *   其余（`loading` / `paused` / `unavailable`）→ `side.broadcast`。
 *
 * **图标不在本函数职责内**：调用方按 `canPause || status === "pausing"` 选暂停/开始图标
 * （`pausing` 属于"暂停/停止"那一族的展示终态，所以它也算暂停侧）。
 */
export function deriveBroadcastAffordances(status: BroadcastStatus, canResume: boolean): BroadcastAffordances {
  const canPause = status === "streaming" || status === "starting" || status === "waiting";
  const disabled = status === "loading" || status === "pausing" || (!canPause && !canResume);
  const label: MessageKey = status === "pausing" ? "peer.stopping"
    : canPause ? "side.stop_broadcast"
    : status === "error" ? "share.retry"
    : status === "manual" ? "share.start" : "side.broadcast";
  return { canPause, disabled, label };
}
