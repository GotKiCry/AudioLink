/**
 * 外观切换器：两条独立的轴。
 *
 *  风格 data-style：fluent（Windows 11 的 Mica/Layer/Card）| macos（半透明 vibrancy + 浮起的面）
 *  主题 data-theme：system | light | dark —— system 会先解析成实际的 light/dark 再写进 DOM，
 *  这样 CSS 只需要处理两种光照，不必到处写媒体查询。
 *
 * 两个偏好都是**纯前端偏好**，存 localStorage，不进内核设置文件：
 * 面板选错了不会让谁没声音，没有任何理由让它们跨进程往返一次。
 */

import { useEffect, useState } from "react";

import { t } from "../i18n";
import { IconThemeAuto, IconThemeDark, IconThemeLight } from "./icons";

export type StyleId = "fluent" | "macos";
export type ThemeMode = "system" | "light" | "dark";

const STYLE_KEY = "audiolink.style";
const THEME_KEY = "audiolink.theme";
const THEME_ORDER: readonly ThemeMode[] = ["system", "light", "dark"];

function readStyle(): StyleId {
  if (typeof localStorage === "undefined") return "fluent";
  return localStorage.getItem(STYLE_KEY) === "macos" ? "macos" : "fluent";
}

function readTheme(): ThemeMode {
  if (typeof localStorage === "undefined") return "system";
  const raw = localStorage.getItem(THEME_KEY);
  return raw === "light" || raw === "dark" ? raw : "system";
}

function systemPrefersDark(): boolean {
  return typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches;
}

export function AppearanceSwitcher() {
  const [style, setStyle] = useState<StyleId>(readStyle);
  const [theme, setTheme] = useState<ThemeMode>(readTheme);
  // system 模式下要跟着系统变，所以自己留一份解析结果
  const [systemDark, setSystemDark] = useState<boolean>(systemPrefersDark);

  useEffect(() => {
    if (typeof matchMedia !== "function") return;
    const mq = matchMedia("(prefers-color-scheme: dark)");
    const onChange = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const resolved: "light" | "dark" = theme === "system" ? (systemDark ? "dark" : "light") : theme;

  useEffect(() => {
    const root = document.documentElement;
    root.dataset.style = style;
    root.dataset.theme = resolved;
    try {
      localStorage.setItem(STYLE_KEY, style);
      localStorage.setItem(THEME_KEY, theme);
    } catch {
      // 隐私模式：本次会话照样生效，只是记不住。
    }

    // 原生标题栏跟着光照走（风格不改变系统标题栏，那是操作系统的地盘）。
    void (async () => {
      try {
        if (!("__TAURI_INTERNALS__" in window)) return;
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        await getCurrentWindow().setTheme(resolved);
      } catch {
        // 非 Tauri 环境（测试 / 浏览器预览）：标题栏不归我们管。
      }
    })();
  }, [style, theme, resolved]);

  const themeLabel =
    theme === "light" ? t("topbar.theme_light") : theme === "dark" ? t("topbar.theme_dark") : t("topbar.theme_system");
  const ThemeIcon = theme === "light" ? IconThemeLight : theme === "dark" ? IconThemeDark : IconThemeAuto;

  return (
    <div className="flex items-center gap-2">
      <div
        role="group"
        aria-label={t("appearance.style")}
        className="well segmented flex items-center gap-0.5 p-0.5"
      >
        {(["fluent", "macos"] as const).map((id) => (
          <button
            key={id}
            type="button"
            aria-pressed={style === id}
            onClick={() => setStyle(id)}
            className={`h-6 rounded-[3px] px-2 t-cap transition-colors ${
              style === id ? "bg-surface-2 text-text font-medium" : "text-text-2 hover:text-text"
            }`}
          >
            {/* 品牌名不进翻译表：Fluent 与 macOS 在任何语言里都这么写（i18n 护栏也只管中文）。 */}
            {id === "fluent" ? "Fluent" : "macOS"}
          </button>
        ))}
      </div>

      <button
        type="button"
        onClick={() => setTheme(THEME_ORDER[(THEME_ORDER.indexOf(theme) + 1) % THEME_ORDER.length] ?? "system")}
        aria-label={t("appearance.theme", { mode: themeLabel })}
        title={t("appearance.theme", { mode: themeLabel })}
        className="key h-7 w-7"
      >
        <ThemeIcon className="h-4 w-4" />
      </button>
    </div>
  );
}
