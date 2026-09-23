/**
 * 遥测面板（契约 §6 `TelemetryView`）。
 *
 * M2 起从「只有数字」升级为「数字 + 曲线 + 日志导出」：
 * - **曲线**用 SVG 手绘：四条折线不值得多一个图表运行时依赖，而且这样能自己控制
 *   「只画最近 5 分钟」「刷新不重排布局」这些细节；
 * - **导出**把累积的采样点交给后端写成 CSV（落盘路径由命令返回，UI 原样显示）。
 *
 * 数据来自 `audiolink://telemetry` 事件（后端 500 ms 节流），或首屏 `telemetry` 命令的快照。
 *
 * 视觉（On-Air Console）：指标格 = 机箱背部的一格一格读数 —— 1px `line` 网格隔开、
 * 数字一律 `.num`（刷新时不跳字）；缺失读数写「—」并降一级亮度，而不是写 0
 * （0 看着像"链路完美"，而面板最容易骗人的地方正是这里）。
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

/** 读数缺失的占位：不是 0，也不是空（否则看着像"链路完美"）。 */
const DASH = "—";

/**
 * 一个指标格（网格里的一格）。
 *
 * 数值用等宽 + tabular-nums，刷新时不抖动布局（UI 规格 §5）；单位比数字小一号、
 * 暗一级 —— 读数的主次靠字号与丝印层级，不靠颜色饱和。
 */
function Metric({ label, value, unit }: { label: string; value: string; unit?: string }) {
  const missing = value === DASH;
  return (
    <div className="border-r border-b border-stroke-control bg-surface-sunken px-2.5 py-1.5">
      <div className="text-caption font-semibold text-text-tertiary">{label}</div>
      <div className={`num text-body leading-tight ${missing ? "text-text-tertiary" : "text-text-primary"}`}>
        {value}
        {unit === undefined ? null : <span className="ml-1 text-caption text-text-tertiary">{unit}</span>}
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

/** 把一串读数映射成折线上的点（`viewBox="0 0 100 32"`）。 */
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
 * 一条折线：下沉的机箱窗口里的一段走线。
 *
 * 纵轴按**本窗口最大值**归一化：遥测的关注点是「趋势与尖峰」，不是绝对刻度；
 * 因此不画网格与坐标轴，只把「最新值」写在右上角（数字面板里有精确值）。
 * 线用 `silk-2`（中性丝印色）：数据本身不带"好/坏"语义，别让曲线的颜色冒充状态灯。
 */
function Sparkline({ label, unit, values, latest }: SparklineProps) {
  // 空串 = 画不出来：不足两个点、或窗口内全零（画出来是假线）
  const points = sparklinePoints(values);
  const ready = points !== "";

  return (
    <div className="al-well p-2">
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-caption font-semibold text-text-tertiary">{label}</span>
        <span className="num text-caption text-text-secondary">
          {ready ? latest : DASH}
          {ready ? <span className="ml-1 text-text-tertiary">{unit}</span> : null}
        </span>
      </div>
      <svg
        viewBox="0 0 100 32"
        preserveAspectRatio="none"
        className="mt-1 h-8 w-full text-text-secondary"
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
          <text x="50" y="20" textAnchor="middle" className="fill-text-tertiary text-[8px]">
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
  const idle = telemetry === null || telemetry.peers === 0;
  const noReceiver = idle || telemetry?.receiverReport === false;
  const noE2e = noReceiver || !telemetry?.e2eLatencyUs;
  const tele = telemetry;
  // `at(-1)` 在严格模式下是 `T | undefined`：显式收敛成 null，避免四处各写一遍下标。
  const last = history.at(-1) ?? null;

  return (
    // id 保留给状态条的 aria-controls（"遥测"方键指向这块面板）
    <section id="telemetry-readout" aria-labelledby="telemetry-heading" className="al-card p-3">
      <header className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <h2 id="telemetry-heading" className="text-caption font-semibold text-text-secondary text-body">
          {t("tm.title")}
        </h2>
        <span className="flex items-baseline gap-1.5">
          <span className="text-caption font-semibold text-text-tertiary text-text-secondary">{t("tm.peers")}</span>
          <span className="num text-caption text-text-secondary">{tele === null ? DASH : tele.peers}</span>
        </span>
        <button
          type="button"
          onClick={onExport}
          disabled={exporting || history.length === 0}
          className="al-btn ml-auto h-7 shrink-0 px-2.5 text-caption text-text-primary disabled:cursor-not-allowed"
        >
          {exporting ? t("tm.exporting") : t("tm.export", { count: history.length })}
        </button>
      </header>

      <p className="mt-1 mb-2 text-caption text-text-tertiary">{t("tm.footer", { count: history.length })}</p>

      {/* 网格线靠「容器上/左边框 + 每格右/下边框」拼出来：最后一格不满行时不会露出一块底色 */}
      <div className="grid border-t border-l border-stroke-control [grid-template-columns:repeat(auto-fill,minmax(140px,1fr))]">
        <Metric label={t("tm.peers")} value={tele === null ? DASH : String(tele.peers)} />
        <Metric label="RTT" value={tele === null || idle ? DASH : usToMs(tele.rttUs)} unit="ms" />
        <Metric label={t("tm.jitter")} value={tele === null || idle ? DASH : usToMs(tele.jitterUs)} unit="ms" />
        <Metric label={t("quality.loss")} value={tele === null || noReceiver ? DASH : tele.lossPct.toFixed(2)} unit="%" />
        <Metric label={t("tm.bitrate")} value={tele === null || idle ? DASH : bpsToKbps(tele.bitrateBps)} unit="kbps" />
        <Metric label={t("tm.buffer")} value={tele === null || noReceiver ? DASH : usToMs(tele.bufferLevelUs)} unit="ms" />
        <Metric label={t("tm.underruns")} value={tele === null || noReceiver ? DASH : String(tele.underruns)} unit={t("tm.count")} />
        <Metric label={t("tm.e2e")} value={tele === null || noE2e ? DASH : usToMs(tele.e2eLatencyUs)} unit="ms" />
        <Metric label={t("tm.e2e_p50")} value={tele === null || noE2e ? DASH : usToMs(tele.e2eP50Us)} unit="ms" />
        <Metric label={t("tm.e2e_p95")} value={tele === null || noE2e ? DASH : usToMs(tele.e2eP95Us)} unit="ms" />
      </div>

      <div className="mt-2 grid gap-2 [grid-template-columns:repeat(auto-fill,minmax(200px,1fr))]">
        <Sparkline
          label={t("tm.e2e_latency")}
          unit="ms"
          values={history.filter((row) => row.e2eLatencyUs > 0).map((row) => row.e2eLatencyUs)}
          latest={noE2e ? DASH : usToMs(last?.e2eLatencyUs ?? 0)}
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
          values={history.filter((row) => row.receiverReport !== false).map((row) => Math.round(row.lossPct * 100))}
          latest={noReceiver ? DASH : (last?.lossPct ?? 0).toFixed(2)}
        />
      </div>

      {idle ? <p className="mt-2 text-caption text-text-tertiary">{t("tm.idle")}</p> : null}
      {!idle && noE2e ? <p className="mt-2 text-caption text-text-tertiary">{t("quality.e2e_unavailable")}</p> : null}
    </section>
  );
}
