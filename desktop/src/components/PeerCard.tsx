/**
 * 设备行（Device Row）—— 一台对端占一行。
 *
 * 「音源即主机」把它从通道条改成行：主区要答的是「谁在听、听得怎么样」，
 * 一行一句话读完，比一列竖排的推子更适合这个答案。
 *
 * 契约不变（UI 规格 §2.2 的 DeviceCard）：名字 / 指纹 / 状态 / 是否受信 / 地址 /
 * 开始·停止推流 / 音量 / 两段式移除设备；断开后这一行**不消失**（规格 §2.2 微交互）。
 * 状态一律三通道：灯 + 图标 + 文字（规格 §5：不靠颜色单独传达）。
 */

import { useState } from "react";
import type { ComponentType } from "react";

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
  IconStop,
  IconUnplug,
} from "./icons";

const STATE_VISUAL: Record<
  PeerState,
  { lamp: string; ink: string; Icon: ComponentType<{ className?: string }> }
> = {
  idle: { lamp: "off", ink: "text-text-3", Icon: IconStateIdle },
  handshaking: { lamp: "busy", ink: "text-text-2", Icon: IconStateConnecting },
  streaming: { lamp: "on", ink: "text-ok-text", Icon: IconStateStreaming },
  degraded: { lamp: "warn", ink: "text-warn-text", Icon: IconStateDegraded },
  failed: { lamp: "live", ink: "text-danger-text", Icon: IconStateFailed },
};

interface PeerCardProps {
  peer: PeerView;
  /** 该对端正在执行 start/stop（防连点）。 */
  busy: boolean;
  canStart: boolean;
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
    <article className="plate flex flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <div className="flex min-w-0 items-center gap-2.5">
          <span className="lamp h-2.5 w-2.5" data-on={visual.lamp} />
          <span className="truncate t-body font-medium text-text">{peer.name}</span>
          <span className={"inline-flex items-center gap-1.5 t-cap " + visual.ink}>
            <visual.Icon className="h-3.5 w-3.5" />
            {peerStateLabel(peer.state)}
          </span>
        </div>

        <span className={peer.trusted ? "t-cap text-ok-text" : "t-cap text-warn-text"}>
          {peer.trusted ? t("peer.trusted") : t("peer.untrusted")}
        </span>

        <div className="num min-w-0 t-cap text-text-3">
          {t("peer.fingerprint")} {peer.idShort}
          <span className="mx-1.5 text-line-2">·</span>
          {peer.addr}
        </div>

        {/*
          FR-18「移除设备」：只对**已在信任库里**的对端出现 —— 未配对的对端没有信任记录可撤，
          给它一个「移除」入口只会让人以为能撤销什么（真正该做的是不去配对）。
        */}
        {peer.trusted ? (
          <div className="ml-auto">
            <button
              type="button"
              disabled={busy}
              onClick={() => setConfirmingRevoke(true)}
              title={t("peer.revoke_hint")}
              aria-label={t("peer.revoke")}
              className="key key-danger h-8 w-8"
            >
              <IconUnplug className="h-4 w-4" />
            </button>
          </div>
        ) : (
          <div className="ml-auto" />
        )}
      </div>

      {/* §13 能力协商：把「这一对能一起做什么」写在行上，缺什么也一眼看得出来 */}
      {peer.capabilities === null ? null : (
        <p className="t-cap leading-relaxed text-text-3">
          {t("peer.caps_agreed", { list: peer.capabilities.agreed })}
          {peer.capabilities.missingOnPeer.length === 0 ? null : (
            <span className="ml-1 text-warn-text">
              {t("peer.caps_missing", { list: peer.capabilities.missingOnPeer.join("、") })}
            </span>
          )}
        </p>
      )}

      {peer.state === "degraded" ? (
        <p className="t-cap text-warn-text">{t("peer.degraded")}</p>
      ) : null}

      <div className="flex flex-wrap items-center gap-x-4 gap-y-3">
        {/* §4.1 音量：只在推流中给入口（引擎侧 SET_GAIN 已生效，渐变 200 ms） */}
        {streaming ? (
          <label className="flex min-w-[220px] flex-1 items-center gap-3">
            <span className="silk-sm shrink-0">{t("peer.volume")}</span>
            <span className="fader flex-1">
              <input
                type="range"
                min={0}
                max={2}
                step={0.05}
                value={gain}
                aria-label={t("peer.volume_label")}
                title={t("peer.volume_hint")}
                onChange={(event) => {
                  const next = Number(event.target.value);
                  setGain(next);
                  onGain(next);
                }}
              />
            </span>
            <span className="num w-10 shrink-0 text-right t-cap text-text-2">
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
                className="key key-primary h-9 px-4 t-body"
              >
                {t("peer.pin_entry")}
              </button>
            ) : (
              <button
                type="button"
                disabled
                title={t("peer.pin_hint")}
                className="key h-9 px-4 t-body"
              >
                {t("peer.waiting")}
              </button>
            )
          ) : streaming ? (
            <button
              type="button"
              disabled={busy}
              onClick={() => void onStop()}
              className="key h-9 gap-2 px-4 t-body"
            >
              <IconStop className="h-4 w-4" />
              {busy ? t("peer.stopping") : t("peer.stop")}
            </button>
          ) : (
            <button
              type="button"
              disabled={busy || !canStart || peer.state === "failed"}
              onClick={() => void onStart(peer.idShort)}
              className="key key-primary h-9 gap-2 px-4 t-body"
            >
              <IconStart className="h-4 w-4" />
              {busy ? t("peer.starting") : t("peer.start")}
            </button>
          )}
        </div>
      </div>

      {confirmingRevoke ? (
        <div className="flex flex-wrap items-center gap-2 border-t border-line pt-3">
          <span className="t-cap text-danger-text">{t("peer.revoke_hint")}</span>
          <div className="ml-auto flex items-center gap-2">
            <button
              type="button"
              onClick={() => setConfirmingRevoke(false)}
              className="key h-9 px-3 t-cap"
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
              className="key key-danger h-9 px-4 t-cap"
            >
              {busy ? t("peer.revoking") : t("peer.revoke_confirm")}
            </button>
          </div>
        </div>
      ) : null}
    </article>
  );
}
