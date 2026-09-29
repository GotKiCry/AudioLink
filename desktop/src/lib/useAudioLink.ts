/**
 * 桌面端界面状态：把 command/event 收敛成一个 hook，组件只读状态、只调动作。
 *
 * 数据来源（契约 §6）：
 * - 首屏水合：`version` / `local_status` / `list_peers` / `telemetry` 各一次；
 * - 之后持续推进：`audiolink://peer`（变化即推）、`audiolink://telemetry`（500 ms 节流）。
 *
 * 为什么不引入状态库：M1 的状态就 6 个字段，React 自带 useState 足够；
 * 引 zustand/jotai 只会多一层需要同步的真相（而且遥测是"覆盖式最新值"，天然适合 setState）。
 *
 * ## 内部结构（对外仍是**一个** hook、**一份**契约）
 *
 * `AudioLinkController` 是冻结的对外契约，一个字段都不能变；实现的域边界在 `src/lib/state/`：
 *
 * | 文件 | 管什么 |
 * |---|---|
 * | `state/shell.ts` | 版本 / 身份 / 对端 / 遥测流 / 错误与回执 + 首屏水合 + 事件订阅 + 自动重连 + `connect` |
 * | `state/settings.ts` | 自动推流偏好的持久化（含"设置读回前不许自动推流"） |
 * | `state/broadcast.ts` | 推流状态机：意图 / 暂停 / 停止 / 忙碌 / 错误 + 单路发送的同步占位 |
 * | `state/capture.ts` | 采集设备枚举、选择、当前设备、`canStartCapture` |
 * | `state/autoStart.ts` | 自动推流决策（条件横跨采集与推流两个域，所以单独一个文件） |
 * | `state/groups.ts` | 同步组（M3 / FR-22） |
 * | `state/alignment.ts` | 多源对齐快照（M4，1 Hz 轮询） |
 * | `state/telemetryExport.ts` | 遥测历史导出 CSV |
 * | `state/shared.ts` | 跨域共享的写入口与桥类型（`SetError` / `SendGate` / `PeersTap` …） |
 * | `state/reductions.ts` | 两个纯归约（`appendTelemetryRow` / `upsert`），本文件原样 re-export |
 *
 * 这个文件只剩三件事：**契约、桥、编排**。
 *
 * 为什么域之间还会咬合：采集的可用性喂给推流，推流的锁定态喂给采集 —— 是**双向**的。
 * 环用两个桥打破，都摆在本文件的编排函数里：
 * - `sendGate`：采集侧写 `canStartCapture` / `selectedCaptureId` / 刷新函数，推流侧在**事件回调**里读；
 * - `peersTap` / `groupsTap`：事件订阅只注册一次，回调必须从 ref 里取**最新**实现，
 *   否则它会永远指向首帧那份闭包 —— 那正是"点了没反应"类问题的来源。
 */

import { useEffect, useRef } from "react";

import { useAlignmentState } from "./state/alignment";
import { useAutoBroadcastStart } from "./state/autoStart";
import { useBroadcastState } from "./state/broadcast";
import { useCaptureState } from "./state/capture";
import { useGroupsState } from "./state/groups";
import { useSettingsState } from "./state/settings";
import { useShellState } from "./state/shell";
import { useTelemetryExport } from "./state/telemetryExport";
import type { GroupsTap, PeersTap, SendGate } from "./state/shared";
import type {
  AlignmentView,
  CaptureDeviceView,
  CommandError,
  GroupView,
  LocalStatus,
  PeerView,
  TelemetryRow,
  TelemetryView,
} from "../types";

/**
 * 纯归约与常量仍由**本模块**对外提供（`reductions.test.ts` 的 import 路径不变）。
 *
 * 实现搬去 `state/reductions.ts` 是唯一能让 `state/shell.ts` 直接用它们而不成环的办法：
 * 否则就是 `useAudioLink` → `state/shell` → `useAudioLink`。对外看不出位置变化。
 */
export { TELEMETRY_HISTORY_LIMIT, appendTelemetryRow, upsert } from "./state/reductions";

export interface AudioLinkController {
  /** 外壳版本号（`version` command）。 */
  version: string;
  /** 本机身份；水合完成前为 null。 */
  local: LocalStatus | null;
  /** 对端列表（事件持续推进）。 */
  peers: PeerView[];
  /** 最近一次遥测快照；无会话时是内核给的全零值。 */
  telemetry: TelemetryView | null;
  /** 遥测历史（500 ms 一个采样点，最多保留 `TELEMETRY_HISTORY_LIMIT` 条）—— 曲线与导出都用它。 */
  telemetryHistory: TelemetryRow[];
  /** 同步组列表（M3 / FR-22）：引擎侧账本是权威，UI 只做展示。 */
  groups: GroupView[];
  /** 组操作进行中（建组 / 加入 / 退出）。 */
  groupBusy: boolean;
  /** §4.1：调某台对端的音量（0.0–2.0）。 */
  setPeerGain: (idShort: string, gain: number) => Promise<void>;
  refreshGroups: () => Promise<void>;
  /** M4 多源对齐快照（1 Hz 轮询；没有对端时为 null）。 */
  alignment: AlignmentView | null;
  /** M4 广播共同基准进行中。 */
  alignmentBusy: boolean;
  /** M4：本机作为接收端广播共同时间基准（leadMs 是给发送端的准备时间）。 */
  broadcastEpoch: (leadMs: number) => Promise<void>;
  joinGroup: (idShort: string, groupId: number) => Promise<void>;
  leaveGroup: (idShort: string, groupId: number) => Promise<void>;
  createGroup: (idShorts: string[], leadMs: number) => Promise<void>;
  /** 把当前遥测历史导出成 CSV；成功时用 `notice` 报出落盘路径。 */
  exportTelemetryLog: () => Promise<void>;
  /** 导出进行中（按钮禁用用）。 */
  exportingTelemetry: boolean;
  /** 最近一次失败的原因（连接失败、推流被拒…）。 */
  error: CommandError | null;
  /**
   * 动作回执（非错误）：例如「已导出 {count} 个采样点」「{peer} 音量已设为 {pct}%」
   * 「已自动连接上次的主机」。它们是动作的**结果**，不是失败 ——
   * 用红色错误横幅表达会让用户以为出了问题。
   */
  notice: string | null;
  /** `connect` 进行中（按钮禁用用）。 */
  connecting: boolean;
  /** 正在 start/stop 的对端 id（防连点；null = 无）。 */
  busyPeer: string | null;
  captureDevices: CaptureDeviceView[];
  captureLoading: boolean;
  captureError: CommandError | null;
  selectedCaptureId: string;
  activeCapture: CaptureDeviceView | null;
  captureLocked: boolean;
  canStartCapture: boolean;
  selectCapture: (id: string) => void;
  refreshCaptureDevices: () => Promise<void>;

  connect: (addr: string) => Promise<boolean>;
  /** 单台推流（对端卡片上的按钮）—— 也是自动推流唯一允许走的入口，不另造旁路。 */
  startSend: (idShort: string) => Promise<void>;
  stopSend: () => Promise<void>;
  /** 已请求共享；只有真实的对端 streaming 状态才能代表正在发送。 */
  broadcastRequested: boolean;
  /** 暂停整个共享会话，后续接入与重连都不能自行取消。 */
  broadcastPaused: boolean;
  broadcastStopping: boolean;
  broadcastError: CommandError | null;
  autoBroadcastReady: boolean;
  /** 恢复共享。无设备时重新进入等待连接。 */
  startBroadcast: (idShorts: string[]) => void;
  /** 暂停整个共享会话，保留设备连接。 */
  stopBroadcast: () => Promise<void>;
  /**
   * 「有人接入时自动开始推流」：默认开，存在设置文件里（关掉 = 完全回到手动模式）。
   */
  autoBroadcast: boolean;
  /** 开关正在落盘（界面禁用用）。 */
  autoBroadcastBusy: boolean;
  setAutoBroadcast: (enabled: boolean) => void;
  dismissError: () => void;
  dismissNotice: () => void;
}

/**
 * 对外唯一入口：把八个子 hook 按依赖顺序串起来，再按 `AudioLinkController` 逐字段组装。
 *
 * 顺序不是随意的 —— 变的是"谁能看见谁"：
 * shell 先立（其他人要它的 setError / setNotice / peers）
 * → settings（推流要读开关）
 * → broadcast（采集要读它的锁定态）
 * → capture（推流要读它的门控，经 `sendGate` 桥）
 * → 自动推流决策（两边都齐了）
 * → 组 / 对齐 / 导出（彼此独立，只依赖 shell 的写入口）。
 */
export function useAudioLink(): AudioLinkController {
  // 桥 1：采集侧派生值 → 推流侧（推流只在事件回调里读，见 SendGate 的说明）。
  const sendGate = useRef<SendGate>({
    canStartCapture: false,
    selectedCaptureId: "",
    refreshActiveCapture: async () => undefined,
  });
  // 桥 2：订阅回调要调的后置域动作（订阅只注册一次，必须取最新实现）。
  const peersTap: PeersTap = useRef<(next: PeerView[]) => void>(() => undefined);
  const groupsTap: GroupsTap = useRef<() => void>(() => undefined);

  const shell = useShellState(peersTap, groupsTap);
  const settings = useSettingsState(shell.setError);
  const broadcast = useBroadcastState(sendGate, shell.setError);

  const captureLocked = broadcast.broadcastStopping || broadcast.busyPeer !== null ||
    shell.peers.some((peer) => peer.state === "streaming" || peer.state === "degraded");
  const capture = useCaptureState(captureLocked, shell.setError);

  // 门控镜像：必须排在自动推流 effect **之前**，这样同一个提交里推流读到的是这一帧的值。
  useEffect(() => {
    sendGate.current = {
      canStartCapture: capture.canStartCapture,
      selectedCaptureId: capture.selectedCaptureId,
      refreshActiveCapture: capture.refreshActiveCapture,
    };
  }, [capture.canStartCapture, capture.selectedCaptureId, capture.refreshActiveCapture]);

  useAutoBroadcastStart({
    peers: shell.peers,
    autoBroadcast: settings.autoBroadcast,
    autoBroadcastReady: settings.autoBroadcastReady,
    broadcastRequested: broadcast.broadcastRequested,
    broadcastPaused: broadcast.broadcastPaused,
    canStartCapture: capture.canStartCapture,
    captureLocked,
  }, broadcast.autoStart);

  const groups = useGroupsState(shell.setError, shell.setNotice);
  const alignment = useAlignmentState(shell.peers.length, shell.setError, shell.setNotice);
  const telemetryExport = useTelemetryExport(shell.telemetryHistory, shell.setError, shell.setNotice);

  // 桥 2 的镜像：两个动作都是空依赖的 useCallback，所以这个 effect 只会跑一次。
  useEffect(() => {
    peersTap.current = broadcast.reconcilePeers;
    groupsTap.current = groups.refreshGroups;
  }, [broadcast.reconcilePeers, groups.refreshGroups]);

  return {
    version: shell.version,
    local: shell.local,
    peers: shell.peers,
    telemetry: shell.telemetry,
    telemetryHistory: shell.telemetryHistory,
    groups: groups.groups,
    groupBusy: groups.groupBusy,
    alignment: alignment.alignment,
    alignmentBusy: alignment.alignmentBusy,
    broadcastEpoch: alignment.broadcastEpoch,
    setPeerGain: shell.setPeerGain,
    refreshGroups: groups.refreshGroups,
    createGroup: groups.createGroup,
    joinGroup: groups.joinGroup,
    leaveGroup: groups.leaveGroup,
    exportTelemetryLog: telemetryExport.exportTelemetryLog,
    exportingTelemetry: telemetryExport.exportingTelemetry,
    error: shell.error,
    notice: shell.notice,
    connecting: shell.connecting,
    busyPeer: broadcast.busyPeer,
    captureDevices: capture.captureDevices,
    captureLoading: capture.captureLoading,
    captureError: capture.captureError,
    selectedCaptureId: capture.selectedCaptureId,
    activeCapture: capture.activeCapture,
    captureLocked,
    canStartCapture: capture.canStartCapture,
    selectCapture: capture.selectCapture,
    refreshCaptureDevices: capture.refreshCaptureDevices,
    connect: shell.connect,
    startSend: broadcast.startSend,
    stopSend: broadcast.stopSend,
    broadcastRequested: broadcast.broadcastRequested,
    broadcastPaused: broadcast.broadcastPaused,
    broadcastStopping: broadcast.broadcastStopping,
    broadcastError: broadcast.broadcastError,
    autoBroadcastReady: settings.autoBroadcastReady,
    startBroadcast: broadcast.startBroadcast,
    stopBroadcast: broadcast.stopBroadcast,
    autoBroadcast: settings.autoBroadcast,
    autoBroadcastBusy: settings.autoBroadcastBusy,
    setAutoBroadcast: settings.setAutoBroadcast,
    dismissError: shell.dismissError,
    dismissNotice: shell.dismissNotice,
  };
}
