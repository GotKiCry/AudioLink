/**
 * 提示横幅（非错误）。
 *
 * 存在的理由：有些"命令返回 Err"的情形在**用户语义上不是失败** ——
 * 最典型的是 `connect` 返回 `1002 NOT_PAIRED`：会话与控制通道还活着，
 * 用户接下来要做的是输配对码（见 `EngineBridge::connect` 的注释）。
 * 把它丢进红色错误横幅，等于教用户"连接失败了，重试吧"，那是错的指引。
 */
import { t } from "../i18n";

interface NoticeBannerProps {
  message: string;
  onDismiss: () => void;
}

export function NoticeBanner({ message, onDismiss }: NoticeBannerProps) {
  return (
    <div
      role="status"
      className="flex items-start gap-3 rounded-xl border border-indigo-200 bg-indigo-50 px-4 py-3 text-sm dark:border-indigo-900 dark:bg-indigo-950"
    >
      <span aria-hidden="true" className="text-indigo-600">
        i
      </span>
      <div className="min-w-0 text-indigo-700 dark:text-indigo-300">{message}</div>
      <button
        type="button"
        onClick={onDismiss}
        aria-label={t("banner.dismiss")}
        className="ml-auto h-6 shrink-0 rounded px-2 text-xs text-indigo-600 hover:bg-indigo-100 dark:hover:bg-indigo-900"
      >
        {t("banner.close")}
      </button>
    </div>
  );
}
