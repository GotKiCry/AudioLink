/**
 * 推流状态域：意图 / 暂停 / 停止 / 忙碌 / 错误，加上"单路发送"的同步占位。
 *
 * 只收两个入参：**采集门控**与**错误出口**。自动推流的决策不在这里 ——
 * 它的条件横跨三个域，单独由 `state/autoStart.ts` 排在采集之后执行。
 *
 * 采集门控（`canStartCapture` / `selectedCaptureId` / `refreshActiveCapture`）从 `sendGate` 里现读，
 * 而不是当参数收：它们是采集侧的渲染期派生值，当参数收就会与采集域形成调用顺序环。
 * 现读也让门控永远是最新值 —— 原实现里它落后一帧（`useCallback` 的闭包）。
 */

import { useCallback, useRef, useState, type RefObject } from "react";

import { api } from "../ipc";
import { toCommandError, type CommandError, type PeerView } from "../../types";
import type { AutoStartParts, SendGate, SetError } from "./shared";

export interface BroadcastState {
  broadcastRequested: boolean;
  broadcastPaused: boolean;
  broadcastStopping: boolean;
  broadcastError: CommandError | null;
  busyPeer: string | null;
  startSend: (idShort: string) => Promise<void>;
  stopSend: () => Promise<void>;
  startBroadcast: (idShorts: string[]) => void;
  stopBroadcast: () => Promise<void>;
  /** 内部：订阅回调里的记账（见 `state/shell.ts` 的 `onPeers`）。 */
  reconcilePeers: (next: PeerView[]) => void;
  /** 内部：自动推流要用的动作与 ref（见 `state/autoStart.ts`）。 */
  autoStart: AutoStartParts;
}

export function useBroadcastState(sendGate: RefObject<SendGate>, setError: SetError): BroadcastState {
  /** 推流意图：见 AudioLinkController.broadcastRequested 的说明。 */
  const [broadcastRequested, setBroadcastRequested] = useState(false);
  const [broadcastPaused, setBroadcastPaused] = useState(false);
  const [broadcastStopping, setBroadcastStopping] = useState(false);
  const [broadcastError, setBroadcastError] = useState<CommandError | null>(null);
  const [busyPeer, setBusyPeer] = useState<string | null>(null);
  const pausedRef = useRef(false);
  const sendPendingRef = useRef<Promise<void> | null>(null);
  const stopPendingRef = useRef<Promise<void> | null>(null);
  const activeSendRef = useRef<string | null>(null);
  const reconnectsRef = useRef(new Map<string, number>());
  /** 这一轮里**已经自动发起过**的设备：同一台只发一次，事件抖动不再重复调 start_send。 */
  const autoStartedRef = useRef<Set<string>>(new Set());
  /** 每台设备**上一拍**观测到的状态：会话被静默顶替时，唯一的可见线索是状态从活跃档掉下来。 */
  const prevStatesRef = useRef(new Map<string, PeerView["state"]>());

  /**
   * 对端列表到达时的记账（`audiolink://peer`，借用 shell 的订阅回调执行）。
   *
   * 断开或重连才重置本轮尝试；重复 idle/握手事件不能反复打开采集。
   *
   * 静默顶替（对端进程被杀后立刻重连）：旧连接要等 idle 超时才判死，新连接以同一身份
   * **原位顶替**会话表项 —— peer 不消失、不报 failed、reconnects 不变，上面的既有条件
   * 全都抓不住它。但旧推流已随旧会话死亡，外壳的推流标记失效后状态会从 streaming/degraded
   * 掉回 handshaking/idle —— 这个迁移就是「该重新自动推流了」的信号。
   */
  const reconcilePeers = useCallback((next: PeerView[]): void => {
    for (const id of autoStartedRef.current) {
      const peer = next.find((item) => item.idShort === id);
      if (!peer || peer.state === "failed" || (peer.reconnects ?? 0) > (reconnectsRef.current.get(id) ?? 0)) {
        autoStartedRef.current.delete(id);
        if (activeSendRef.current === id) activeSendRef.current = null;
      }
    }
    for (const peer of next) {
      const prev = prevStatesRef.current.get(peer.idShort);
      const wasLive = prev === "streaming" || prev === "degraded";
      const isLive = peer.state === "streaming" || peer.state === "degraded";
      if (wasLive && !isLive) {
        autoStartedRef.current.delete(peer.idShort);
        if (activeSendRef.current === peer.idShort) activeSendRef.current = null;
      }
    }
    reconnectsRef.current = new Map(next.map((peer) => [peer.idShort, peer.reconnects ?? 0]));
    prevStatesRef.current = new Map(next.map((peer) => [peer.idShort, peer.state]));
  }, []);

  // 桌面外壳当前只支持一路发送。同步占位防止同一帧的自动/手动操作重复启动。
  const requestSend = useCallback((idShort: string): Promise<void> => {
    const gate = sendGate.current;
    if (sendPendingRef.current || stopPendingRef.current || activeSendRef.current || !gate.canStartCapture) {
      return Promise.resolve();
    }
    autoStartedRef.current.add(idShort);
    setBroadcastRequested(true);
    setBroadcastError(null);
    setError(null);
    setBusyPeer(idShort);
    const operation = (async () => {
      try {
        await api.startSend(idShort, gate.selectedCaptureId === "" ? null : gate.selectedCaptureId);
        activeSendRef.current = idShort;
        await gate.refreshActiveCapture();
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
  }, [sendGate, setError]);

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
        await sendGate.current.refreshActiveCapture();
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
  }, [sendGate, setError]);

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
  }, [requestSend, setError]);

  const stopBroadcast = stopSend;

  return {
    broadcastRequested, broadcastPaused, broadcastStopping, broadcastError, busyPeer,
    startSend, stopSend, startBroadcast, stopBroadcast,
    reconcilePeers,
    autoStart: { requestSend, pausedRef, activeSendRef, autoStartedRef },
  };
}
