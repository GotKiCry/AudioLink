/**
 * 遥测面板（契约 §6 `TelemetryView`）。
 *
 * M2 起从「只有数字」升级为「数字 + 曲线 + 日志导出」：
 * - **曲线**用 SVG 手绘：四条折线不值得多一个图表运行时依赖，而且这样能自己控制
 *   「只画最近 5 分钟」「刷新不重排布局」这些细节；
 * - **导出**把累积的采样点交给后端写成 CSV（落盘路径由命令返回，UI 原样显示）。
 *
 * 数据来自 `audiolink://telemetry` 事件（后端 500 ms 节流），或首屏 `telemetry` 命令的快照。
 */

import type { TelemetryRow, TelemetryView } from "../types";
import { bpsToKbps, usToMs } from "../types";
import { t } from "../i18n";

interface TelemetryPanelProps {
  telemetry: TelemetryView | null;
  /** 历史采样点（时间升序）；曲线的数据源，也是导出的内容。 */
  history: TelemetryRow[];
  open: boolean;
  /** 导出进行中（按钮禁用用）。 */
  exporting: boolean;
  onExport: () => void;
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

interface SparklineProps {
  label: string;
  unit: string;
  values: number[];
  /** 最新值的展示文案（由调用方决定小数位与换算）。 */
  latest: string;
}

/**
 * 把一串读数映射成折线上的点（`viewBox="0 0 100 32"`）。
 *
 * 纵轴按**本窗口峰值**归一化：峰值落在 y=2、零落在 y=30（上下各留 2 的余量）；
 * 横轴按序号均匀铺满 0–100。
 *
 * 为什么单独成函数：这是曲线唯一的数学，而它的缺陷在界面上**看不见** ——
 * 归一化基准写错只是"形状怪一点"，没人看得出来（docs/25 §3 自认的未验证项就是这条）。
 * 抽出来用单测钉住「峰值贴顶 / 零点贴底 / 点数与采样点数一致」。
 * 返回空串 = 画不出来（不足两个点，或窗口内全零 —— 这时画的是假线）。
 */
export function sparklinePoints(values: number[]): string {
  const peak = values.reduce((acc, value) => Math.max(acc, value), 0);
  if (values.length < 2 || peak <= 0) {
    return "";
  }
  return values
    .map((value, index) => {
      const x = (index / (values.length - 1)) * 100;
      const y = 30 - (value / peak) * 28;
      return `${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(" ");
}

/**
 * 一条折线。
 *
 * 纵轴按**本窗口最大值**归一化：遥测的关注点是「趋势与尖峰」，不是绝对刻度；
 * 因此不画网格与坐标轴，只把「最新值」写在右上角（数字面板里有精确值）。
 */
function Sparkline({ label, unit, values, latest }: SparklineProps) {
  // 空串 = 画不出来：不足两个点、或窗口内全零（画出来是假线）
  const points = sparklinePoints(values);
  const ready = points !== "";

  return (
    <div className="rounded-lg border border-slate-200 p-2 dark:border-slate-800">
      <div className="flex items-baseline justify-between text-xs">
        <span className="text-slate-500">{label}</span>
        <span className="font-mono tabular-nums">
          {ready ? latest : "—"}
          {ready ? <span className="ml-1 text-slate-400">{unit}</span> : null}
        </span>
      </div>
      <svg
        viewBox="0 0 100 32"
        preserveAspectRatio="none"
        className="mt-1 h-8 w-full text-indigo-500"
        role="img"
        aria-label={t("tm.samples_aria", { label, count: values.length })}
      >
        {ready ? (
          <polyline
            points={points}
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            vectorEffect="non-scaling-stroke"
          />
        ) : (
          <text x="50" y="20" textAnchor="middle" className="fill-slate-300 text-[8px]">
            {t("tm.waiting")}
          </text>
        )}
      </svg>
    </div>
  );
}

export function TelemetryPanel({ telemetry, history, open, exporting, onExport }: TelemetryPanelProps) {
  if (!open) {
    return null;
  }

  // 没有快照（水合中）或没有会话时，遥测是内核给的全零值 —— 显示 "—" 而不是假的 0.0 ms
  const idle = telemetry === null || telemetry.peers === 0 || telemetry.e2eLatencyUs === 0;
  const dash = "—";
  const tele = telemetry;
  // `at(-1)` 在严格模式下是 `T | undefined`：显式收敛成 null，避免四处各写一遍下标。
  const last = history.at(-1) ?? null;

  return (
    <section
      id="telemetry-panel"
      aria-label={t("tm.title")}
      className="rounded-xl border border-slate-200 bg-white p-4 shadow-sm dark:border-slate-800 dark:bg-slate-950"
    >
      <header className="mb-3 flex items-center gap-3">
        <h2 className="text-sm font-medium">{t("tm.title")}</h2>
        <span className="text-xs text-slate-400">
          {t("tm.footer", { count: history.length })}
        </span>
        <button
          type="button"
          onClick={onExport}
          disabled={exporting || history.length === 0}
          className="ml-auto rounded-md border border-slate-300 px-2 py-1 text-xs transition hover:bg-slate-50 disabled:cursor-not-allowed disabled:opacity-50 dark:border-slate-700 dark:hover:bg-slate-900"
        >
          {exporting ? t("tm.exporting") : t("tm.export", { count: history.length })}
        </button>
      </header>

      <div className="grid gap-2 [grid-template-columns:repeat(auto-fill,minmax(140px,1fr))]">
        <Metric label={t("tm.peers")} value={tele === null ? dash : String(tele.peers)} />
        <Metric label="RTT" value={tele === null || idle ? dash : usToMs(tele.rttUs)} unit="ms" />
        <Metric label={t("tm.jitter")} value={tele === null || idle ? dash : usToMs(tele.jitterUs)} unit="ms" />
        <Metric label={t("tm.loss")} value={tele === null || idle ? dash : tele.lossPct.toFixed(2)} unit="%" />
        <Metric label={t("tm.bitrate")} value={tele === null || idle ? dash : bpsToKbps(tele.bitrateBps)} unit="kbps" />
        <Metric label={t("tm.buffer")} value={tele === null || idle ? dash : usToMs(tele.bufferLevelUs)} unit="ms" />
        <Metric label={t("tm.underruns")} value={tele === null ? dash : String(tele.underruns)} unit={t("tm.count")} />
        <Metric label={t("tm.e2e")} value={tele === null || idle ? dash : usToMs(tele.e2eLatencyUs)} unit="ms" />
        <Metric label={t("tm.e2e_p50")} value={tele === null || idle ? dash : usToMs(tele.e2eP50Us)} unit="ms" />
        <Metric label={t("tm.e2e_p95")} value={tele === null || idle ? dash : usToMs(tele.e2eP95Us)} unit="ms" />
      </div>

      <div className="mt-3 grid gap-2 [grid-template-columns:repeat(auto-fill,minmax(200px,1fr))]">
        <Sparkline
          label={t("tm.e2e_latency")}
          unit="ms"
          values={history.map((row) => row.e2eLatencyUs)}
          latest={usToMs(last?.e2eLatencyUs ?? 0)}
        />
        <Sparkline
          label={t("tm.buffer")}
          unit="ms"
          values={history.map((row) => row.bufferLevelUs)}
          latest={usToMs(last?.bufferLevelUs ?? 0)}
        />
        <Sparkline
          label={t("tm.bitrate")}
          unit="kbps"
          values={history.map((row) => row.bitrateBps)}
          latest={bpsToKbps(last?.bitrateBps ?? 0)}
        />
        <Sparkline
          label={t("tm.loss_rate")}
          unit="%"
          values={history.map((row) => Math.round(row.lossPct * 100))}
          latest={(last?.lossPct ?? 0).toFixed(2)}
        />
      </div>

      {idle ? <p className="mt-3 text-xs text-slate-400">{t("tm.idle")}</p> : null}
    </section>
  );
}
