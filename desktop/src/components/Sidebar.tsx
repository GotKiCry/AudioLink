import type { CaptureDeviceView, CommandError, LocalStatus } from "../types";
import { t } from "../i18n";
import { broadcastHints, broadcastLabels, broadcastLamp, type BroadcastStatus } from "../lib/broadcast";
import { CaptureSourcePanel } from "./CaptureSourcePanel";
import { IconMonitor, IconPause, IconStart } from "./icons";

interface SidebarProps {
  version: string;
  local: LocalStatus | null;
  status: BroadcastStatus;
  streamingCount: number;
  autoBroadcast: boolean;
  canResume: boolean;
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

export function Sidebar({ version, local, status, streamingCount, autoBroadcast, canResume,
  captureDevices, selectedCaptureId, activeCapture, captureLoading, captureLocked,
  captureError, onSelectCapture, onRefreshCapture, onBroadcast, onStopBroadcast }: SidebarProps) {
  const canPause = status === "streaming" || status === "starting" || status === "waiting";
  const disabled = status === "loading" || status === "pausing" || (!canPause && !canResume);
  const label = status === "pausing" ? t("peer.stopping") : canPause ? t("side.stop_broadcast")
    : status === "error" ? t("share.retry") : status === "manual" ? t("share.start") : t("side.broadcast");

  return (
    <aside aria-label={t("side.local")} className="al-sidebar al-chrome">
      <div className="al-brand">
        <IconMonitor className="h-5 w-5 text-accent" />
        <span className="text-body font-semibold">AudioLink</span>
        <span className="num ml-auto text-caption text-text-tertiary">{version ? `v${version}` : "…"}</span>
      </div>
      <div className="al-sidebar-body">
        <div className="al-local-identity">
          <p className="text-caption text-text-secondary">{t("side.local")}</p>
          <h2 className="mt-1 break-words text-xl font-semibold">{local?.name ?? "…"}</h2>
        </div>
        <section className="al-share-control" aria-label={t("share.title")}>
          <div role="status" aria-live="polite">
            <div className="flex items-center gap-2.5">
              <span className="al-lamp h-2 w-2" data-on={broadcastLamp(status)} />
              <span className="text-body font-semibold">{t(broadcastLabels[status])}</span>
            </div>
            <p className="mt-2 text-caption leading-relaxed text-text-secondary">
              {t(broadcastHints[status], { count: streamingCount })}
            </p>
          </div>
          <button type="button" disabled={disabled} onClick={canPause ? onStopBroadcast : onBroadcast}
            className={`al-btn mt-4 h-10 w-full gap-2 text-body ${canPause ? "" : "al-btn-accent"}`}>
            {canPause || status === "pausing" ? <IconPause className="h-4 w-4" /> : <IconStart className="h-4 w-4" />}
            {label}
          </button>
          <p className="mt-2 text-caption text-text-tertiary">
            {autoBroadcast ? t("share.automatic") : t("share.manual")}
          </p>
        </section>
        <CaptureSourcePanel devices={captureDevices} selectedId={selectedCaptureId} active={activeCapture}
          loading={captureLoading} locked={captureLocked} error={captureError}
          onSelect={onSelectCapture} onRefresh={onRefreshCapture} />
      </div>
      <p className="al-sidebar-footer text-caption text-text-secondary">{t("share.background")}</p>
    </aside>
  );
}
