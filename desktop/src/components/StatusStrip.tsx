/**
 * 本机状态条：本机身份 + 活跃会话汇总 + 遥测开关（UI 规格 §2.1 的状态条）。
 */

import type { LocalStatus, TelemetryView } from "../types";
import { bpsToKbps, usToMs } from "../types";

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
  return (
    <section className="flex flex-wrap items-center gap-x-5 gap-y-2 border-b border-slate-200 px-4 py-3 text-sm dark:border-slate-800">
      <span>
        本机「{local?.name ?? "…"}」{" "}
        {/* 指纹短码用等宽字体：数字/字母混排时不可读性最高（UI 规格 §5 等宽数字要求） */}
        <span className="font-mono text-slate-400">fp:{local?.idShort ?? "--------"}</span>
      </span>
      <span className="font-mono text-xs text-slate-500">{local?.addr ?? ""}</span>

      <span className="inline-flex items-center gap-1.5">
        <span className={`h-2 w-2 rounded-full ${active ? "bg-emerald-500" : "bg-slate-400"}`} />
        {active ? `推送中 ${streamingCount} 台` : "未在推流"}
      </span>

      {active && telemetry ? (
        <span className="tabular-nums text-slate-500">
          端到端 {usToMs(telemetry.e2eLatencyUs)} ms · {bpsToKbps(telemetry.bitrateBps)} kbps · 丢包{" "}
          {telemetry.lossPct.toFixed(2)}%
        </span>
      ) : null}

      <button
        type="button"
        onClick={onToggleTelemetry}
        aria-expanded={telemetryOpen}
        aria-controls="telemetry-panel"
        className="ml-auto h-8 rounded-lg border border-slate-200 px-3 text-sm hover:bg-slate-100 dark:border-slate-700 dark:hover:bg-slate-800"
      >
        遥测 {telemetryOpen ? "▾" : "▸"}
      </button>
    </section>
  );
}
