import type { MessageKey } from "../i18n";

export type BroadcastStatus = "loading" | "waiting" | "starting" | "streaming" | "pausing" | "paused" | "unavailable" | "error" | "manual";

export const broadcastLabels: Record<BroadcastStatus, MessageKey> = {
  loading: "share.loading", waiting: "share.waiting", starting: "share.starting",
  streaming: "share.streaming", pausing: "share.pausing", paused: "share.paused",
  unavailable: "share.unavailable", error: "share.error", manual: "share.manual",
};

export const broadcastHints: Record<BroadcastStatus, MessageKey> = {
  loading: "share.loading_hint", waiting: "share.waiting_hint", starting: "share.starting_hint",
  streaming: "share.streaming_hint", pausing: "share.pausing_hint", paused: "share.paused_hint",
  unavailable: "share.unavailable_hint", error: "share.error_hint", manual: "share.manual_hint",
};

export function broadcastLamp(status: BroadcastStatus): string {
  if (status === "streaming") return "on";
  if (status === "starting" || status === "pausing" || status === "loading") return "busy";
  if (status === "unavailable") return "warn";
  if (status === "error") return "live";
  return "off";
}
