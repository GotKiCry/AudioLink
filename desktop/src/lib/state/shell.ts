/**
 * 外壳状态域：版本 / 身份 / 对端 / 遥测流 / 错误与回执。
 *
 * 它同时是 **command/event 的唯一订阅点**：首屏水合（`version` / `local_status` /
 * `list_peers` / `telemetry` 各一次）与之后的事件推进（`audiolink://peer` /
 * `audiolink://telemetry` / `audiolink://error`）都在这里发生，其他域只消费它的写入口。
 *
 * 两个 tap 的调用点在事件回调里，顺序刻意与拆分前一致：先做推流侧的记账，
 * 再落 `peers` 状态 —— 记账要用"这一批的新列表"而不是上一次的渲染值。
 */

import { useCallback, useEffect, useState } from "react";

import { api, subscribeEvents } from "../ipc";
import {
  toCommandError,
  type CommandError,
  type LocalStatus,
  type PeerView,
  type TelemetryRow,
  type TelemetryView,
} from "../../types";
import { t } from "../../i18n";
import { appendTelemetryRow, upsert } from "./reductions";
import type { GroupsTap, PeersTap, SetError, SetNotice } from "./shared";

export interface ShellState {
  version: string;
  local: LocalStatus | null;
  peers: PeerView[];
  telemetry: TelemetryView | null;
  telemetryHistory: TelemetryRow[];
  error: CommandError | null;
  notice: string | null;
  connecting: boolean;
  connect: (addr: string) => Promise<boolean>;
  /** §4.1：调对端音量。放在这一域，因为它改的是对端的声音，不是遥测读数。 */
  setPeerGain: (idShort: string, gain: number) => Promise<void>;
  dismissError: () => void;
  dismissNotice: () => void;
  /** 内部：其他子 hook 的写入口。 */
  setError: SetError;
  setNotice: SetNotice;
}

export function useShellState(peersTap: PeersTap, groupsTap: GroupsTap): ShellState {
  const [version, setVersion] = useState("");
  const [local, setLocal] = useState<LocalStatus | null>(null);
  const [peers, setPeers] = useState<PeerView[]>([]);
  const [telemetry, setTelemetry] = useState<TelemetryView | null>(null);
  const [telemetryHistory, setTelemetryHistory] = useState<TelemetryRow[]>([]);
  const [error, setError] = useState<CommandError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(false);

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
  }, []);

  useEffect(() => {
    let cancelled = false;
    let peersUpdated = false;
    // 订阅在挂载时就绪：对端列表、遥测与同步组都靠事件推进。
    const unsubscribe = subscribeEvents({
      // §7 同步组变化（建组 / 成员加入退出）：顺手拉一次最新组列表。
      // 为什么不做轮询：这是低频人工动作，事件驱动既省 IPC 也不会漏。
      onGroupUpdated: () => {
        void groupsTap.current();
      },
      onPeers: (next) => {
        peersUpdated = true;
        // 断开或重连才重置本轮尝试；重复 idle/握手事件不能反复打开采集（逻辑在推流域）。
        peersTap.current(next);
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
    // 两个 tap 是 ref，身份恒定 —— 所以订阅仍然只注册一次。
  }, [peersTap, groupsTap]);

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

  /** §4.1：调对端音量。渐变 200 ms：拖滑块不该有咔哒声。 */
  const setPeerGain = useCallback(async (idShort: string, gain: number): Promise<void> => {
    try {
      await api.setPeerGain(idShort, gain, 200);
      setNotice(t("toast.gain", { peer: idShort, pct: Math.round(gain * 100) }));
    } catch (raw) {
      setError(toCommandError(raw));
    }
  }, []);

  const dismissError = useCallback(() => setError(null), []);
  const dismissNotice = useCallback(() => setNotice(null), []);

  return {
    version, local, peers, telemetry, telemetryHistory,
    error, notice, connecting,
    connect, setPeerGain, dismissError, dismissNotice,
    setError, setNotice,
  };
}
