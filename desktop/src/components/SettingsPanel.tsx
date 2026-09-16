/**
 * M5 · 设置（目前只有开机自启一项）。
 *
 * 为什么单独一块而不是塞进「关于」：关于页讲的是「这个软件是什么、用了谁的代码」，
 * 设置页讲的是「它怎么运行」—— 两件事放在一起，用户找起来会慢。
 *
 * 状态来自系统（`autostart_enabled`），不是本地记住的一个布尔：用户在系统设置里改过之后，
 * 这里显示的必须仍然是**真实状态**。
 */

import { useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { toCommandError } from "../types";

export function SettingsPanel() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void api
      .autostartEnabled()
      .then((value) => {
        setEnabled(value);
        setError(null);
      })
      .catch((raw: unknown) => setError(toCommandError(raw).message));
  }, []);

  const toggle = (next: boolean): void => {
    setBusy(true);
    void api
      .setAutostart(next)
      .then(() => {
        setEnabled(next);
        setError(null);
      })
      .catch((raw: unknown) => setError(toCommandError(raw).message))
      .finally(() => setBusy(false));
  };

  return (
    <section className="rounded-lg border border-slate-200 bg-white p-4 shadow-sm dark:border-slate-700 dark:bg-slate-800">
      <header>
        <h2 className="text-sm font-semibold">设置（M5）</h2>
        <p className="mt-0.5 text-xs text-slate-500 dark:text-slate-400">
          关掉窗口不会退出程序：AudioLink 常驻托盘（右键托盘图标可退出）。
        </p>
      </header>

      <label className="mt-3 flex items-center gap-2 text-xs text-slate-700 dark:text-slate-300">
        <input
          type="checkbox"
          checked={enabled === true}
          disabled={busy || enabled === null}
          onChange={(event) => toggle(event.target.checked)}
        />
        开机自动启动
        {enabled === null ? <span className="text-slate-400">（读取中…）</span> : null}
      </label>

      {error === null ? null : (
        <p className="mt-2 text-xs text-rose-600 dark:text-rose-400">{error}</p>
      )}
    </section>
  );
}
