/**
 * M5 · 关于（第三方组件声明）。
 *
 * 合规要求的不只是「仓库里有一份声明」，而是**用户能看见它**。这块面板做两件事：
 * ① 把随包分发的 THIRD-PARTY-NOTICES.md 读出来给用户看；② 找不到时**说清楚怎么生成**，
 * 而不是显示一个空面板（空面板只会让人以为软件坏了）。
 *
 * 懒加载：声明约 1 MB，只在用户真的点开时才拉。
 */

import { useState } from "react";

import { api } from "../lib/ipc";
import { toCommandError, type NoticesView } from "../types";

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
    <section className="rounded-lg border border-slate-200 bg-white p-4 shadow-sm dark:border-slate-700 dark:bg-slate-800">
      <header className="flex items-center justify-between gap-2">
        <div>
          <h2 className="text-sm font-semibold">关于 / 第三方声明（M5）</h2>
          <p className="mt-0.5 text-xs text-slate-500 dark:text-slate-400">
            清单（含构建期依赖的超集）与去重后的许可全文；随安装包一起分发。
          </p>
        </div>
        <button
          type="button"
          onClick={toggle}
          className="rounded border border-slate-300 px-2 py-1 text-xs font-medium hover:bg-slate-50 dark:border-slate-600 dark:hover:bg-slate-700"
        >
          {open ? "收起" : busy ? "读取中…" : "查看"}
        </button>
      </header>

      {open ? (
        <div className="mt-3 space-y-2">
          {error === null ? null : (
            <p className="text-xs text-rose-600 dark:text-rose-400">{error}</p>
          )}
          {view === null ? (
            busy ? <p className="text-xs text-slate-400">读取中…</p> : null
          ) : (
            <>
              <p className="text-xs text-slate-500 dark:text-slate-400">
                {view.available
                  ? `${view.source} · ${(view.bytes / 1024).toFixed(1)} KB`
                  : "未找到声明文件"}
              </p>
              <pre className="max-h-80 overflow-auto rounded bg-slate-50 p-2 text-[11px] leading-relaxed whitespace-pre-wrap text-slate-700 dark:bg-slate-900 dark:text-slate-300">
                {view.text}
              </pre>
            </>
          )}
        </div>
      ) : null}
    </section>
  );
}
