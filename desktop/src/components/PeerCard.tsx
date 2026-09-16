/**
 * 对端卡片（UI 规格 §2.2 的 `DeviceCard`，M1 最小版）。
 *
 * 保留：名字 / 短指纹 / 状态 / 是否受信 / 地址 / 开始·停止推流。
 * 音量滑块：§4.1 的 SET_GAIN 已在引擎侧生效（M3），外壳经 set_peer_gain 命令下发（渐变 200 ms）。
 * 不做：`⋯` 菜单、重命名、延迟补偿（M2/M5）。
 * 断开后卡片**不消失**（UI 规格 §2.2 微交互）：M1 里"已断开"由 `state: failed` 表达。
 */

import type { PeerView } from "../types";
import { PEER_STATE_LABEL, PEER_STATE_STYLE } from "../types";

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
}: PeerCardProps) {
  const style = PEER_STATE_STYLE[peer.state];
  const streaming = peer.state === "streaming" || peer.state === "degraded";

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
          {style.icon} {PEER_STATE_LABEL[peer.state]}
        </span>
      </header>

      <dl className="mt-3 space-y-1 pl-1 text-xs text-slate-500">
        <div className="flex gap-2">
          <dt className="shrink-0">指纹</dt>
          <dd className="font-mono">{peer.idShort}</dd>
        </div>
        <div className="flex gap-2">
          <dt className="shrink-0">地址</dt>
          <dd className="truncate font-mono">{peer.addr}</dd>
        </div>
        <div className="flex gap-2">
          <dt className="shrink-0">信任</dt>
          <dd className={peer.trusted ? "text-emerald-600" : "text-amber-600"}>
            {peer.trusted ? "已配对（白名单命中）" : "未配对"}
          </dd>
        </div>
      </dl>

      {peer.state === "degraded" ? (
        <p className="mt-3 rounded-lg bg-amber-50 px-2 py-1 text-xs text-amber-700 dark:bg-amber-950 dark:text-amber-300">
          网络不稳，已自动降码率
        </p>
      ) : null}

      {/* §4.1 音量：只在推流中给入口（引擎侧 SET_GAIN 已生效，渐变 200 ms） */}
      {streaming ? (
        <label className="mt-3 flex items-center gap-2 pl-1 text-xs text-slate-500 dark:text-slate-400">
          <span className="shrink-0">音量</span>
          <input
            type="range"
            min={0}
            max={2}
            step={0.05}
            defaultValue={1}
            aria-label="对端音量"
            title="对端音量（0–200%），拖动时按 200 ms 渐变生效"
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
              输入配对码
            </button>
          ) : (
            <button
              type="button"
              disabled
              title="对方需要在它的设备上输入本机显示的 6 位配对码"
              className="h-9 flex-1 rounded-lg border border-slate-200 text-sm text-slate-400 dark:border-slate-700"
            >
              等待配对
            </button>
          )
        ) : streaming ? (
          <button
            type="button"
            disabled={busy}
            onClick={() => void onStop()}

            className="h-9 flex-1 rounded-lg border border-slate-300 text-sm font-medium disabled:opacity-50 dark:border-slate-600"
          >
            {busy ? "停止中…" : "停止推流"}
          </button>
        ) : (
          <button
            type="button"
            disabled={busy || !canStart || peer.state === "failed"}
            onClick={() => void onStart(peer.idShort)}
            className="h-9 flex-1 rounded-lg bg-indigo-600 text-sm font-medium text-white disabled:opacity-50"
          >
            {busy ? "启动中…" : "开始推流"}
          </button>
        )}
      </div>
    </article>
  );
}
