/**
 * 遥测**数字面板**（契约 §6 `TelemetryView` 全字段）。
 *
 * M1 范围纪律：**只做数字，不做曲线图**（曲线属 M2，见 `docs/11-m1-contract.md` §6「不做」）。
 * 数据来自 `audiolink://telemetry` 事件（后端 500 ms 节流），或首屏 `telemetry` command 的快照。
 */

import type { TelemetryView } from "../types";
import { bpsToKbps, usToMs } from "../types";

interface TelemetryPanelProps {
  telemetry: TelemetryView | null;
  open: boolean;
}

/** 一个指标格。数值用等宽 + tabular-nums，刷新时不抖动布局（UI 规格 §5）。 */
function Metric({ label, value, unit }: { label: string; value: string; unit?: string }) {
  return (
    <div className="rounded-lg bg-slate-50 px-3 py-2 dark:bg-slate-900">
      <div className="text-xs text-slate-500">{label}</div>
      <div className="font-mono text-lg tabular-nums leading-tight">
        {value}
        {unit === undefined ? null : <span className="ml-1 text-xs text-slate-400">{unit}</span>}
      </div>
    </div>
  );
}

export function TelemetryPanel({ telemetry, open }: TelemetryPanelProps) {
  if (!open) {
    return null;
  }

  // 没有快照（水合中）或没有会话时，遥测是内核给的全零值 —— 显示 "—" 而不是假的 0.0 ms
  const idle = telemetry === null || telemetry.peers === 0 || telemetry.e2eLatencyUs === 0;
  const dash = "—";
  const t = telemetry;

  return (
    <section
      id="telemetry-panel"
      aria-label="遥测"
      className="rounded-xl border border-slate-200 bg-white p-4 shadow-sm dark:border-slate-800 dark:bg-slate-950"
    >
      <header className="mb-3 flex items-center gap-3">
        <h2 className="text-sm font-medium">遥测</h2>
        <span className="text-xs text-slate-400">
          后端每 500 ms 推送一次；无会话时各项为零值
        </span>
        {idle ? <span className="ml-auto text-xs text-slate-400">未推流</span> : null}
      </header>

      <div className="grid gap-2 [grid-template-columns:repeat(auto-fill,minmax(140px,1fr))]">
        <Metric label="对端数" value={t === null ? dash : String(t.peers)} />
        <Metric label="RTT" value={t === null || idle ? dash : usToMs(t.rttUs)} unit="ms" />
        <Metric label="抖动" value={t === null || idle ? dash : usToMs(t.jitterUs)} unit="ms" />
        <Metric
          label="丢包"
          value={t === null || idle ? dash : t.lossPct.toFixed(2)}
          unit="%"
        />
        <Metric
          label="码率"
          value={t === null || idle ? dash : bpsToKbps(t.bitrateBps)}
          unit="kbps"
        />
        <Metric
          label="缓冲水位"
          value={t === null || idle ? dash : usToMs(t.bufferLevelUs)}
          unit="ms"
        />
        <Metric label="欠载" value={t === null ? dash : String(t.underruns)} unit="次" />
        <Metric
          label="端到端"
          value={t === null || idle ? dash : usToMs(t.e2eLatencyUs)}
          unit="ms"
        />
        <Metric label="端到端 P50" value={t === null || idle ? dash : usToMs(t.e2eP50Us)} unit="ms" />
        <Metric label="端到端 P95" value={t === null || idle ? dash : usToMs(t.e2eP95Us)} unit="ms" />
      </div>
    </section>
  );
}
