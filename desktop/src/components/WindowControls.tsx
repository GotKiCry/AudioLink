import { useEffect, useState } from "react";

import { t } from "../i18n";
import {
  isTauriRuntime,
  isWindowMaximized,
  onWindowResized,
  windowClose,
  windowMinimize,
  windowToggleMaximize,
} from "../lib/ipc";
import { IconClose, IconMaximize, IconMinimize, IconRestore } from "./icons";

/**
 * 沉浸式顶栏的窗口控件：窗口无边框（tauri.conf.json 的 decorations:false），
 * 系统不再画 最小化 / 最大化 / 关闭 —— 这三个按钮就是窗口全部的「系统出口」。
 *
 * 只在 Tauri 外壳里渲染：浏览器预览与 Vitest（jsdom）没有外壳，那里窗口归浏览器管。
 * 关闭走 windowClose() → CloseRequested → Rust 侧转成「隐藏到托盘」，与原生 × 一致。
 */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!isTauriRuntime()) {
      return;
    }
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    // 最大化状态以外壳为准：点击按钮、双击拖拽区、拖到屏幕边缘吸附，最后都汇成 resize 事件。
    const sync = (): void => {
      void isWindowMaximized()
        .then((value) => {
          if (!cancelled) {
            setMaximized(value);
          }
        })
        .catch((error: unknown) => {
          console.warn("[audiolink] failed to read window maximize state:", error);
        });
    };
    sync();
    void onWindowResized(sync)
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch((error: unknown) => {
        console.warn("[audiolink] failed to subscribe window resize:", error);
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  if (!isTauriRuntime()) {
    return null;
  }

  const report = (action: string) => (error: unknown) => {
    console.warn(`[audiolink] window control failed (${action}):`, error);
  };

  return (
    <div className="al-caption">
      <button
        type="button"
        className="al-caption-btn"
        title={t("win.minimize")}
        aria-label={t("win.minimize")}
        onClick={() => void windowMinimize().catch(report("minimize"))}
      >
        <IconMinimize className="h-3 w-3" />
      </button>
      <button
        type="button"
        className="al-caption-btn"
        title={maximized ? t("win.restore") : t("win.maximize")}
        aria-label={maximized ? t("win.restore") : t("win.maximize")}
        onClick={() => void windowToggleMaximize().catch(report("toggleMaximize"))}
      >
        {maximized ? <IconRestore className="h-3 w-3" /> : <IconMaximize className="h-3 w-3" />}
      </button>
      <button
        type="button"
        className="al-caption-btn"
        data-kind="close"
        title={t("win.close")}
        aria-label={t("win.close")}
        onClick={() => void windowClose().catch(report("close"))}
      >
        <IconClose className="h-3 w-3" />
      </button>
    </div>
  );
}
