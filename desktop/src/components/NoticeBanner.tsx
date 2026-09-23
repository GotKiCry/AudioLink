/**
 * 提示横幅（非错误）。
 *
 * 存在的理由：有些结果是**动作回执**而不是失败 —— 例如「已导出 120 个采样点」、
 * 「aaaa1111 音量已设为 60%」、启动时自动重连成功。
 * 把它们丢进红色错误横幅，等于教用户"出问题了，重试吧"，那是错的指引。
 *
 * 世界语汇：铜金是这台机器上唯一的"注意"色（刻度、告警、待办），它不喊，但它一直在。
 */
import { t } from "../i18n";
import { IconStateConnecting } from "./icons";

interface NoticeBannerProps {
  message: string;
  onDismiss: () => void;
}

export function NoticeBanner({ message, onDismiss }: NoticeBannerProps) {
  return (
    <div
      role="status"
      className="flex items-start gap-2.5 border border-caution bg-surface-card-solid px-3 py-2.5 text-caption"
    >
      <IconStateConnecting className="mt-0.5 h-4 w-4 shrink-0 text-caution" />
      <div className="min-w-0 text-caution">{message}</div>
      <button
        type="button"
        onClick={onDismiss}
        aria-label={t("banner.dismiss")}
        className="al-btn ml-auto h-7 shrink-0 px-2 text-caption"
      >
        {t("banner.close")}
      </button>
    </div>
  );
}
