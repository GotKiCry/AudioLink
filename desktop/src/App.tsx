/**
 * AudioLink 桌面端 · 主界面
 *
 * 架构：**音源即主机**。
 *   主机（这台 PC 在往外送声音）不需要知道对方地址 —— 它把自己的地址摆出来，等人来连；
 *   接收端才需要主动输入主机地址（那件事在设备列表下方，一键可达，不藏二级菜单）。
 *
 * 外观（Fluent 2 / Windows 11 口径）：**五层结构**，下层永远为上层提供可采样的背景。
 *   z0 BackgroundLayer  应用背景（用户可配：单色 / 双色 / 三色 + 方向）
 *   z1 BackgroundLayer  窗口 Mica 等效：半透明 + blur(60px)，玻璃的来源
 *                       （z0 / z1 由同一个组件 portal 到 body，见该组件头部注释）
 *   z2 .al-chrome       侧栏 / 工具栏 / 抽屉：自有底色 + blur(32px)
 *   z3 .al-card         卡片：填充合计 ≈ .565 + blur(16px)（含在 z2 内部时不再自建 backdrop）
 *   z4 .al-pop          「背景」浮层：Acrylic + blur(40px)，**portal 到 body**
 *
 * 注意 z0 / z1 是 .al-shell 的**兄弟层**而不是它的自身背景：这样卡片的 backdrop-filter
 * 采样到的是「整张壁纸 + Mica 结果」，而不是被困在某个祖先盒子里（DESIGN.md 红线 1）。
 *
 * 数据流不变：useAudioLink() 订阅 audiolink://peer / audiolink://telemetry /
 * audiolink://pair-required，组件只渲染状态 —— 组件里**没有** invoke/listen（都在 src/lib/ipc.ts）。
 */

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
import { PairDialog } from "./components/PairDialog";
import { PeerCard } from "./components/PeerCard";
import { SettingsPanel } from "./components/SettingsPanel";
import { Sidebar } from "./components/Sidebar";
import { StatusStrip } from "./components/StatusStrip";
import { TelemetryPanel } from "./components/TelemetryPanel";
import { UpdatePanel } from "./components/UpdatePanel";
import { DEFAULT_BACKGROUND, normalizeBackground } from "./lib/background";
import { api } from "./lib/ipc";
import { useAudioLink } from "./lib/useAudioLink";
import { useTheme } from "./lib/useTheme";
import { detectLocale, setLocale, t, useLocale, type Locale } from "./i18n";
import { toCommandError, type BackgroundConfig } from "./types";

export default function App() {
  const al = useAudioLink();
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
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

  /**
   * 活跃会话数：正在推流**或**降级运行（链路不稳时引擎自动降码率，会话还在推）。
   *
   * 口径与 useAudioLink.stopSend 的作用域严格对齐 —— 按钮说"能停"，命令就必须真能停：
   * 只数 streaming 的话，一台降级会话会让界面回到「未推流」，于是又出现"没有停止入口"。
   */
  const streamingCount = al.peers.filter(
    (peer) => peer.state === "streaming" || peer.state === "degraded",
  ).length;
  /** 一键广播的候选：已配对、且还没在推的通道（未配对的不碰 —— 那会必然失败）。 */
  const broadcastTargets = al.peers.filter(
    (peer) => peer.trusted && (peer.state === "idle" || peer.state === "failed"),
  );
  /**
   * 「在播」由两个来源合成，都不看 streamingCount 的脸色：
   *   ① 推流意图 al.broadcastRequested —— 人工点的开始，或"接入即自动推流"替他点的（都在 hook 里）；
   *   ② 引擎里真实的会话 —— 兜住"点了开始又立刻点停止"的握手窗口：那期间意图已被撤销，
   *      会话却可能刚建起来，只有把它也算进来，停止入口才不会消失。
   * 合成之后，只要有①或②，主按钮就是可点的「停止推流」。
   */
  const onAir = al.broadcastRequested || streamingCount > 0;
  /** 已发起、却还没有任何会话在推：界面必须说出来，而不是装作没事。 */
  const waitingForDevice = onAir && streamingCount === 0;

  return (
    <div className="al-shell text-text-primary">
      {/* z0 壁纸 + z1 Mica 等效：组件内部 portal 到 body，与应用壳平级（理由见组件头） */}
      <BackgroundLayer config={background} />

      <Sidebar
        version={al.version}
        local={al.local}
        onAir={onAir}
        waitingForDevice={waitingForDevice}
        streamingCount={streamingCount}
        broadcastTargets={broadcastTargets.length}
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
          // 逐台发起交给 hook：它先落意图再发起（候选为空时界面也必须立刻切到「推流中」）。
          al.startBroadcast(broadcastTargets.map((peer) => peer.idShort));
        }}
        // 停止时 hook 还会记下"用户拒绝了谁"，自动推流不得再拉起它们
        onStopBroadcast={() => void al.stopBroadcast()}
      />

      <div className="flex min-w-0 flex-1 flex-col">
        {/* z2 工具栏（52px）：自有底色 + blur(32px)，比 Mica 更实 */}
        <header className="al-chrome al-chrome-b flex h-13 shrink-0 items-center gap-4 pl-4">
          <StatusStrip
            onAir={onAir}
            waitingForDevice={waitingForDevice}
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

            <HostAddressCard local={al.local} />

            <section aria-labelledby="devices-heading" className="flex flex-col gap-3">
              <h1 id="devices-heading" className="al-page-title">
                {t("devices.title")}
              </h1>

              {al.peers.length === 0 ? (
                <div className="al-well px-6 py-8 text-center">
                  <p className="text-body leading-relaxed text-text-secondary">{t("devices.empty")}</p>
                  {/* 空状态只说"没有"会让人不知道下一步；再给一句动作指引（复用地址卡片的提示，不另造文案）。 */}
                  <p className="mt-2 text-caption text-text-tertiary">{t("host.hint")}</p>
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

            {/*
              角色切换区：上面整条是主机主路径（出示地址 → 等接入 → 推流），到这里才换角色。
              两者不是平级的两个主行动 —— 主机永远不会去连别人，所以这一格下沉一级，
              并用一条分隔线收尾（细节见 ConnectToHost 头注释）。
            */}
            <div className="mt-1 border-t border-stroke-control pt-4">
              <ConnectToHost connecting={al.connecting} onConnect={al.connect} />
            </div>
          </div>

          <DiagnosticsDrawer
            open={diagnosticsOpen}
            onClose={() => setDiagnosticsOpen(false)}
            local={al.local}
          >
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
            {/* 自动推流的开关状态与执行都在 hook 里（见 useAudioLink），面板只做开关 */}
            <SettingsPanel
              autoBroadcast={al.autoBroadcast}
              autoBroadcastBusy={al.autoBroadcastBusy}
              onToggleAutoBroadcast={al.setAutoBroadcast}
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
