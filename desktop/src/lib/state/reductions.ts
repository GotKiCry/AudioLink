/**
 * 两个纯归约：遥测曲线推进（`appendTelemetryRow`）与对端列表乐观更新（`upsert`）。
 *
 * 为什么在这里、而不是留在 `useAudioLink.ts`：`state/useShellState` 要用它们，
 * 而反向 `import "../useAudioLink"` 会成环（`useAudioLink` → `state/shell` → `useAudioLink`）。
 * `useAudioLink.ts` 仍然**原样 re-export** 这三个名字，所以对外（含 `reductions.test.ts`）
 * 看不出位置变化 —— 这一层只是让依赖方向保持单向。
 */

import { telemetryRowOf, type PeerView, type TelemetryRow, type TelemetryView } from "../../types";

/**
 * 遥测历史的长度上限：500 ms 一个采样点 × 600 = **5 分钟**曲线。
 *
 * 为什么是 5 分钟：这是「看出趋势、又不用翻页」的尺度；更长的历史由导出的 CSV 承担。
 */
export const TELEMETRY_HISTORY_LIMIT = 600;

/**
 * 追加一个采样点并截断到上限（纯函数）。
 *
 * 导出只为单测（`src/lib/reductions.test.ts`）：**截断方向**（丢最旧、留最新）是
 * 界面上看不出来的 —— 写反了曲线只是「冻结在最初 5 分钟」，不报错、不崩溃。
 * 注意：这里不注入时钟（`Date.now()` 在调用点），所以断言只钉顺序与长度，不钉时刻。
 */
export function appendTelemetryRow(history: TelemetryRow[], view: TelemetryView): TelemetryRow[] {
  const next = [...history, telemetryRowOf(view, Date.now())];
  return next.length > TELEMETRY_HISTORY_LIMIT
    ? next.slice(next.length - TELEMETRY_HISTORY_LIMIT)
    : next;
}

/**
 * 把单个对端并入列表（事件到达前先本地乐观更新，避免"点了没反应"）。
 *
 * 导出只为单测：它必须是**不可变**更新 —— 直接改数组元素会让 React 拿不到新引用，
 * 卡片状态与按钮就停在旧值上（点了没反应，且不报错）。
 */
export function upsert(peers: PeerView[], peer: PeerView): PeerView[] {
  const index = peers.findIndex((item) => item.idShort === peer.idShort);
  if (index < 0) {
    return [...peers, peer];
  }
  const next = [...peers];
  next[index] = peer;
  return next;
}
