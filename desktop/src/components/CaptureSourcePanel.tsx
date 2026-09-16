import type { CaptureDeviceView, CommandError } from "../types";
import { t } from "../i18n";

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
    ? t("cap.no_device")
    : missing
      ? t("cap.device_lost")
      : selected?.unavailableReason ?? (!loading && selected === undefined ? t("cap.no_default") : null));

  return (
    <section aria-labelledby="capture-heading" className="rounded-xl border border-slate-200 bg-white p-4 dark:border-slate-800 dark:bg-slate-950">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:gap-6">
        <div className="sm:w-56 sm:shrink-0">
          <h2 id="capture-heading" className="text-sm font-semibold">{t("cap.title")}</h2>
          <p className="mt-1 text-xs leading-5 text-slate-600 dark:text-slate-400">{t("cap.hint")}</p>
        </div>
        <div className="min-w-0 flex-1">
          <label htmlFor="capture-device" className="block text-xs font-medium text-slate-700 dark:text-slate-300">{t("cap.device")}</label>
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
              <option value="">{loading && devices.length === 0 ? t("cap.reading") : t("cap.system_default")}</option>
              {missing ? <option value={selectedId}>{t("cap.unavailable")}</option> : null}
              {devices.map((device) => (
                <option key={device.id} value={device.id}>
                  {device.name}{devices.filter((item) => item.name === device.name).length > 1 ? ` · ${device.id.slice(-9, -1)}` : ""}
                  {device.isDefault ? t("cap.tag_default") : ""} · {device.sampleRate / 1000} kHz{device.unavailableReason ? t("cap.tag_format") : ""}
                </option>
              ))}
            </select>
            <button type="button" onClick={() => void onRefresh()} disabled={loading}
              className="h-10 shrink-0 rounded-lg border border-slate-300 px-3 text-sm font-medium hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 disabled:cursor-wait dark:border-slate-700 dark:hover:bg-slate-800"
            >{loading ? t("cap.refreshing") : t("cap.refresh")}</button>
          </div>
          <p id="capture-help" className="mt-2 text-xs leading-5 text-slate-600 dark:text-slate-400">
            {locked ? t("cap.locked_hint") : t("cap.hint_idle")}
          </p>
          <div id="capture-status" role="status" aria-live="polite" className="mt-2 text-xs leading-5">
            {active ? <p className="break-words text-emerald-700 dark:text-emerald-400">{t("cap.active", { name: active.name, rate: active.sampleRate / 1000, channels: active.channels })}</p> : null}
            {(!locked || error !== null) && problem ? <p className="text-amber-800 dark:text-amber-300">{problem}</p> : null}
            {!locked && !problem && selected ? <p className="break-words text-slate-600 dark:text-slate-400">{t("cap.selected", { name: selected.name })}{selected.isVirtual ? t("cap.selected_virtual") : ""}</p> : null}
          </div>
        </div>
      </div>
    </section>
  );
}
