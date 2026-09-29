import type { CaptureDeviceView, CommandError, LocalStatus } from "../types";
import { t } from "../i18n";
import { broadcastHints, broadcastLabels, broadcastLamp, deriveBroadcastAffordances, type BroadcastStatus } from "../lib/broadcast";
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
  // 主按钮的三项由纯函数折算（与 status 同一层，自带穷举等价测试）。两条用法约定：
  // `label` 是 **i18n 键**、不是成品文案（纯函数不依赖当前语言），所以这里要 t() 一次 ——
  // 直接渲染会把 "side.stop_broadcast" 印在按钮上；`disabled` 里**已经含 canResume**，
  // 不要再叠一次判断，那会变成第三处真相。图标不在函数职责内，按 canPause 这一侧自选。
  const { canPause, disabled, label } = deriveBroadcastAffordances(status, canResume);

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
            {t(label)}
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
