/**
 * 外壳 IPC 门面：**前端唯一**碰 `invoke` / `listen` 的地方。
 *
 * 为什么集中：Tauri 的 command 名、参数键、事件名是与 Rust 侧的字符串契约 ——
 * 散落在组件里就会"改了一个忘了一个"。组件只允许 import 本模块。
 *
 * 与 `desktop/src-tauri/src/lib.rs`（command 签名）和 `engine_bridge.rs`（事件名常量）对齐。
 * 非 command 的原生面（窗口主题、窗口控件）同样收在这里（见文末两节）：组件永远不 import
 * `@tauri-apps/*`。
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AlignmentView,
  DiscoveredHost,
  AutoConnectPolicy,
  BackgroundConfig,
  CaptureDeviceView,
  GroupView,
  NoticesView,
  LocalStatus,
  PeerView,
  StartSendResult,
  TelemetryRow,
  TelemetryView,
  UpdateCheckView,
} from "../types";

// --- 事件名（契约 §6 冻结；Rust 侧同名常量在 engine_bridge.rs） ---
export const EVENT_PEER = "audiolink://peer";
export const EVENT_TELEMETRY = "audiolink://telemetry";
/** §7 同步组变化：收到就去拉一次最新的组列表。 */
export const EVENT_GROUPS = "audiolink://groups";

/** 契约 §6 的 command 表。参数键名严格照契约（`start_send` 用 `id_short`）。 */
export const api = {
  discoveredHosts: (refresh = false): Promise<DiscoveredHost[]> => invoke("discovered_hosts", { refresh }),
  /** `version` —— 外壳版本号（= Cargo workspace 版本）。 */
  version: (): Promise<string> => invoke<string>("version"),
  /** `local_status` —— 本机身份。 */
  localStatus: (): Promise<LocalStatus> => invoke<LocalStatus>("local_status"),
  /** `list_peers` —— 当前对端列表（首屏水合用；之后靠 `audiolink://peer` 事件）。 */
  listPeers: (): Promise<PeerView[]> => invoke<PeerView[]>("list_peers"),
  listCaptureDevices: (): Promise<CaptureDeviceView[]> => invoke("list_capture_devices"),
  activeCaptureDevice: (): Promise<CaptureDeviceView | null> => invoke("active_capture_device"),
  /** `connect` —— 手工 IP 连接。 */
  connect: (addr: string): Promise<PeerView> => invoke<PeerView>("connect", { addr }),
  /** `start_send` —— 开始推流；返回 `{ stream_id }`。 */
  startSend: (idShort: string, captureDeviceId: string | null): Promise<StartSendResult> =>
    invoke<StartSendResult>("start_send", { id_short: idShort, capture_device_id: captureDeviceId }),
  /** `stop_send` —— 停止推流（无入参，返回 `null`）。 */
  stopSend: (): Promise<null> => invoke<null>("stop_send"),
  /** `telemetry` —— 遥测快照（首屏水合用；之后靠 `audiolink://telemetry` 事件）。 */
  telemetry: (): Promise<TelemetryView> => invoke<TelemetryView>("telemetry"),
  /** `export_telemetry` —— 把前端累积的采样点写成 CSV，返回落盘路径（M2 的日志导出）。 */
  exportTelemetry: (rows: TelemetryRow[]): Promise<string> =>
    invoke<string>("export_telemetry", { rows }),
  setPeerGain: (idShort: string, gain: number, rampMs: number): Promise<null> =>
    invoke<null>("set_peer_gain", { id_short: idShort, gain, ramp_ms: rampMs }),
  listGroups: (): Promise<GroupView[]> => invoke<GroupView[]>("list_groups"),
  /**
   * `alignment` —— M4 多源对齐快照（各路样本编号 + 当前跨度）。
   *
   * 它是**观测量**：引擎里每个音频数据报到达时都会更新读数，界面按 1 Hz 拉一次即可。
   */
  alignment: (): Promise<AlignmentView> => invoke<AlignmentView>("alignment"),
  /**
   * `broadcast_epoch` —— M4：本机作为**接收端**广播共同时间基准，返回发出的会话数。
   *
   * 方向与 `announce_group_epoch`（发送端指定）相反：多台发送端 → 一台接收端混音时，
   * 只有混音方知道共同原点该在哪。
   */
  broadcastEpoch: (leadMs: number): Promise<number> =>
    invoke<number>("broadcast_epoch", { lead_ms: leadMs }),
  /**
   * `third_party_notices` —— M5：第三方组件声明（清单 + 许可全文）。
   *
   * 懒加载：只在用户真的点了「查看」时才拉（它 1 MB，不该在启动时白付）。
   */
  thirdPartyNotices: (): Promise<NoticesView> => invoke<NoticesView>("third_party_notices"),
  /** `autostart_enabled` —— M5：读取开机自启状态。 */
  autostartEnabled: (): Promise<boolean> => invoke<boolean>("autostart_enabled"),
  /** `auto_connect_state` —— M5：启动时自动连接上次设备的设置。 */
  autoConnectState: (): Promise<AutoConnectPolicy> =>
    invoke<AutoConnectPolicy>("auto_connect_state"),
  /** `set_auto_connect` —— M5：开关「启动时自动连接上次设备」。 */
  setAutoConnect: (enabled: boolean): Promise<null> =>
    invoke<null>("set_auto_connect", { enabled }),
  /**
   * `auto_broadcast_state` —— 读取「有人接入时自动开始推流」的开关（默认开）。
   *
   * 与自启 / 自动重连同一条路（设置文件），所以这里读的是**真实状态**，不是前端记住的布尔；
   * 自动推流本身在 useAudioLink 里执行，这个开关只负责让用户的选择跨启动活下来。
   */
  autoBroadcastState: (): Promise<boolean> => invoke<boolean>("auto_broadcast_state"),
  /** `set_auto_broadcast` —— 开关「有人接入时自动开始推流」。 */
  setAutoBroadcast: (enabled: boolean): Promise<null> =>
    invoke<null>("set_auto_broadcast", { enabled }),
  /**
   * `low_latency_state` —— M6：低延迟档的开关（**默认关** = 20 ms 帧的标准档）。
   *
   * 与自动推流同一条路（settings.json），所以读的是**真实状态**而不是前端记住的布尔。
   * 注意生效时机：档位落在引擎启动配置里，改完要**下次启动引擎**才生效 ——
   * 界面必须把这一点说清楚（后端刻意不为了换档重启引擎，那会掐断正在进行的会话）。
   */
  lowLatencyState: (): Promise<boolean> => invoke<boolean>("low_latency_state"),
  /** `set_low_latency` —— 开关「低延迟档」（10 ms Opus 帧）。 */
  setLowLatency: (enabled: boolean): Promise<null> =>
    invoke<null>("set_low_latency", { enabled }),
  /**
   * `try_auto_connect` —— M5：启动时试一次自动重连。
   *
   * 没开、没记录、连不上都返回 `null`（**不报错**）：用户什么都没点，不该弹错误横幅。
   */
  tryAutoConnect: (): Promise<PeerView | null> => invoke<PeerView | null>("try_auto_connect"),
  /** `locale` —— M5：界面语言偏好（null = 用户还没选过，前端跟随系统语言）。 */
  locale: (): Promise<string | null> => invoke<string | null>("locale"),
  /** `set_locale` —— M5：保存界面语言偏好（只接受 zh-CN / en-US）。 */
  setLocale: (tag: string): Promise<null> => invoke<null>("set_locale", { tag }),
  /**
   * `background` —— 应用背景（z0 壁纸）。
   *
   * `null` = 用户还没配过：由前端用本主题的默认值（不要把深色默认硬灌进浅色界面）。
   * 读坏了（settings.json 被手改成非法形状）后端也返回 `null`，不报错 —— 背景只是外观。
   */
  background: (): Promise<BackgroundConfig | null> => invoke<BackgroundConfig | null>("background"),
  /** `set_background` —— 保存应用背景；后端校验色值，非法一律拒掉。 */
  setBackground: (config: BackgroundConfig): Promise<null> =>
    invoke<null>("set_background", { config }),
  /** `set_autostart` —— M5：开关开机自启。 */
  setAutostart: (enabled: boolean): Promise<null> =>
    invoke<null>("set_autostart", { enabled }),
  /**
   * `check_update` —— M5：检查更新（**只由用户主动触发**；后端不做启动期自动检查）。
   *
   * 为什么不直接用 `@tauri-apps/plugin-updater` 的 JS API：① 失败原因要在 Rust 侧翻译成
   * 人话（插件自己的错误是英文机器话，界面不该显示它）；② webview 侧就不必持有
   * `plugin:updater|*` 权限。两条理由详见 `desktop/src-tauri/src/update.rs`。
   */
  checkUpdate: (): Promise<UpdateCheckView> => invoke<UpdateCheckView>("check_update"),
  /**
   * `install_update` —— M5：下载并安装**用户确认过的**那个版本，返回装上的版本号。
   *
   * `version` 必须是上一步界面上显示的版本号：后端会再查一次更新源，对不上就报错 ——
   * 用户授权的是「升级到 X」，不是「随便装个新的」。
   * Windows 上这条命令会让应用退出、安装器接管，所以正常路径多半等到不 resolve。
   */
  installUpdate: (version: string): Promise<string> =>
    invoke<string>("install_update", { version }),
  createGroup: (idShorts: string[], leadMs: number): Promise<number> =>
    invoke<number>("create_group", { id_shorts: idShorts, lead_ms: leadMs }),
  joinGroup: (idShort: string, groupId: number): Promise<null> =>
    invoke<null>("join_group", { id_short: idShort, group_id: groupId }),
  leaveGroup: (idShort: string, groupId: number): Promise<null> =>
    invoke<null>("leave_group", { id_short: idShort, group_id: groupId }),
};

// --- 窗口主题（不是 command：核心窗口插件的原生 API 面） ---

/**
 * 是否跑在 Tauri 外壳里。
 *
 * 浏览器预览与 Vitest（jsdom）下没有 `__TAURI_INTERNALS__`：那里的原生能力一律跳过，
 * 界面照样能用，只是"操作系统的地盘"归别人管。
 */
export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * 让窗框的暗色属性跟随光照（需要 `core:window:allow-set-theme`，见 `capabilities/default.json`）。
 *
 * 窗口已是沉浸式顶栏（decorations:false，没有原生标题栏），但这条仍然有用：
 * 无边框窗口的 1px 窗框描边与阴影由 DWM 画，深浅随它走。
 *
 * 它严格说不是 `invoke` / `listen`，但同属"与外壳的原生契约"：散在组件里同样会
 * "改了一个忘了一个"，所以收在这里，前端其余地方一律不 import `@tauri-apps/*`。
 *
 * 权限没放开、窗口还没就绪等情况这里会 **reject** —— 调用方必须自己决定怎么告诉人，
 * 不要再像以前那样把 ACL 拒绝当成"非 Tauri 环境"吞掉。
 *
 * `@tauri-apps/api/window` 走动态 import：非 Tauri 环境连模块都不加载（不必为 jsdom 兜底）。
 */
export async function setWindowTheme(theme: "light" | "dark"): Promise<void> {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().setTheme(theme);
}

// --- 窗口控件（沉浸式顶栏：decorations:false，系统不再画这三个按钮，由 WindowControls 自绘） ---

/** 最小化到任务栏。 */
export async function windowMinimize(): Promise<void> {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().minimize();
}

/** 最大化 / 还原（二态切换；当前状态用 `isWindowMaximized` 读，不自己猜）。 */
export async function windowToggleMaximize(): Promise<void> {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().toggleMaximize();
}

/** 当前是否最大化 —— 窗口控件「最大化 ↔ 还原」图标的唯一真相来源。 */
export async function isWindowMaximized(): Promise<boolean> {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow().isMaximized();
}

/**
 * 订阅窗口尺寸变化（最大化 / 还原 / 拖到屏幕边缘吸附都走它），返回取消订阅函数。
 * 调用方卸载时必须取消（StrictMode 下 effect 跑两遍，不取消就是双份订阅）。
 */
export async function onWindowResized(handler: () => void): Promise<UnlistenFn> {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow().onResized(handler);
}

/**
 * 关闭窗口。**不等于退出**：Rust 侧把 CloseRequested 转成「隐藏到托盘」
 * （lib.rs 的 on_window_event），真退出在托盘菜单 —— 与原生标题栏的 × 行为一致。
 */
export async function windowClose(): Promise<void> {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().close();
}

/**
 * 订阅三个后端事件。
 *
 * 返回值是"取消订阅"的函数；调用方**必须**在组件卸载时调用它
 * （StrictMode 下 effect 会跑两遍，不取消就会出现双份订阅 + 双倍 IPC）。
 */
export function subscribeEvents(handlers: {
  onPeers: (peers: PeerView[]) => void;
  onTelemetry: (view: TelemetryView) => void;
  /** §7 同步组变化（不节流：建组/加入/退出都是低频人工动作）。 */
  onGroupUpdated: () => void;
  onError: (raw: unknown) => void;
}): () => void {
  let cancelled = false;
  const unlisteners: UnlistenFn[] = [];

  const pending: Promise<UnlistenFn>[] = [
    listen<PeerView[]>(EVENT_PEER, (event) => handlers.onPeers(event.payload)),
    listen<TelemetryView>(EVENT_TELEMETRY, (event) => handlers.onTelemetry(event.payload)),
    listen<[number, number, number]>(EVENT_GROUPS, () => handlers.onGroupUpdated()),
  ];

  for (const subscription of pending) {
    // 不吞异常：订阅失败（例如 capability 没放开 core:event）必须让用户看见原因
    void subscription
      .then((unlisten) => {
        if (cancelled) {
          unlisten();
          return;
        }
        unlisteners.push(unlisten);
      })
      .catch((raw: unknown) => handlers.onError(raw));
  }

  return () => {
    cancelled = true;
    for (const unlisten of unlisteners) {
      unlisten();
    }
  };
}
