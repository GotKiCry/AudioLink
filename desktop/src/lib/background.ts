/**
 * 应用背景（z0 壁纸）：单色 / 双色 / 三色 + 方向。
 *
 * 与 Rust 侧 desktop/src-tauri/src/settings.rs::BackgroundConfig 一字对齐，
 * 存在 settings.json 的 background 键里（与 locale / autostart 同一条路，不用 localStorage
 * —— 背景和语言的持久化在用户眼里没有区别，没理由两套机制）。
 *
 * 默认值有**三处**同值（Rust 常量 / index.css 的 --al-wall-* / 这里的 DEFAULT_BACKGROUND）：
 * 改一处就要改三处。CSS 那份是「用户没配过」时真正生效的底色，Rust 那份是浮层初值与
 * 「恢复默认」的目标，这里那份让界面在拿到 Rust 答案之前就能正确显示。
 */

import type { BackgroundConfig, BackgroundDir, BackgroundMode } from "../types";

/** 模式顺序（浮层的分段控件用）。 */
export const BACKGROUND_MODES: readonly BackgroundMode[] = ["mono", "duo", "tri"];

/** 方向顺序。单色下整组禁用置灰 —— 单色没有方向。 */
export const BACKGROUND_DIRS: readonly BackgroundDir[] = ["diag", "h", "v"];

/** 主题默认背景（深色对角三色 / 浅色对角双色）。 */
export const DEFAULT_BACKGROUND: Record<"dark" | "light", BackgroundConfig> = {
  dark: { mode: "tri", c1: "#1b2a4a", c2: "#3d2a6b", c3: "#0f6b6b", dir: "diag" },
  light: { mode: "duo", c1: "#f4f0e6", c2: "#a9c8f2", c3: "", dir: "diag" },
};

/** 一个预设：只换颜色，模式与方向保持用户当前的选择。 */
export interface BackgroundPreset {
  /** i18n 键后缀（bg.preset.*），预设名不写死在这里。 */
  key: string;
  c1: string;
  c2: string;
  c3: string;
}

/** 预设按主题各一套（深色用得上饱和，浅色不能用重色）。 */
export const BACKGROUND_PRESETS: Record<"dark" | "light", readonly BackgroundPreset[]> = {
  dark: [
    { key: "deep_purple", c1: "#17244f", c2: "#4a2a86", c3: "#1f5f8f" },
    { key: "graphite", c1: "#191c22", c2: "#3a4049", c3: "#5b6572" },
    { key: "indigo", c1: "#12294f", c2: "#1e5a96", c3: "#2f92c4" },
    { key: "deep_teal", c1: "#0c2a26", c2: "#155a50", c3: "#1f8570" },
    { key: "warm_charcoal", c1: "#1d1815", c2: "#3a2c22", c3: "#5c4630" },
    { key: "wine", c1: "#25090f", c2: "#5e1626", c3: "#8c2a34" },
  ],
  light: [
    { key: "cream", c1: "#f7f2e6", c2: "#e0d7c3", c3: "#c3b89f" },
    { key: "pale_blue", c1: "#eaf3ff", c2: "#bcd8f7", c3: "#8fbdf0" },
    { key: "warm_grey", c1: "#f5f1ec", c2: "#ddd5cb", c3: "#c2b8ac" },
    { key: "mint", c1: "#eafaf3", c2: "#b6ebd6", c3: "#83d6b8" },
    { key: "apricot", c1: "#fff2e3", c2: "#ffd8b0", c3: "#ffbe80" },
    { key: "haze_violet", c1: "#f5effc", c2: "#dbc9f5", c3: "#bda1e8" },
  ],
};

/** #rrggbb（六位十六进制）—— 与 Rust 侧 is_hex_color 同一口径。 */
export function isHexColor(value: string): boolean {
  return /^#[0-9a-fA-F]{6}$/.test(value);
}

/** 当前模式下可见的颜色槽数量：单色 1 / 双色 2 / 三色 3。 */
export function colorSlotCount(mode: BackgroundMode): number {
  return mode === "mono" ? 1 : mode === "duo" ? 2 : 3;
}

/**
 * 把配置算成两个 CSS 值（z0 层的底色 + 图像）。
 *
 * 单色 → 纯色；双色 / 三色 → 线性渐变。三色的第三个颜色留空（空串）时按双色处理 ——
 * 「留空」是一个明确的用户选择，不是缺数据。
 */
export function wallCss(config: BackgroundConfig): { base: string; image: string } {
  const deg = config.dir === "h" ? "90deg" : config.dir === "v" ? "180deg" : "135deg";
  const stops = [config.c1, config.c2];
  if (config.mode === "tri" && config.c3 !== "") {
    stops.push(config.c3);
  }
  return {
    base: config.c1,
    image: config.mode === "mono" ? "none" : "linear-gradient(" + deg + ", " + stops.join(", ") + ")",
  };
}

/**
 * 背景写在 :root 的两个自定义属性上 —— index.css 的 .al-desktop 读它们。
 *
 * config = null 表示「用户没配过」：把属性删掉，交给 CSS 令牌按主题给默认值
 * （这样深浅主题各有一套默认，而不是把深色默认硬灌进浅色界面）。
 */
export function applyWall(config: BackgroundConfig | null): void {
  const root = document.documentElement;
  if (config === null) {
    root.style.removeProperty("--al-wall-base");
    root.style.removeProperty("--al-wall-image");
    return;
  }
  const css = wallCss(config);
  root.style.setProperty("--al-wall-base", css.base);
  root.style.setProperty("--al-wall-image", css.image);
}

/**
 * 归一化一份可能来自设置文件（可被手改）的背景配置。
 *
 * 非法值逐项回落到主题默认，而不是整份丢弃：用户手改坏了一个颜色，不该连他配的
 * 模式与方向一起丢掉。返回 null 表示"这份数据整体不可用"（不是对象 / 不是我们认识的形状）。
 */
export function normalizeBackground(raw: unknown, theme: "dark" | "light"): BackgroundConfig | null {
  if (raw === null || typeof raw !== "object") {
    return null;
  }
  const fallback = DEFAULT_BACKGROUND[theme];
  const value = raw as Partial<Record<keyof BackgroundConfig, unknown>>;
  const mode: BackgroundMode =
    value.mode === "mono" || value.mode === "duo" || value.mode === "tri" ? value.mode : fallback.mode;
  const dir: BackgroundDir =
    value.dir === "diag" || value.dir === "h" || value.dir === "v" ? value.dir : fallback.dir;
  const c1 = typeof value.c1 === "string" && isHexColor(value.c1) ? value.c1.toLowerCase() : fallback.c1;
  const c2 = typeof value.c2 === "string" && isHexColor(value.c2) ? value.c2.toLowerCase() : fallback.c2;
  // 空串是合法语义（第三个颜色留空）；只有"非空但格式不对"才回落。
  const c3 =
    value.c3 === "" || value.c3 === null || value.c3 === undefined
      ? ""
      : typeof value.c3 === "string" && isHexColor(value.c3)
        ? value.c3.toLowerCase()
        : fallback.c3;
  return { mode, c1, c2, c3, dir };
}
