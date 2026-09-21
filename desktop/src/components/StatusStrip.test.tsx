import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { TelemetryView } from "../types";
import { bpsToKbps, usToMs } from "../types";
import { t } from "../i18n";
import { StatusStrip } from "./StatusStrip";

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

function strip(
  over: { streamingCount?: number; telemetryOpen?: boolean } = {},
) {
  const onToggleTelemetry = vi.fn();
  const streamingCount = over.streamingCount ?? 0;
  const { container } = render(
    <StatusStrip
      status={streamingCount > 0 ? "streaming" : "waiting"}
      telemetry={telemetry}
      streamingCount={streamingCount}
      telemetryOpen={over.telemetryOpen ?? true}
      onToggleTelemetry={onToggleTelemetry}
    />,
  );
  return { onToggleTelemetry, container };
}

/** 汇总行的期望文案：换算函数与组件共用，避免测试自己抄一遍公式。 */
const summary = t("status.summary", {
  e2e: usToMs(telemetry.e2eLatencyUs),
  rate: bpsToKbps(telemetry.bitrateBps),
  loss: telemetry.lossPct.toFixed(2),
});

describe("状态汇总", () => {
  it("没有推流时只说「等待设备连接」，不显示任何汇总数字", () => {
    // 改坏：把 `streaming &&` 去掉 → 空闲时状态条上还挂着一串上一次会话的延迟/码率。
    strip({ streamingCount: 0 });

    expect(screen.getByText(t("share.waiting"))).toBeTruthy();
    expect(screen.queryByText(summary)).toBeNull();
  });

  it("推流中显示台数，并把 µs/bps 换算后写进汇总", () => {
    strip({ streamingCount: 2 });

    expect(screen.getByText(t("status.streaming", { count: 2 }))).toBeTruthy();
    expect(screen.getByText(summary)).toBeTruthy();
  });

  it("没有设备接入时显示等待，且不把上一次的汇总当当前值报出来", () => {
    // 改坏：只认 streamingCount → 用户点了开始，状态条冷冷地说「未推流」，看起来像没反应。
    const { container } = strip({ streamingCount: 0 });

    expect(screen.getByText(t("side.broadcasting_waiting"))).toBeTruthy();
    expect(screen.queryByText(summary)).toBeNull();
    // 等待为中性色，不能显示正在发送的绿灯。
    expect(container.querySelector(".al-lamp")?.getAttribute("data-on")).toBe("off");
  });

  it("指纹与监听地址不再出现在状态条上", () => {
    // 改坏：把 fp/addr 那段加回来 → 首屏又多了两串只有开发者看得懂的东西。
    const { container } = strip({ streamingCount: 1 });
    const text = container.textContent ?? "";

    expect(text).not.toContain("fp:");
    expect(text).not.toContain("0.0.0.0");
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
