/**
 * 本机状态条（`StatusStrip`）—— 用户第一眼看的那一行。
 *
 * 钉三件容易糊掉的事：① 没在推流时**不**显示汇总数字（拿上一次遥测的旧值当"当前"很骗人）；
 * ② 点了开始但还没有设备接入时，与侧栏说同一句话 —— 一边"推流中"一边"未推流"比不说更糟；
 * ③ 指纹与监听地址不上这条带子（开发信息不占首屏，地址只在地址卡片里给一次）。
 */
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
  over: { onAir?: boolean; waitingForDevice?: boolean; streamingCount?: number; telemetryOpen?: boolean } = {},
) {
  const onToggleTelemetry = vi.fn();
  const streamingCount = over.streamingCount ?? 0;
  const { container } = render(
    <StatusStrip
      onAir={over.onAir ?? streamingCount > 0}
      waitingForDevice={over.waitingForDevice ?? false}
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
  it("没有推流时只说「未推流」，不显示任何汇总数字", () => {
    // 改坏：把 `streaming &&` 去掉 → 空闲时状态条上还挂着一串上一次会话的延迟/码率。
    strip({ streamingCount: 0 });

    expect(screen.getByText(t("status.idle"))).toBeTruthy();
    expect(screen.queryByText(summary)).toBeNull();
  });

  it("推流中显示台数，并把 µs/bps 换算后写进汇总", () => {
    strip({ streamingCount: 2 });

    expect(screen.getByText(t("status.streaming", { count: 2 }))).toBeTruthy();
    expect(screen.getByText(summary)).toBeTruthy();
  });

  it("已发起但还没有设备接入：说「推流中 · 等待设备接入」，且不把上一次的汇总当当前值报出来", () => {
    // 改坏：只认 streamingCount → 用户点了开始，状态条冷冷地说「未推流」，看起来像没反应。
    const { container } = strip({ onAir: true, waitingForDevice: true, streamingCount: 0 });

    expect(screen.getByText(t("side.broadcasting_waiting"))).toBeTruthy();
    expect(screen.queryByText(t("status.idle"))).toBeNull();
    expect(screen.queryByText(summary)).toBeNull();
    // 黄灯 = 已开闸但还没有人接；红灯留给"真的有人在听"。
    expect(container.querySelector(".al-lamp")?.getAttribute("data-on")).toBe("warn");
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
