/**
 * M5 · 设置（开关自启 + 启动时自动连接上次设备）。
 *
 * 为什么单独一块而不是塞进「关于」：关于页讲的是「这个软件是什么、用了谁的代码」，
 * 设置页讲的是「它怎么运行」—— 两件事放在一起，用户找起来会慢。
 *
 * 两个开关的状态都来自**系统/存储**（`autostart_enabled` / `auto_connect_state`），
 * 不是本地记住的布尔：用户在系统设置里改过之后，这里显示的必须仍然是真实状态。
 */

import { useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { toCommandError, type AutoConnectPolicy } from "../types";

export function SettingsPanel() {
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [policy, setPolicy] = useState<AutoConnectPolicy | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void api
      .autostartEnabled()
      .then((value) => {
        setAutostart(value);
        setError(null);
      })
      .catch((raw: unknown) => setError(toCommandError(raw).message));
    void api
      .autoConnectState()
      .then(setPolicy)
      .catch((raw: unknown) => setError(toCommandError(raw).message));
  }, []);

  const toggleAutostart = (next: boolean): void => {
    setBusy(true);
    void api
      .setAutostart(next)
      .then(() => {
        setAutostart(next);
        setError(null);
      })
      .catch((raw: unknown) => setError(toCommandError(raw).message))
      .finally(() => setBusy(false));
  };

  const toggleAutoConnect = (next: boolean): void => {
    setBusy(true);
    void api
      .setAutoConnect(next)
      .then(() => {
        setPolicy((current) => (current === null ? current : { ...current, enabled: next }));
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

      <div className="mt-3 space-y-2">
        <label className="flex items-center gap-2 text-xs text-slate-700 dark:text-slate-300">
          <input
            type="checkbox"
            checked={autostart === true}
            disabled={busy || autostart === null}
            onChange={(event) => toggleAutostart(event.target.checked)}
          />
          开机自动启动
          {autostart === null ? <span className="text-slate-400">（读取中…）</span> : null}
        </label>

        <label className="flex items-center gap-2 text-xs text-slate-700 dark:text-slate-300">
          <input
            type="checkbox"
            checked={policy?.enabled === true}
            disabled={busy || policy === null}
            onChange={(event) => toggleAutoConnect(event.target.checked)}
          />
          启动时自动连接上次设备
          {policy === null ? <span className="text-slate-400">（读取中…）</span> : null}
        </label>
        <p className="pl-5 text-xs text-slate-500 dark:text-slate-400">
          {policy?.lastPeer == null
            ? "还没有成功连接过的设备。"
            : `上次设备：${policy.lastPeer}`}
        </p>
      </div>

      {error === null ? null : (
        <p className="mt-2 text-xs text-rose-600 dark:text-rose-400">{error}</p>
      )}
    </section>
  );
}
