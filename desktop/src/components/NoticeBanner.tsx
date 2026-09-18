/**
 * 提示横幅（非错误）。
 *
 * 存在的理由：有些"命令返回 Err"的情形在**用户语义上不是失败** ——
 * 最典型的是 `connect` 返回 `1002 NOT_PAIRED`：会话与控制通道还活着，
 * 用户接下来要做的是输配对码（见 `EngineBridge::connect` 的注释）。
 * 把它丢进红色错误横幅，等于教用户"连接失败了，重试吧"，那是错的指引。
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
