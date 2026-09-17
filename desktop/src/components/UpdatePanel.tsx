/**
 * M5 · 软件更新（检查 → 下载 → 验签 → 安装）。
 *
 * 三条纪律，都摆在这里而不是藏进别处：
 *
 * ① **不自动**：没有 useEffect 去检查、没有定时器、没有「启动时自动检查」的开关。
 *    AudioLink 是个音频工具，用户很可能正在推流或录音 —— 后台静默地检查/下载/安装会把
 *    正在进行的会话打断，所以检查只能由用户点按钮触发。
 * ② **安装要确认**：点「下载并安装」只进入确认态，把代价（先与对端收尾 → 应用退出 →
 *    安装器接管 → 装完自动重开）说清楚；再点「确认安装」才真的调后端。后端还会核对
 *    版本号是不是用户看到的那个（见 src-tauri/src/update.rs 的 install_update）。
 * ③ **错误不拼**：失败原因一律来自后端 `CommandError.message`（错误层已翻译成人话），
 *    这里只用 `toCommandError` 兜底归一化，不拼字符串。
 */

import { useState } from "react";

import { api } from "../lib/ipc";
import { toCommandError, type UpdateCheckView } from "../types";
import { t } from "../i18n";

export function UpdatePanel() {
  const [checking, setChecking] = useState(false);
  const [view, setView] = useState<UpdateCheckView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [installed, setInstalled] = useState<string | null>(null);

  /** 用户主动触发的检查（这是唯一入口：没有启动期自动检查）。 */
  const check = (): void => {
    setChecking(true);
    setConfirming(false);
    setInstalled(null);
    void api
      .checkUpdate()
      .then((next) => {
        setView(next);
        setError(null);
      })
      .catch((raw: unknown) => setError(toCommandError(raw).message))
      .finally(() => setChecking(false));
  };

  /** 用户**二次确认**后的安装。 */
  const install = (version: string): void => {
    setInstalling(true);
    void api
      .installUpdate(version)
      // Windows 上这条命令会拉起安装器并退出进程，所以正常路径通常等不到 resolve；
      // 真 resolve 了（其它平台）就照实说「装完了、要重启」。
      .then(() => {
        setInstalled(version);
        setError(null);
        setConfirming(false);
      })
      .catch((raw: unknown) => {
        setError(toCommandError(raw).message);
        setConfirming(false);
        setInstalling(false);
      });
  };

  const pending = view !== null && view.version !== null ? view.version : null;

  return (
    <section className="rounded-lg border border-slate-200 bg-white p-4 shadow-sm dark:border-slate-700 dark:bg-slate-800">
      <header className="flex items-start justify-between gap-2">
        <div>
          <h2 className="text-sm font-semibold">{t("upd.title")}</h2>
          <p className="mt-0.5 text-xs text-slate-500 dark:text-slate-400">{t("upd.hint")}</p>
        </div>
        <button
          type="button"
          onClick={check}
          disabled={checking || installing}
          className="shrink-0 rounded border border-slate-300 px-2 py-1 text-xs font-medium hover:bg-slate-50 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-700"
        >
          {checking ? t("upd.checking") : t("upd.check")}
        </button>
      </header>

      <div className="mt-3 space-y-2 text-xs">
        {error === null ? null : (
          <p className="text-rose-600 dark:text-rose-400">{error}</p>
        )}

        {installed === null ? null : (
          <p className="text-emerald-600 dark:text-emerald-400">
            {t("upd.done", { version: installed })}
          </p>
        )}

        {view === null ? null : pending === null ? (
          <p className="text-slate-600 dark:text-slate-300">
            {t("upd.up_to_date", { version: view.currentVersion })}
          </p>
        ) : (
          <div className="space-y-2 rounded border border-amber-300 bg-amber-50 p-3 dark:border-amber-700 dark:bg-amber-900/20">
            <p className="font-medium text-amber-800 dark:text-amber-200">
              {t("upd.available", { version: pending, current: view.currentVersion })}
            </p>

            {view.pubDate === null ? null : (
              <p className="text-slate-500 dark:text-slate-400">
                {t("upd.published", { date: view.pubDate })}
              </p>
            )}

            {view.notes === null ? null : (
              <div>
                <p className="text-slate-500 dark:text-slate-400">{t("upd.notes")}</p>
                <pre className="mt-1 max-h-40 overflow-auto rounded bg-white/70 p-2 text-[11px] leading-relaxed whitespace-pre-wrap text-slate-700 dark:bg-slate-900/60 dark:text-slate-300">
                  {view.notes}
                </pre>
              </div>
            )}

            {confirming ? (
              <div className="space-y-2">
                <p className="text-amber-800 dark:text-amber-200">{t("upd.confirm_hint")}</p>
                <div className="flex gap-2">
                  <button
                    type="button"
                    disabled={installing}
                    onClick={() => install(pending)}
                    className="rounded bg-amber-600 px-2 py-1 text-xs font-medium text-white hover:bg-amber-700 disabled:opacity-50"
                  >
                    {installing ? t("upd.installing") : t("upd.confirm")}
                  </button>
                  <button
                    type="button"
                    disabled={installing}
                    onClick={() => setConfirming(false)}
                    className="rounded border border-slate-300 px-2 py-1 text-xs font-medium hover:bg-slate-50 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-700"
                  >
                    {t("upd.cancel")}
                  </button>
                </div>
              </div>
            ) : (
              <button
                type="button"
                onClick={() => setConfirming(true)}
                className="rounded border border-amber-500 px-2 py-1 text-xs font-medium text-amber-700 hover:bg-amber-100 dark:text-amber-200 dark:hover:bg-amber-900/40"
              >
                {t("upd.install", { version: pending })}
              </button>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
