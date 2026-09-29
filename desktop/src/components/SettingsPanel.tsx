/**
 * M5 · 设置（界面语言 + 开关自启 + 启动时自动连接上次设备 + 接入即自动推流 + 低延迟档）。
 *
 * 为什么单独一块而不是塞进「关于」：关于页讲的是「这个软件是什么、用了谁的代码」，
 * 设置页讲的是「它怎么运行」—— 两件事放在一起，用户找起来会慢。
 *
 * 两个开关的状态都来自**系统/存储**（`autostart_enabled` / `auto_connect_state`），
 * 不是本地记住的布尔：用户在系统设置里改过之后，这里显示的必须仍然是真实状态。
 *
 * 视觉：开关行是一排拨杆位；拨杆只在"开"的时候上色，红色这类告警色不该在面板上常驻。
 * 字号与颜色只走语义 token（`text-*` / `accent-caution` / `al-well`），明暗由 `index.css`
 * 的变量决定 —— 组件里没有任何一处写死的明暗假设。
 * 本面板没有按钮，所以没有 `.al-btn-*` 档位可换；唯一的控件是原生 `select` 与 `checkbox`。
 */

import { useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { toCommandError, type AutoConnectPolicy } from "../types";
import { LOCALES, setLocale, t, useLocale, type Locale } from "../i18n";
import { Panel } from "./Panel";

/** 三个本地开关的操作键：`busy` 按它分片，一个请求返回只会解锁它自己那一个。 */
type SettingKey = "autostart" | "auto-connect" | "low-latency";

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
  /**
   * 正在落盘的操作键（**不是一个布尔**）。
   *
   * 共用一个布尔有两个后果：① 切 A 时 B、C 一起变灰（用户以为界面卡了）；
   * ② 并发时第一个请求返回就把布尔清掉 —— B 还在飞，界面却已经解锁（防连点失效）。
   * 按键分片之后，每个开关只对自己的那次落盘负责。
   */
  const [pending, setPending] = useState<readonly SettingKey[]>([]);
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

  /** 这个开关是否正在落盘（只影响它自己）。 */
  const isPending = (key: SettingKey): boolean => pending.includes(key);

  /**
   * 一次「写设置」的公共外壳：入队 → 落盘 → 无论成败都只摘下**自己**那把锁。
   *
   * 这里只有一个 finally，而它摘的是自己那把锁 —— 这就是"提前解锁"的修法：
   * A 返回时只会把 A 从集合里摘掉，还在飞的 B 仍然是 pending。
   */
  const writeSetting = (
    key: SettingKey,
    action: () => Promise<unknown>,
    apply: () => void,
  ): void => {
    setPending((current) => (current.includes(key) ? current : [...current, key]));
    void action()
      .then(() => {
        apply();
        setError(null);
      })
      .catch((raw: unknown) => setError(toCommandError(raw).message))
      .finally(() =>
        setPending((current) => current.filter((item) => item !== key)),
      );
  };

  const toggleAutostart = (next: boolean): void => {
    writeSetting("autostart", () => api.setAutostart(next), () => setAutostart(next));
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
    writeSetting("auto-connect", () => api.setAutoConnect(next), () =>
      setPolicy((current) => (current === null ? current : { ...current, enabled: next })),
    );
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
    writeSetting("low-latency", () => api.setLowLatency(next), () => setLowLatency(next));
  };

  return (
    <Panel id="set" title={t("set.title")} hint={t("set.tray_hint")}>
      <div className="mt-2 space-y-2 text-caption text-text-secondary">
        <label className="flex items-center justify-between gap-2">
          <span className="text-caption font-semibold text-text-secondary">{t("set.language")}</span>
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
            disabled={isPending("autostart") || autostart === null}
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
            disabled={isPending("auto-connect") || policy === null}
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
            // 自动推流有它自己的 busy（来自 useAudioLink）：本地在写别的设置时不该把它一起锁住
            disabled={autoBroadcastBusy}
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
            disabled={isPending("low-latency") || lowLatency === null}
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
    </Panel>
  );
}
