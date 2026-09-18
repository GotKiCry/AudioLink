/**
 * 图标集：这个世界自己的手 —— 20px 视框、1.5px 描边、方角端点。
 *
 * 三条纪律（对应 UI 规格 §5 与设计底线）：
 *  1. **不用 Unicode 符号冒充图标**：`▶` `✔` `▾` 会被屏幕阅读器逐字符朗读成"黑色右三角"；
 *  2. 状态图标永远与文字同时出现（不靠颜色、也不靠图标单独传达状态）；
 *  3. 描边、端点、圆角与面板上的丝印同一种手：直、硬、不带曲线装饰。
 */

import type { ReactNode } from "react";

type IconProps = { className?: string };

function Base({ className = "", children }: IconProps & { children: ReactNode }) {
  return (
    <svg
      viewBox="0 0 20 20"
      width="18"
      height="18"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="square"
      strokeLinejoin="miter"
      aria-hidden="true"
      focusable="false"
      className={`shrink-0 ${className}`}
    >
      {children}
    </svg>
  );
}

/* ── 会话状态：五个，各自有形状，颜色只是第二通道 ─────────────────── */
export const IconStateIdle = ({ className }: IconProps) => (
  <Base className={className}><circle cx="10" cy="10" r="5.5" /></Base>
);
export const IconStateConnecting = ({ className }: IconProps) => (
  <Base className={className}>
    <circle cx="10" cy="10" r="5.5" strokeDasharray="3 3" />
    <circle cx="10" cy="10" r="1.6" fill="currentColor" stroke="none" />
  </Base>
);
export const IconStateStreaming = ({ className }: IconProps) => (
  <Base className={className}>
    <circle cx="10" cy="10" r="7.5" />
    <path d="M8 6.8 14 10 8 13.2Z" fill="currentColor" stroke="none" />
  </Base>
);
export const IconStateDegraded = ({ className }: IconProps) => (
  <Base className={className}>
    <path d="M10 3.2 17.2 16.4H2.8Z" />
    <path d="M10 7.6v4" />
    <circle cx="10" cy="13.9" r="0.9" fill="currentColor" stroke="none" />
  </Base>
);
export const IconStateFailed = ({ className }: IconProps) => (
  <Base className={className}><path d="M5 5l10 10M15 5 5 15" /></Base>
);

/* ── 面板（主题）三态 ─────────────────────────────────────────────── */
export const IconThemeAuto = ({ className }: IconProps) => (
  <Base className={className}>
    <circle cx="10" cy="10" r="6" />
    <path d="M10 4a6 6 0 0 1 0 12Z" fill="currentColor" stroke="none" />
  </Base>
);
export const IconThemeLight = ({ className }: IconProps) => (
  <Base className={className}>
    <circle cx="10" cy="10" r="3.6" />
    <path d="M10 1.8v2.2M10 16v2.2M1.8 10h2.2M16 10h2.2M4.2 4.2l1.6 1.6M14.2 14.2l1.6 1.6M15.8 4.2l-1.6 1.6M5.8 14.2l-1.6 1.6" />
  </Base>
);
export const IconThemeDark = ({ className }: IconProps) => (
  <Base className={className}><path d="M15.2 12.4A6 6 0 0 1 7.6 4.8a6 6 0 1 0 7.6 7.6Z" /></Base>
);

/* ── 动作 ────────────────────────────────────────────────────────── */
export const IconStart = ({ className }: IconProps) => (
  <Base className={className}><path d="M6.5 4.5 15.5 10 6.5 15.5Z" fill="currentColor" stroke="none" /></Base>
);
export const IconStop = ({ className }: IconProps) => (
  <Base className={className}><rect x="5.5" y="5.5" width="9" height="9" /></Base>
);
export const IconMic = ({ className }: IconProps) => (
  <Base className={className}>
    <rect x="8" y="2.5" width="4" height="8.5" rx="2" />
    <path d="M5 9.5a5 5 0 0 0 10 0M10 14.5V17.5" />
  </Base>
);
export const IconMonitor = ({ className }: IconProps) => (
  <Base className={className}>
    <rect x="2.5" y="3.5" width="15" height="10.5" />
    <path d="M7 17h6M10 14v3" />
  </Base>
);
export const IconSpeaker = ({ className }: IconProps) => (
  <Base className={className}>
    <path d="M4 7.5h2.8L11 4.5v11L6.8 12.5H4Z" />
    <path d="M14 7.5a3.5 3.5 0 0 1 0 5" />
  </Base>
);
export const IconLink = ({ className }: IconProps) => (
  <Base className={className}>
    <path d="M3.5 10h9M10 6.5 13.5 10 10 13.5" />
    <path d="M15.5 4v12" />
  </Base>
);
export const IconUnplug = ({ className }: IconProps) => (
  <Base className={className}>
    <path d="M4 6.5h12M8 6.5V4h4v2.5M5.5 6.5 6.8 17h6.4L14.5 6.5" />
  </Base>
);
export const IconRefresh = ({ className }: IconProps) => (
  <Base className={className}>
    <path d="M16 10a6 6 0 1 1-1.8-4.3" />
    <path d="M16.2 3.5V6.8h-3.3" />
  </Base>
);
export const IconChevron = ({ className }: IconProps) => (
  <Base className={className}><path d="M6 8.5 10 12.5 14 8.5" /></Base>
);
