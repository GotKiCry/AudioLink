/**
 * 会话汇总条（工具栏内容）：本机状态 + 活跃会话 + 诊断开关。
 *
 * 「音源即主机」下它不再是一条独立的横幅，而是主区工具栏的左半段 ——
 * 右边留给外观切换器，两者共用同一条 12px 高的带子。
 */

import type { LocalStatus, TelemetryView } from "../types";
import { bpsToKbps, usToMs } from "../types";
import { t } from "../i18n";
import { IconChevron } from "./icons";

interface StatusStripProps {
  local: LocalStatus | null;
  telemetry: TelemetryView | null;
  /** 正在推流的对端台数。 */
  streamingCount: number;
  telemetryOpen: boolean;
  onToggleTelemetry: () => void;
}

export function StatusStrip({
  local,
  telemetry,
  streamingCount,
  telemetryOpen,
  onToggleTelemetry,
}: StatusStripProps) {
  const active = streamingCount > 0;
  // 可访问名保留完整措辞（含方向符），可见文字是它的前缀 —— 不出现"看得见的是 A、读出来的是 B"。
  const toggleLabel = t("status.telemetry", { arrow: telemetryOpen ? "▾" : "▸" });
  const toggleText = t("status.telemetry", { arrow: "" }).trim();

  return (
    <section className="flex min-w-0 flex-1 flex-wrap items-center gap-x-5 gap-y-1 t-cap">
      <span className="flex items-baseline gap-2">
        <span className="num text-text-3">fp:{local?.idShort ?? "--------"}</span>
        <span className="num text-text-3">{local?.addr ?? ""}</span>
      </span>

      <span className="inline-flex items-center gap-2">
        <span className="lamp h-2 w-2" data-on={active ? "on" : "off"} />
        <span className={active ? "text-text" : "text-text-3"}>
          {active ? t("status.streaming", { count: streamingCount }) : t("status.idle")}
        </span>
      </span>

      {active && telemetry ? (
        <span className="num text-text-2">
          {t("status.summary", { e2e: usToMs(telemetry.e2eLatencyUs), rate: bpsToKbps(telemetry.bitrateBps), loss: telemetry.lossPct.toFixed(2) })}
        </span>
      ) : null}

      <button
        type="button"
        onClick={onToggleTelemetry}
        aria-expanded={telemetryOpen}
        aria-controls="telemetry-panel"
        aria-label={toggleLabel}
        title={toggleLabel}
        className="key ml-auto h-7 gap-1.5 px-2.5 t-cap"
      >
        {toggleText}
        <IconChevron
          className={"h-3.5 w-3.5 transition-transform duration-200 " + (telemetryOpen ? "rotate-180" : "")}
        />
      </button>
    </section>
  );
}
