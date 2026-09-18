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
 *
 * 视觉：读数表是一张丝印图纸 —— 1px `line` 网格、表头 `.silk-sm`、数字一律 `.num` 右对齐；
 * 结论不是一个彩色药丸，而是**一盏灯 + 一句人话**（灯用世界里的三盏灯）。
 */

import { useState } from "react";

import type { AlignmentView, AlignmentVerdict } from "../types";
import { t, type MessageKey } from "../i18n";

interface AlignmentPanelProps {
  alignment: AlignmentView | null;
  /** 广播进行中（按钮禁用，避免连点重复发）。 */
  busy: boolean;
  onBroadcast: (leadMs: number) => void;
}

/** 读数缺失时的占位（与遥测面板同一条约定）。 */
const DASH = "—";

/** 对齐结论文案：同样写成函数（理由见 `types.ts` 的 `peerStateLabel`）。 */
function verdictText(verdict: AlignmentVerdict): string {
  const key: Record<AlignmentVerdict, MessageKey> = {
    aligned: "align.aligned",
    drifting: "align.drifting",
    unknown: "align.unknown",
  };
  return t(key[verdict]);
}

/**
 * 结论 → 灯 + 字色。
 *
 * 用世界里的灯（通 / 注意 / 熄）而不是三块彩色底：错位的**严重程度**由灯语表达，
 * 而且灯旁边永远跟着人话 —— 分级不靠颜色单独承载（UI 规格 §5）。
 */
const VERDICT_STYLE: Record<AlignmentVerdict, { lamp: string; text: string }> = {
  aligned: { lamp: "on", text: "text-ink-on" },
  drifting: { lamp: "warn", text: "text-ink-warn" },
  unknown: { lamp: "off", text: "text-ink-idle" },
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
  const verdictStyle = VERDICT_STYLE[verdict];

  return (
    <section aria-labelledby="align-heading" className="plate p-3">
      <header className="mb-1.5 flex flex-wrap items-center gap-x-3 gap-y-2">
        <h2 id="align-heading" className="silk t-body">
          {t("align.title")}
        </h2>
        <span className="flex items-baseline gap-1.5">
          <span className="silk-sm !text-silk-2">{t("align.peer")}</span>
          <span className="num t-cap text-silk-2">
            {alignment === null ? DASH : alignment.axes.length}
          </span>
        </span>

        <span className="flex items-center gap-1.5 rounded-chip border border-line px-2 py-0.5">
          <span className="lamp h-1.5 w-1.5 shrink-0" data-on={verdictStyle.lamp} />
          <span className={`silk-sm ${verdictStyle.text}`}>{verdictText(verdict)}</span>
        </span>

        <label htmlFor="align-lead" className="ml-auto flex items-center gap-1.5 t-cap text-silk-2">
          <span className="silk-sm !text-silk-2">{t("group.lead")}</span>
          <input
            id="align-lead"
            type="number"
            min={0}
            max={5000}
            step={50}
            value={leadMs}
            onChange={(event) => setLeadMs(Number(event.target.value))}
            className="well num w-16 rounded-chip px-1.5 py-0.5 text-right t-cap text-silk"
          />
          <span className="num text-silk-3">ms</span>
        </label>

        <button
          type="button"
          disabled={busy}
          onClick={() => onBroadcast(leadMs)}
          className="key key-primary h-7 shrink-0 px-2.5 t-cap disabled:cursor-not-allowed"
        >
          {busy ? t("align.busy") : t("align.broadcast")}
        </button>
      </header>

      <p className="mb-2 t-cap leading-4 text-silk-3">{t("align.hint")}</p>

      {alignment === null || alignment.axes.length === 0 ? (
        <p className="t-cap text-silk-3">{t("align.no_session")}</p>
      ) : (
        <>
          <p className="mb-1.5 t-cap text-silk-2">
            {spreadSamples === null
              ? t("align.need_two")
              : spreadMs === null
                ? t("align.spread", { samples: spreadSamples })
                : t("align.spread_ms", { samples: spreadSamples, ms: spreadMs })}
          </p>
          <table className="w-full border-collapse t-cap">
            <thead>
              <tr>
                <th scope="col" className="silk-sm border-b border-line px-2 py-1 text-left">
                  {t("align.peer")}
                </th>
                <th scope="col" className="silk-sm border-b border-line px-2 py-1 text-right">
                  {t("align.recent")}
                </th>
                <th scope="col" className="silk-sm border-b border-line px-2 py-1 text-right">
                  {t("align.estimated")}
                </th>
                <th scope="col" className="silk-sm border-b border-line px-2 py-1 text-right">
                  {t("align.arrival")}
                </th>
              </tr>
            </thead>
            <tbody>
              {alignment.axes.map((axis) => (
                <tr key={axis.peerShort} className="border-b border-line">
                  <td className="num px-2 py-1 text-silk-2">{axis.peerShort}</td>
                  <td className="num px-2 py-1 text-right text-silk">{axis.sampleIndex ?? DASH}</td>
                  <td className="num px-2 py-1 text-right text-silk">{axis.indexNow ?? DASH}</td>
                  <td className="num px-2 py-1 text-right text-silk-2">{axis.atMs}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </section>
  );
}
