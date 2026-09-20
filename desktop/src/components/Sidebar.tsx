/**
 * 侧边栏：这台机器的身份与控制台。
 *
 * 「音源即主机」架构把它放回最自然的位置 —— 左边永远答三个问题：
 * 我是谁（本机身份）、我的声音从哪来（采集源）、现在开不开播（主操作）。
 * 主区负责答第四个：谁在听。
 *
 * 主按钮的三态是这条侧栏的门面，判据取自**显式的推流意图**（见 App.tsx 的 broadcastRequested），
 * 而不是"有几个设备在听"—— 用后者推导时，没设备接入就等于【没有停止入口】。
 */

import type { CaptureDeviceView, CommandError, LocalStatus } from "../types";
import { t } from "../i18n";
import { CaptureSourcePanel } from "./CaptureSourcePanel";
import { IconStart, IconStop } from "./icons";

interface SidebarProps {
  version: string;
  local: LocalStatus | null;
  /** 在播：用户已发起推流，或引擎里已经有会话在推。为真时主按钮是「停止推流」，且**永远可点**。 */
  onAir: boolean;
  /** 已发起但还没有设备在听：灯与状态行要说「推流中 · 等待设备接入」。 */
  waitingForDevice: boolean;
  /** 活跃会话数（在推 + 链路不稳降级的）：状态行报的台数与灯都用它。 */
  streamingCount: number;
  /** 可推流的候选台数（已配对、还没在推）；为 0 且未发起时按钮给「等待设备接入」而不是死按钮。 */
  broadcastTargets: number;
  busy: boolean;
  captureDevices: CaptureDeviceView[];
  selectedCaptureId: string;
  activeCapture: CaptureDeviceView | null;
  captureLoading: boolean;
  captureLocked: boolean;
  captureError: CommandError | null;
  onSelectCapture: (id: string) => void;
  onRefreshCapture: () => Promise<void>;
  onBroadcast: () => void;
  onStopBroadcast: () => void;
}

export function Sidebar({
  version,
  local,
  onAir,
  waitingForDevice,
  streamingCount,
  broadcastTargets,
  busy,
  captureDevices,
  selectedCaptureId,
  activeCapture,
  captureLoading,
  captureLocked,
  captureError,
  onSelectCapture,
  onRefreshCapture,
  onBroadcast,
  onStopBroadcast,
}: SidebarProps) {
  // 三态互斥且穷尽：
  //   已发起 → 「停止推流」，**即使 busy 也保持可点** —— 握手卡住时把它禁用，等于又把停止入口藏起来；
  //   无候选 → 「等待设备接入」禁用，并在按钮下面补一句"怎么办"（禁用的按钮解释不了自己）；
  //   有候选 → 「开始推流」可用。
  const blocked = !onAir && broadcastTargets === 0;
  const disabled = onAir ? false : busy || blocked;
  const label = onAir
    ? t("side.stop_broadcast")
    : blocked
      ? t("side.broadcast_blocked")
      : t("side.broadcast");

  return (
    <aside
      aria-label={t("side.local")}
      className="al-chrome flex w-[280px] shrink-0 flex-col border-r border-stroke-control"
    >
      <div className="flex h-11 shrink-0 items-center gap-2 px-4">
        <span className="text-body font-semibold tracking-tight text-text-primary">AudioLink</span>
        <span className="num text-caption text-text-tertiary">{version === "" ? "…" : `v${version}`}</span>
      </div>

      <div className="shrink-0 px-4 pb-4">
        <div className="text-caption font-semibold text-text-secondary">{t("side.local")}</div>
        <div className="mt-2 flex items-center gap-2">
          {/* 灯分三档：红 = 有人真的在听；黄 = 已开闸但还没人接入；灰 = 没推流。 */}
          <span
            className="al-lamp h-2.5 w-2.5"
            data-on={onAir ? (waitingForDevice ? "warn" : "live") : "off"}
          />
          <span className="truncate text-body text-text-primary">{local?.name ?? "…"}</span>
        </div>
        <div className="mt-2 text-caption text-text-secondary">
          {waitingForDevice
            ? t("side.broadcasting_waiting")
            : onAir
              ? t("console.channels", { count: streamingCount })
              : t("console.off_air")}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
        <CaptureSourcePanel
          devices={captureDevices}
          selectedId={selectedCaptureId}
          active={activeCapture}
          loading={captureLoading}
          locked={captureLocked}
          error={captureError}
          onSelect={onSelectCapture}
          onRefresh={onRefreshCapture}
        />
      </div>

      <div className="shrink-0 border-t border-stroke-control p-3">
        <button
          type="button"
          disabled={disabled}
          onClick={onAir ? onStopBroadcast : onBroadcast}
          className="al-btn al-btn-accent h-10 w-full gap-2 text-body"
        >
          {onAir ? <IconStop className="h-4 w-4" /> : <IconStart className="h-4 w-4" />}
          {label}
        </button>
        {blocked ? (
          <p className="mt-2 text-caption leading-relaxed text-text-tertiary">
            {t("side.broadcast_blocked_hint")}
          </p>
        ) : null}
      </div>
    </aside>
  );
}
