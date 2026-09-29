/**
 * M5 · 关于（第三方组件声明）。
 *
 * 合规要求的不只是「仓库里有一份声明」，而是**用户能看见它**。这块面板做两件事：
 * ① 把随包分发的 THIRD-PARTY-NOTICES.md 读出来给用户看；② 找不到时**说清楚怎么生成**，
 * 而不是显示一个空面板（空面板只会让人以为软件坏了）。
 *
 * 懒加载：声明约 1 MB，只在用户真的点开时才拉。
 *
 * 视觉：声明正文用下沉窗口（`.al-well`）承载，尺寸读数（KB）用 `.num` 放在标题右侧，
 * 读取中是一盏呼吸灯而不是转圈动画。字号与颜色只走语义 token，明暗由 `index.css` 的
 * 变量决定 —— 组件里没有任何一处写死的明暗假设。
 * 按钮尺寸一律用 `.al-btn-*` 档位类，且不留 `h-* / px-* / text-*`：utilities 层在
 * components 层之上，留着会把档位高度盖回去。
 */

import { useState } from "react";

import { api } from "../lib/ipc";
import { toCommandError, type NoticesView } from "../types";
import { t } from "../i18n";
import { Panel } from "./Panel";

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
    <Panel
      id="about"
      align="start"
      title={t("about.title")}
      hint={t("about.body")}
      // 读数跟着标题走（`meta` 与标题同一条 baseline）
      meta={
        <span className="num text-caption text-text-tertiary">
          {view === null || !view.available ? DASH : `${(view.bytes / 1024).toFixed(1)} KB`}
        </span>
      }
      right={
        <>
          {busy ? <span className="al-lamp h-2 w-2" data-on="busy" /> : null}
          <button type="button" onClick={toggle} className="al-btn al-btn-md text-text-primary">
            {open ? t("about.hide") : busy ? t("about.loading") : t("about.show")}
          </button>
        </>
      }
    >
      {open ? (
        <div className="mt-2 space-y-2">
          {error === null ? null : (
            <p role="alert" className="text-caption text-critical">{error}</p>
          )}
          {view === null ? (
            busy ? <p className="text-caption text-text-tertiary">{t("about.loading")}</p> : null
          ) : (
            <>
              <p className="text-caption text-text-tertiary">
                {view.available
                  ? `${view.source} · ${(view.bytes / 1024).toFixed(1)} KB`
                  : t("about.missing")}
              </p>
              <pre className="al-well max-h-80 overflow-auto p-2 text-caption leading-relaxed whitespace-pre-wrap text-text-secondary">
                {view.text}
              </pre>
            </>
          )}
        </div>
      ) : null}
    </Panel>
  );
}
