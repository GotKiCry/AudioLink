/**
 * 失败原因横幅。
 *
 * 存在的理由：UI 规格 §4「错误必说人话」+ 架构 §11「要么处理、要么上报」。
 * 因此这里**一定**要显示三件事：错误码（`1002`/`1008`…，方便对照协议文档）、
 * 人话原因（用户看）、机械上下文（排查用，不隐藏也不喧哗）。
 */

import type { CommandError } from "../types";
import { t } from "../i18n";
import { IconStateFailed } from "./icons";

interface ErrorBannerProps {
  error: CommandError;
  onDismiss: () => void;
}

export function ErrorBanner({ error, onDismiss }: ErrorBannerProps) {
  return (
    <div
      role="alert"
      className="flex items-start gap-2.5 border border-lamp-live bg-panel px-3 py-2.5 t-cap"
    >
      <IconStateFailed className="mt-0.5 h-4 w-4 shrink-0 text-ink-live" />
      <div className="min-w-0">
        <div className="font-medium text-ink-live">{error.message}</div>
        <div className="num mt-1 break-all t-cap text-silk-3">
          code={error.code}
          {error.context === "" ? "" : ` · ${error.context}`}
        </div>
      </div>
      <button
        type="button"
        onClick={onDismiss}
        aria-label={t("banner.dismiss")}
        className="key ml-auto h-7 shrink-0 px-2 t-cap"
      >
        {t("banner.close")}
      </button>
    </div>
  );
}
