import { useState } from "react";
import type { ComponentType, CSSProperties } from "react";

import type { PeerState, PeerView } from "../types";
import { peerStateLabel } from "../types";
import { t } from "../i18n";
import {
  IconStart,
  IconStateConnecting,
  IconStateDegraded,
  IconStateFailed,
  IconStateIdle,
  IconStateStreaming,
  IconPause,
  IconMonitor,
  IconUnplug,
} from "./icons";

const STATE_VISUAL: Record<
  PeerState,
  { lamp: string; ink: string; Icon: ComponentType<{ className?: string }> }
> = {
  idle: { lamp: "off", ink: "text-text-tertiary", Icon: IconStateIdle },
  handshaking: { lamp: "busy", ink: "text-text-secondary", Icon: IconStateConnecting },
  streaming: { lamp: "on", ink: "text-success", Icon: IconStateStreaming },
  degraded: { lamp: "warn", ink: "text-caution", Icon: IconStateDegraded },
  failed: { lamp: "live", ink: "text-critical", Icon: IconStateFailed },
};

interface PeerCardProps {
  peer: PeerView;
  /** 该对端正在执行 start/stop（防连点）。 */
  busy: boolean;
  canStart: boolean;
  paused?: boolean;
  /** 本机是否需要为这个对端**输入**配对码（即配对由本机发起）。 */
  canInputPin: boolean;
  onStart: (idShort: string) => Promise<void>;
  onStop: () => Promise<void>;
  onBeginPair: (idShort: string) => void;
  onGain: (gain: number) => void;
  onRevoke: (idShort: string) => Promise<void>;
}

export function PeerCard({
  peer,
  busy,
  canStart,
  paused = false,
  canInputPin,
  onStart,
  onStop,
  onBeginPair,
  onGain,
  onRevoke,
}: PeerCardProps) {
  const visual = STATE_VISUAL[peer.state];
  const streaming = peer.state === "streaming" || peer.state === "degraded";
  /** 两段式确认：默认收起，避免误点这个不可逆动作（FR-18 代价不对称）。 */
  const [confirmingRevoke, setConfirmingRevoke] = useState(false);
  /** 推子受控：原来是非受控的 defaultValue，既显示不出真实增益，重连后也回不到真值。 */
  const [gain, setGain] = useState(1);

  return (
    <article className="al-device-row">
      <div className="flex min-w-0 flex-wrap items-center gap-3">
        <div className="al-device-icon"><IconMonitor className="h-5 w-5" /></div>
        <div className="min-w-0 flex-1">
          <h3 className="break-words text-body font-semibold">{peer.name}</h3>
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-caption">
            <span className={`inline-flex items-center gap-1.5 ${visual.ink}`}>
              <visual.Icon className="h-3.5 w-3.5" />
              {peer.receiving && peer.state === "idle" && peer.trusted ? t("peer.host_connected") : paused && peer.state === "idle" && peer.trusted ? t("peer.paused") : peerStateLabel(peer.state)}
            </span>
            <span className="text-text-secondary">{peer.trusted ? t("peer.trusted") : t("peer.untrusted")}</span>
          </div>
        </div>
        <span className="al-lamp h-2 w-2" data-on={visual.lamp} />
        {peer.trusted ? (
          <button type="button" disabled={busy} onClick={() => setConfirmingRevoke(true)}
            title={t("peer.revoke_hint")} aria-label={t("peer.revoke")} className="al-btn h-8 w-8">
            <IconUnplug className="h-4 w-4" />
          </button>
        ) : null}
      </div>

      {peer.state === "degraded" ? (
        <p className="text-caption text-caution">{t("peer.degraded")}</p>
      ) : null}

      <div className="flex flex-wrap items-center gap-x-4 gap-y-3">
        {/* §4.1 音量：只在推流中给入口（引擎侧 SET_GAIN 已生效，渐变 200 ms） */}
        {streaming ? (
          <label className="flex min-w-0 basis-56 flex-1 items-center gap-3">
            <span className="text-caption font-semibold text-text-tertiary shrink-0">{t("peer.volume")}</span>
            <span className="al-fader flex-1">
              <input
                type="range"
                min={0}
                max={2}
                step={0.05}
                value={gain}
                // 已填充部分由 --fill 驱动（.al-fader 的轨道渐变）：Fluent 的滑块是
                // 「已填充 accent + 剩余中性」，而不是一根通体中性色的轨道。
                style={{ "--fill": Math.round((gain / 2) * 100) + "%" } as CSSProperties}
                aria-label={t("peer.volume_label")}
                title={t("peer.volume_hint")}
                onChange={(event) => {
                  const next = Number(event.target.value);
                  setGain(next);
                  onGain(next);
                }}
              />
            </span>
            <span className="num w-10 shrink-0 text-right text-caption text-text-secondary">
              {Math.round(gain * 100)}%
            </span>
          </label>
        ) : null}

        <div className="ml-auto flex items-center gap-2">
          {!peer.trusted ? (
            canInputPin ? (
              // 本机发起配对：给一条"重新打开输入框"的路 —— 配对请求事件只发一次，
              // 用户如果把弹窗关掉了，不该被迫整条重连（重连还会被去重拦下）。
              <button
                type="button"
                onClick={() => onBeginPair(peer.idShort)}
                className="al-btn al-btn-accent h-9 px-4 text-body"
              >
                {t("peer.pin_entry")}
              </button>
            ) : (
              <button
                type="button"
                disabled
                title={t("peer.pin_hint")}
                className="al-btn h-9 px-4 text-body"
              >
                {t("peer.waiting")}
              </button>
            )
          ) : peer.receiving ? <span className="text-caption text-text-secondary">{t("peer.receive_hint")}</span> : streaming ? (
            <button
              type="button"
              disabled={busy}
              onClick={() => void onStop()}
              className="al-btn h-9 gap-2 px-4 text-body"
            >
              <IconPause className="h-4 w-4" />
              {busy ? t("peer.stopping") : t("peer.stop")}
            </button>
          ) : (
            <button
              type="button"
              disabled={busy || !canStart || peer.state !== "idle"}
              onClick={() => void onStart(peer.idShort)}
              className="al-btn h-9 gap-2 px-4 text-body"
            >
              <IconStart className="h-4 w-4" />
              {busy ? t("peer.starting") : paused ? t("peer.resume") : t("peer.start")}
            </button>
          )}
        </div>
      </div>

      <details className="al-peer-details">
        <summary>{t("peer.details")}</summary>
        <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-caption">
          <dt>{t("peer.address")}</dt><dd className="num break-all">{peer.addr}</dd>
          <dt>{t("peer.fingerprint")}</dt><dd className="num break-all">{peer.idShort}</dd>
        </dl>
        {peer.capabilities ? (
          <p className="mt-2 text-caption leading-relaxed">
            {t("peer.caps_agreed", { list: peer.capabilities.agreed })}
            {peer.capabilities.missingOnPeer.length ? <span className="ml-1 text-caution">
              {t("peer.caps_missing", { list: peer.capabilities.missingOnPeer.join(", ") })}
            </span> : null}
          </p>
        ) : null}
      </details>
      {peer.state === "failed" ? <p className="text-caption text-text-secondary">{t("peer.disconnected_hint")}</p> : null}

      {confirmingRevoke ? (
        <div className="flex flex-wrap items-center gap-2 border-t border-stroke-control pt-3">
          <span className="text-caption text-critical">{t("peer.revoke_hint")}</span>
          <div className="ml-auto flex items-center gap-2">
            <button
              type="button"
              onClick={() => setConfirmingRevoke(false)}
              className="al-btn h-9 px-3 text-caption"
            >
              {t("peer.revoke_cancel")}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                setConfirmingRevoke(false);
                void onRevoke(peer.idShort);
              }}
              className="al-btn al-btn-danger h-9 px-4 text-caption"
            >
              {busy ? t("peer.revoking") : t("peer.revoke_confirm")}
            </button>
          </div>
        </div>
      ) : null}
    </article>
  );
}
