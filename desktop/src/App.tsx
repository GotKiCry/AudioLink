/**
 * AudioLink 桌面端主界面（M1 最小版）。
 *
 * 数据流：`useAudioLink()` 订阅 `audiolink://peer` / `audiolink://telemetry` /
 * `audiolink://pair-required`，组件只渲染状态 —— 组件里**没有** `invoke`/`listen`（都在 `src/lib/ipc.ts`）。
 *
 * 本轮范围（`docs/11-m1-contract.md` §6）：手工 IP → 连接 → 对端卡片 → 开始/停止推流 →
 * 遥测数字面板 → 失败原因可见 → 未配对弹 6 位 PIN。
 * **不做**：托盘、开机自启、双语、设备自动发现列表、曲线图（属 M5/M2）——
 * 采集端点选择器通过真实 WASAPI 枚举与 start_send 的可选端点 ID 接线。
 */

import { useState } from "react";

import { AddManualCard } from "./components/AddManualCard";
import { AlignmentPanel } from "./components/AlignmentPanel";
import { CaptureSourcePanel } from "./components/CaptureSourcePanel";
import { ErrorBanner } from "./components/ErrorBanner";
import { GroupPanel } from "./components/GroupPanel";
import { NoticeBanner } from "./components/NoticeBanner";
import { PairDialog } from "./components/PairDialog";
import { PeerCard } from "./components/PeerCard";
import { StatusStrip } from "./components/StatusStrip";
import { TelemetryPanel } from "./components/TelemetryPanel";
import { TopBar } from "./components/TopBar";
import { useAudioLink } from "./lib/useAudioLink";

export default function App() {
  const al = useAudioLink();
  const [telemetryOpen, setTelemetryOpen] = useState(true);

  const streamingCount = al.peers.filter((peer) => peer.state === "streaming").length;

  return (
    <div className="flex min-h-screen flex-col bg-slate-50 text-slate-900 dark:bg-slate-900 dark:text-slate-100">
      <TopBar version={al.version} />

      <StatusStrip
        local={al.local}
        telemetry={al.telemetry}
        streamingCount={streamingCount}
        telemetryOpen={telemetryOpen}
        onToggleTelemetry={() => setTelemetryOpen((open) => !open)}
      />

      <main className="flex-1 space-y-4 overflow-auto p-4">
        {al.error === null ? null : (
          <ErrorBanner error={al.error} onDismiss={al.dismissError} />
        )}
        {al.notice === null ? null : (
          <NoticeBanner message={al.notice} onDismiss={al.dismissNotice} />
        )}

        <CaptureSourcePanel
          devices={al.captureDevices} selectedId={al.selectedCaptureId} active={al.activeCapture}
          loading={al.captureLoading} locked={al.captureLocked} error={al.captureError}
          onSelect={al.selectCapture} onRefresh={al.refreshCaptureDevices}
        />

        <div className="grid gap-4 [grid-template-columns:repeat(auto-fill,minmax(260px,1fr))]">
          <AddManualCard connecting={al.connecting} onConnect={al.connect} />
          {al.peers.map((peer) => (
            <PeerCard
              key={peer.idShort}
              peer={peer}
              busy={al.busyPeer === peer.idShort}
              canStart={al.canStartCapture && !al.captureLocked}
              canInputPin={al.pairableIds.includes(peer.idShort)}
              onStart={al.startSend}
              onStop={al.stopSend}
              onBeginPair={al.beginPairing}
              onGain={(gain) => void al.setPeerGain(peer.idShort, gain)}
            />
          ))}
        </div>

        <GroupPanel
          peers={al.peers}
          groups={al.groups}
          busy={al.groupBusy}
          onRefresh={() => void al.refreshGroups()}
          onCreate={(idShorts, leadMs) => void al.createGroup(idShorts, leadMs)}
          onJoin={(idShort, groupId) => void al.joinGroup(idShort, groupId)}
          onLeave={(idShort, groupId) => void al.leaveGroup(idShort, groupId)}
        />

        <AlignmentPanel alignment={al.alignment} />

        <TelemetryPanel
          telemetry={al.telemetry}
          history={al.telemetryHistory}
          open={telemetryOpen}
          exporting={al.exportingTelemetry}
          onExport={() => void al.exportTelemetryLog()}
        />

        {al.peers.length === 0 ? (
          <p className="text-xs text-slate-400">
            还没有设备。填入对方 IP 手动连接；自动发现（mDNS/广播）属 M2，本轮不做。
          </p>
        ) : null}
      </main>

      {al.pairRequest === null ? null : (
        <PairDialog
          request={al.pairRequest}
          reason={al.pairReason}
          onSubmit={al.submitPin}
          onDismiss={al.dismissPairRequest}
        />
      )}
    </div>
  );
}
