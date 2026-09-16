/**
 * M5 · 中英双语（i18n）。
 *
 * 三个设计取舍，都是为了「让翻译这件事不变成样板代码」：
 *
 * 1. **`t` 是模块级函数，不是 hook**：界面里有 120+ 处文案，若每处都要 `const t = useT()`，
 *    改文案就变成了改 16 个组件的样板。这里让组件直接 `import { t }`，语言变化由 App 顶层的
 *    `useLocale()` 订阅（`useSyncExternalStore`）触发整棵树重渲染 —— 切换语言时文案全部跟着变。
 * 2. **缺键是编译错误**：英文表声明为 `Record<MessageKey, string>`，漏一条 `tsc` 就红。
 *    这是本项目唯一能自动验证「翻译完整性」的手段（没有 i18n 框架，也就不需要它的运行时校验）。
 * 3. **不做复数/性别规则**：当前文案没有需要复数的句子；真需要时再加 ICU 这类依赖，
 *    现在就引进来只是为了「以后可能用到」。
 *
 * 持久化：语言偏好写进 `settings.json` 的 `locale` 键（Tauri 侧 `locale` / `set_locale` 命令），
 * 与 `autostart` / `auto_connect` 同一条路 —— 用户的选择必须跨重启活着。
 * 未设置时跟随系统语言（`navigator.language`），并把推断结果写回去。
 */

import { useSyncExternalStore } from "react";

export type Locale = "zh-CN" | "en-US";

/** 语言选项（设置面板的下拉用）。 */
export const LOCALES: ReadonlyArray<{ value: Locale; label: string }> = [
  { value: "zh-CN", label: "中文" },
  { value: "en-US", label: "English" },
];

/** 中文表 = 基准：键集合由它定义。 */
const zh = {
  "topbar.subtitle": "局域网低延迟音频分发",
  "status.local": "本机「{name}」",
  "status.streaming": "推送中 {count} 台",
  "status.idle": "未在推流",
  "status.summary": "端到端 {e2e} ms · {rate} kbps · 丢包 {loss}%",
  "status.telemetry": "遥测 {arrow}",
  "empty.devices": "还没有设备。填入对方 IP 手动连接；自动发现（mDNS/广播）属 M2 后续版本。",
  "manual.title": "手动添加设备",
  "manual.hint": "两台设备需在同一 Wi-Fi/局域网；若自动发现失效（AP 客户端隔离），用对方「本机状态条」上的地址直连。",
  "manual.address": "对方地址",
  "manual.placeholder": "192.168.1.23 或 192.168.1.23:58290",
  "manual.connecting": "连接中…",
  "manual.connect": "连接",
  "pair.title": "与「{name}」配对",
  "pair.receive": "请在对方的设备上输入下面这 6 位数字；两端显示一致才说明中间没有第三台设备。",
  "pair.code": "配对码",
  "pair.expired_receive": "已超过 60 秒，请让对方重新发起连接",
  "pair.ttl": "有效期剩余 {seconds} 秒",
  "pair.got_it": "知道了",
  "pair.send": "对方要求配对。请输入对方设备屏幕上显示的 6 位数字。",
  "pair.input": "6 位配对码",
  "pair.expired_send": "配对码已过期，请让对方重新发起连接",
  "pair.later": "稍后再说",
  "pair.verifying": "校验中…",
  "pair.confirm": "确认配对",
  "peer.fingerprint": "指纹",
  "peer.address": "地址",
  "peer.trust": "信任",
  "peer.caps_agreed": "可用：{list}",
  "peer.caps_missing": "对端不支持：{list}",
  "peer.trusted": "已配对（白名单命中）",
  "peer.untrusted": "未配对",
  "peer.degraded": "网络不稳，已自动降码率",
  "peer.volume": "音量",
  "peer.volume_label": "对端音量",
  "peer.volume_hint": "对端音量（0–200%），拖动时按 200 ms 渐变生效",
  "peer.pin_entry": "输入配对码",
  "peer.pin_hint": "对方需要在它的设备上输入本机显示的 6 位配对码",
  "peer.waiting": "等待配对",
  "peer.stopping": "停止中…",
  "peer.stop": "停止推流",
  "peer.starting": "启动中…",
  "peer.start": "开始推流",
  "set.title": "设置（M5）",
  "set.tray_hint": "关掉窗口不会退出程序：AudioLink 常驻托盘（右键托盘图标可退出）。",
  "set.autostart": "开机自动启动",
  "set.autoconnect": "启动时自动连接上次设备",
  "set.loading": "（读取中…）",
  "set.no_history": "还没有成功连接过的设备。",
  "set.last_peer": "上次设备：{peer}",
  "set.language": "界面语言",
  "cap.title": "采集声音",
  "cap.hint": "将此输出设备正在播放的声音发送给对方。",
  "cap.device": "输出设备",
  "cap.reading": "正在读取输出设备…",
  "cap.system_default": "系统默认（开始推流时使用）",
  "cap.unavailable": "所选设备不可用",
  "cap.tag_default": " · 系统默认",
  "cap.tag_format": " · 需调整格式",
  "cap.refresh": "刷新设备",
  "cap.refreshing": "读取中…",
  "cap.locked_hint": "切换音源前请先停止推流。更改 Windows 默认输出不会切换当前正在采集的设备。",
  "cap.hint_idle": "让播放器使用同一个输出设备。更换耳机或声卡后，可刷新设备列表。",
  "cap.active": "正在采集：{name} · {rate} kHz · {channels} 声道",
  "cap.selected": "已选择：{name}",
  "cap.selected_virtual": "。这是虚拟设备，请确认播放器将声音输出到这里。",
  "cap.no_device": "未找到输出设备。连接扬声器、耳机或声卡后刷新。",
  "cap.device_lost": "所选设备已断开或不可用。重新连接后刷新，或选择其他输出设备。",
  "cap.no_default": "系统未设置默认输出设备，请选择下方的具体设备。",
  "group.title": "同步组（临时组 · 组内对齐靠 epoch 排播）",
  "group.refresh": "刷新",
  "group.lead": "提前量",
  "group.busy": "处理中…",
  "group.create": "建组（已选 {count} 台）",
  "group.no_peers": "还没有可加入的设备：先连接并对端进入会话。",
  "group.none": "当前没有同步组。",
  "group.summary": "组 #{id} · {count} 台 · 提前量 {lead} ms",
  "group.no_clock": "还没有时钟估计",
  "group.offset": "偏移 {us} µs",
  "group.poor": "同步质量差",
  "group.leave": "退出",
  "align.title": "多源对齐（M4）",
  "align.hint": "跨度 = 用同一个「现在」推算各路样本编号的差值；一帧 = 960 样本 / 20 ms。",
  "align.lead": "提前量",
  "align.busy": "广播中…",
  "align.broadcast": "广播共同基准",
  "align.no_session": "还没有会话：接受对端推流后这里会显示每一路的读数。",
  "align.need_two": "有效读数不足两路，无法比较（需要至少两路同时在收音频）。",
  "align.spread": "当前跨度：{samples} 样本",
  "align.spread_ms": "当前跨度：{samples} 样本 ≈ {ms} ms",
  "align.peer": "对端",
  "align.recent": "最近编号",
  "align.estimated": "推算编号",
  "align.arrival": "到达 (ms)",
  "align.aligned": "已对齐",
  "align.drifting": "错位",
  "align.unknown": "读数不足",
  "tm.title": "遥测",
  "tm.waiting": "等待采样…",
  "tm.footer": "后端每 500 ms 推送一次；曲线为最近 {count} 个采样点",
  "tm.exporting": "导出中…",
  "tm.export": "导出 CSV（{count} 点）",
  "tm.peers": "对端数",
  "tm.jitter": "抖动",
  "tm.loss": "丢包",
  "tm.bitrate": "码率",
  "tm.buffer": "缓冲水位",
  "tm.underruns": "欠载",
  "tm.e2e": "端到端",
  "tm.e2e_p50": "端到端 P50",
  "tm.e2e_p95": "端到端 P95",
  "tm.e2e_latency": "端到端延迟",
  "tm.loss_rate": "丢包率",
  "tm.count": "次",
  "tm.samples_aria": "{label} 最近 {count} 个采样点",
  "tm.idle": "未推流：数字与曲线都还没有真实数据。",
  "about.title": "关于 / 第三方声明（M5）",
  "about.body": "清单（含构建期依赖的超集）与去重后的许可全文；随安装包一起分发。",
  "about.hide": "收起",
  "about.loading": "读取中…",
  "about.show": "查看",
  "about.missing": "未找到声明文件",
  "banner.dismiss": "关闭提示",
  "banner.close": "关闭",
  "state.idle": "空闲",
  "state.handshaking": "连接中",
  "state.streaming": "推送中",
  "state.degraded": "网络不稳",
  "state.failed": "已断开",
  "err.ipc": "界面与内核通信失败，请重试；若持续出现请查看日志",
  "err.unknown": "未知错误",
  "toast.auto_connected": "已自动连接上次设备 {peer}",
  "toast.this_device": "该设备",
  "toast.no_telemetry": "还没有遥测历史可导出：先连接并开始推流。",
  "toast.exported": "已导出 {count} 个采样点：{path}",
  "toast.gain": "{peer} 音量已设为 {pct}%",
  "toast.pick_peer": "先勾选至少一台已连接设备再建组。",
  "toast.group_created": "已建组 #{id}，成员 {count} 台",
  "toast.group_joined": "已把 {peer} 加入组 #{id}",
  "toast.epoch_sent": "已向 {count} 条会话广播共同基准（提前量 {lead} ms）",
  "toast.group_left": "已让 {peer} 退出组 #{id}",
} as const;

/** 键集合由中文表定义；英文表必须一条不漏（漏了 tsc 直接报错）。 */
export type MessageKey = keyof typeof zh;

const en: Record<MessageKey, string> = {
  "topbar.subtitle": "Low-latency audio over your local network",
  "status.local": "This device \"{name}\"",
  "status.streaming": "Streaming to {count}",
  "status.idle": "Not streaming",
  "status.summary": "End-to-end {e2e} ms · {rate} kbps · loss {loss}%",
  "status.telemetry": "Telemetry {arrow}",
  "empty.devices": "No devices yet. Enter the other side IP to connect manually; automatic discovery (mDNS/broadcast) is planned for a later milestone.",
  "manual.title": "Add a device manually",
  "manual.hint": "Both devices must be on the same Wi-Fi/LAN. If discovery fails (AP client isolation), connect directly using the address shown on the other device status strip.",
  "manual.address": "Peer address",
  "manual.placeholder": "192.168.1.23 or 192.168.1.23:58290",
  "manual.connecting": "Connecting…",
  "manual.connect": "Connect",
  "pair.title": "Pair with \"{name}\"",
  "pair.receive": "Enter these 6 digits on the other device. Only when both sides show the same digits is there no third device in between.",
  "pair.code": "Pairing code",
  "pair.expired_receive": "More than 60 seconds have passed - ask the other device to start again",
  "pair.ttl": "{seconds}s remaining",
  "pair.got_it": "Got it",
  "pair.send": "The other device wants to pair. Enter the 6 digits shown on its screen.",
  "pair.input": "6-digit pairing code",
  "pair.expired_send": "The pairing code has expired - ask the other device to start again",
  "pair.later": "Not now",
  "pair.verifying": "Verifying…",
  "pair.confirm": "Confirm pairing",
  "peer.fingerprint": "Fingerprint",
  "peer.address": "Address",
  "peer.trust": "Trust",
  "peer.caps_agreed": "Available: {list}",
  "peer.caps_missing": "peer lacks: {list}",
  "peer.trusted": "Paired (trusted)",
  "peer.untrusted": "Not paired",
  "peer.degraded": "Unstable link - bitrate reduced automatically",
  "peer.volume": "Volume",
  "peer.volume_label": "Peer volume",
  "peer.volume_hint": "Peer volume (0-200%); changes ramp over 200 ms",
  "peer.pin_entry": "Enter pairing code",
  "peer.pin_hint": "The other device must enter the 6-digit code shown here",
  "peer.waiting": "Waiting to pair",
  "peer.stopping": "Stopping…",
  "peer.stop": "Stop streaming",
  "peer.starting": "Starting…",
  "peer.start": "Start streaming",
  "set.title": "Settings (M5)",
  "set.tray_hint": "Closing the window does not quit: AudioLink stays in the tray (right-click the tray icon to quit).",
  "set.autostart": "Start on sign-in",
  "set.autoconnect": "Reconnect to the last device on launch",
  "set.loading": "(loading…)",
  "set.no_history": "No device has connected successfully yet.",
  "set.last_peer": "Last device: {peer}",
  "set.language": "Interface language",
  "cap.title": "Capture audio",
  "cap.hint": "Send what this output device is playing to the peer.",
  "cap.device": "Output device",
  "cap.reading": "Reading output devices…",
  "cap.system_default": "System default (used when streaming starts)",
  "cap.unavailable": "Selected device unavailable",
  "cap.tag_default": " · system default",
  "cap.tag_format": " · format mismatch",
  "cap.refresh": "Refresh devices",
  "cap.refreshing": "Reading…",
  "cap.locked_hint": "Stop streaming before switching sources. Changing the Windows default output does not switch the device being captured.",
  "cap.hint_idle": "Point the player at the same output device. After swapping headsets or sound cards, refresh the list.",
  "cap.active": "Capturing: {name} · {rate} kHz · {channels} ch",
  "cap.selected": "Selected: {name}",
  "cap.selected_virtual": ". This is a virtual device - make sure the player sends its audio here.",
  "cap.no_device": "No output device found. Connect speakers, headphones or a sound card, then refresh.",
  "cap.device_lost": "The selected device is disconnected or unavailable. Reconnect and refresh, or choose another output device.",
  "cap.no_default": "Windows has no default output device - choose one below.",
  "group.title": "Sync groups (ad-hoc; in-group alignment via epoch scheduling)",
  "group.refresh": "Refresh",
  "group.lead": "Lead time",
  "group.busy": "Working…",
  "group.create": "Create group ({count} selected)",
  "group.no_peers": "No peers to join yet - connect first and wait for a session.",
  "group.none": "No sync groups yet.",
  "group.summary": "Group #{id} · {count} peers · lead {lead} ms",
  "group.no_clock": "No clock estimate yet",
  "group.offset": "offset {us} µs",
  "group.poor": "Poor sync quality",
  "group.leave": "Leave",
  "align.title": "Multi-source alignment (M4)",
  "align.hint": "Spread = difference between per-source sample indices derived from the same \"now\"; one frame = 960 samples / 20 ms.",
  "align.lead": "Lead time",
  "align.busy": "Broadcasting…",
  "align.broadcast": "Broadcast common epoch",
  "align.no_session": "No session yet - accept an incoming stream and per-source readings appear here.",
  "align.need_two": "Fewer than two valid readings, cannot compare (at least two sources must be receiving audio).",
  "align.spread": "Current spread: {samples} samples",
  "align.spread_ms": "Current spread: {samples} samples ≈ {ms} ms",
  "align.peer": "Peer",
  "align.recent": "Latest index",
  "align.estimated": "Estimated index",
  "align.arrival": "Arrival (ms)",
  "align.aligned": "Aligned",
  "align.drifting": "Misaligned",
  "align.unknown": "Not enough readings",
  "tm.title": "Telemetry",
  "tm.waiting": "Waiting for samples…",
  "tm.footer": "The engine pushes every 500 ms; the curve shows the last {count} samples",
  "tm.exporting": "Exporting…",
  "tm.export": "Export CSV ({count} points)",
  "tm.peers": "Peers",
  "tm.jitter": "Jitter",
  "tm.loss": "Loss",
  "tm.bitrate": "Bitrate",
  "tm.buffer": "Buffer level",
  "tm.underruns": "Underruns",
  "tm.e2e": "End-to-end",
  "tm.e2e_p50": "End-to-end P50",
  "tm.e2e_p95": "End-to-end P95",
  "tm.e2e_latency": "End-to-end latency",
  "tm.loss_rate": "Loss rate",
  "tm.count": "events",
  "tm.samples_aria": "{label}: last {count} samples",
  "tm.idle": "Not streaming - no real data for the numbers or the curve yet.",
  "about.title": "About / third-party notices (M5)",
  "about.body": "Component list (a superset including build-time dependencies) plus deduplicated license texts, shipped with the installer.",
  "about.hide": "Hide",
  "about.loading": "Loading…",
  "about.show": "Show",
  "about.missing": "Notices file not found",
  "banner.dismiss": "Dismiss",
  "banner.close": "Close",
  "state.idle": "Idle",
  "state.handshaking": "Connecting",
  "state.streaming": "Streaming",
  "state.degraded": "Unstable",
  "state.failed": "Disconnected",
  "err.ipc": "The UI could not reach the engine. Retry; if it keeps happening, check the logs",
  "err.unknown": "Unknown error",
  "toast.auto_connected": "Reconnected to the last device {peer}",
  "toast.this_device": "this device",
  "toast.no_telemetry": "No telemetry history to export yet - connect and start streaming.",
  "toast.exported": "Exported {count} samples: {path}",
  "toast.gain": "Volume for {peer} set to {pct}%",
  "toast.pick_peer": "Select at least one connected device before creating a group.",
  "toast.group_created": "Group #{id} created with {count} member(s)",
  "toast.group_joined": "Added {peer} to group #{id}",
  "toast.epoch_sent": "Broadcast the common epoch to {count} session(s) (lead {lead} ms)",
  "toast.group_left": "Removed {peer} from group #{id}",
};

const TABLES: Record<Locale, Record<MessageKey, string>> = {
  "zh-CN": zh,
  "en-US": en,
};

let locale: Locale = detectLocale();
const listeners = new Set<() => void>();

/** 系统语言 → 支持的语言（只有中/英两种，其余一律回落英文）。 */
export function detectLocale(): Locale {
  if (typeof navigator === "undefined") return "en-US";
  return navigator.language.toLowerCase().startsWith("zh") ? "zh-CN" : "en-US";
}

/** 当前语言（渲染用；变化时由 `useLocale()` 通知 React）。 */
export function currentLocale(): Locale {
  return locale;
}

/** 切换语言。持久化由调用方负责（见 `SettingsPanel`）。 */
export function setLocale(next: Locale): void {
  if (next === locale) return;
  locale = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** 订阅当前语言；语言一变，调用它的组件（App 顶层）就会重渲染。 */
export function useLocale(): Locale {
  return useSyncExternalStore(subscribe, currentLocale, currentLocale);
}

/** 取文案。`{name}` 形式占位，`vars` 提供值；缺哪个键就把占位符原样留在结果里。 */
export function t(key: MessageKey, vars?: Record<string, string | number>): string {
  const raw: string = TABLES[locale][key];
  if (vars === undefined) return raw;
  return raw.replace(/\{(\w+)\}/g, (match: string, name: string) =>
    Object.prototype.hasOwnProperty.call(vars, name) ? String(vars[name]) : match,
  );
}
