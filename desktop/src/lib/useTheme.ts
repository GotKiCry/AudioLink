/**
 * 主题轴（光照）：system | light | dark。
 *
 * 只剩这一条轴了 —— data-style（设计语言）在「只保留 Fluent」之后被删掉，界面不再有
 * 第二套外观。system 会先解析成实际的 light / dark 再写进 DOM，这样 CSS 只需要处理
 * 两种光照，不必到处写媒体查询。
 *
 * 主题是**纯前端偏好**（localStorage）：面板选错了不会让谁没声音，没有任何理由让它
 * 跨进程往返一次。真正需要跨进程活着的是界面语言与应用背景（见 lib/ipc.ts）。
 */

import { useEffect, useState } from "react";

export type ThemeMode = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

const THEME_KEY = "audiolink.theme";
const THEME_ORDER: readonly ThemeMode[] = ["system", "light", "dark"];

function readTheme(): ThemeMode {
  if (typeof localStorage === "undefined") {
    return "system";
  }
  const raw = localStorage.getItem(THEME_KEY);
  return raw === "light" || raw === "dark" ? raw : "system";
}

function systemPrefersDark(): boolean {
  return typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches;
}

interface ThemeController {
  /** 用户选的三态（system 也如实保留）。 */
  mode: ThemeMode;
  /** 实际生效的光照 —— 背景默认值、浮层预设都按它取。 */
  resolved: ResolvedTheme;
  /** 依次切换 system → light → dark。 */
  cycle: () => void;
}

export function useTheme(): ThemeController {
  const [mode, setMode] = useState<ThemeMode>(readTheme);
  // system 模式下要跟着系统变，所以自己留一份解析结果
  const [systemDark, setSystemDark] = useState<boolean>(systemPrefersDark);

  useEffect(() => {
    if (typeof matchMedia !== "function") {
      return;
    }
    const query = matchMedia("(prefers-color-scheme: dark)");
    const onChange = (event: MediaQueryListEvent): void => setSystemDark(event.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);

  const resolved: ResolvedTheme = mode === "system" ? (systemDark ? "dark" : "light") : mode;

  useEffect(() => {
    document.documentElement.dataset.theme = resolved;
    try {
      localStorage.setItem(THEME_KEY, mode);
    } catch {
      // 隐私模式：本次会话照样生效，只是记不住。
    }

    // 原生标题栏跟着光照走（这是操作系统的地盘，我们只报告光照）。
    void (async () => {
      try {
        if (!("__TAURI_INTERNALS__" in window)) {
          return;
        }
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        await getCurrentWindow().setTheme(resolved);
      } catch {
        // 非 Tauri 环境（测试 / 浏览器预览）：标题栏不归我们管。
      }
    })();
  }, [mode, resolved]);

  const cycle = (): void => {
    const next = THEME_ORDER[(THEME_ORDER.indexOf(mode) + 1) % THEME_ORDER.length] ?? "system";
    setMode(next);
  };

  return { mode, resolved, cycle };
}
