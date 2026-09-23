import { useEffect, useRef, useState } from "react";

import { AboutPanel } from "./components/AboutPanel";
import { AlignmentPanel } from "./components/AlignmentPanel";
import { AppearanceSwitcher } from "./components/AppearanceSwitcher";
import { BackgroundLayer } from "./components/BackgroundLayer";
import { BackgroundPanel } from "./components/BackgroundPanel";
import { ConnectToHost } from "./components/ConnectToHost";
import { DiagnosticsDrawer } from "./components/DiagnosticsDrawer";
import { ErrorBanner } from "./components/ErrorBanner";
import { GroupPanel } from "./components/GroupPanel";
import { HostAddressCard } from "./components/HostAddressCard";
import { IconBackground } from "./components/icons";
import { NoticeBanner } from "./components/NoticeBanner";
import { PeerCard } from "./components/PeerCard";
import { SettingsPanel } from "./components/SettingsPanel";
import { Sidebar } from "./components/Sidebar";
import { StatusStrip } from "./components/StatusStrip";
import { TelemetryPanel } from "./components/TelemetryPanel";
import { UpdatePanel } from "./components/UpdatePanel";
import type { BroadcastStatus } from "./lib/broadcast";
import { DEFAULT_BACKGROUND, normalizeBackground } from "./lib/background";
import { api } from "./lib/ipc";
import { useAudioLink } from "./lib/useAudioLink";
import { useTheme } from "./lib/useTheme";
import { detectLocale, setLocale, t, useLocale, type Locale } from "./i18n";
import { toCommandError, type BackgroundConfig } from "./types";

export default function App() {
  const al = useAudioLink();
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  const [receiveOpen, setReceiveOpen] = useState(false);
  // 订阅语言：语言一变，App 重渲染 → 整棵子树跟着换文案（t 是渲染期求值的模块函数）。
  const locale = useLocale();
  const theme = useTheme();

  /** 用户配过的背景；null = 还没配过（用本主题的默认值，CSS 令牌与它同值）。 */
  const [background, setBackground] = useState<BackgroundConfig | null>(null);
  const [backgroundOpen, setBackgroundOpen] = useState(false);
  const [backgroundError, setBackgroundError] = useState<string | null>(null);
  /** 浮层的定位锚 + 关闭时焦点归还的目标（红线 3 的两半都靠它）。 */
  const backgroundTrigger = useRef<HTMLButtonElement | null>(null);

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

  // 背景同样存在设置文件里（settings.json 的 background 键）；读失败用主题默认。
  useEffect(() => {
    let cancelled = false;
    void api
      .background()
      .then((saved) => {
        if (cancelled || saved === null) {
          return;
        }
        setBackground(normalizeBackground(saved, theme.resolved));
      })
      .catch(() => {
        // 读设置失败不该打扰用户：用本主题的默认背景继续跑。
      });
    return () => {
      cancelled = true;
    };
    // 只在挂载时读一次：主题切换不会改变用户配过的背景（没配过时由 CSS 令牌跟着主题走）。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** 生效的背景：用户配过就用他的，否则是本主题的默认值。 */
  const effectiveBackground = background ?? DEFAULT_BACKGROUND[theme.resolved];

  const changeBackground = (next: BackgroundConfig): void => {
    setBackground(next); // 先落界面：取色要即时看得见效果
    void api
      .setBackground(next)
      .then(() => setBackgroundError(null))
      .catch((raw: unknown) => setBackgroundError(toCommandError(raw).message));
  };

  const streamingCount = al.peers.filter(
    (peer) => peer.state === "streaming" || peer.state === "degraded",
  ).length;
  const broadcastTargets = al.peers.filter((peer) => !peer.receiving && peer.state === "idle");
  const status: BroadcastStatus = al.broadcastStopping ? "pausing"
    : streamingCount > 0 ? "streaming"
    : al.broadcastPaused ? "paused"
    : !al.autoBroadcastReady || al.captureLoading ? "loading"
    : !al.canStartCapture ? "unavailable"
    : al.busyPeer !== null ? "starting"
    : al.broadcastError !== null ? "error"
    : al.autoBroadcast || al.broadcastRequested ? "waiting" : "manual";

  return (
    <div className="al-shell text-text-primary">
      {/* z0 壁纸 + z1 Mica 等效：组件内部 portal 到 body，与应用壳平级（理由见组件头） */}
      <BackgroundLayer config={background} />

      <Sidebar
        version={al.version}
        local={al.local}
        status={status}
        autoBroadcast={al.autoBroadcast}
        canResume={al.canStartCapture && !al.captureLocked}
        streamingCount={streamingCount}
        captureDevices={al.captureDevices}
        selectedCaptureId={al.selectedCaptureId}
        activeCapture={al.activeCapture}
        captureLoading={al.captureLoading}
        captureLocked={al.captureLocked}
        captureError={al.captureError}
        onSelectCapture={al.selectCapture}
        onRefreshCapture={al.refreshCaptureDevices}
        onBroadcast={() => {
          al.startBroadcast(broadcastTargets.map((peer) => peer.idShort));
        }}
        onStopBroadcast={() => void al.stopBroadcast()}
      />

      <div className="flex min-w-0 flex-1 flex-col">
        {/* z2 工具栏（52px）：自有底色 + blur(32px)，比 Mica 更实 */}
        <header className="al-toolbar al-chrome al-chrome-b">
          <StatusStrip
            status={status}
            telemetry={al.telemetry}
            streamingCount={streamingCount}
            telemetryOpen={diagnosticsOpen}
            onToggleTelemetry={() => setDiagnosticsOpen((open) => !open)}
          />
          <div className="flex shrink-0 items-center gap-2 pr-3">
            <AppearanceSwitcher mode={theme.mode} onCycle={theme.cycle} />
            <button
              ref={backgroundTrigger}
              type="button"
              aria-haspopup="dialog"
              aria-expanded={backgroundOpen}
              title={t("bg.button")}
              className="al-btn h-8 gap-1.5 px-3 text-caption"
              onClick={(event) => {
                // 不让这次点击冒泡到 document：浮层的「点外部关闭」监听就在那儿，
                // 不拦住的话按钮会在同一帧里先开又关。
                event.stopPropagation();
                setBackgroundOpen((open) => !open);
              }}
            >
              <IconBackground className="h-4 w-4" />
              {t("bg.button")}
            </button>
          </div>
        </header>

        <main className="al-main flex min-h-0 flex-1 flex-col overflow-hidden">
          <div className="al-content-inner min-h-0 flex-1 overflow-y-auto">
            {al.error === null ? null : (
              <ErrorBanner error={al.error} onDismiss={al.dismissError} />
            )}
            {al.notice === null ? null : (
              <NoticeBanner message={al.notice} onDismiss={al.dismissNotice} />
            )}

            <header className="al-page-header">
              <h1 className="al-page-title">{t("share.title")}</h1>
              <p className="mt-2 text-body text-text-secondary">{t("share.subtitle")}</p>
            </header>
            <HostAddressCard local={al.local} />

            <section aria-labelledby="devices-heading" className="flex flex-col gap-3">
              <div className="flex items-baseline gap-3">
                <h2 id="devices-heading" className="text-base font-semibold">{t("devices.title")}</h2>
                <span className="num text-caption text-text-secondary">{al.peers.length}</span>
              </div>

              {al.peers.length === 0 ? (
                <div className="al-devices-empty">
                  <p className="text-body leading-relaxed text-text-secondary">{t("devices.empty")}</p>
                  <p className="mt-2 text-caption leading-relaxed text-text-tertiary">{t("devices.empty_hint")}</p>
                </div>
              ) : (
                <div className="al-device-list">{al.peers.map((peer) => (
                  <PeerCard
                    key={peer.idShort}
                    peer={peer}
                    busy={al.broadcastStopping || al.busyPeer === peer.idShort}
                    paused={al.broadcastPaused}
                    canStart={al.canStartCapture && !al.captureLocked}
                    onStart={al.startSend}
                    onStop={al.stopSend}
                    onGain={(gain) => void al.setPeerGain(peer.idShort, gain)}
                  />
                ))}</div>
              )}
            </section>

            <details className="al-receive-section" onToggle={(event) => setReceiveOpen(event.currentTarget.open)}>
              <summary>{t("connect.title")}</summary>
              {receiveOpen && <ConnectToHost connecting={al.connecting} connectedIds={al.peers.map((peer) => peer.idShort)} onConnect={al.connect} />}
            </details>
          </div>

          <DiagnosticsDrawer
            open={diagnosticsOpen}
            onClose={() => setDiagnosticsOpen(false)}
            local={al.local}
          >
            {/* 自动推流的开关状态与执行都在 hook 里（见 useAudioLink），面板只做开关 */}
            <SettingsPanel
              autoBroadcast={al.autoBroadcast}
              autoBroadcastBusy={al.autoBroadcastBusy}
              onToggleAutoBroadcast={al.setAutoBroadcast}
            />
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
            <UpdatePanel />
            <AboutPanel />
          </DiagnosticsDrawer>
        </main>
      </div>

      {/*
        z4 背景浮层：portal 到 body（组件内部 createPortal），只在这里决定要不要挂载。
        anchor 传触发按钮：定位按它的 rect 算，关闭时焦点也还给它是红线 3 的另一半。
      */}
      {backgroundOpen ? (
        <BackgroundPanel
          config={effectiveBackground}
          theme={theme.resolved}
          anchor={backgroundTrigger.current}
          error={backgroundError}
          onChange={changeBackground}
          onClose={() => setBackgroundOpen(false)}
        />
      ) : null}
    </div>
  );
}
