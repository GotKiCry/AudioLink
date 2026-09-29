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
import { DEFAULT_BACKGROUND, normalizeBackground } from "./lib/background";
import { deriveBroadcastStatus } from "./lib/broadcast";
import { api } from "./lib/ipc";
import { useAudioLink } from "./lib/useAudioLink";
import { useTheme } from "./lib/useTheme";
import { detectLocale, setLocale, t, useLocale, type Locale } from "./i18n";
import { toCommandError, type BackgroundConfig, type CommandError } from "./types";

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
  /** 落盘失败的原因；null = 一切正常。**在全局渲染**，不留在浮层里（理由见下面的横幅注释）。 */
  const [backgroundError, setBackgroundError] = useState<CommandError | null>(null);
  /** 最近一次**确认落盘**的背景（null = 从没配过）；乐观更新失败时就是回滚的落点。 */
  const savedBackground = useRef<BackgroundConfig | null>(null);
  /** 保存请求序号：连续操作会并发多次保存，只有最新一次的结果算数（见 changeBackground）。 */
  const backgroundSaveSeq = useRef(0);
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
        const normalized = normalizeBackground(saved, theme.resolved);
        savedBackground.current = normalized;
        setBackground(normalized);
      })
      .catch(() => {
        // 读设置失败不该打扰用户：用本主题的默认背景继续跑。
      });
    return () => {
      cancelled = true;
    };
    // 只在挂载时读一次；依赖数组留空是**有意**的（仓库没有 eslint 兜底，所以把取舍写在这里）：
    // 主题切换不会改变用户配过的背景（没配过时由 CSS 令牌跟着主题走），
    // 而跟着 theme.resolved 重跑会把用户配过的值按新主题再归一化一次 —— 那不是我们想要的。
  }, []);

  /** 生效的背景：用户配过就用他的，否则是本主题的默认值。 */
  const effectiveBackground = background ?? DEFAULT_BACKGROUND[theme.resolved];

  /**
   * 换背景：先落界面（取色要即时看得见效果），再发命令。
   *
   * 乐观更新的代价必须补上，否则「屏幕上是什么」与「settings.json 里是什么」会分家：
   * - 失败 → 回滚到**最近一次确认落盘**的值（不是「上一次点的那一下」，因为并发保存里
   *   先成功的那次就已经写进文件了，回滚必须回到文件里真实的那份）；
   * - 序号过滤：拖动色轮 / 连点预设会并发多次保存，过期请求的回执一律丢弃，
   *   免得一个旧失败把新结果覆盖掉。错误本身由全局横幅显示，不随浮层关闭而消失。
   */
  const changeBackground = (next: BackgroundConfig): void => {
    const seq = (backgroundSaveSeq.current += 1);
    setBackground(next);
    void api
      .setBackground(next)
      .then(() => {
        savedBackground.current = next;
        if (seq === backgroundSaveSeq.current) {
          setBackgroundError(null);
        }
      })
      .catch((raw: unknown) => {
        // 过期的失败：后面还有一次保存在路上，最终结果由它决定。
        if (seq !== backgroundSaveSeq.current) {
          return;
        }
        setBackgroundError(toCommandError(raw));
        setBackground(savedBackground.current);
      });
  };

  const streamingCount = al.peers.filter(
    (peer) => peer.state === "streaming" || peer.state === "degraded",
  ).length;
  const broadcastTargets = al.peers.filter((peer) => !peer.receiving && peer.state === "idle");
  /**
   * 「本机此刻有没有条件开共享」——侧栏主按钮的**恢复**准入与卡片的**推流**准入是同一条规则
   * （`deriveBroadcastAffordances` 的第二个参数也叫 canResume，那条规则的定义住在那里）。
   *
   * 所以在这里只算一次：原先两处各写一遍，改一处忘一处就会分家 ——
   * 侧栏说「能恢复」而卡片是灰的，用户只会以为界面坏了。
   */
  const canResume = al.canStartCapture && !al.captureLocked;
  // 九档的优先级顺序（pausing > streaming > paused > loading > unavailable > starting >
  // error > waiting > manual）住在纯函数里，它自带穷举等价测试；这里只负责把原始 flag 递过去。
  const status = deriveBroadcastStatus({
    broadcastStopping: al.broadcastStopping,
    streamingCount,
    broadcastPaused: al.broadcastPaused,
    autoBroadcastReady: al.autoBroadcastReady,
    captureLoading: al.captureLoading,
    canStartCapture: al.canStartCapture,
    busyPeer: al.busyPeer,
    broadcastError: al.broadcastError,
    autoBroadcast: al.autoBroadcast,
    broadcastRequested: al.broadcastRequested,
  });

  return (
    <div className="al-shell text-text-primary">
      {/* z0 壁纸 + z1 Mica 等效：组件内部 portal 到 body，与应用壳平级（理由见组件头） */}
      <BackgroundLayer config={background} />

      <Sidebar
        version={al.version}
        local={al.local}
        status={status}
        autoBroadcast={al.autoBroadcast}
        canResume={canResume}
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
        {/* z2 工具栏（60px，见 .al-toolbar 的 min-height）：自有底色 + blur(32px)，比 Mica 更实 */}
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
              className="al-btn al-btn-lg gap-1.5"
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
            {/*
              背景落盘失败必须**在这一层**：原先它唯一的渲染点是壁纸浮层内部，
              浮层一关（点外部 / Esc）这条错误就永久消失，而屏幕上留的是乐观更新后的新背景 ——
              用户看到一份、settings.json 里是另一份，且没有任何去处。
              现在它与推流错误同列在内容区顶部，关掉浮层也还在；
              与此同时 changeBackground 已经把背景回滚到已保存值，所以这条横幅说的是
              「刚才那次没存上」，不是「现在这份是坏的」，用户重开浮层再选一次即可。
            */}
            {backgroundError === null ? null : (
              <ErrorBanner error={backgroundError} onDismiss={() => setBackgroundError(null)} />
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
                  {/* 副文用 secondary 而不是 tertiary：它落在**主区的渐变背景**上（比侧栏亮），
                      而 tertiary 是给侧栏那种更实的底准备的（报告 §一 实测深色下 tertiary
                      在这个底上 3.73:1，不到 AA 正文门槛）。 */}
                  <p className="mt-2 text-caption leading-relaxed text-text-secondary">{t("devices.empty_hint")}</p>
                </div>
              ) : (
                <div className="al-device-list">{al.peers.map((peer) => (
                  <PeerCard
                    key={peer.idShort}
                    peer={peer}
                    busy={al.broadcastStopping || al.busyPeer === peer.idShort}
                    paused={al.broadcastPaused}
                    canStart={canResume}
                    onStart={al.startSend}
                    onStop={al.stopSend}
                    onGain={(gain) => void al.setPeerGain(peer.idShort, gain)}
                  />
                ))}</div>
              )}
            </section>

            {/* 折叠区：React state 是唯一真相（`open` 是它的投影），但内容**始终挂载**。
                原先按 state 条件渲染，收起就卸载 ConnectToHost —— 手输地址与已发现列表
                每次展开都归零（实测复现）。state 另一个用途是暂停收起时的发现轮询：
                收起不等于卸载，状态留着，轮询停了（每次轮询都会 setHosts 一个新数组，
                白跑一轮就白重渲染一次）。 */}
            <details
              className="al-receive-section"
              open={receiveOpen}
              onToggle={(event) => setReceiveOpen(event.currentTarget.open)}
            >
              <summary>{t("connect.title")}</summary>
              <ConnectToHost
                active={receiveOpen}
                connecting={al.connecting}
                connectedIds={al.peers.map((peer) => peer.idShort)}
                onConnect={al.connect}
              />
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
            {/*
              ⚠️ 这个 onRefresh **不能**用 `void` 包住：GroupPanel 拿它返回的 promise 当
              「刷新正在飞」的锁（内部 `await onRefresh()`，锁到刷新结束）。吞掉 promise
              等于这把锁不存在 —— 只剩组件自己那半把，挡不住用户单独狂点刷新。
              相邻的 onCreate / onJoin / onLeave 不交 promise 是**有意**的：它们没有跨点击
              连点的问题，进行中状态由 `busy` prop 承担，那三处的 void 不是漏改。
            */}
            <GroupPanel
              peers={al.peers}
              groups={al.groups}
              busy={al.groupBusy}
              onRefresh={() => al.refreshGroups()}
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
        落盘失败的原因**不在这里**渲染 —— 它归内容区顶部的全局横幅，关掉浮层也还在。
      */}
      {backgroundOpen ? (
        <BackgroundPanel
          config={effectiveBackground}
          theme={theme.resolved}
          anchor={backgroundTrigger.current}
          onChange={changeBackground}
          onClose={() => setBackgroundOpen(false)}
        />
      ) : null}
    </div>
  );
}
