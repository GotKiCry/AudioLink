/**
 * M4 · 多源对齐面板。
 *
 * 它显示的是**采样级对齐的直接读数**：用同一个「现在」推算各路样本编号，跨度就是时间轴错位量。
 * 为什么值得单独一块面板：多源混音出错时，耳朵听到的是"回声"，而这里能看到**差了多少毫秒**。
 *
 * 判据（与内核一致，`docs/39-m4-common-time-base.md`）：跨度 ≤ 1 帧（960 样本 / 20 ms）= 已对齐。
 */

import type { AlignmentView, AlignmentVerdict } from "../types";

interface AlignmentPanelProps {
  alignment: AlignmentView | null;
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

export function AlignmentPanel({ alignment }: AlignmentPanelProps) {
  const verdict: AlignmentVerdict = alignment?.verdict ?? "unknown";
  const spreadSamples = alignment?.spreadSamples ?? null;
  const spreadMs = alignment?.spreadMs ?? null;

  return (
    <section className="rounded-lg border border-slate-200 bg-white p-4 shadow-sm dark:border-slate-700 dark:bg-slate-800">
      <header className="mb-3 flex items-center justify-between gap-2">
        <div>
          <h2 className="text-sm font-semibold">多源对齐（M4）</h2>
          <p className="mt-0.5 text-xs text-slate-500 dark:text-slate-400">
            跨度 = 用同一个「现在」推算各路样本编号的差值；一帧 = 960 样本 / 20 ms。
          </p>
        </div>
        <span className={`rounded px-2 py-0.5 text-xs font-medium ${VERDICT_STYLE[verdict]}`}>
          {VERDICT_TEXT[verdict]}
        </span>
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
