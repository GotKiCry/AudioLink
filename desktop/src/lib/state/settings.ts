/**
 * 自动推流偏好域：`autoBroadcast` 及其持久化。
 *
 * 水合与写入放在同一域，是因为 `preferenceRevision` 必须把这两件事串起来：
 * 读回来的旧值不许覆盖用户在等待期间做的新选择（见 `setAutoBroadcast`）。
 * `autoBroadcastReady`（设置是否已读回）也在这里产生 —— 它是"设置域的事"，
 * 但推流与界面状态机都要用它：未读回前自动推流不许抢跑，界面则显示 `loading`。
 */

import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../ipc";
import { toCommandError } from "../../types";
import type { SetError } from "./shared";

export interface SettingsState {
  autoBroadcast: boolean;
  autoBroadcastBusy: boolean;
  autoBroadcastReady: boolean;
  setAutoBroadcast: (enabled: boolean) => void;
}

export function useSettingsState(setError: SetError): SettingsState {
  /** 「接入即自动推流」：默认开（用户明确要的默认行为），真实值随后端设置水合并覆盖。 */
  const [autoBroadcast, setAutoBroadcastState] = useState(true);
  const [autoBroadcastBusy, setAutoBroadcastBusy] = useState(false);
  const [autoBroadcastReady, setAutoBroadcastReady] = useState(false);
  const preferenceRevision = useRef(0);

  useEffect(() => {
    // 自动推流开关存在设置文件里；读不到（老内核还没有这条命令）就保持默认开 ——
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

  const setAutoBroadcast = useCallback((enabled: boolean): void => {
    preferenceRevision.current += 1;
    setAutoBroadcastState(enabled);
    // 自动偏好与本次暂停独立：调整设置不会意外取消暂停。
    setAutoBroadcastBusy(true);
    void api.setAutoBroadcast(enabled)
      .catch((raw: unknown) => setError(toCommandError(raw)))
      .finally(() => setAutoBroadcastBusy(false));
  }, [setError]);

  return { autoBroadcast, autoBroadcastBusy, autoBroadcastReady, setAutoBroadcast };
}
