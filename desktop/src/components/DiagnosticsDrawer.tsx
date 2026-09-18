/**
 * 诊断抽屉：这台机器的背板。
 *
 * PRODUCT.md 的原则 1 与 5：首屏只回答「在推什么、给谁、质量如何」，
 * 而内核真值（水位、欠载、丢包掩盖、缓冲帧、对齐、组账本、设置、更新、第三方声明）
 * **一个都不删**，只是默认折到背板后面 —— 它服务工程师，不参与首屏叙事。
 */

import type { ReactNode } from "react";

import { t } from "../i18n";
import { IconChevron } from "./icons";

interface DiagnosticsDrawerProps {
  open: boolean;
  onClose: () => void;
  children: ReactNode;
}

export function DiagnosticsDrawer({ open, onClose, children }: DiagnosticsDrawerProps) {
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
        {children}
      </div>
    </section>
  );
}
