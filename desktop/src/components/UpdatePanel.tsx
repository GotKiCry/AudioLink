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
 *
 * 视觉（On-Air Console）：进行中由一盏呼吸灯表达（`.lamp[data-on="busy"]`），
 * 有新版时整块内容是机器内部的一个下沉窗口 + 琥珀边框（注意，不是危险 —— 危险才用 `live`）。
 */

import { useState } from "react";

import { api } from "../lib/ipc";
import { toCommandError, type UpdateCheckView } from "../types";
import { t } from "../i18n";

/** 读数缺失时的占位（与其它诊断面板同一条约定）。 */
const DASH = "—";

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
  const working = checking || installing;

  return (
    <section aria-labelledby="upd-heading" className="plate p-3">
      <header className="flex flex-wrap items-start gap-x-3 gap-y-2">
        <div className="min-w-0">
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
            <h2 id="upd-heading" className="silk t-body">
              {t("upd.title")}
            </h2>
            <span className="num t-cap text-silk-3">
              {view === null ? DASH : view.currentVersion}
            </span>
          </div>
          <p className="mt-0.5 t-cap leading-4 text-silk-3">{t("upd.hint")}</p>
        </div>
        <span className="ml-auto flex shrink-0 items-center gap-2">
          {working ? <span className="lamp h-2 w-2" data-on="busy" /> : null}
          <button
            type="button"
            onClick={check}
            disabled={working}
            className="key h-7 px-2.5 t-cap text-silk disabled:cursor-not-allowed"
          >
            {checking ? t("upd.checking") : t("upd.check")}
          </button>
        </span>
      </header>

      <div className="mt-2 space-y-2 t-cap">
        {error === null ? null : (
          <p role="alert" className="text-ink-live">{error}</p>
        )}

        {installed === null ? null : (
          <p className="text-ink-on">{t("upd.done", { version: installed })}</p>
        )}

        {view === null ? null : pending === null ? (
          <p className="text-silk-2">{t("upd.up_to_date", { version: view.currentVersion })}</p>
        ) : (
          <div className="well border-lamp-warn p-2.5">
            <p className="flex items-center gap-1.5 font-medium text-ink-warn">
              <span className="lamp h-2 w-2 shrink-0" data-on="warn" />
              {t("upd.available", { version: pending, current: view.currentVersion })}
            </p>

            {view.pubDate === null ? null : (
              <p className="mt-1.5 text-silk-3">
                {t("upd.published", { date: view.pubDate })}
              </p>
            )}

            {view.notes === null ? null : (
              <div className="mt-1.5">
                <p className="silk-sm">{t("upd.notes")}</p>
                <pre className="mt-1 max-h-40 overflow-auto border border-line bg-panel p-2 t-cap leading-relaxed whitespace-pre-wrap text-silk-2">
                  {view.notes}
                </pre>
              </div>
            )}

            {confirming ? (
              <div className="mt-2 space-y-2 border-t border-line pt-2">
                <p className="text-ink-warn">{t("upd.confirm_hint")}</p>
                <div className="flex flex-wrap gap-2">
                  <button
                    type="button"
                    disabled={installing}
                    onClick={() => install(pending)}
                    className="key key-primary h-7 px-2.5 t-cap disabled:cursor-not-allowed"
                  >
                    {installing ? t("upd.installing") : t("upd.confirm")}
                  </button>
                  <button
                    type="button"
                    disabled={installing}
                    onClick={() => setConfirming(false)}
                    className="key h-7 px-2.5 t-cap text-silk-2 disabled:cursor-not-allowed"
                  >
                    {t("upd.cancel")}
                  </button>
                </div>
              </div>
            ) : (
              <button
                type="button"
                onClick={() => setConfirming(true)}
                className="key mt-2 h-7 px-2.5 t-cap text-ink-warn"
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
