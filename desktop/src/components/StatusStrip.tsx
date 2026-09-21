import type { TelemetryView } from "../types";
import { bpsToKbps, usToMs } from "../types";
import { t } from "../i18n";
import { broadcastLabels, broadcastLamp, type BroadcastStatus } from "../lib/broadcast";
import { IconChevron } from "./icons";

interface StatusStripProps {
  status: BroadcastStatus;
  telemetry: TelemetryView | null;
  streamingCount: number;
  telemetryOpen: boolean;
  onToggleTelemetry: () => void;
}

export function StatusStrip({ status, telemetry, streamingCount, telemetryOpen, onToggleTelemetry }: StatusStripProps) {
  const streaming = status === "streaming" && streamingCount > 0;
  const toggleLabel = t("status.telemetry", { arrow: telemetryOpen ? "▾" : "▸" });
  return (
    <section className="flex min-w-0 flex-1 flex-wrap items-center gap-x-4 gap-y-1 text-caption">
      <span className="inline-flex items-center gap-2 text-text-secondary">
        <span className="al-lamp h-2 w-2" data-on={broadcastLamp(status)} />
        {streaming ? t("status.streaming", { count: streamingCount }) : t(broadcastLabels[status])}
      </span>
      {streaming && telemetry ? (
        <span className="al-status-metrics num text-text-secondary">
          {t("status.summary", { e2e: usToMs(telemetry.e2eLatencyUs), rate: bpsToKbps(telemetry.bitrateBps), loss: telemetry.lossPct.toFixed(2) })}
        </span>
      ) : null}
      <button type="button" onClick={onToggleTelemetry} aria-expanded={telemetryOpen}
        aria-controls="telemetry-panel" aria-label={toggleLabel} title={toggleLabel}
        className="al-btn ml-auto h-8 gap-1.5 px-2.5 text-caption">
        {t("status.telemetry", { arrow: "" }).trim()}
        <IconChevron className={`h-3.5 w-3.5 transition-transform duration-200 ${telemetryOpen ? "rotate-180" : ""}`} />
      </button>
    </section>
  );
}
