/**
 * 遥测导出域：把历史采样点导成 CSV。
 *
 * 只做"导出"这一件事：历史数据本身由 `state/shell.ts` 持有（它订阅 `audiolink://telemetry`），
 * 这里收的是快照，不收状态归属。
 */

import { useCallback, useState } from "react";

import { api } from "../ipc";
import { toCommandError, type TelemetryRow } from "../../types";
import { t } from "../../i18n";
import type { SetError, SetNotice } from "./shared";

export interface TelemetryExportState {
  exportingTelemetry: boolean;
  exportTelemetryLog: () => Promise<void>;
}

/** 导出遥测历史：成功用 notice 报路径，失败用 error 报人话原因。 */
export function useTelemetryExport(history: TelemetryRow[], setError: SetError, setNotice: SetNotice): TelemetryExportState {
  const [exportingTelemetry, setExportingTelemetry] = useState(false);

  const exportTelemetryLog = useCallback(async (): Promise<void> => {
    if (history.length === 0) {
      setNotice(t("toast.no_telemetry"));
      return;
    }
    setExportingTelemetry(true);
    try {
      const path = await api.exportTelemetry(history);
      setNotice(t("toast.exported", { count: history.length, path }));
    } catch (raw) {
      setError(toCommandError(raw));
    } finally {
      setExportingTelemetry(false);
    }
  }, [history, setError, setNotice]);

  return { exportingTelemetry, exportTelemetryLog };
}
