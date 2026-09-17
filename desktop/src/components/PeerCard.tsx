/**
 * 对端卡片（UI 规格 §2.2 的 `DeviceCard`，M1 最小版）。
 *
 * 保留：名字 / 短指纹 / 状态 / 是否受信 / 地址 / 开始·停止推流。
 * 音量滑块：§4.1 的 SET_GAIN 已在引擎侧生效（M3），外壳经 set_peer_gain 命令下发（渐变 200 ms）。
 * 不做：`⋯` 菜单、重命名、延迟补偿（M2/M5）。
 * 断开后卡片**不消失**（UI 规格 §2.2 微交互）：M1 里t("state.failed")由 `state: failed` 表达。
 */

import { useState } from "react";

import type { PeerView } from "../types";
import { PEER_STATE_STYLE, peerStateLabel } from "../types";
import { t } from "../i18n";

interface PeerCardProps {
  peer: PeerView;
  /** 该对端正在执行 start/stop（防连点）。 */
  busy: boolean;
  canStart: boolean;
  /**
   * 本机是否需要为这个对端**输入**配对码（即配对由本机发起）。
   * 为 false 时说明是对方在等我们亮码（接收端场景），此时不该给输入入口。
   */
  canInputPin: boolean;
  onStart: (idShort: string) => Promise<void>;
  onStop: () => Promise<void>;
  onBeginPair: (idShort: string) => void;
  /** §4.1：调这台对端的音量（0.0–2.0），由外壳带 200 ms 渐变下发。 */
  onGain: (gain: number) => void;
  /**
   * FR-18：移除这台设备（断开会话 + 从信任库撤销 + 清掉指向它的「上次设备」记录）。
   *
   * 这是**不可逆**的隐私操作，所以界面上是两段式（先点「移除设备」、再点「确认移除」）：
   * 一次误点就断掉正在用的链路、还得重新配对才能回来，代价不对称。
   */
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
  const style = PEER_STATE_STYLE[peer.state];
  const streaming = peer.state === "streaming" || peer.state === "degraded";
  /** 两段式确认（见 `onRevoke`）：默认收起，避免误点这个不可逆动作。 */
  const [confirmingRevoke, setConfirmingRevoke] = useState(false);

  return (
    <article
      className={`relative overflow-hidden rounded-xl border bg-white p-4 shadow-sm dark:bg-slate-950 ${
        peer.state === "failed"
          ? "border-red-200 dark:border-red-900"
          : peer.state === "degraded"
            ? "border-amber-200 dark:border-amber-900"
            : "border-slate-200 dark:border-slate-800"
      }`}
    >
      {/* 推流中：左侧 4px 呼吸条（低频渐变，UI 规格 §2.2 / §5） */}
      {streaming ? (
        <span className="absolute inset-y-2 left-0 w-1 rounded-r bg-emerald-500 opacity-80" />
      ) : null}

      <header className="flex items-center gap-2 pl-1">
        <span className={`h-2 w-2 rounded-full ${style.dot}`} />
        <span className="truncate font-medium">{peer.name}</span>
        {/* 状态同时给图标 + 文字：不靠颜色单独传达信息（UI 规格 §5） */}
        <span className={`ml-auto shrink-0 text-xs ${style.text}`}>
          {style.icon} {peerStateLabel(peer.state)}
        </span>
      </header>

      <dl className="mt-3 space-y-1 pl-1 text-xs text-slate-500">
        <div className="flex gap-2">
          <dt className="shrink-0">{t("peer.fingerprint")}</dt>
          <dd className="font-mono">{peer.idShort}</dd>
        </div>
        <div className="flex gap-2">
          <dt className="shrink-0">{t("peer.address")}</dt>
          <dd className="truncate font-mono">{peer.addr}</dd>
        </div>
        <div className="flex gap-2">
          <dt className="shrink-0">{t("peer.trust")}</dt>
          <dd className={peer.trusted ? "text-emerald-600" : "text-amber-600"}>
            {peer.trusted ? t("peer.trusted") : t("peer.untrusted")}
          </dd>
        </div>
      </dl>

      {/* §13 能力协商：把「这一对能一起做什么」写在卡片上，缺什么也一眼看得出来 */}
      {peer.capabilities === null ? null : (
        <p className="mt-3 pl-1 text-xs text-slate-500 dark:text-slate-400">
          {t("peer.caps_agreed", { list: peer.capabilities.agreed })}
          {peer.capabilities.missingOnPeer.length === 0 ? null : (
            <span className="ml-1 text-amber-600 dark:text-amber-400">
              {t("peer.caps_missing", { list: peer.capabilities.missingOnPeer.join("、") })}
            </span>
          )}
        </p>
      )}

      {peer.state === "degraded" ? (
        <p className="mt-3 rounded-lg bg-amber-50 px-2 py-1 text-xs text-amber-700 dark:bg-amber-950 dark:text-amber-300">
          {t("peer.degraded")}
        </p>
      ) : null}

      {/* §4.1 音量：只在推流中给入口（引擎侧 SET_GAIN 已生效，渐变 200 ms） */}
      {streaming ? (
        <label className="mt-3 flex items-center gap-2 pl-1 text-xs text-slate-500 dark:text-slate-400">
          <span className="shrink-0">{t("peer.volume")}</span>
          <input
            type="range"
            min={0}
            max={2}
            step={0.05}
            defaultValue={1}
            aria-label={t("peer.volume_label")}
            title={t("peer.volume_hint")}
            onChange={(event) => onGain(Number(event.target.value))}
            className="flex-1"
          />
        </label>
      ) : null}

      <div className="mt-3 flex gap-2 pl-1">
        {!peer.trusted ? (
          canInputPin ? (
            // 本机发起配对：给一条"重新打开输入框"的路 —— 配对请求事件只发一次，
            // 用户如果把弹窗关掉了，不该被迫整条重连（重连还会被去重拦下）。
            <button
              type="button"
              onClick={() => onBeginPair(peer.idShort)}
              className="h-9 flex-1 rounded-lg bg-indigo-600 text-sm font-medium text-white"
            >
              {t("peer.pin_entry")}
            </button>
          ) : (
            <button
              type="button"
              disabled
              title={t("peer.pin_hint")}
              className="h-9 flex-1 rounded-lg border border-slate-200 text-sm text-slate-400 dark:border-slate-700"
            >
              {t("peer.waiting")}
            </button>
          )
        ) : streaming ? (
          <button
            type="button"
            disabled={busy}
            onClick={() => void onStop()}

            className="h-9 flex-1 rounded-lg border border-slate-300 text-sm font-medium disabled:opacity-50 dark:border-slate-600"
          >
            {busy ? t("peer.stopping") : t("peer.stop")}
          </button>
        ) : (
          <button
            type="button"
            disabled={busy || !canStart || peer.state === "failed"}
            onClick={() => void onStart(peer.idShort)}
            className="h-9 flex-1 rounded-lg bg-indigo-600 text-sm font-medium text-white disabled:opacity-50"
          >
            {busy ? t("peer.starting") : t("peer.start")}
          </button>
        )}
      </div>

      {/*
        FR-18「移除设备」：只对**已在信任库里**的对端出现 —— 未配对的对端没有信任记录可撤，
        给它一个「移除」入口只会让人以为能撤销什么（真正该做的是不去配对）。
        这是隐私说明里「你可以取消配对」的兑现口，此前只能手动删 trust.json / 清应用数据。
      */}
      {peer.trusted ? (
        <div className="mt-2 flex gap-2 pl-1">
          {confirmingRevoke ? (
            <>
              <button
                type="button"
                disabled={busy}
                onClick={() => {
                  setConfirmingRevoke(false);
                  void onRevoke(peer.idShort);
                }}
                className="h-8 flex-1 rounded-lg bg-red-600 text-xs font-medium text-white disabled:opacity-50"
              >
                {busy ? t("peer.revoking") : t("peer.revoke_confirm")}
              </button>
              <button
                type="button"
                onClick={() => setConfirmingRevoke(false)}
                className="h-8 rounded-lg border border-slate-300 px-3 text-xs dark:border-slate-600"
              >
                {t("peer.revoke_cancel")}
              </button>
            </>
          ) : (
            <button
              type="button"
              disabled={busy}
              title={t("peer.revoke_hint")}
              onClick={() => setConfirmingRevoke(true)}
              className="h-8 flex-1 rounded-lg border border-red-200 text-xs font-medium text-red-600 disabled:opacity-50 dark:border-red-900 dark:text-red-400"
            >
              {t("peer.revoke")}
            </button>
          )}
        </div>
      ) : null}
    </article>
  );
}
