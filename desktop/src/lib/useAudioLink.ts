/**
 * 桌面端界面状态：把 command/event 收敛成一个 hook，组件只读状态、只调动作。
 *
 * 数据来源（契约 §6）：
 * - 首屏水合：`version` / `local_status` / `list_peers` / `telemetry` 各一次；
 * - 之后持续推进：`audiolink://peer`（变化即推）、`audiolink://telemetry`（500 ms 节流）。
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
  type PeerView,
  type TelemetryRow,
  type TelemetryView,
  telemetryRowOf,
} from "../types";
import { t } from "../i18n";

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
  const [error, setError] = useState<CommandError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(false);
  const [busyPeer, setBusyPeer] = useState<string | null>(null);
  const [captureDevices, setCaptureDevices] = useState<CaptureDeviceView[]>([]);
  const [captureLoading, setCaptureLoading] = useState(true);
  const [captureError, setCaptureError] = useState<CommandError | null>(null);
  const [selectedCaptureId, setSelectedCaptureId] = useState("");
  const [activeCapture, setActiveCapture] = useState<CaptureDeviceView | null>(null);
  /** 推流意图：见 AudioLinkController.broadcastRequested 的说明。 */
  const [broadcastRequested, setBroadcastRequested] = useState(false);
  /** 「接入即自动推流」：默认开（用户明确要的默认行为），真实值随后端设置水合并覆盖。 */
  const [autoBroadcast, setAutoBroadcastState] = useState(true);
  const [autoBroadcastBusy, setAutoBroadcastBusy] = useState(false);
  const [autoBroadcastReady, setAutoBroadcastReady] = useState(false);
  const [broadcastPaused, setBroadcastPaused] = useState(false);
  const [broadcastStopping, setBroadcastStopping] = useState(false);
  const [broadcastError, setBroadcastError] = useState<CommandError | null>(null);
  const pausedRef = useRef(false);
  const sendPendingRef = useRef<Promise<void> | null>(null);
  const stopPendingRef = useRef<Promise<void> | null>(null);
  const activeSendRef = useRef<string | null>(null);
  const reconnectsRef = useRef(new Map<string, number>());
  const preferenceRevision = useRef(0);
  const captureRequest = useRef(0);
  const activeRequest = useRef(0);
  /** 这一轮里**已经自动发起过**的设备：同一台只发一次，事件抖动不再重复调 start_send。 */
  const autoStartedRef = useRef<Set<string>>(new Set());
  const captureLocked = broadcastStopping || busyPeer !== null || peers.some((peer) => peer.state === "streaming" || peer.state === "degraded");
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

  useEffect(() => {
    // M5：启动时试一次自动重连（FR-31）。静默失败是刻意的 —— 用户什么都没点，
    // 不该在开机时弹一条「连不上」的错误横幅；连上了才给一句轻提示。
    void api
      .tryAutoConnect()
      .then((peer) => {
        if (peer !== null) {
          setNotice(t("toast.auto_connected", { peer: peer.idShort }));
        }
      })
      .catch(() => undefined);

    // 自动推流开关同样存在设置文件里；读不到（老内核还没有这条命令）就保持默认开 ——
    // 自动推流在前端执行，不该因为一个持久化读不到就静默失效。
    const revision = preferenceRevision.current;
    let cancelled = false;
    void api
      .autoBroadcastState()
      .then((enabled) => {
        if (!cancelled && revision === preferenceRevision.current) setAutoBroadcastState(enabled);
      })
      .catch(() => undefined)
      .finally(() => { if (!cancelled) setAutoBroadcastReady(true); });
    return () => { cancelled = true; };
  }, []);

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
    let cancelled = false;
    let peersUpdated = false;
    // 订阅在挂载时就绪：对端列表、遥测与同步组都靠事件推进。
    const unsubscribe = subscribeEvents({
      // §7 同步组变化（建组 / 成员加入退出）：顺手拉一次最新组列表。
      // 为什么不做轮询：这是低频人工动作，事件驱动既省 IPC 也不会漏。
      onGroupUpdated: () => {
        void refreshGroups();
      },
      onPeers: (next) => {
        peersUpdated = true;
        // 断开或重连才重置本轮尝试；重复 idle/握手事件不能反复打开采集。
        for (const id of autoStartedRef.current) {
          const peer = next.find((item) => item.idShort === id);
          if (!peer || peer.state === "failed" || (peer.reconnects ?? 0) > (reconnectsRef.current.get(id) ?? 0)) {
            autoStartedRef.current.delete(id);
            if (activeSendRef.current === id) activeSendRef.current = null;
          }
        }
        reconnectsRef.current = new Map(next.map((peer) => [peer.idShort, peer.reconnects ?? 0]));
        setPeers(next);
      },
      onTelemetry: (view) => {
        setTelemetry(view);
        setTelemetryHistory((current) => appendTelemetryRow(current, view));
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
        if (cancelled) return;
        setVersion(ver);
        setLocal(status);
        // 初始化请求可能晚于连接事件返回，旧快照不能抹掉刚接入的设备。
        if (!peersUpdated) setPeers(peerList);
        setTelemetry(snapshot);
      } catch (raw) {
        if (!cancelled) setError(toCommandError(raw));
      }
    })();

    return () => { cancelled = true; unsubscribe(); };
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
      setError(toCommandError(raw));
      return false;
    } finally {
      setConnecting(false);
    }
  }, []);

  // 桌面外壳当前只支持一路发送。同步占位防止同一帧的自动/手动操作重复启动。
  const requestSend = useCallback((idShort: string): Promise<void> => {
    if (sendPendingRef.current || stopPendingRef.current || activeSendRef.current || !canStartCapture) {
      return Promise.resolve();
    }
    autoStartedRef.current.add(idShort);
    setBroadcastRequested(true);
    setBroadcastError(null);
    setError(null);
    setBusyPeer(idShort);
    const operation = (async () => {
      try {
        await api.startSend(idShort, selectedCaptureId === "" ? null : selectedCaptureId);
        activeSendRef.current = idShort;
        await refreshActiveCapture();
      } catch (raw) {
        setBroadcastRequested(false);
        setBroadcastError(toCommandError(raw));
        setError(toCommandError(raw));
      } finally {
        sendPendingRef.current = null;
        setBusyPeer(null);
      }
    })();
    sendPendingRef.current = operation;
    return operation;
  }, [canStartCapture, selectedCaptureId, refreshActiveCapture]);

  const startSend = useCallback((idShort: string): Promise<void> => {
    if (stopPendingRef.current) return stopPendingRef.current;
    pausedRef.current = false;
    setBroadcastPaused(false);
    return requestSend(idShort);
  }, [requestSend]);

  const stopSend = useCallback((): Promise<void> => {
    // 先关自动入口，再等待已经发出的 start 完成，最后 stop；慢握手不能越过用户的暂停。
    pausedRef.current = true;
    setBroadcastPaused(true);
    setBroadcastRequested(false);
    setBroadcastError(null);
    if (stopPendingRef.current) return stopPendingRef.current;
    setBroadcastStopping(true);
    setError(null);
    const operation = (async () => {
      try {
        await sendPendingRef.current;
        await api.stopSend();
        activeSendRef.current = null;
        await refreshActiveCapture();
      } catch (raw) {
        // 保留暂停意图，禁止自动重启；真实对端状态仍提供再次暂停的入口。
        setError(toCommandError(raw));
      } finally {
        // 停止可能已生效但回执失败；是否仍在发送以 peers 为准，不留下阻塞恢复的旧占位。
        activeSendRef.current = null;
        stopPendingRef.current = null;
        setBroadcastStopping(false);
      }
    })();
    stopPendingRef.current = operation;
    return operation;
  }, [refreshActiveCapture]);

  const startBroadcast = useCallback((idShorts: string[]): void => {
    if (stopPendingRef.current || sendPendingRef.current || activeSendRef.current) return;
    pausedRef.current = false;
    setBroadcastPaused(false);
    setBroadcastRequested(true);
    setBroadcastError(null);
    setError(null);
    autoStartedRef.current.clear();
    const target = idShorts[0];
    if (target) void requestSend(target);
  }, [requestSend]);

  const stopBroadcast = stopSend;

  useEffect(() => {
    if (!autoBroadcastReady || (!autoBroadcast && !broadcastRequested) || pausedRef.current ||
        !canStartCapture || captureLocked || activeSendRef.current) return;
    const target = peers.find((peer) => !peer.receiving && peer.state === "idle" &&
      !autoStartedRef.current.has(peer.idShort));
    if (target) void requestSend(target.idShort);
  }, [peers, autoBroadcast, autoBroadcastReady, broadcastRequested, broadcastPaused,
      canStartCapture, captureLocked, requestSend]);

  const setAutoBroadcastEnabled = useCallback((enabled: boolean): void => {
    preferenceRevision.current += 1;
    setAutoBroadcastState(enabled);
    // 自动偏好与本次暂停独立：调整设置不会意外取消暂停。
    setAutoBroadcastBusy(true);
    void api.setAutoBroadcast(enabled)
      .catch((raw: unknown) => setError(toCommandError(raw)))
      .finally(() => setAutoBroadcastBusy(false));
  }, []);

  const dismissError = useCallback(() => setError(null), []);
  const dismissNotice = useCallback(() => setNotice(null), []);

  /** 导出遥测历史：成功用 notice 报路径，失败用 error 报人话原因。 */
  const exportTelemetryLog = useCallback(async (): Promise<void> => {
    if (telemetryHistory.length === 0) {
      setNotice(t("toast.no_telemetry"));
      return;
    }
    setExportingTelemetry(true);
    try {
      const path = await api.exportTelemetry(telemetryHistory);
      setNotice(t("toast.exported", { count: telemetryHistory.length, path }));
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
      setNotice(t("toast.gain", { peer: idShort, pct: Math.round(gain * 100) }));
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
        setNotice(t("toast.pick_peer"));
        return;
      }
      setGroupBusy(true);
      try {
        const groupId = await api.createGroup(idShorts, leadMs);
        setNotice(t("toast.group_created", { id: groupId, count: idShorts.length }));
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
        setNotice(t("toast.group_joined", { peer: idShort, id: groupId }));
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
        setNotice(t("toast.epoch_sent", { count: sent, lead: leadMs }));
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
        setNotice(t("toast.group_left", { peer: idShort, id: groupId }));
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
    error,
    notice,
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
    broadcastRequested,
    broadcastPaused,
    broadcastStopping,
    broadcastError,
    autoBroadcastReady,
    startBroadcast,
    stopBroadcast,
    autoBroadcast,
    autoBroadcastBusy,
    setAutoBroadcast: setAutoBroadcastEnabled,
    dismissError,
    dismissNotice,
  };
}
