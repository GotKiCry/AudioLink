/**
 * 外壳 IPC 门面：**前端唯一**碰 `invoke` / `listen` 的地方。
 *
 * 为什么集中：Tauri 的 command 名、参数键、事件名是与 Rust 侧的字符串契约 ——
 * 散落在组件里就会"改了一个忘了一个"。组件只允许 import 本模块。
 *
 * 与 `desktop/src-tauri/src/lib.rs`（command 签名）和 `engine_bridge.rs`（事件名常量）对齐。
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
  PairRequiredPayload,
  PeerView,
  RevokeTrustResult,
  StartSendResult,
  TrustedPeerView,
  SubmitPinResult,
  TelemetryRow,
  TelemetryView,
  UpdateCheckView,
} from "../types";

// --- 事件名（契约 §6 冻结；Rust 侧同名常量在 engine_bridge.rs） ---
export const EVENT_PEER = "audiolink://peer";
export const EVENT_TELEMETRY = "audiolink://telemetry";
export const EVENT_PAIR_REQUIRED = "audiolink://pair-required";
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
  /**
   * `list_trusted_peers` —— **已配对设备**（信任库快照），含当前没有会话的那些。
   *
   * 与 `listPeers` 的分工：那个是会话表（有卡片的对端），这个覆盖「白名单里有、但没卡片」的
   * 设备 —— 换机后残留的旧记录只能从这条读侧接口才看得见、才移得掉。
   */
  listTrustedPeers: (): Promise<TrustedPeerView[]> =>
    invoke<TrustedPeerView[]>("list_trusted_peers"),
  listCaptureDevices: (): Promise<CaptureDeviceView[]> => invoke("list_capture_devices"),
  activeCaptureDevice: (): Promise<CaptureDeviceView | null> => invoke("active_capture_device"),
  /** `connect` —— 手工 IP 连接。 */
  connect: (addr: string): Promise<PeerView> => invoke<PeerView>("connect", { addr }),
  /** `start_send` —— 开始推流；返回 `{ stream_id }`。 */
  startSend: (idShort: string, captureDeviceId: string | null): Promise<StartSendResult> =>
    invoke<StartSendResult>("start_send", { id_short: idShort, capture_device_id: captureDeviceId }),
  /** `stop_send` —— 停止推流（无入参，返回 `null`）。 */
  stopSend: (): Promise<null> => invoke<null>("stop_send"),
  /**
   * `submit_pin` —— 提交 6 位配对码。
   *
   * **比契约 §6 的表格多一个 `id_short`**：引擎的 `submit_pin(peer, pin)` 必须知道是哪条会话
   * （PIN 本身没有归属信息，同时可能有多个对端在配对）。契约文档那一行待同步更新。
   */
  submitPin: (idShort: string, pin: string): Promise<SubmitPinResult> =>
    invoke<SubmitPinResult>("submit_pin", { id_short: idShort, pin }),
  /** `telemetry` —— 遥测快照（首屏水合用；之后靠 `audiolink://telemetry` 事件）。 */
  telemetry: (): Promise<TelemetryView> => invoke<TelemetryView>("telemetry"),
  /** `export_telemetry` —— 把前端累积的采样点写成 CSV，返回落盘路径（M2 的日志导出）。 */
  exportTelemetry: (rows: TelemetryRow[]): Promise<string> =>
    invoke<string>("export_telemetry", { rows }),
  setPeerGain: (idShort: string, gain: number, rampMs: number): Promise<null> =>
    invoke<null>("set_peer_gain", { id_short: idShort, gain, ramp_ms: rampMs }),
  /**
   * `revoke_trust` —— 移除设备（FR-18）：断开该对端 + 撤销信任 + 清掉指向它的「上次设备」记录。
   *
   * 这是隐私说明里「你可以取消配对」的兑现口：此前只能手动删 `trust.json` 或清应用数据。
   * 断会话与撤信任的**顺序**由内核保证（先断后撤），这里只负责把结果如实带给界面。
   */
  revokeTrust: (idShort: string): Promise<RevokeTrustResult> =>
    invoke<RevokeTrustResult>("revoke_trust", { id_short: idShort }),
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
  onPairRequired: (payload: PairRequiredPayload) => void;
  onError: (raw: unknown) => void;
}): () => void {
  let cancelled = false;
  const unlisteners: UnlistenFn[] = [];

  const pending: Promise<UnlistenFn>[] = [
    listen<PeerView[]>(EVENT_PEER, (event) => handlers.onPeers(event.payload)),
    listen<TelemetryView>(EVENT_TELEMETRY, (event) => handlers.onTelemetry(event.payload)),
    listen<[number, number, number]>(EVENT_GROUPS, () => handlers.onGroupUpdated()),
    listen<PairRequiredPayload>(EVENT_PAIR_REQUIRED, (event) => handlers.onPairRequired(event.payload)),
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
