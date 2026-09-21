import type { CaptureDeviceView, CommandError } from "../types";
import { t } from "../i18n";
import { IconRefresh } from "./icons";

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

/**
 * 采集源面板 —— 本机总线上的输入端。
 *
 * 布局按**本机总线侧栏**（约 300px 宽）重排：原来是"标题在左、控件在右"的横向两栏，
 * 在窄列里会挤成一行两三个字，所以改成纵向堆叠，控件撑满整列。
 */
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
    <section aria-labelledby="capture-heading" className="al-capture-panel">
      <h2 id="capture-heading" className="text-body font-semibold">{t("cap.title")}</h2>
      <p className="mt-1.5 text-caption leading-relaxed text-text-tertiary">{t("cap.hint")}</p>

      <label htmlFor="capture-device" className="text-caption font-semibold text-text-tertiary mt-3 block">{t("cap.device")}</label>
      <div className="mt-1.5 flex gap-2">
        <select
          id="capture-device"
          value={selectedId}
          disabled={locked || loading || noDevices || error !== null}
          onChange={(event) => onSelect(event.target.value)}
          aria-describedby="capture-help capture-status"
          aria-invalid={!locked && problem !== null}
          className="h-10 min-w-0 flex-1 border border-stroke-control bg-surface-card-solid px-2 text-caption text-text-primary disabled:cursor-not-allowed disabled:text-text-tertiary"
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
        {/* 侧栏只有 280px：文字按钮会把下拉框挤到读不出设备名，所以做成图标键，
            可访问名与 title 仍然是那句完整的「刷新设备」。 */}
        <button
          type="button"
          onClick={() => void onRefresh()}
          disabled={loading}
          aria-label={loading ? t("cap.refreshing") : t("cap.refresh")}
          title={loading ? t("cap.refreshing") : t("cap.refresh")}
          className="al-btn h-10 w-10 shrink-0"
        >
          <IconRefresh className="h-4 w-4" />
        </button>
      </div>

      <p id="capture-help" className="mt-2 text-caption leading-relaxed text-text-tertiary">
        {locked ? t("cap.locked_hint") : t("cap.hint_idle")}
      </p>
      <div id="capture-status" role="status" aria-live="polite" className="mt-2 border-t border-stroke-control pt-2 text-caption leading-relaxed">
        {active ? <p className="break-words text-success">{t("cap.active", { name: active.name, rate: active.sampleRate / 1000, channels: active.channels })}</p> : null}
        {(!locked || error !== null) && problem ? <p className="text-caution">{problem}</p> : null}
        {!locked && !problem && selected ? <p className="break-words text-text-tertiary">{t("cap.selected", { name: selected.name })}{selected.isVirtual ? t("cap.selected_virtual") : ""}</p> : null}
      </div>
    </section>
  );
}
