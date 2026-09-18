/**
 * M5 · 关于（第三方组件声明）。
 *
 * 合规要求的不只是「仓库里有一份声明」，而是**用户能看见它**。这块面板做两件事：
 * ① 把随包分发的 THIRD-PARTY-NOTICES.md 读出来给用户看；② 找不到时**说清楚怎么生成**，
 * 而不是显示一个空面板（空面板只会让人以为软件坏了）。
 *
 * 懒加载：声明约 1 MB，只在用户真的点开时才拉。
 *
 * 视觉（On-Air Console）：声明正文是机箱里的一张贴纸 —— 下沉窗口（`.well`）+ 丝印标签，
 * 尺寸读数（KB）用 `.num` 放在标题右侧；读取中用一盏呼吸灯，而不是转圈动画。
 */

import { useState } from "react";

import { api } from "../lib/ipc";
import { toCommandError, type NoticesView } from "../types";
import { t } from "../i18n";

/** 读数缺失时的占位（与其它诊断面板同一条约定）。 */
const DASH = "—";

export function AboutPanel() {
  const [view, setView] = useState<NoticesView | null>(null);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const toggle = (): void => {
    const next = !open;
    setOpen(next);
    if (next && view === null && !busy) {
      setBusy(true);
      void api
        .thirdPartyNotices()
        .then((loaded) => {
          setView(loaded);
          setError(null);
        })
        .catch((raw: unknown) => setError(toCommandError(raw).message))
        .finally(() => setBusy(false));
    }
  };

  return (
    <section aria-labelledby="about-heading" className="plate p-3">
      <header className="flex flex-wrap items-start gap-x-3 gap-y-2">
        <div className="min-w-0">
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
            <h2 id="about-heading" className="silk t-body">
              {t("about.title")}
            </h2>
            <span className="num t-cap text-silk-3">
              {view === null || !view.available ? DASH : `${(view.bytes / 1024).toFixed(1)} KB`}
            </span>
          </div>
          <p className="mt-0.5 t-cap leading-4 text-silk-3">{t("about.body")}</p>
        </div>
        <span className="ml-auto flex shrink-0 items-center gap-2">
          {busy ? <span className="lamp h-2 w-2" data-on="busy" /> : null}
          <button
            type="button"
            onClick={toggle}
            className="key h-7 px-2.5 t-cap text-silk"
          >
            {open ? t("about.hide") : busy ? t("cap.refreshing") : t("about.show")}
          </button>
        </span>
      </header>

      {open ? (
        <div className="mt-2 space-y-2">
          {error === null ? null : (
            <p role="alert" className="t-cap text-ink-live">{error}</p>
          )}
          {view === null ? (
            busy ? <p className="t-cap text-silk-3">{t("cap.refreshing")}</p> : null
          ) : (
            <>
              <p className="t-cap text-silk-3">
                {view.available
                  ? `${view.source} · ${(view.bytes / 1024).toFixed(1)} KB`
                  : t("about.missing")}
              </p>
              <pre className="well max-h-80 overflow-auto p-2 t-cap leading-relaxed whitespace-pre-wrap text-silk-2">
                {view.text}
              </pre>
            </>
          )}
        </div>
      ) : null}
    </section>
  );
}
