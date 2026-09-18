/**
 * AudioLink 桌面端 · 主界面
 *
 * 架构：**音源即主机**。
 *   主机（这台 PC 在往外送声音）不需要知道对方地址 —— 它把自己的地址摆出来，等人来连；
 *   接收端才需要主动输入主机地址（那件事在设备列表下方，一键可达，不藏二级菜单）。
 *
 * 外观：两条正交的轴，由 AppearanceSwitcher 写在 <html> 上 ——
 *   data-style = fluent | macos（设计语言）· data-theme = light | dark（光照）。
 *
 * 数据流不变：`useAudioLink()` 订阅 `audiolink://peer` / `audiolink://telemetry` /
 * `audiolink://pair-required`，组件只渲染状态 —— 组件里**没有** `invoke`/`listen`（都在 `src/lib/ipc.ts`）。
 */

import { useEffect, useState } from "react";

import { AboutPanel } from "./components/AboutPanel";
import { AlignmentPanel } from "./components/AlignmentPanel";
import { AppearanceSwitcher } from "./components/AppearanceSwitcher";
import { ConnectToHost } from "./components/ConnectToHost";
import { DiagnosticsDrawer } from "./components/DiagnosticsDrawer";
import { ErrorBanner } from "./components/ErrorBanner";
import { GroupPanel } from "./components/GroupPanel";
import { HostAddressCard } from "./components/HostAddressCard";
import { NoticeBanner } from "./components/NoticeBanner";
import { PairDialog } from "./components/PairDialog";
import { PeerCard } from "./components/PeerCard";
import { SettingsPanel } from "./components/SettingsPanel";
import { Sidebar } from "./components/Sidebar";
import { StatusStrip } from "./components/StatusStrip";
import { TelemetryPanel } from "./components/TelemetryPanel";
import { UpdatePanel } from "./components/UpdatePanel";
import { api } from "./lib/ipc";
import { useAudioLink } from "./lib/useAudioLink";
import { detectLocale, setLocale, t, useLocale, type Locale } from "./i18n";

export default function App() {
  const al = useAudioLink();
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  // 订阅语言：语言一变，App 重渲染 → 整棵子树跟着换文案（`t` 是渲染期求值的模块函数）。
  const locale = useLocale();

  // 语言偏好与自启 / 自动重连同一条路（设置文件）；没存过就跟随系统语言，并把推断结果写回去。
  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  useEffect(() => {
    void api
      .locale()
      .then((saved) => {
        const next: Locale = saved === "zh-CN" || saved === "en-US" ? saved : detectLocale();
        setLocale(next);
        if (saved === null) void api.setLocale(next);
      })
      .catch(() => {
        // 读设置失败不该打扰用户：回落跟随系统语言即可。
      });
  }, []);

  const streamingCount = al.peers.filter((peer) => peer.state === "streaming").length;
  /** 一键广播的候选：已配对、且还没在推的通道（未配对的不碰 —— 那会必然失败）。 */
  const broadcastTargets = al.peers.filter(
    (peer) => peer.trusted && (peer.state === "idle" || peer.state === "failed"),
  );

  return (
    <div className="flex h-screen bg-window text-text">
      <Sidebar
        version={al.version}
        local={al.local}
        streamingCount={streamingCount}
        busy={al.busyPeer !== null && al.busyPeer !== ""}
        captureDevices={al.captureDevices}
        selectedCaptureId={al.selectedCaptureId}
        activeCapture={al.activeCapture}
        captureLoading={al.captureLoading}
        captureLocked={al.captureLocked}
        captureError={al.captureError}
        onSelectCapture={al.selectCapture}
        onRefreshCapture={al.refreshCaptureDevices}
        onBroadcast={() => {
          for (const peer of broadcastTargets) void al.startSend(peer.idShort);
        }}
        onStopBroadcast={() => void al.stopSend()}
      />

      <div className="flex min-w-0 flex-1 flex-col">
        <div className="chrome-material flex h-12 shrink-0 items-center gap-4 border-b border-line pl-4">
          <StatusStrip
            local={al.local}
            telemetry={al.telemetry}
            streamingCount={streamingCount}
            telemetryOpen={diagnosticsOpen}
            onToggleTelemetry={() => setDiagnosticsOpen((open) => !open)}
          />
          <div className="shrink-0 pr-3">
            <AppearanceSwitcher />
          </div>
        </div>

        {/* .app-content 的样式由外观层按风格分叉：Fluent 下它是全宽的 Mica 底，macOS 下它是一整块浮起的面板 */}
        <main className="app-content flex min-h-0 flex-1 flex-col overflow-hidden">
          <div className="app-content-inner min-h-0 flex-1 overflow-y-auto">
            {al.error === null ? null : (
              <ErrorBanner error={al.error} onDismiss={al.dismissError} />
            )}
            {al.notice === null ? null : (
              <NoticeBanner message={al.notice} onDismiss={al.dismissNotice} />
            )}

            <HostAddressCard local={al.local} />

            <section aria-labelledby="devices-heading" className="flex flex-col gap-3">
              <h1 id="devices-heading" className="page-title">
                {t("devices.title")}
              </h1>

              {al.peers.length === 0 ? (
                <div className="well px-6 py-8 text-center">
                  <p className="t-body leading-relaxed text-text-2">{t("devices.empty")}</p>
                </div>
              ) : (
                al.peers.map((peer) => (
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
                    onRevoke={al.revokeTrust}
                  />
                ))
              )}
            </section>

            <ConnectToHost connecting={al.connecting} onConnect={al.connect} />
          </div>

          <DiagnosticsDrawer open={diagnosticsOpen} onClose={() => setDiagnosticsOpen(false)}>
            <TelemetryPanel
              telemetry={al.telemetry}
              history={al.telemetryHistory}
              open={diagnosticsOpen}
              exporting={al.exportingTelemetry}
              onExport={() => void al.exportTelemetryLog()}
            />
            <GroupPanel
              peers={al.peers}
              groups={al.groups}
              busy={al.groupBusy}
              onRefresh={() => void al.refreshGroups()}
              onCreate={(idShorts, leadMs) => void al.createGroup(idShorts, leadMs)}
              onJoin={(idShort, groupId) => void al.joinGroup(idShort, groupId)}
              onLeave={(idShort, groupId) => void al.leaveGroup(idShort, groupId)}
            />
            <AlignmentPanel
              alignment={al.alignment}
              busy={al.alignmentBusy}
              onBroadcast={(leadMs) => void al.broadcastEpoch(leadMs)}
            />
            <SettingsPanel />
            <UpdatePanel />
            <AboutPanel />
          </DiagnosticsDrawer>
        </main>
      </div>

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
