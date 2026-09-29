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
 * 视觉：读数表是 1px `line` 网格，表头加一级字重、数字一律 `.num` 右对齐；结论不是一个彩色
 * 药丸，而是**一盏灯 + 一句人话** —— 灯与结论文字共用同一档语义色（`.al-lamp` 的 on/warn/off
 * 与 `text-success / text-caution / text-text-tertiary`），分级不靠颜色单独承载（UI 规格 §5）。
 * 字号与颜色只走语义 token，明暗由 `index.css` 的变量决定。
 * 按钮尺寸一律用 `.al-btn-*` 档位类，且不留 `h-* / px-* / text-*`：utilities 层在
 * components 层之上，留着会把档位高度盖回去。
 */

import { useState } from "react";

import type { AlignmentView, AlignmentVerdict } from "../types";
import { t, type MessageKey } from "../i18n";
import { parseLeadMs, type LeadBounds } from "../lib/leadInput";
import { Panel } from "./Panel";

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
 *
 * ⚠️ `text` 是结论文字**唯一**的颜色来源：调用处不能再挂一个基础色类。utilities 之间没有
 * 优先级，只有产物里的出现顺序 —— 一旦多挂一个 `text-text-tertiary`，它会因为排在
 * `text-success` 之后而把绿/琥珀压成灰（三态看起来一模一样）。
 */
const VERDICT_STYLE: Record<AlignmentVerdict, { lamp: string; text: string }> = {
  aligned: { lamp: "on", text: "text-success" },
  drifting: { lamp: "warn", text: "text-caution" },
  unknown: { lamp: "off", text: "text-text-tertiary" },
};

/**
 * 默认提前量（ms）。
 *
 * 太小发送端来不及等到那个时刻（会整段丢掉），太大只是让大家多等一会 —— 所以默认给宽一点。
 */
const DEFAULT_LEAD_MS = 200;

/** 输入框的取值域：与 `min` / `max` 属性是同一条约束（属性只是提示，真正执行的是 parseLeadMs）。 */
const MIN_LEAD_MS = 0;
const MAX_LEAD_MS = 5000;

/**
 * 本面板的取值域。
 *
 * 解析规则本身（空串 = 未填、超范围收敛、0 合法）在 `lib/leadInput.ts`，与同步组面板共用一份 ——
 * 两个面板之间只有这里的数字不同，规则必须完全一致：各写一份就是漂移的开始
 * （改了一处忘了另一处，同一件事在两个面板上给出不同结论）。
 */
const LEAD_BOUNDS: LeadBounds = { min: MIN_LEAD_MS, max: MAX_LEAD_MS, fallback: DEFAULT_LEAD_MS };

export function AlignmentPanel({ alignment, busy, onBroadcast }: AlignmentPanelProps) {
  // 存用户看到的原文；`lead` 是**提交时**才算出来的（valid: false = 还没填好）
  const [leadText, setLeadText] = useState(String(DEFAULT_LEAD_MS));
  const lead = parseLeadMs(leadText, LEAD_BOUNDS);
  const verdict: AlignmentVerdict = alignment?.verdict ?? "unknown";
  const spreadSamples = alignment?.spreadSamples ?? null;
  const spreadMs = alignment?.spreadMs ?? null;
  const verdictStyle = VERDICT_STYLE[verdict];

  return (
    <Panel
      id="align"
      title={t("align.title")}
      meta={
        <>
          <span className="flex items-baseline gap-1.5">
            <span className="text-caption font-semibold text-text-secondary">{t("align.peer")}</span>
            <span className="num text-caption text-text-secondary">
              {alignment === null ? DASH : alignment.axes.length}
            </span>
          </span>

          <span className="flex items-center gap-1.5 rounded-control border border-stroke-control px-2 py-0.5">
            <span className="al-lamp h-1.5 w-1.5 shrink-0" data-on={verdictStyle.lamp} />
            <span className={`text-caption font-semibold ${verdictStyle.text}`}>
              {verdictText(verdict)}
            </span>
          </span>
        </>
      }
      right={
        <>
          <label
            htmlFor="align-lead"
            className="flex items-center gap-1.5 text-caption text-text-secondary"
          >
            <span className="text-caption font-semibold text-text-secondary">{t("align.lead")}</span>
            <input
              id="align-lead"
              type="number"
              min={MIN_LEAD_MS}
              max={MAX_LEAD_MS}
              step={50}
              value={leadText}
              onChange={(event) => setLeadText(event.target.value)}
              className="al-well num w-16 rounded-control px-1.5 py-0.5 text-right text-caption text-text-primary"
            />
            <span className="num text-text-tertiary">ms</span>
          </label>

          <button
            type="button"
            // 提前量没填（空串 / 非数字）时不给提交：按钮灰着，比发一个用户没看见的值诚实
            disabled={busy || !lead.valid}
            onClick={() => {
              if (lead.valid) {
                onBroadcast(lead.value);
              }
            }}
            className="al-btn al-btn-accent al-btn-md disabled:cursor-not-allowed"
          >
            {busy ? t("align.busy") : t("align.broadcast")}
          </button>
        </>
      }
    >
      {/* 头部不带外边距（间距由这里自己给），换一块内容只需改这一行 */}
      <p className="mt-1.5 mb-2 text-caption leading-4 text-text-tertiary">{t("align.hint")}</p>

      {alignment === null || alignment.axes.length === 0 ? (
        <p className="text-caption text-text-tertiary">{t("align.no_session")}</p>
      ) : (
        <>
          <p className="mb-1.5 text-caption text-text-secondary">
            {spreadSamples === null
              ? t("align.need_two")
              : spreadMs === null
                ? t("align.spread", { samples: spreadSamples })
                : t("align.spread_ms", { samples: spreadSamples, ms: spreadMs })}
          </p>
          <table className="w-full border-collapse text-caption">
            <thead>
              <tr>
                <th scope="col" className="text-caption font-semibold text-text-tertiary border-b border-stroke-control px-2 py-1 text-left">
                  {t("align.peer")}
                </th>
                <th scope="col" className="text-caption font-semibold text-text-tertiary border-b border-stroke-control px-2 py-1 text-right">
                  {t("align.recent")}
                </th>
                <th scope="col" className="text-caption font-semibold text-text-tertiary border-b border-stroke-control px-2 py-1 text-right">
                  {t("align.estimated")}
                </th>
                <th scope="col" className="text-caption font-semibold text-text-tertiary border-b border-stroke-control px-2 py-1 text-right">
                  {t("align.arrival")}
                </th>
              </tr>
            </thead>
            <tbody>
              {alignment.axes.map((axis) => (
                <tr key={axis.peerShort} className="border-b border-stroke-control">
                  <td className="num px-2 py-1 text-text-secondary">{axis.peerShort}</td>
                  <td className="num px-2 py-1 text-right text-text-primary">{axis.sampleIndex ?? DASH}</td>
                  <td className="num px-2 py-1 text-right text-text-primary">{axis.indexNow ?? DASH}</td>
                  <td className="num px-2 py-1 text-right text-text-secondary">{axis.atMs}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </Panel>
  );
}
