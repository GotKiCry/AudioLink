/**
 * M5 · 设置（界面语言 + 开关自启 + 启动时自动连接上次设备）。
 *
 * 为什么单独一块而不是塞进「关于」：关于页讲的是「这个软件是什么、用了谁的代码」，
 * 设置页讲的是「它怎么运行」—— 两件事放在一起，用户找起来会慢。
 *
 * 两个开关的状态都来自**系统/存储**（`autostart_enabled` / `auto_connect_state`），
 * 不是本地记住的布尔：用户在系统设置里改过之后，这里显示的必须仍然是真实状态。
 *
 * 视觉（On-Air Console）：开关行是背板上的一排拨杆位；危险动作（移除信任）平时保持中性按键，
 * 只有 hover 与"确认移除"这一瞬才上 `ink-live` —— 红色不该在面板上常驻。
 */

import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { toCommandError, type AutoConnectPolicy, type TrustedPeerView } from "../types";
import { LOCALES, setLocale, t, useLocale, type Locale } from "../i18n";

/** 读数缺失时的占位（与其它诊断面板同一条约定）。 */
const DASH = "—";

export function SettingsPanel() {
  const locale = useLocale();
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [policy, setPolicy] = useState<AutoConnectPolicy | null>(null);
  /**
   * 已配对设备（**信任库**，不是会话表）。
   *
   * `null` = 还没读到；`[]` = 真的没有 —— 两者在界面上是不同的话术（读取中 vs 空列表）。
   */
  const [trusted, setTrusted] = useState<TrustedPeerView[] | null>(null);
  /** 正在二次确认移除的那一条（`null` = 没有）。 */
  const [confirmingId, setConfirmingId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refreshTrusted = useCallback((): void => {
    void api
      .listTrustedPeers()
      .then((list) => {
        setTrusted(list);
        setError(null);
      })
      .catch((raw: unknown) => setError(toCommandError(raw).message));
  }, []);

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
    refreshTrusted();
  }, [refreshTrusted]);

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

  /**
   * 切换语言：先立即生效（界面马上换），再落盘。
   *
   * 顺序刻意如此 —— 写盘失败也不该让用户觉得「点了没反应」；真失败了错误横幅会出来。
   */
  const changeLocale = (next: Locale): void => {
    setLocale(next);
    void api.setLocale(next).catch((raw: unknown) => setError(toCommandError(raw).message));
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

  /**
   * 移除一台已配对设备（FR-18）。
   *
   * 成功后**立刻重拉列表**：条目当场消失就是用户要的可见反馈；失败则走面板既有的错误行。
   * id 来源是**信任库**（不再是会话表）—— 这正是「没有卡片的旧记录」也能被移除的原因。
   */
  const revokeTrusted = (idShort: string): void => {
    setConfirmingId(null);
    setBusy(true);
    void api
      .revokeTrust(idShort)
      .then(() => refreshTrusted())
      .catch((raw: unknown) => setError(toCommandError(raw).message))
      .finally(() => setBusy(false));
  };

  return (
    <section aria-labelledby="set-heading" className="plate p-3">
      <header className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <h2 id="set-heading" className="silk t-body">
          {t("set.title")}
        </h2>
        <span className="flex items-baseline gap-1.5">
          <span className="silk-sm !text-silk-2">{t("set.trusted_title")}</span>
          <span className="num t-cap text-silk-2">{trusted === null ? DASH : trusted.length}</span>
        </span>
      </header>
      <p className="mt-0.5 t-cap leading-4 text-silk-3">{t("set.tray_hint")}</p>

      <div className="mt-2 space-y-2 t-cap text-silk-2">
        <label className="flex items-center justify-between gap-2">
          <span className="silk-sm !text-silk-2">{t("set.language")}</span>
          <select
            value={locale}
            onChange={(event) => changeLocale(event.target.value as Locale)}
            className="well rounded-chip px-2 py-0.5 t-cap text-silk"
          >
            {LOCALES.map((item) => (
              <option key={item.value} value={item.value}>
                {item.label}
              </option>
            ))}
          </select>
        </label>

        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={autostart === true}
            disabled={busy || autostart === null}
            onChange={(event) => toggleAutostart(event.target.checked)}
            className="accent-lamp-warn disabled:cursor-not-allowed"
          />
          {t("set.autostart")}
          {autostart === null ? <span className="t-cap text-silk-3">{t("set.loading")}</span> : null}
        </label>

        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={policy?.enabled === true}
            disabled={busy || policy === null}
            onChange={(event) => toggleAutoConnect(event.target.checked)}
            className="accent-lamp-warn disabled:cursor-not-allowed"
          />
          {t("set.autoconnect")}
          {policy === null ? <span className="t-cap text-silk-3">{t("set.loading")}</span> : null}
        </label>
        <p className="pl-5 t-cap text-silk-3">
          {policy?.lastPeer == null
            ? t("set.no_history")
            : t("set.last_peer", { peer: policy.lastPeer })}
        </p>
      </div>

      {/*
        FR-18 的读侧：**信任库**里的全部设备，包括当前没有会话、界面上从来没有过卡片的那些
        （换机后残留的旧记录就是这一类）。没有它，那些设备只能靠手删 trust.json 才能清掉 ——
        「你可以取消配对」这句话对它们不成立。
      */}
      <div className="mt-3 border-t border-line pt-2">
        <h3 className="silk-sm">{t("set.trusted_title")}</h3>
        <p className="mt-0.5 t-cap leading-4 text-silk-3">{t("set.trusted_hint")}</p>
        {trusted === null ? (
          <p className="mt-1 t-cap text-silk-3">{t("set.loading")}</p>
        ) : trusted.length === 0 ? (
          <p className="mt-1 t-cap text-silk-3">{t("set.trusted_empty")}</p>
        ) : (
          <ul className="mt-1.5 divide-y divide-line border-t border-line t-cap">
            {trusted.map((item) => (
              <li key={item.idShort} className="flex items-center gap-2 py-1">
                <span className="truncate text-silk-2">{item.name}</span>
                <span className="num shrink-0 text-silk-3">{item.idShort}</span>
                <span className="ml-auto flex shrink-0 gap-1">
                  {confirmingId === item.idShort ? (
                    <>
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() => revokeTrusted(item.idShort)}
                        className="key key-danger h-6 px-2 t-cap text-ink-live disabled:cursor-not-allowed"
                      >
                        {t("set.trusted_confirm")}
                      </button>
                      <button
                        type="button"
                        onClick={() => setConfirmingId(null)}
                        className="key h-6 px-2 t-cap text-silk-2"
                      >
                        {t("set.trusted_cancel")}
                      </button>
                    </>
                  ) : (
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => setConfirmingId(item.idShort)}
                      className="key key-danger h-6 px-2 t-cap text-silk-2 disabled:cursor-not-allowed"
                    >
                      {t("set.trusted_revoke")}
                    </button>
                  )}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>

      {error === null ? null : (
        <p role="alert" className="mt-2 t-cap text-ink-live">{error}</p>
      )}
    </section>
  );
}
