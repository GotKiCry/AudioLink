/**
 * 总控条：这台机器的播出总闸。
 *
 * ON AIR 灯是全屏唯一的"大灯" —— 它回答的是"我现在到底在不在往外送声音"，
 * 比任何数字都先被余光读到；旁边的通道计数与「全部停止」是它仅有的两个邻居。
 */

import { t } from "../i18n";
import { IconStop } from "./icons";

interface ConsoleFooterProps {
  /** 正在推流/降级中的通道数。 */
  streamingCount: number;
  /** 已有的通道总数（含空闲与断开）。 */
  channelCount: number;
  busy: boolean;
  onStopAll: () => void;
}

export function ConsoleFooter({ streamingCount, channelCount, busy, onStopAll }: ConsoleFooterProps) {
  const onAir = streamingCount > 0;
  return (
    <footer className="flex h-14 shrink-0 items-center gap-4 border-t border-line bg-bench px-4">
      <span className="well flex h-9 items-center gap-2.5 px-3">
        <span className="lamp h-3 w-3" data-on={onAir ? "live" : "off"} />
        <span className={`t-cap font-semibold uppercase tracking-[0.16em] ${onAir ? "text-ink-live" : "text-silk-3"}`}>
          {onAir ? t("console.on_air") : t("console.off_air")}
        </span>
      </span>

      <span className="num t-cap text-silk-3">
        {t("console.channels", { count: channelCount })}
      </span>

      <button
        type="button"
        onClick={onStopAll}
        disabled={!onAir || busy}
        className="key key-danger ml-auto h-9 gap-2 px-4 t-cap"
      >
        <IconStop className="h-3.5 w-3.5" />
        {t("console.stop_all")}
      </button>
    </footer>
  );
}
