/**
 * 失败原因横幅。
 *
 * 存在的理由：UI 规格 §4「错误必说人话」+ 架构 §11「要么处理、要么上报」。
 * 因此这里**一定**要显示三件事：错误码（`1002`/`1008`…，方便对照协议文档）、
 * 人话原因（用户看）、机械上下文（排查用，不隐藏也不喧哗）。
 */

import type { CommandError } from "../types";

interface ErrorBannerProps {
  error: CommandError;
  onDismiss: () => void;
}

export function ErrorBanner({ error, onDismiss }: ErrorBannerProps) {
  return (
    <div
      role="alert"
      className="flex items-start gap-3 rounded-xl border border-red-200 bg-red-50 px-4 py-3 text-sm dark:border-red-900 dark:bg-red-950"
    >
      <span aria-hidden="true" className="text-red-600">
        !
      </span>
      <div className="min-w-0">
        <div className="font-medium text-red-700 dark:text-red-300">{error.message}</div>
        <div className="mt-0.5 font-mono text-xs break-all text-red-500/80">
          code={error.code}
          {error.context === "" ? "" : ` · ${error.context}`}
        </div>
      </div>
      <button
        type="button"
        onClick={onDismiss}
        aria-label="关闭提示"
        className="ml-auto h-6 shrink-0 rounded px-2 text-xs text-red-600 hover:bg-red-100 dark:hover:bg-red-900"
      >
        关闭
      </button>
    </div>
  );
}
