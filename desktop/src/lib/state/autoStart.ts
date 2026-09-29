/**
 * 「有人接入就自动开始推流」的决策（FR-31 前端侧）。
 *
 * 为什么单独成文件：判断条件横跨三个域 —— 对端列表（shell）、偏好开关（settings）、
 * 音源可用性与锁定（capture），动作却属于推流（broadcast）。
 * 塞进任何一个域都会形成"钩子调用顺序环"，所以它排在 capture 之后单独执行。
 */

import { useEffect } from "react";

import type { PeerView } from "../../types";
import type { AutoStartParts } from "./shared";

export interface AutoStartDeps {
  peers: PeerView[];
  autoBroadcast: boolean;
  autoBroadcastReady: boolean;
  broadcastRequested: boolean;
  /** 只进 deps：暂停切换时让门控重跑一次（真正判断读的是 `pausedRef`，两者同步更新）。 */
  broadcastPaused: boolean;
  canStartCapture: boolean;
  captureLocked: boolean;
}

export function useAutoBroadcastStart(deps: AutoStartDeps, parts: AutoStartParts): void {
  const { peers, autoBroadcast, autoBroadcastReady, broadcastRequested, broadcastPaused,
    canStartCapture, captureLocked } = deps;
  const { requestSend, pausedRef, activeSendRef, autoStartedRef } = parts;

  useEffect(() => {
    if (!autoBroadcastReady || (!autoBroadcast && !broadcastRequested) || pausedRef.current ||
        !canStartCapture || captureLocked || activeSendRef.current) return;
    const target = peers.find((peer) => !peer.receiving && peer.state === "idle" &&
      !autoStartedRef.current.has(peer.idShort));
    if (target) void requestSend(target.idShort);
  }, [peers, autoBroadcast, autoBroadcastReady, broadcastRequested, broadcastPaused,
      canStartCapture, captureLocked, requestSend, pausedRef, activeSendRef, autoStartedRef]);
}
