/**
 * 背景浮层（z4 Acrylic）：单色 / 双色 / 三色 + 方向 + 预设。
 *
 * **必须 createPortal 到 document.body**（DESIGN.md 红线 1–2，实测过两次）：
 * 带 backdrop-filter 的祖先既是后代的 backdrop root（采样域被锁进祖先盒子 → blur 失效），
 * 又是 fixed 后代的 containing block（定位被劫持）。只改 position 救不了，必须移 DOM。
 *
 * portal 之后 **Tab 顺序会绕到页面底部**，所以这里配了焦点转移（红线 3）：
 * 打开时聚焦浮层内第一个可聚焦元素，关闭时把焦点还给触发按钮，Esc 关闭。
 */

import { useEffect, useLayoutEffect, useRef } from "react";
import type { CSSProperties } from "react";
import { createPortal } from "react-dom";

import { t, type MessageKey } from "../i18n";
import {
  BACKGROUND_DIRS,
  BACKGROUND_MODES,
  BACKGROUND_PRESETS,
  DEFAULT_BACKGROUND,
  colorSlotCount,
} from "../lib/background";
import type { BackgroundConfig, BackgroundDir, BackgroundMode } from "../types";

/** 色槽：序号给可访问名，键名与配置字段一一对应（三色留空 = c3 是空串）。 */
const SLOTS = [
  { index: 1, key: "c1" },
  { index: 2, key: "c2" },
  { index: 3, key: "c3" },
] as const;

/** 模式 / 方向的文案键（英文标签不写死在组件里）。 */
const MODE_LABEL: Record<BackgroundMode, MessageKey> = {
  mono: "bg.mode_mono",
  duo: "bg.mode_duo",
  tri: "bg.mode_tri",
};

const DIR_LABEL: Record<BackgroundDir, MessageKey> = {
  diag: "bg.dir_diag",
  h: "bg.dir_h",
  v: "bg.dir_v",
};

/** 焦点转移的落点：浮层里第一个可聚焦元素。 */
const FOCUSABLE = "button:not([disabled]), input:not([disabled]), select, textarea, [tabindex]:not([tabindex='-1'])";

interface BackgroundPanelProps {
  /** 当前生效的配置（用户没配过时 = 本主题的默认值）。 */
  config: BackgroundConfig;
  theme: "dark" | "light";
  /** 触发按钮：定位用它的 rect，关闭时焦点还给它是红线 3 的另一半。 */
  anchor: HTMLElement | null;
  /** 落盘失败的原因（人话整句，来自后端错误层）；null = 一切正常。 */
  error: string | null;
  onChange: (next: BackgroundConfig) => void;
  onClose: () => void;
}

export function BackgroundPanel({ config, theme, anchor, error, onChange, onClose }: BackgroundPanelProps) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  /** 被「留空」清掉的第三个颜色：留住它，用户点「恢复」时还有得还。 */
  const lastC3 = useRef<string>("");
  const presets = BACKGROUND_PRESETS[theme];
  const visible = colorSlotCount(config.mode);

  const update = (patch: Partial<BackgroundConfig>): void => {
    onChange({ ...config, ...patch });
  };

  // 定位：floating 元素按触发按钮的 rect 算 fixed 的 top/right。
  // 用 right 而不是 left：右缘精确对齐触发按钮，不必先量浮层自身宽度（少一帧误差）。
  useLayoutEffect(() => {
    const panel = panelRef.current;
    if (panel === null || anchor === null) {
      return;
    }
    const place = (): void => {
      const rect = anchor.getBoundingClientRect();
      panel.style.right = Math.max(8, Math.round(window.innerWidth - rect.right)) + "px";
      panel.style.left = "auto";
      panel.style.maxWidth = Math.max(120, Math.round(rect.right) - 16) + "px";
      panel.style.top = Math.round(rect.bottom + 8) + "px";
    };
    place();
    window.addEventListener("resize", place);
    // 捕获阶段：内容区 / 侧栏内部滚动同样要重算（scroll 不冒泡）。
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [anchor]);

  // 焦点转移：进入浮层；卸载时归还触发按钮（Esc 关闭也走这条路径）。
  useEffect(() => {
    panelRef.current?.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    return () => {
      anchor?.focus();
    };
  }, [anchor]);

  // Esc 关闭 + 点击外部关闭。
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        onClose();
      }
    };
    const onClick = (event: MouseEvent): void => {
      const panel = panelRef.current;
      if (panel !== null && event.target instanceof Node && !panel.contains(event.target)) {
        onClose();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    document.addEventListener("click", onClick);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("click", onClick);
    };
  }, [onClose]);

  const panelNode = (
    <div
      ref={panelRef}
      role="dialog"
      aria-label={t("bg.title")}
      className="al-pop al-pop-bg"
    >
      <p className="al-pop-title text-caption">{t("bg.title")}</p>

      <div className="mt-3">
        <p className="al-pop-label text-caption">{t("bg.mode")}</p>
        <div className="al-segmented" role="group" aria-label={t("bg.mode")}>
          {BACKGROUND_MODES.map((mode) => (
            <button
              key={mode}
              type="button"
              className="al-segmented-item text-caption"
              aria-pressed={config.mode === mode}
              onClick={() => update({ mode })}
            >
              {t(MODE_LABEL[mode])}
            </button>
          ))}
        </div>
      </div>

      <div className="mt-3">
        <p className="al-pop-label text-caption">{t("bg.colors")}</p>
        <div className="flex flex-col gap-1.5" role="group" aria-label={t("bg.colors")}>
          {SLOTS.map((slot) => {
            if (slot.index > visible) {
              return null;
            }
            const value = config[slot.key];
            const empty = value === "";
            const label = t("bg.slot", { index: slot.index });
            return (
              <div key={slot.key} className={"flex items-center gap-2" + (empty ? " opacity-40" : "")}>
                <input
                  type="color"
                  className="al-swatch al-swatch-color al-focusable"
                  aria-label={label}
                  title={label}
                  value={empty ? config.c2 : value}
                  onChange={(event) => update({ [slot.key]: event.target.value } as Partial<BackgroundConfig>)}
                />
                <span className="num text-caption text-text-secondary">
                  {empty ? t("bg.slot_empty") : value}
                </span>
                {slot.index === 3 ? (
                  <button
                    type="button"
                    className="al-btn al-btn-subtle ml-auto h-6 px-2 text-caption"
                    aria-pressed={empty}
                    title={empty ? t("bg.c3_restore_hint") : t("bg.c3_blank_hint")}
                    onClick={() => {
                      if (empty) {
                        update({ c3: lastC3.current === "" ? config.c2 : lastC3.current });
                      } else {
                        lastC3.current = value;
                        update({ c3: "" });
                      }
                    }}
                  >
                    {empty ? t("bg.c3_restore") : t("bg.c3_blank")}
                  </button>
                ) : null}
              </div>
            );
          })}
        </div>
      </div>

      <div className="mt-3">
        <p className="al-pop-label text-caption">{t("bg.dir")}</p>
        {/* 单色没有方向：整组禁用置灰，而不是让用户点一个不生效的按钮 */}
        <div
          className={"al-segmented" + (config.mode === "mono" ? " opacity-50" : "")}
          role="group"
          aria-label={t("bg.dir")}
        >
          {BACKGROUND_DIRS.map((dir) => (
            <button
              key={dir}
              type="button"
              className="al-segmented-item text-caption"
              disabled={config.mode === "mono"}
              aria-pressed={config.dir === dir}
              onClick={() => update({ dir })}
            >
              {t(DIR_LABEL[dir])}
            </button>
          ))}
        </div>
      </div>

      <div className="mt-3">
        <p className="al-pop-label text-caption">{t("bg.preset")}</p>
        <div className="flex items-center gap-1.5" role="group" aria-label={t("bg.preset")}>
          {presets.map((preset) => {
            const name = t(("bg.preset." + preset.key) as MessageKey);
            const active =
              preset.c1 === config.c1 && preset.c2 === config.c2 && preset.c3 === config.c3;
            return (
              <button
                key={preset.key}
                type="button"
                className="al-swatch al-swatch-preset al-focusable h-6 w-6"
                // 预设色是**数据**（来自 lib/background.ts），无法写成静态工具类；
                // 这里只注入自定义属性，颜色本身仍由 .al-swatch-preset 的组合规则消费。
                style={
                  {
                    "--sw-a": preset.c1,
                    "--sw-b": preset.c2,
                    "--sw-c": preset.c3,
                  } as CSSProperties
                }
                aria-label={name}
                title={name}
                aria-pressed={active}
                onClick={() => {
                  lastC3.current = preset.c3;
                  update({ c1: preset.c1, c2: preset.c2, c3: preset.c3 });
                }}
              />
            );
          })}
        </div>
      </div>

      <div className="mt-3 flex items-center gap-2">
        <button
          type="button"
          className="al-btn al-btn-subtle h-6 px-2 text-caption"
          title={t("bg.reset_hint")}
          onClick={() => {
            lastC3.current = "";
            // 默认值只有一处来源（lib/background.ts，与 Rust 常量、CSS 令牌三处同值）。
            onChange(DEFAULT_BACKGROUND[theme]);
          }}
        >
          {t("bg.reset")}
        </button>
      </div>

      {error === null ? null : (
        <p role="alert" className="mt-2 text-caption text-critical">
          {error}
        </p>
      )}

      <p className="mt-2.5 text-caption text-text-tertiary">{t("bg.hint")}</p>
    </div>
  );

  return createPortal(panelNode, document.body);
}
