/**
 * 横幅外壳 —— 失败与回执共用的一层皮。
 *
 * 存在的理由：`ErrorBanner` 与 `NoticeBanner` 除 role、色调与图标外**完全同构**
 * （同样的 flex 布局、同样的关闭按钮、同样的 padding 与圆角）。同构写第二遍还能忍，
 * 第三遍起就会各自漂移：一处改了内边距、另一处没跟上，而它们在屏幕上是上下相邻的两条。
 *
 * 分工：这里只管**结构**（图标槽 / 内容槽 / 动作槽 / 关闭键），色调与角色由调用点声明。
 * 两个调用点（`ErrorBanner` / `NoticeBanner`）的对外 props 一个字都没变 ——
 * 它们把「色调 + 角色 + 图标」填进来，`App` 侧看不到这一层。
 */

import type { ReactNode } from "react";

import { t } from "../i18n";

type BannerTone = "critical" | "caution";

/**
 * 色调 → 类名。
 *
 * 写成静态字符串而不是 `border-${tone}` 拼串：Tailwind 扫的是源码里的**字面量**，
 * 拼出来的类名它看不见，构建产物里根本不会有这条规则。
 */
const TONE: Record<BannerTone, { border: string; ink: string }> = {
  critical: { border: "border-critical", ink: "text-critical" },
  caution: { border: "border-caution", ink: "text-caution" },
};

interface BannerProps {
  /** 色调：失败用 critical，动作回执用 caution（这台机器上唯一的「注意」色）。 */
  tone: BannerTone;
  /**
   * 无障碍角色，两者不是风格差异：
   * - `alert` = 失败，读屏立刻打断当前朗读；
   * - `status` = 回执，等用户空闲了再说。
   *
   * 把「已导出 120 个采样点」当 alert 播报，等于在用户正听着什么的时候插一嘴。
   */
  role: "alert" | "status";
  /** 状态图标；颜色由 `tone` 统一给（图标继承 currentColor），调用点只管尺寸。 */
  icon: ReactNode;
  /** 关闭键左侧的动作槽（可选）；不给就只有一个关闭键。 */
  actions?: ReactNode;
  onDismiss: () => void;
  children: ReactNode;
}

export function Banner({ tone, role, icon, actions, onDismiss, children }: BannerProps) {
  const skin = TONE[tone];
  return (
    <div
      role={role}
      className={`flex items-start gap-2.5 border bg-surface-card-solid px-3 py-2.5 text-caption ${skin.border}`}
    >
      {/* 图标与文字首行对齐（flex 子元素已被块级化，`mt-0.5` 在这里是生效的）；
          读屏读的是 children 里那句话，图标本身 aria-hidden（见 icons.tsx 的 Base）。 */}
      <span className={`mt-0.5 shrink-0 ${skin.ink}`}>{icon}</span>
      <div className="min-w-0">{children}</div>
      {/* 动作与关闭键同组靠右：没有动作时它就是一个普通的右侧关闭键，布局与合并前逐像素一致 */}
      <div className="ml-auto flex shrink-0 items-center gap-2">
        {actions}
        <button
          type="button"
          onClick={onDismiss}
          aria-label={t("banner.dismiss")}
          className="al-btn al-btn-md"
        >
          {t("banner.close")}
        </button>
      </div>
    </div>
  );
}
