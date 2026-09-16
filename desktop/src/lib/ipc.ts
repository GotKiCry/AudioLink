/**
 * 外壳 IPC 门面：**前端唯一**碰 `invoke` / `listen` 的地方。
 *
 * 为什么集中：Tauri 的 command 名、参数键、事件名是与 Rust 侧的字符串契约 ——
 * 散落在组件里就会"改了一个忘了一个"。组件只允许 import 本模块。
 *
 * 与 `desktop/src-tauri/src/lib.rs`（command 签名）和 `engine_bridge.rs`（事件名常量）对齐。
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  CaptureDeviceView,
  GroupView,
  LocalStatus,
  PairRequiredPayload,
  PeerView,
  StartSendResult,
  SubmitPinResult,
  TelemetryRow,
  TelemetryView,
} from "../types";

// --- 事件名（契约 §6 冻结；Rust 侧同名常量在 engine_bridge.rs） ---
export const EVENT_PEER = "audiolink://peer";
export const EVENT_TELEMETRY = "audiolink://telemetry";
export const EVENT_PAIR_REQUIRED = "audiolink://pair-required";

/** 契约 §6 的 command 表。参数键名严格照契约（`start_send` 用 `id_short`）。 */
export const api = {
  /** `version` —— 外壳版本号（= Cargo workspace 版本）。 */
  version: (): Promise<string> => invoke<string>("version"),
  /** `local_status` —— 本机身份。 */
  localStatus: (): Promise<LocalStatus> => invoke<LocalStatus>("local_status"),
  /** `list_peers` —— 当前对端列表（首屏水合用；之后靠 `audiolink://peer` 事件）。 */
  listPeers: (): Promise<PeerView[]> => invoke<PeerView[]>("list_peers"),
  listCaptureDevices: (): Promise<CaptureDeviceView[]> => invoke("list_capture_devices"),
  activeCaptureDevice: (): Promise<CaptureDeviceView | null> => invoke("active_capture_device"),
  /** `connect` —— 手工 IP 连接。 */
  connect: (addr: string): Promise<PeerView> => invoke<PeerView>("connect", { addr }),
  /** `start_send` —— 开始推流；返回 `{ stream_id }`。 */
  startSend: (idShort: string, captureDeviceId: string | null): Promise<StartSendResult> =>
    invoke<StartSendResult>("start_send", { id_short: idShort, capture_device_id: captureDeviceId }),
  /** `stop_send` —— 停止推流（无入参，返回 `null`）。 */
  stopSend: (): Promise<null> => invoke<null>("stop_send"),
  /**
   * `submit_pin` —— 提交 6 位配对码。
   *
   * **比契约 §6 的表格多一个 `id_short`**：引擎的 `submit_pin(peer, pin)` 必须知道是哪条会话
   * （PIN 本身没有归属信息，同时可能有多个对端在配对）。契约文档那一行待同步更新。
   */
  submitPin: (idShort: string, pin: string): Promise<SubmitPinResult> =>
    invoke<SubmitPinResult>("submit_pin", { id_short: idShort, pin }),
  /** `telemetry` —— 遥测快照（首屏水合用；之后靠 `audiolink://telemetry` 事件）。 */
  telemetry: (): Promise<TelemetryView> => invoke<TelemetryView>("telemetry"),
  /** `export_telemetry` —— 把前端累积的采样点写成 CSV，返回落盘路径（M2 的日志导出）。 */
  exportTelemetry: (rows: TelemetryRow[]): Promise<string> =>
    invoke<string>("export_telemetry", { rows }),
  listGroups: (): Promise<GroupView[]> => invoke<GroupView[]>("list_groups"),
  createGroup: (idShorts: string[], leadMs: number): Promise<number> =>
    invoke<number>("create_group", { id_shorts: idShorts, lead_ms: leadMs }),
  joinGroup: (idShort: string, groupId: number): Promise<null> =>
    invoke<null>("join_group", { id_short: idShort, group_id: groupId }),
  leaveGroup: (idShort: string, groupId: number): Promise<null> =>
    invoke<null>("leave_group", { id_short: idShort, group_id: groupId }),
};

/**
 * 订阅三个后端事件。
 *
 * 返回值是"取消订阅"的函数；调用方**必须**在组件卸载时调用它
 * （StrictMode 下 effect 会跑两遍，不取消就会出现双份订阅 + 双倍 IPC）。
 */
export function subscribeEvents(handlers: {
  onPeers: (peers: PeerView[]) => void;
  onTelemetry: (view: TelemetryView) => void;
  onPairRequired: (payload: PairRequiredPayload) => void;
  onError: (raw: unknown) => void;
}): () => void {
  let cancelled = false;
  const unlisteners: UnlistenFn[] = [];

  const pending: Promise<UnlistenFn>[] = [
    listen<PeerView[]>(EVENT_PEER, (event) => handlers.onPeers(event.payload)),
    listen<TelemetryView>(EVENT_TELEMETRY, (event) => handlers.onTelemetry(event.payload)),
    listen<PairRequiredPayload>(EVENT_PAIR_REQUIRED, (event) => handlers.onPairRequired(event.payload)),
  ];

  for (const subscription of pending) {
    // 不吞异常：订阅失败（例如 capability 没放开 core:event）必须让用户看见原因
    void subscription
      .then((unlisten) => {
        if (cancelled) {
          unlisten();
          return;
        }
        unlisteners.push(unlisten);
      })
      .catch((raw: unknown) => handlers.onError(raw));
  }

  return () => {
    cancelled = true;
    for (const unlisten of unlisteners) {
      unlisten();
    }
  };
}
