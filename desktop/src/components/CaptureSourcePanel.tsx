import type { CaptureDeviceView, CommandError } from "../types";

interface CaptureSourcePanelProps {
  devices: CaptureDeviceView[];
  selectedId: string;
  active: CaptureDeviceView | null;
  loading: boolean;
  locked: boolean;
  error: CommandError | null;
  onSelect: (id: string) => void;
  onRefresh: () => Promise<void>;
}

export function CaptureSourcePanel({ devices, selectedId, active, loading, locked, error, onSelect, onRefresh }: CaptureSourcePanelProps) {
  const selected = devices.find((device) => selectedId === "" ? device.isDefault : device.id === selectedId);
  const missing = selectedId !== "" && selected === undefined;
  const noDevices = !loading && error === null && devices.length === 0;
  const problem = error?.message ?? (noDevices
    ? "未找到输出设备。连接扬声器、耳机或声卡后刷新。"
    : missing
      ? "所选设备已断开或不可用。重新连接后刷新，或选择其他输出设备。"
      : selected?.unavailableReason ?? (!loading && selected === undefined ? "系统未设置默认输出设备，请选择下方的具体设备。" : null));

  return (
    <section aria-labelledby="capture-heading" className="rounded-xl border border-slate-200 bg-white p-4 dark:border-slate-800 dark:bg-slate-950">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:gap-6">
        <div className="sm:w-56 sm:shrink-0">
          <h2 id="capture-heading" className="text-sm font-semibold">采集声音</h2>
          <p className="mt-1 text-xs leading-5 text-slate-600 dark:text-slate-400">将此输出设备正在播放的声音发送给对方。</p>
        </div>
        <div className="min-w-0 flex-1">
          <label htmlFor="capture-device" className="block text-xs font-medium text-slate-700 dark:text-slate-300">输出设备</label>
          <div className="mt-1 flex gap-2">
            <select
              id="capture-device"
              value={selectedId}
              disabled={locked || loading || noDevices || error !== null}
              onChange={(event) => onSelect(event.target.value)}
              aria-describedby="capture-help capture-status"
              aria-invalid={!locked && problem !== null}
              className="h-10 min-w-0 flex-1 rounded-lg border border-slate-300 bg-white px-2 text-sm text-slate-900 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 disabled:cursor-not-allowed disabled:bg-slate-100 dark:border-slate-700 dark:bg-slate-950 dark:text-slate-100 dark:disabled:bg-slate-900"
            >
              <option value="">{loading && devices.length === 0 ? "正在读取输出设备…" : "系统默认（开始推流时使用）"}</option>
              {missing ? <option value={selectedId}>所选设备不可用</option> : null}
              {devices.map((device) => (
                <option key={device.id} value={device.id}>
                  {device.name}{devices.filter((item) => item.name === device.name).length > 1 ? ` · ${device.id.slice(-9, -1)}` : ""}
                  {device.isDefault ? " · 系统默认" : ""} · {device.sampleRate / 1000} kHz{device.unavailableReason ? " · 需调整格式" : ""}
                </option>
              ))}
            </select>
            <button type="button" onClick={() => void onRefresh()} disabled={loading}
              className="h-10 shrink-0 rounded-lg border border-slate-300 px-3 text-sm font-medium hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 disabled:cursor-wait dark:border-slate-700 dark:hover:bg-slate-800"
            >{loading ? "读取中…" : "刷新设备"}</button>
          </div>
          <p id="capture-help" className="mt-2 text-xs leading-5 text-slate-600 dark:text-slate-400">
            {locked ? "切换音源前请先停止推流。更改 Windows 默认输出不会切换当前正在采集的设备。" : "让播放器使用同一个输出设备。更换耳机或声卡后，可刷新设备列表。"}
          </p>
          <div id="capture-status" role="status" aria-live="polite" className="mt-2 text-xs leading-5">
            {active ? <p className="break-words text-emerald-700 dark:text-emerald-400">正在采集：{active.name} · {active.sampleRate / 1000} kHz · {active.channels} 声道</p> : null}
            {(!locked || error !== null) && problem ? <p className="text-amber-800 dark:text-amber-300">{problem}</p> : null}
            {!locked && !problem && selected ? <p className="break-words text-slate-600 dark:text-slate-400">已选择：{selected.name}{selected.isVirtual ? "。这是虚拟设备，请确认播放器将声音输出到这里。" : ""}</p> : null}
          </div>
        </div>
      </div>
    </section>
  );
}
