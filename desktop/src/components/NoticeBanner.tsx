/**
 * 提示横幅（非错误）。
 *
 * 存在的理由：有些结果是**动作回执**而不是失败 —— 例如「已导出 120 个采样点」、
 * 「aaaa1111 音量已设为 60%」、启动时自动重连成功。
 * 把它们丢进红色错误横幅，等于教用户"出问题了，重试吧"，那是错的指引。
 *
 * 世界语汇：铜金是这台机器上唯一的"注意"色（刻度、告警、待办），它不喊，但它一直在。
 * 外壳与错误横幅共用 `Banner`，差别只在色调、角色与图标这三样。
 */
import { Banner } from "./Banner";
import { IconStateConnecting } from "./icons";

interface NoticeBannerProps {
  message: string;
  onDismiss: () => void;
}

export function NoticeBanner({ message, onDismiss }: NoticeBannerProps) {
  return (
    <Banner
      role="status"
      tone="caution"
      icon={<IconStateConnecting className="h-4 w-4" />}
      onDismiss={onDismiss}
    >
      <div className="text-caution">{message}</div>
    </Banner>
  );
}
