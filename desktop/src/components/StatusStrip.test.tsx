/**
 * 本机状态条（`StatusStrip`）—— 用户第一眼看的那一行。
 *
 * 钉两件容易糊掉的事：① 没在推流时**不**显示汇总数字（拿上一次遥测的旧值当"当前"很骗人）；
 * ② 水合未完成时用占位符，绝不把 undefined 印在状态条上。
 */
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { LocalStatus, TelemetryView } from "../types";
import { bpsToKbps, usToMs } from "../types";
import { t } from "../i18n";
import { StatusStrip } from "./StatusStrip";

const local: LocalStatus = {
  idShort: "3f9a1c0b",
  name: "本机",
  addr: "192.168.1.10:58290",
  platform: "windows",
};

const telemetry: TelemetryView = {
  peers: 1,
  rttUs: 12000,
  jitterUs: 800,
  lossPct: 0.15,
  bitrateBps: 320000,
  bufferLevelUs: 40000,
  underruns: 0,
  e2eLatencyUs: 45000,
  e2eP50Us: 42000,
  e2eP95Us: 61000,
};

function strip(over: { local?: LocalStatus | null; streamingCount?: number; telemetryOpen?: boolean } = {}) {
  const onToggleTelemetry = vi.fn();
  render(
    <StatusStrip
      local={over.local === undefined ? local : over.local}
      telemetry={telemetry}
      streamingCount={over.streamingCount ?? 0}
      telemetryOpen={over.telemetryOpen ?? true}
      onToggleTelemetry={onToggleTelemetry}
    />,
  );
  return { onToggleTelemetry };
}

describe("状态汇总", () => {
  it("没有推流时只说「未在推流」，不显示任何汇总数字", () => {
    // 改坏：把 `active &&` 去掉 → 空闲时状态条上还挂着一串上一次会话的延迟/码率。
    strip({ streamingCount: 0 });

    expect(screen.getByText(t("status.idle"))).toBeTruthy();
    expect(
      screen.queryByText(
        t("status.summary", {
          e2e: usToMs(telemetry.e2eLatencyUs),
          rate: bpsToKbps(telemetry.bitrateBps),
          loss: telemetry.lossPct.toFixed(2),
        }),
      ),
    ).toBeNull();
  });

  it("推流中显示台数，并把 µs/bps 换算后写进汇总", () => {
    strip({ streamingCount: 2 });

    expect(screen.getByText(t("status.streaming", { count: 2 }))).toBeTruthy();
    expect(
      screen.getByText(
        t("status.summary", {
          e2e: usToMs(telemetry.e2eLatencyUs),
          rate: bpsToKbps(telemetry.bitrateBps),
          loss: telemetry.lossPct.toFixed(2),
        }),
      ),
    ).toBeTruthy();
  });

  it("本机身份还没水合时给占位符，不把 undefined 印到状态条上", () => {
    strip({ local: null });

    expect(screen.getByText("fp:--------")).toBeTruthy();
    expect(screen.queryByText(/undefined/)).toBeNull();
  });

  it("遥测开关的可访问状态跟着展开状态走（箭头与 aria-expanded 同步）", () => {
    strip({ telemetryOpen: false });

    const toggle = screen.getByRole("button", { name: t("status.telemetry", { arrow: "▸" }) });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(toggle.getAttribute("aria-controls")).toBe("telemetry-panel");
  });

  it("点开关把动作交给上层（本组件不自己管展开状态）", () => {
    const { onToggleTelemetry } = strip({});

    fireEvent.click(screen.getByRole("button", { name: t("status.telemetry", { arrow: "▾" }) }));
    expect(onToggleTelemetry).toHaveBeenCalledTimes(1);
  });
});
