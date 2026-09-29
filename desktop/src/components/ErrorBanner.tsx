/**
 * 失败原因横幅。
 *
 * 存在的理由：UI 规格 §4「错误必说人话」+ 架构 §11「要么处理、要么上报」。
 * 因此这里**一定**要显示三件事：错误码（`1002`/`1008`…，方便对照协议文档）、
 * 人话原因（用户看）、机械上下文（排查用，不隐藏也不喧哗）。
 *
 * 外壳（布局 / 关闭键 / 图标槽）与提示横幅共用 `Banner` —— 两条横幅在屏幕上是上下相邻的，
 * 各自维护一份内边距只会让它们慢慢长得不一样。
 */

import type { CommandError } from "../types";
import { Banner } from "./Banner";
import { IconStateFailed } from "./icons";

interface ErrorBannerProps {
  error: CommandError;
  onDismiss: () => void;
}

export function ErrorBanner({ error, onDismiss }: ErrorBannerProps) {
  return (
    <Banner
      role="alert"
      tone="critical"
      icon={<IconStateFailed className="h-4 w-4" />}
      onDismiss={onDismiss}
    >
      <div className="font-medium text-critical">{error.message}</div>
      <div className="num mt-1 break-all text-caption text-text-tertiary">
        code={error.code}
        {error.context === "" ? "" : ` · ${error.context}`}
      </div>
    </Banner>
  );
}
