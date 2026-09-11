/**
 * AudioLink 桌面端主界面骨架（规格见 docs/08-ui-spec.md §2）
 *
 * 布局：顶栏（采集设备 / 采样率 / 主题）→ 状态条（本机身份 + 活跃会话汇总 + 遥测开关）
 *       → 设备卡片网格 → 同步组操作条 → 遥测抽屉
 *
 * 当前为静态骨架：数据接入点在 M1（Tauri command + event 遥测流）。
 */

type DeviceStatus = "idle" | "connecting" | "streaming" | "degraded" | "reconnecting" | "failed";

interface DeviceCard {
  id: string;
  name: string;
  platform: "windows" | "android";
  status: DeviceStatus;
  e2eMs?: number;
  kbps?: number;
  lossPct?: number;
  gain: number;
  paired: boolean;
}

const STATUS_STYLE: Record<DeviceStatus, { label: string; dot: string; text: string }> = {
  idle: { label: "空闲", dot: "bg-slate-400", text: "text-slate-500" },
  connecting: { label: "连接中", dot: "bg-indigo-500 animate-pulse", text: "text-indigo-600" },
  streaming: { label: "推送中", dot: "bg-emerald-500", text: "text-emerald-600" },
  degraded: { label: "网络不稳", dot: "bg-amber-500", text: "text-amber-600" },
  reconnecting: { label: "重连中", dot: "bg-amber-500 animate-spin", text: "text-amber-600" },
  failed: { label: "已断开", dot: "bg-red-500", text: "text-red-600" },
};

// 占位数据：M1 起由 audiolink-engine 通过 Tauri event 推送
const PLACEHOLDER: DeviceCard[] = [
  { id: "demo-1", name: "客厅 R1", platform: "android", status: "streaming", e2eMs: 58, kbps: 160, lossPct: 0.1, gain: 80, paired: true },
  { id: "demo-2", name: "卧室 手机", platform: "android", status: "idle", gain: 50, paired: true },
];

function SignalBars({ lossPct }: { lossPct?: number }) {
  const bars = lossPct === undefined ? 0 : lossPct < 0.5 ? 3 : lossPct < 2 ? 2 : 1;
  return (
    <span className="inline-flex items-end gap-[2px]" title={`丢包 ${lossPct ?? "—"}%`}>
      {[1, 2, 3].map((i) => (
        <span
          key={i}
          className={`w-[3px] rounded-sm ${i <= bars ? "bg-emerald-500" : "bg-slate-300 dark:bg-slate-700"}`}
          style={{ height: `${4 + i * 4}px` }}
        />
      ))}
    </span>
  );
}

export default function App() {
  return (
    <div className="min-h-screen flex flex-col">
      {/* 顶栏 */}
      <header className="h-12 px-4 flex items-center gap-3 border-b border-slate-200 dark:border-slate-800">
        <span className="font-semibold tracking-tight">AudioLink</span>
        <select className="ml-4 h-8 rounded-lg border border-slate-200 bg-transparent px-2 text-sm dark:border-slate-700">
          <option>扬声器 (Realtek)</option>
        </select>
        <select className="h-8 rounded-lg border border-slate-200 bg-transparent px-2 text-sm dark:border-slate-700">
          <option>48 kHz</option>
        </select>
        <div className="ml-auto text-sm text-slate-500">v0.1.0</div>
      </header>

      {/* 状态条 */}
      <section className="px-4 py-3 flex items-center gap-4 text-sm border-b border-slate-200 dark:border-slate-800">
        <span>
          本机「书房 PC」 <span className="text-slate-400">fp:00000000</span>
        </span>
        <span className="inline-flex items-center gap-1.5">
          <span className="w-2 h-2 rounded-full bg-emerald-500" />
          推送中 1 台
        </span>
        <span className="text-slate-500">端到端 58 ms · 160 kbps · 丢包 0.1%</span>
        <button className="ml-auto h-8 px-3 rounded-lg border border-slate-200 text-sm dark:border-slate-700">
          遥测
        </button>
      </section>

      {/* 设备卡片网格 */}
      <main className="flex-1 p-4 grid gap-4 [grid-template-columns:repeat(auto-fill,minmax(260px,1fr))]">
        {PLACEHOLDER.map((d) => {
          const s = STATUS_STYLE[d.status];
          return (
            <article
              key={d.id}
              className="rounded-xl border border-slate-200 bg-white p-4 shadow-sm dark:border-slate-800 dark:bg-slate-900"
            >
              <div className="flex items-center gap-2">
                <span className={`w-2 h-2 rounded-full ${s.dot}`} />
                <span className="font-medium">{d.name}</span>
                <span className={`ml-auto text-xs ${s.text}`}>{s.label}</span>
              </div>

              <div className="mt-3 flex items-center gap-3 text-xs tabular-nums text-slate-500">
                <span>{d.e2eMs ? `${d.e2eMs} ms` : "—"}</span>
                <span>{d.kbps ? `${d.kbps} kbps` : "—"}</span>
                <SignalBars lossPct={d.lossPct} />
              </div>

              <div className="mt-3 flex items-center gap-2">
                <span className="text-xs text-slate-400">音量</span>
                <input
                  type="range"
                  defaultValue={d.gain}
                  className="flex-1 accent-indigo-600"
                  aria-label={`${d.name} 音量`}
                />
                <span className="w-8 text-right text-xs tabular-nums">{d.gain}</span>
              </div>

              <div className="mt-3 flex gap-2">
                <button className="flex-1 h-8 rounded-lg bg-indigo-600 text-sm text-white">
                  {d.status === "streaming" ? "断开" : "连接"}
                </button>
                <button className="h-8 px-3 rounded-lg border border-slate-200 text-sm dark:border-slate-700">
                  静音
                </button>
              </div>
            </article>
          );
        })}

        <button className="rounded-xl border border-dashed border-slate-300 p-4 text-sm text-slate-500 hover:border-slate-400 dark:border-slate-700">
          ＋ 手动添加设备（IP:端口）
        </button>
      </main>

      {/* 同步组操作条 */}
      <footer className="h-14 px-4 flex items-center gap-4 text-sm border-t border-slate-200 dark:border-slate-800">
        <span className="text-slate-500">同步组</span>
        {PLACEHOLDER.map((d) => (
          <label key={d.id} className="inline-flex items-center gap-1.5">
            <input type="checkbox" className="accent-indigo-600" />
            {d.name}
          </label>
        ))}
        <button className="ml-auto h-8 px-3 rounded-lg border border-slate-200 dark:border-slate-700">
          建立同步组
        </button>
      </footer>
    </div>
  );
}
