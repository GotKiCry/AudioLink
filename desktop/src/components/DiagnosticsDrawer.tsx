/**
 * 诊断抽屉：这台机器的背板。
 *
 * PRODUCT.md 的原则 1 与 5：首屏只回答「在推什么、给谁、质量如何」，
 * 而内核真值（水位、欠载、丢包掩盖、缓冲帧、对齐、组账本、设置、更新、第三方声明）
 * **一个都不删**，只是默认折到背板后面 —— 它服务工程师，不参与首屏叙事。
 *
 * 「本机身份」是背板的第一格。机器名 / 指纹 / 监听地址 / 可达地址都是排障时**要被抄走**的值
 * （报日志、核对是不是同一台机器、确认端口），所以它们不能消失，只能从首屏挪到这里 ——
 * 去噪的正确做法是下沉，不是删除。
 */

import type { ReactNode } from "react";

import type { LocalStatus } from "../types";
import { t } from "../i18n";
import { IconChevron } from "./icons";

interface DiagnosticsDrawerProps {
  open: boolean;
  onClose: () => void;
  /** 本机身份；首屏水合完成前为 null（那时身份格给占位符，绝不把 undefined 印在背板上）。 */
  local: LocalStatus | null;
  children: ReactNode;
}

/** 水合未完成时的占位（与其它面板同一条约定：不留空白）。 */
const PLACEHOLDER = "…";

/**
 * 身份字段取值。两种"没有"必须分开说：
 *
 * - 未水合（local 还是 null）→ 占位符，意思是"马上就来"；
 * - 水合了、这一项却是空 → `diag.identity_none`（「未知」）。
 *
 * 为什么不能留空白格：空白看起来像界面坏了，而"未知"是**一条读数** —— 排障时这两种结论完全不同。
 */
function identityValue(
  local: LocalStatus | null,
  pick: (status: LocalStatus) => string | null | undefined,
): string {
  if (local === null) return PLACEHOLDER;
  const raw = pick(local);
  return raw === null || raw === undefined || raw.trim() === "" ? t("diag.identity_none") : raw;
}

/**
 * 一格「标签 + 值」。
 *
 * 值一律 `.num`（等宽）并且整行可选：这几条是用户要抄进工单或日志的东西，
 * 排版的首要任务是不让人抄错（0/O、l/1 在等宽字里才分得清）。
 */
function IdentityRow({ label, value }: { label: string; value: string }) {
  return (
    <>
      <dt className="font-semibold text-text-tertiary">{label}</dt>
      <dd className="num min-w-0 truncate text-text-secondary">{value}</dd>
    </>
  );
}

export function DiagnosticsDrawer({ open, onClose, local, children }: DiagnosticsDrawerProps) {
  if (!open) return null;

  return (
    <section
      id="telemetry-panel"
      aria-label={t("diag.title")}
      className="al-chrome al-chrome-t flex h-[46%] min-h-[240px] shrink-0 flex-col"
    >
      {/* 抽屉是 z2 的一层面（不是 Acrylic）：内部卡片只留填充，不再自建 backdrop（红线 3） */}
      <header className="al-chrome-b flex h-10 shrink-0 items-center gap-3 px-4">
        <span className="text-caption font-semibold text-text-secondary">{t("diag.title")}</span>
        <span className="truncate text-caption text-text-tertiary">{t("diag.hint")}</span>
        <button
          type="button"
          onClick={onClose}
          className="al-btn ml-auto h-7 gap-1.5 px-2.5 text-caption"
        >
          <IconChevron className="h-3.5 w-3.5 rotate-180" />
          <span className="text-caption font-semibold text-text-secondary">{t("banner.close")}</span>
        </button>
      </header>

      <div className="grid min-h-0 flex-1 auto-rows-min grid-cols-1 gap-3 overflow-y-auto p-4 xl:grid-cols-2">
        {/* 身份铭牌：排障第一眼要看的四条读数，排在 children 之前 —— 背板第一格是"这是哪台机器"。 */}
        <section aria-labelledby="diag-identity-heading" className="al-card p-3">
          <header className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
            <h2
              id="diag-identity-heading"
              className="text-caption font-semibold text-text-secondary text-body"
            >
              {t("diag.identity")}
            </h2>
            <span className="min-w-0 truncate text-body text-text-primary">
              {identityValue(local, (status) => status.name)}
            </span>
          </header>
          <dl className="mt-2 grid grid-cols-[max-content_minmax(0,1fr)] items-baseline gap-x-4 gap-y-1 text-caption">
            <IdentityRow label={t("diag.identity_fp")} value={identityValue(local, (status) => status.idShort)} />
            <IdentityRow label={t("diag.identity_listen")} value={identityValue(local, (status) => status.addr)} />
            <IdentityRow label={t("diag.identity_lan")} value={identityValue(local, (status) => status.displayAddr)} />
          </dl>
        </section>

        {children}
      </div>
    </section>
  );
}
