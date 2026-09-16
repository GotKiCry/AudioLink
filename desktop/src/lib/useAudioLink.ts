/**
 * 桌面端界面状态：把 command/event 收敛成一个 hook，组件只读状态、只调动作。
 *
 * 数据来源（契约 §6）：
 * - 首屏水合：`version` / `local_status` / `list_peers` / `telemetry` 各一次；
 * - 之后持续推进：`audiolink://peer`（变化即推）、`audiolink://telemetry`（500 ms 节流）、
 *   `audiolink://pair-required`（配对请求，不节流）。
 *
 * 为什么不引入状态库：M1 的状态就 6 个字段，React 自带 useState 足够；
 * 引 zustand/jotai 只会多一层需要同步的真相（而且遥测是"覆盖式最新值"，天然适合 setState）。
 */

import { useCallback, useEffect, useRef, useState } from "react";

import { api, subscribeEvents } from "./ipc";
import {
  toCommandError,
  type AlignmentView,
  type CommandError,
  type CaptureDeviceView,
  type GroupView,
  type LocalStatus,
  type PairRequiredPayload,
  type PeerView,
  type TelemetryRow,
  type TelemetryView,
  telemetryRowOf,
} from "../types";

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
  /** 待处理的配对请求；非 null 时 UI 必须弹 PIN 输入框。 */
  pairRequest: PairRequiredPayload | null;
  /** 配对提交失败的原因（来自 `submit_pin` 的 `reason`，人话整句）。 */
  pairReason: string;
  /** 最近一次失败的原因（连接失败、推流被拒…）。 */
  error: CommandError | null;
  /**
   * 提示（非错误）：目前只有一种 —— `connect` 返回 `1002 NOT_PAIRED`。
   * 那**不是失败**：会话与命令通道还活着（见 `Engine::connect` 文档），
   * 用户要做的是去输配对码。用错误横幅表达会误导用户去重连。
   */
  notice: string | null;
  /**
   * 需要**本机输入**配对码的对端（来自 `PinNeeded`）。
   * 用途：弹窗被"稍后再说"关掉后，用户还得有一条路把输入框叫回来 ——
   * 而 `PinNeeded` 是一次性事件，不会重发。
   * 接收端（对端来输码）不在这个集合里，卡片上也不会出现"输入配对码"。
   */
  pairableIds: string[];
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
  startSend: (idShort: string) => Promise<void>;
  stopSend: () => Promise<void>;
  /** `idShort` 决定是哪条会话（可能同时有多个对端在配对）。 */
  submitPin: (idShort: string, pin: string) => Promise<boolean>;
  /** 手动把某个对端的配对输入框叫回来（配合 `pairableIds` 使用）。 */
  beginPairing: (idShort: string) => void;
  dismissError: () => void;
  dismissNotice: () => void;
  dismissPairRequest: () => void;
}

/** `1002 NOT_PAIRED`：不是失败，而是"请去输配对码"。 */
const CODE_NOT_PAIRED = 1002;

/**
 * 遥测历史的长度上限：500 ms 一个采样点 × 600 = **5 分钟**曲线。
 *
 * 为什么是 5 分钟：这是「看出趋势、又不用翻页」的尺度；更长的历史由导出的 CSV 承担。
 */
const TELEMETRY_HISTORY_LIMIT = 600;

/** 追加一个采样点并截断到上限（纯函数，便于在无浏览器环境里推演）。 */
function appendTelemetryRow(history: TelemetryRow[], view: TelemetryView): TelemetryRow[] {
  const next = [...history, telemetryRowOf(view, Date.now())];
  return next.length > TELEMETRY_HISTORY_LIMIT
    ? next.slice(next.length - TELEMETRY_HISTORY_LIMIT)
    : next;
}

/** 把单个对端并入列表（事件到达前先本地乐观更新，避免"点了没反应"）。 */
function upsert(peers: PeerView[], peer: PeerView): PeerView[] {
  const index = peers.findIndex((item) => item.idShort === peer.idShort);
  if (index < 0) {
    return [...peers, peer];
  }
  const next = [...peers];
  next[index] = peer;
  return next;
}

export function useAudioLink(): AudioLinkController {
  const [version, setVersion] = useState("");
  const [local, setLocal] = useState<LocalStatus | null>(null);
  const [peers, setPeers] = useState<PeerView[]>([]);
  const [telemetry, setTelemetry] = useState<TelemetryView | null>(null);
  const [telemetryHistory, setTelemetryHistory] = useState<TelemetryRow[]>([]);
  const [groups, setGroups] = useState<GroupView[]>([]);
  const [alignment, setAlignment] = useState<AlignmentView | null>(null);
  const [alignmentBusy, setAlignmentBusy] = useState(false);
  const [groupBusy, setGroupBusy] = useState(false);
  const [exportingTelemetry, setExportingTelemetry] = useState(false);
  const [pairRequest, setPairRequest] = useState<PairRequiredPayload | null>(null);
  const [pairReason, setPairReason] = useState("");
  const [error, setError] = useState<CommandError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [pairableIds, setPairableIds] = useState<string[]>([]);
  const [connecting, setConnecting] = useState(false);
  const [busyPeer, setBusyPeer] = useState<string | null>(null);
  const [captureDevices, setCaptureDevices] = useState<CaptureDeviceView[]>([]);
  const [captureLoading, setCaptureLoading] = useState(true);
  const [captureError, setCaptureError] = useState<CommandError | null>(null);
  const [selectedCaptureId, setSelectedCaptureId] = useState("");
  const [activeCapture, setActiveCapture] = useState<CaptureDeviceView | null>(null);
  const captureRequest = useRef(0);
  const activeRequest = useRef(0);
  const captureLocked = busyPeer !== null || peers.some((peer) => peer.state === "streaming" || peer.state === "degraded");
  const selectedCapture = captureDevices.find((device) => selectedCaptureId === "" ? device.isDefault : device.id === selectedCaptureId);
  const canStartCapture = !captureLoading && captureError === null && selectedCapture !== undefined && selectedCapture.unavailableReason === null;

  const refreshCaptureDevices = useCallback(async () => {
    const request = ++captureRequest.current;
    setCaptureLoading(true);
    setCaptureError(null);
    try {
      const devices = await api.listCaptureDevices();
      if (request === captureRequest.current) setCaptureDevices(devices);
    } catch (raw) {
      if (request === captureRequest.current) setCaptureError(toCommandError(raw));
    } finally {
      if (request === captureRequest.current) setCaptureLoading(false);
    }
  }, []);

  const refreshActiveCapture = useCallback(async () => {
    const request = ++activeRequest.current;
    try {
      const device = await api.activeCaptureDevice();
      if (request === activeRequest.current) setActiveCapture(device);
    } catch (raw) {
      if (request === activeRequest.current) setError(toCommandError(raw));
    }
  }, []);

  useEffect(() => {
    void refreshCaptureDevices();
    return () => { captureRequest.current += 1; activeRequest.current += 1; };
  }, [refreshCaptureDevices]);

  useEffect(() => {
    void refreshActiveCapture();
  }, [captureLocked, refreshActiveCapture]);

  const refreshAlignment = useCallback(async (): Promise<void> => {
    const next = await api.alignment().catch(() => null);
    if (next !== null) {
      setAlignment(next);
    }
  }, []);

  useEffect(() => {
    // M4 对齐快照：读数本身在引擎里**每包**更新，但界面只需要看趋势 —— 按 1 Hz 拉，
    // 事件驱动反而会变成「每包一次 IPC」。没有对端时不空转。
    if (peers.length === 0) {
      setAlignment(null);
      return;
    }
    void refreshAlignment();
    const timer = window.setInterval(() => void refreshAlignment(), 1000);
    return () => window.clearInterval(timer);
  }, [peers.length, refreshAlignment]);

  useEffect(() => {
    // 订阅必须在挂载时就绪：配对请求可能在用户还没做任何动作时由对端发起
    const unsubscribe = subscribeEvents({
      // §7 同步组变化（建组 / 成员加入退出）：顺手拉一次最新组列表。
      // 为什么不做轮询：这是低频人工动作，事件驱动既省 IPC 也不会漏。
      onGroupUpdated: () => {
        void refreshGroups();
      },
      // 对端列表变化时顺手收敛配对对话框：对端没了、或已经变成"已受信"，
      // 对话框就没有存在意义了（接收端场景下用户全程不点任何按钮，全靠这条规则关闭）。
      onPeers: (next) => {
        setPeers(next);
        setPairRequest((current) => {
          if (current === null) {
            return null;
          }
          const peer = next.find((item) => item.idShort === current.idShort);
          if (peer === undefined || peer.trusted) {
            return null;
          }
          return current;
        });
        // 只有"还没受信且还在列表里"的对端才保留"可输码"资格
        setPairableIds((current) =>
          current.filter((idShort) => {
            const peer = next.find((item) => item.idShort === idShort);
            return peer !== undefined && !peer.trusted;
          }),
        );
      },
      onTelemetry: (view) => {
        setTelemetry(view);
        setTelemetryHistory((current) => appendTelemetryRow(current, view));
      },
      onPairRequired: (payload) => {
        setPairRequest(payload);
        setPairReason("");
        // 收到"请本机输入"的请求 → 记下这个对端，弹窗被关掉后还能叫回来
        if (payload.pin === "") {
          setPairableIds((current) =>
            current.includes(payload.idShort) ? current : [...current, payload.idShort],
          );
        }
      },
      onError: (raw) => setError(toCommandError(raw)),
    });

    // 首屏水合：事件流只会推"之后的变化"，不拉一次初始值卡片会是空的
    void (async () => {
      try {
        const [ver, status, peerList, snapshot] = await Promise.all([
          api.version(),
          api.localStatus(),
          api.listPeers(),
          api.telemetry(),
        ]);
        setVersion(ver);
        setLocal(status);
        setPeers(peerList);
        setTelemetry(snapshot);
      } catch (raw) {
        setError(toCommandError(raw));
      }
    })();

    return unsubscribe;
  }, []);

  const connect = useCallback(async (addr: string): Promise<boolean> => {
    setError(null);
    setNotice(null);
    setConnecting(true);
    try {
      const peer = await api.connect(addr);
      setPeers((prev) => upsert(prev, peer));
      return true;
    } catch (raw) {
      const failure = toCommandError(raw);
      if (failure.code === CODE_NOT_PAIRED) {
        // 会话还活着，用户只需要输配对码 → 提示而不是报错
        setNotice(failure.message);
      } else {
        setError(failure);
      }
      return false;
    } finally {
      setConnecting(false);
    }
  }, []);

  const startSend = useCallback(async (idShort: string): Promise<void> => {
    setError(null);
    setBusyPeer(idShort);
    try {
      await api.startSend(idShort, selectedCaptureId === "" ? null : selectedCaptureId);
      await refreshActiveCapture();
    } catch (raw) {
      setError(toCommandError(raw));
    } finally {
      setBusyPeer(null);
    }
  }, [selectedCaptureId, refreshActiveCapture]);

  const stopSend = useCallback(async (): Promise<void> => {
    setError(null);
    const sending = peers.find((peer) => peer.state === "streaming" || peer.state === "degraded");
    setBusyPeer(sending?.idShort ?? null);
    try {
      await api.stopSend();
      await refreshActiveCapture();
    } catch (raw) {
      setError(toCommandError(raw));
    } finally {
      setBusyPeer(null);
    }
  }, [peers, refreshActiveCapture]);

  const submitPin = useCallback(async (idShort: string, pin: string): Promise<boolean> => {
    try {
      const result = await api.submitPin(idShort, pin);
      if (!result.ok) {
        setPairReason(result.reason);
        return false;
      }
      setPairRequest(null);
      setPairReason("");
      setNotice(null);
      // 配对成功后 trusted 变了，主动拉一次，避免完全依赖事件时序
      setPeers(await api.listPeers());
      return true;
    } catch (raw) {
      setError(toCommandError(raw));
      return false;
    }
  }, []);

  const dismissError = useCallback(() => setError(null), []);
  const dismissNotice = useCallback(() => setNotice(null), []);
  const dismissPairRequest = useCallback(() => {
    setPairRequest(null);
    setPairReason("");
  }, []);

  /** 把某个对端的配对输入框叫回来（`pin` 留空 = 本机要输入）。 */
  const beginPairing = useCallback(
    (idShort: string): void => {
      const peer = peers.find((item) => item.idShort === idShort);
      setPairReason("");
      setPairRequest({ idShort, name: peer?.name ?? "该设备", pin: "" });
    },
    [peers],
  );

  /** 导出遥测历史：成功用 notice 报路径，失败用 error 报人话原因。 */
  const exportTelemetryLog = useCallback(async (): Promise<void> => {
    if (telemetryHistory.length === 0) {
      setNotice("还没有遥测历史可导出：先连接并开始推流。");
      return;
    }
    setExportingTelemetry(true);
    try {
      const path = await api.exportTelemetry(telemetryHistory);
      setNotice(`已导出 ${telemetryHistory.length} 个采样点：${path}`);
    } catch (raw) {
      setError(toCommandError(raw));
    } finally {
      setExportingTelemetry(false);
    }
  }, [telemetryHistory]);

  /** §4.1：调对端音量。渐变 200 ms：拖滑块不该有咔哒声。 */
  const setPeerGain = useCallback(async (idShort: string, gain: number): Promise<void> => {
    try {
      await api.setPeerGain(idShort, gain, 200);
      setNotice(`${idShort} 音量已设为 ${Math.round(gain * 100)}%`);
    } catch (raw) {
      setError(toCommandError(raw));
    }
  }, []);

  /** 刷新同步组列表（M3）：操作后由调用方刷新，UI 不自己缓存真相。 */
  const refreshGroups = useCallback(async (): Promise<void> => {
    try {
      setGroups(await api.listGroups());
    } catch (raw) {
      setError(toCommandError(raw));
    }
  }, []);

  /** 建组：把点名的对端组成临时同步组（提前量给排播留余量）。 */
  const createGroup = useCallback(
    async (idShorts: string[], leadMs: number): Promise<void> => {
      if (idShorts.length === 0) {
        setNotice("先勾选至少一台已连接设备再建组。");
        return;
      }
      setGroupBusy(true);
      try {
        const groupId = await api.createGroup(idShorts, leadMs);
        setNotice(`已建组 #${groupId}，成员 ${idShorts.length} 台`);
      } catch (raw) {
        setError(toCommandError(raw));
      } finally {
        await refreshGroups();
        setGroupBusy(false);
      }
    },
    [refreshGroups],
  );

  /** 成员动态加入：运行中的其它成员不受影响（§7 / FR-22）。 */
  const joinGroup = useCallback(
    async (idShort: string, groupId: number): Promise<void> => {
      setGroupBusy(true);
      try {
        await api.joinGroup(idShort, groupId);
        setNotice(`已把 ${idShort} 加入组 #${groupId}`);
      } catch (raw) {
        setError(toCommandError(raw));
      } finally {
        await refreshGroups();
        setGroupBusy(false);
      }
    },
    [refreshGroups],
  );

  /**
   * M4：广播共同基准。接收端才是基准来源 —— 所以这个动作只在混音方按（docs/39）。
   *
   * 成功后 1 s 内的轮询就会把新读数带回来，这里不再手动拉一次（避免两处真相）。
   */
  const broadcastEpoch = useCallback(
    async (leadMs: number): Promise<void> => {
      setAlignmentBusy(true);
      try {
        const sent = await api.broadcastEpoch(leadMs);
        setNotice(`已向 ${sent} 条会话广播共同基准（提前量 ${leadMs} ms）`);
      } catch (raw) {
        setError(toCommandError(raw));
      } finally {
        await refreshAlignment();
        setAlignmentBusy(false);
      }
    },
    [refreshAlignment],
  );

  /** 成员退出：组空了引擎会自己清掉条目。 */
  const leaveGroup = useCallback(
    async (idShort: string, groupId: number): Promise<void> => {
      setGroupBusy(true);
      try {
        await api.leaveGroup(idShort, groupId);
        setNotice(`已让 ${idShort} 退出组 #${groupId}`);
      } catch (raw) {
        setError(toCommandError(raw));
      } finally {
        await refreshGroups();
        setGroupBusy(false);
      }
    },
    [refreshGroups],
  );

  return {
    version,
    local,
    peers,
    telemetry,
    telemetryHistory,
    groups,
    groupBusy,
    alignment,
    alignmentBusy,
    broadcastEpoch,
    setPeerGain,
    refreshGroups,
    createGroup,
    joinGroup,
    leaveGroup,
    exportTelemetryLog,
    exportingTelemetry,
    pairRequest,
    pairReason,
    error,
    notice,
    pairableIds,
    connecting,
    busyPeer,
    captureDevices,
    captureLoading,
    captureError,
    selectedCaptureId,
    activeCapture,
    captureLocked,
    canStartCapture,
    selectCapture: (id) => { if (!captureLocked) setSelectedCaptureId(id); },
    refreshCaptureDevices,
    connect,
    startSend,
    stopSend,
    submitPin,
    beginPairing,
    dismissError,
    dismissNotice,
    dismissPairRequest,
  };
}
