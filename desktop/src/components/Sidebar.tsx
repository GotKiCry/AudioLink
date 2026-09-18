/**
 * 侧边栏：这台机器的身份与控制台。
 *
 * 「音源即主机」架构把它放回最自然的位置 —— 左边永远答三个问题：
 * 我是谁（本机身份）、我的声音从哪来（采集源）、现在开不开播（主操作）。
 * 主区负责答第四个：谁在听。
 */

import type { CaptureDeviceView, CommandError, LocalStatus } from "../types";
import { t } from "../i18n";
import { CaptureSourcePanel } from "./CaptureSourcePanel";
import { IconStart, IconStop } from "./icons";

interface SidebarProps {
  version: string;
  local: LocalStatus | null;
  /** 正在收听本机的设备数。 */
  streamingCount: number;
  busy: boolean;
  captureDevices: CaptureDeviceView[];
  selectedCaptureId: string;
  activeCapture: CaptureDeviceView | null;
  captureLoading: boolean;
  captureLocked: boolean;
  captureError: CommandError | null;
  onSelectCapture: (id: string) => void;
  onRefreshCapture: () => Promise<void>;
  onBroadcast: () => void;
  onStopBroadcast: () => void;
}

export function Sidebar({
  version,
  local,
  streamingCount,
  busy,
  captureDevices,
  selectedCaptureId,
  activeCapture,
  captureLoading,
  captureLocked,
  captureError,
  onSelectCapture,
  onRefreshCapture,
  onBroadcast,
  onStopBroadcast,
}: SidebarProps) {
  const broadcasting = streamingCount > 0;

  return (
    <aside
      aria-label={t("side.local")}
      className="chrome-material flex w-[280px] shrink-0 flex-col border-r border-line"
    >
      <div className="flex h-11 shrink-0 items-center gap-2 px-4">
        <span className="t-body font-semibold tracking-tight text-text">AudioLink</span>
        <span className="num t-cap text-text-3">{version === "" ? "…" : `v${version}`}</span>
      </div>

      <div className="shrink-0 px-4 pb-4">
        <div className="silk">{t("side.local")}</div>
        <div className="mt-2 flex items-center gap-2">
          <span className="lamp h-2.5 w-2.5" data-on={broadcasting ? "live" : "off"} />
          <span className="truncate t-body text-text">{local?.name ?? "…"}</span>
        </div>
        <div className="num mt-1 truncate t-cap text-text-3">fp:{local?.idShort ?? "--------"}</div>
        <div className="mt-2 t-cap text-text-2">
          {broadcasting ? t("console.channels", { count: streamingCount }) : t("console.off_air")}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
        <CaptureSourcePanel
          devices={captureDevices}
          selectedId={selectedCaptureId}
          active={activeCapture}
          loading={captureLoading}
          locked={captureLocked}
          error={captureError}
          onSelect={onSelectCapture}
          onRefresh={onRefreshCapture}
        />
      </div>

      <div className="shrink-0 border-t border-line p-3">
        <button
          type="button"
          disabled={busy}
          onClick={broadcasting ? onStopBroadcast : onBroadcast}
          className="key key-primary h-10 w-full gap-2 t-body"
        >
          {broadcasting ? <IconStop className="h-4 w-4" /> : <IconStart className="h-4 w-4" />}
          {broadcasting ? t("side.stop_broadcast") : t("side.broadcast")}
        </button>
      </div>
    </aside>
  );
}
