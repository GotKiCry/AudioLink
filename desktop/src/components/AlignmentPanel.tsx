/**
 * M4 · 多源对齐面板。
 *
 * 它显示的是**采样级对齐的直接读数**：用同一个「现在」推算各路样本编号，跨度就是时间轴错位量。
 * 为什么值得单独一块面板：多源混音出错时，耳朵听到的是"回声"，而这里能看到**差了多少毫秒**。
 *
 * 判据（与内核一致，`docs/39-m4-common-time-base.md`）：跨度 ≤ 1 帧（960 样本 / 20 ms）= 已对齐。
 *
 * 那个按钮不是装饰：**接收端才是基准来源**，所以「广播共同基准」只能在混音方按 ——
 * 它把各发送端的样本编号钉到同一个原点上（`docs/40-m4-alignment-panel.md`）。
 */

import { useState } from "react";

import type { AlignmentView, AlignmentVerdict } from "../types";

interface AlignmentPanelProps {
  alignment: AlignmentView | null;
  /** 广播进行中（按钮禁用，避免连点重复发）。 */
  busy: boolean;
  onBroadcast: (leadMs: number) => void;
}

const VERDICT_TEXT: Record<AlignmentVerdict, string> = {
  aligned: "已对齐",
  drifting: "错位",
  unknown: "读数不足",
};

const VERDICT_STYLE: Record<AlignmentVerdict, string> = {
  aligned: "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300",
  drifting: "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-300",
  unknown: "bg-slate-100 text-slate-500 dark:bg-slate-800 dark:text-slate-400",
};

/**
 * 默认提前量（ms）。
 *
 * 太小发送端来不及等到那个时刻（会整段丢掉），太大只是让大家多等一会 —— 所以默认给宽一点。
 */
const DEFAULT_LEAD_MS = 200;

export function AlignmentPanel({ alignment, busy, onBroadcast }: AlignmentPanelProps) {
  const [leadMs, setLeadMs] = useState(DEFAULT_LEAD_MS);
  const verdict: AlignmentVerdict = alignment?.verdict ?? "unknown";
  const spreadSamples = alignment?.spreadSamples ?? null;
  const spreadMs = alignment?.spreadMs ?? null;

  return (
    <section className="rounded-lg border border-slate-200 bg-white p-4 shadow-sm dark:border-slate-700 dark:bg-slate-800">
      <header className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div>
          <h2 className="text-sm font-semibold">多源对齐（M4）</h2>
          <p className="mt-0.5 text-xs text-slate-500 dark:text-slate-400">
            跨度 = 用同一个「现在」推算各路样本编号的差值；一帧 = 960 样本 / 20 ms。
          </p>
        </div>
        <div className="flex items-center gap-2">
          <label className="flex items-center gap-1 text-xs text-slate-500 dark:text-slate-400">
            提前量
            <input
              type="number"
              min={0}
              max={5000}
              step={50}
              value={leadMs}
              onChange={(event) => setLeadMs(Number(event.target.value))}
              className="w-16 rounded border border-slate-300 px-1 py-0.5 text-xs dark:border-slate-600 dark:bg-slate-900"
            />
            ms
          </label>
          <button
            type="button"
            disabled={busy}
            onClick={() => onBroadcast(leadMs)}
            className="rounded bg-sky-600 px-2 py-1 text-xs font-medium text-white hover:bg-sky-500 disabled:opacity-50"
          >
            {busy ? "广播中…" : "广播共同基准"}
          </button>
          <span className={`rounded px-2 py-0.5 text-xs font-medium ${VERDICT_STYLE[verdict]}`}>
            {VERDICT_TEXT[verdict]}
          </span>
        </div>
      </header>

      {alignment === null || alignment.axes.length === 0 ? (
        <p className="text-xs text-slate-400">还没有会话：接受对端推流后这里会显示每一路的读数。</p>
      ) : (
        <>
          <p className="mb-2 text-xs text-slate-600 dark:text-slate-300">
            {spreadSamples === null
              ? "有效读数不足两路，无法比较（需要至少两路同时在收音频）。"
              : `当前跨度：${spreadSamples} 样本${spreadMs === null ? "" : ` ≈ ${spreadMs} ms`}`}
          </p>
          <table className="w-full text-xs">
            <thead className="text-slate-500 dark:text-slate-400">
              <tr>
                <th className="text-left font-medium">对端</th>
                <th className="text-right font-medium">最近编号</th>
                <th className="text-right font-medium">推算编号</th>
                <th className="text-right font-medium">到达 (ms)</th>
              </tr>
            </thead>
            <tbody>
              {alignment.axes.map((axis) => (
                <tr key={axis.peerShort} className="border-t border-slate-100 dark:border-slate-700">
                  <td className="py-1 font-mono">{axis.peerShort}</td>
                  <td className="py-1 text-right font-mono">{axis.sampleIndex ?? "—"}</td>
                  <td className="py-1 text-right font-mono">{axis.indexNow ?? "—"}</td>
                  <td className="py-1 text-right font-mono">{axis.atMs}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </section>
  );
}
