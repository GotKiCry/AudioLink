/**
 * 对齐域（M4）：多源对齐快照 + 广播共同基准。
 *
 * 读数在引擎里**每包**更新，界面只需要看趋势 —— 按 1 Hz 拉，事件驱动反而会变成「每包一次 IPC」。
 * 入参是 `peerCount` 而不是 `peers`：轮询只关心"有没有对端"，传数组会让每次列表变化都重启定时器。
 */

import { useCallback, useEffect, useState } from "react";

import { api } from "../ipc";
import { toCommandError, type AlignmentView } from "../../types";
import { t } from "../../i18n";
import type { SetError, SetNotice } from "./shared";

export interface AlignmentState {
  alignment: AlignmentView | null;
  alignmentBusy: boolean;
  broadcastEpoch: (leadMs: number) => Promise<void>;
}

export function useAlignmentState(peerCount: number, setError: SetError, setNotice: SetNotice): AlignmentState {
  const [alignment, setAlignment] = useState<AlignmentView | null>(null);
  const [alignmentBusy, setAlignmentBusy] = useState(false);

  const refreshAlignment = useCallback(async (): Promise<void> => {
    const next = await api.alignment().catch(() => null);
    if (next !== null) {
      setAlignment(next);
    }
  }, []);

  useEffect(() => {
    // M4 对齐快照：读数本身在引擎里**每包**更新，但界面只需要看趋势 —— 按 1 Hz 拉，
    // 事件驱动反而会变成「每包一次 IPC」。没有对端时不空转。
    if (peerCount === 0) {
      setAlignment(null);
      return;
    }
    void refreshAlignment();
    const timer = window.setInterval(() => void refreshAlignment(), 1000);
    return () => window.clearInterval(timer);
  }, [peerCount, refreshAlignment]);

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
    [refreshAlignment, setError, setNotice],
  );

  return { alignment, alignmentBusy, broadcastEpoch };
}
