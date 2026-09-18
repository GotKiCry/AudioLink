/**
 * 外观切换器：现在只剩**一条轴** —— 光照（data-theme = light | dark）。
 *
 * data-style（Fluent / macOS 两种设计语言）已经删掉：产品只保留 Fluent 2 一套语言，
 * 留着那半个分段控件只会让人以为「选了 macOS 会更好看」，而它背后什么都没有。
 *
 * 主题状态由 App 的 useTheme() 持有（App 还要按它取背景默认值），这里只渲染按钮。
 */

import { t } from "../i18n";
import { IconThemeAuto, IconThemeDark, IconThemeLight } from "./icons";
import type { ThemeMode } from "../lib/useTheme";

interface AppearanceSwitcherProps {
  mode: ThemeMode;
  onCycle: () => void;
}

export function AppearanceSwitcher({ mode, onCycle }: AppearanceSwitcherProps) {
  const label =
    mode === "light"
      ? t("topbar.theme_light")
      : mode === "dark"
        ? t("topbar.theme_dark")
        : t("topbar.theme_system");
  const Icon = mode === "light" ? IconThemeLight : mode === "dark" ? IconThemeDark : IconThemeAuto;

  return (
    <button
      type="button"
      onClick={onCycle}
      aria-label={t("appearance.theme", { mode: label })}
      title={t("appearance.theme", { mode: label })}
      className="al-btn h-8 w-8"
    >
      <Icon className="h-4 w-4" />
    </button>
  );
}
