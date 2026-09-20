/**
 * 会话汇总条（工具栏内容）：推流状态 + 诊断开关。
 *
 * 「音源即主机」下它不再是一条独立的横幅，而是主区工具栏的左半段 ——
 * 右边留给外观切换器，两者共用同一条 12px 高的带子。
 *
 * 指纹与监听地址**不在这里**：那是开发信息，而且地址只在地址卡片里给一次就够 ——
 * 同一串东西在两处各说一遍，用户反而分不清该抄哪一个给对方。
 */

import type { TelemetryView } from "../types";
import { bpsToKbps, usToMs } from "../types";
import { t } from "../i18n";
import { IconChevron } from "./icons";

interface StatusStripProps {
  /** 在播：用户已发起推流，或引擎里已经有会话在推。 */
  onAir: boolean;
  /** 已发起但还没有设备在听 —— 这里必须与侧栏说同一句话，不能一边"推流中"一边"未推流"。 */
  waitingForDevice: boolean;
  telemetry: TelemetryView | null;
  /** 活跃会话数（在推 + 链路不稳降级的）。 */
  streamingCount: number;
  telemetryOpen: boolean;
  onToggleTelemetry: () => void;
}

export function StatusStrip({
  onAir,
  waitingForDevice,
  telemetry,
  streamingCount,
  telemetryOpen,
  onToggleTelemetry,
}: StatusStripProps) {
  // 汇总数字只在**真的有会话**时露面：等待设备接入时它还是上一次会话的旧值，当"当前"报很骗人。
  const streaming = streamingCount > 0;
  // 可访问名保留完整措辞（含方向符），可见文字是它的前缀 —— 不出现"看得见的是 A、读出来的是 B"。
  const toggleLabel = t("status.telemetry", { arrow: telemetryOpen ? "▾" : "▸" });
  const toggleText = t("status.telemetry", { arrow: "" }).trim();

  return (
    <section className="flex min-w-0 flex-1 flex-wrap items-center gap-x-5 gap-y-1 text-caption">
      <span className="inline-flex items-center gap-2">
        <span className="al-lamp h-2 w-2" data-on={onAir ? (waitingForDevice ? "warn" : "on") : "off"} />
        <span className={onAir ? "text-text-primary" : "text-text-tertiary"}>
          {waitingForDevice
            ? t("side.broadcasting_waiting")
            : streaming
              ? t("status.streaming", { count: streamingCount })
              : t("status.idle")}
        </span>
      </span>

      {streaming && telemetry ? (
        <span className="num text-text-secondary">
          {t("status.summary", { e2e: usToMs(telemetry.e2eLatencyUs), rate: bpsToKbps(telemetry.bitrateBps), loss: telemetry.lossPct.toFixed(2) })}
        </span>
      ) : null}

      <button
        type="button"
        onClick={onToggleTelemetry}
        aria-expanded={telemetryOpen}
        aria-controls="telemetry-panel"
        aria-label={toggleLabel}
        title={toggleLabel}
        className="al-btn ml-auto h-7 gap-1.5 px-2.5 text-caption"
      >
        {toggleText}
        <IconChevron
          className={"h-3.5 w-3.5 transition-transform duration-200 " + (telemetryOpen ? "rotate-180" : "")}
        />
      </button>
    </section>
  );
}
