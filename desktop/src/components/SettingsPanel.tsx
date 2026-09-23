/**
 * M5 · 设置（界面语言 + 开关自启 + 启动时自动连接上次设备 + 接入即自动推流 + 低延迟档）。
 *
 * 为什么单独一块而不是塞进「关于」：关于页讲的是「这个软件是什么、用了谁的代码」，
 * 设置页讲的是「它怎么运行」—— 两件事放在一起，用户找起来会慢。
 *
 * 两个开关的状态都来自**系统/存储**（`autostart_enabled` / `auto_connect_state`），
 * 不是本地记住的布尔：用户在系统设置里改过之后，这里显示的必须仍然是真实状态。
 *
 * 视觉（On-Air Console）：开关行是背板上的一排拨杆位；拨杆只在"开"的时候上色，
 * 红色这类告警色不该在面板上常驻。
 */

import { useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { toCommandError, type AutoConnectPolicy } from "../types";
import { LOCALES, setLocale, t, useLocale, type Locale } from "../i18n";

interface SettingsPanelProps {
  /**
   * 「有人接入时自动开始推流」。状态与执行都在 useAudioLink（自动推流就在那里跑），
   * 面板只做开关 —— 两处各记一份"开没开"，关掉之后自动推流还在跑，是最难查的那种 bug。
   */
  autoBroadcast: boolean;
  autoBroadcastBusy: boolean;
  onToggleAutoBroadcast: (enabled: boolean) => void;
}

export function SettingsPanel({
  autoBroadcast,
  autoBroadcastBusy,
  onToggleAutoBroadcast,
}: SettingsPanelProps) {
  const locale = useLocale();
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [policy, setPolicy] = useState<AutoConnectPolicy | null>(null);
  /**
   * 低延迟档（M6）。与 autostart / policy 同一条路：状态在 settings.json 里，
   * 面板自己读自己写，**不经过 useAudioLink** —— 这个开关不影响任何正在跑的音频路径，
   * 只在**下次启动引擎**时被 engine_config 读走，所以没必要让它进 hook。
   */
  const [lowLatency, setLowLatency] = useState<boolean | null>(null);
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
    void api
      .lowLatencyState()
      .then(setLowLatency)
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
   * 切换低延迟档。
   *
   * 与 toggleAutoConnect 同一处置：只落盘，不重启引擎。档位是引擎**启动配置**的一部分
   * （EngineConfig.codec），只有下次 `Engine::start` 才读它 —— 改完立刻重启引擎会掐断
   * 正在进行的会话，而用户只是点了一个开关。这句「下次启动引擎生效」写在提示文案里
   * （set.low_latency_hint），不能省：用户改完发现没变化才是真的会困惑。
   */
  const toggleLowLatency = (next: boolean): void => {
    setBusy(true);
    void api
      .setLowLatency(next)
      .then(() => {
        setLowLatency(next);
        setError(null);
      })
      .catch((raw: unknown) => setError(toCommandError(raw).message))
      .finally(() => setBusy(false));
  };

  return (
    <section aria-labelledby="set-heading" className="al-card p-3">
      <header className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <h2 id="set-heading" className="text-caption font-semibold text-text-secondary text-body">
          {t("set.title")}
        </h2>
      </header>
      <p className="mt-0.5 text-caption leading-4 text-text-tertiary">{t("set.tray_hint")}</p>

      <div className="mt-2 space-y-2 text-caption text-text-secondary">
        <label className="flex items-center justify-between gap-2">
          <span className="text-caption font-semibold text-text-tertiary text-text-secondary">{t("set.language")}</span>
          <select
            value={locale}
            onChange={(event) => changeLocale(event.target.value as Locale)}
            className="al-well rounded-control px-2 py-0.5 text-caption text-text-primary"
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
            className="accent-caution disabled:cursor-not-allowed"
          />
          {t("set.autostart")}
          {autostart === null ? <span className="text-caption text-text-tertiary">{t("set.loading")}</span> : null}
        </label>

        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={policy?.enabled === true}
            disabled={busy || policy === null}
            onChange={(event) => toggleAutoConnect(event.target.checked)}
            className="accent-caution disabled:cursor-not-allowed"
          />
          {t("set.autoconnect")}
          {policy === null ? <span className="text-caption text-text-tertiary">{t("set.loading")}</span> : null}
        </label>
        <p className="pl-5 text-caption text-text-tertiary">
          {policy?.lastPeer == null
            ? t("set.no_history")
            : t("set.last_peer", { peer: policy.lastPeer })}
        </p>

        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={autoBroadcast}
            disabled={busy || autoBroadcastBusy}
            onChange={(event) => onToggleAutoBroadcast(event.target.checked)}
            // 名字钉在开关自己身上：label 里还会追加"读取中…"，让它去拼 accessible name 会得到
            // 一个随忙碌态变化的字符串（读屏用户听到的名字忽长忽短）。
            aria-label={t("set.auto_broadcast")}
            className="accent-caution disabled:cursor-not-allowed"
          />
          {t("set.auto_broadcast")}
          {autoBroadcastBusy ? <span className="text-caption text-text-tertiary">{t("set.loading")}</span> : null}
        </label>
        {/* 说清代价与边界：默认开、以及"你停过的不会被自动重启"——自动行为最需要被信任的一点 */}
        <p className="pl-5 text-caption text-text-tertiary">{t("set.auto_broadcast_hint")}</p>

        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={lowLatency === true}
            disabled={busy || lowLatency === null}
            onChange={(event) => toggleLowLatency(event.target.checked)}
            aria-label={t("set.low_latency")}
            className="accent-caution disabled:cursor-not-allowed"
          />
          {t("set.low_latency")}
          {lowLatency === null ? <span className="text-caption text-text-tertiary">{t("set.loading")}</span> : null}
        </label>
        {/* 这条提示是**必须的**：两端不同档位不会报错，只会听到发闷/断续的声音（见文案）。 */}
        <p className="pl-5 text-caption text-text-tertiary">{t("set.low_latency_hint")}</p>
      </div>

      {error === null ? null : (
        <p role="alert" className="mt-2 text-caption text-critical">{error}</p>
      )}
    </section>
  );
}
