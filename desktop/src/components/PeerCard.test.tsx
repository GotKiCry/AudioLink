/**
 * 对端卡片（`PeerCard`）—— 界面上**唯一**能发起/停止推流的地方。
 *
 * 这一组不是"渲染不崩"：每条都对应一个用户动作的准入判断。点错一次的代价是
 * 对着已经断线的设备点推流、或把接收端场景变成"点了没反应"的假按钮。
 */
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { PeerView } from "../types";
import { t } from "../i18n";
import { PeerCard } from "./PeerCard";

function peer(over: Partial<PeerView> = {}): PeerView {
  return {
    idShort: "3f9a1c0b",
    name: "客厅 R1",
    addr: "192.168.1.23:58290",
    state: "idle",
    capabilities: null,
    ...over,
  };
}

function card(over: Partial<PeerView> = {}, props: Partial<Parameters<typeof PeerCard>[0]> = {}) {
  const handlers = {
    onStart: vi.fn(async () => undefined),
    onStop: vi.fn(async () => undefined),
    onGain: vi.fn(),
    ...props,
  };
  render(
    <PeerCard
      peer={peer(over)}
      busy={props.busy === true}
      canStart={props.canStart !== false}
      {...handlers}
    />,
  );
  return handlers;
}

describe("质量读数的占位（缺失反馈不伪装成零）", () => {
  it("没有端到端测量时仍显示 RTT 和零丢包，缺失的反馈使用占位", () => {
    card({ receiving: true, quality: { rttUs: 12500, lossPctX100: 0, underruns: 3, bufferLevelUs: 80000 } });
    expect(screen.getByText("12.5 ms")).toBeTruthy();
    expect(screen.getByText("0.00%")).toBeTruthy();
    expect(screen.queryByText(t("quality.waiting"))).toBeNull();
  });

  it("接收端没有反馈时不能显示零丢包", () => {
    card({ quality: { rttUs: 18000, lossPctX100: null, underruns: null, bufferLevelUs: null } });
    expect(screen.getByText("18.0 ms")).toBeTruthy();
    expect(screen.queryByText("0.00%")).toBeNull();
    expect(screen.getByText(t("quality.waiting"))).toBeTruthy();
  });
});

describe("推流按钮的准入", () => {
  it("空闲 + 可用采集设备 → 「开始推流」可点", () => {
    card({ state: "idle" }, { canStart: true });

    const start = screen.getByRole("button", { name: t("peer.start") });
    expect((start as HTMLButtonElement).disabled).toBe(false);
  });

  it("本机没有可用采集设备（canStart=false）→ 禁用", () => {
    // 改坏：把 canStart 从 disabled 里去掉 → 用户点了必然失败（内核会拒），
    // 却要走一次 IPC 才看到失败原因。
    card({ state: "idle" }, { canStart: false });

    expect((screen.getByRole("button", { name: t("peer.start") }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("会话已断开（state=failed）→ 无论如何都禁用推流", () => {
    // 改坏：删掉 `peer.state === "failed"` → 能对着一台已经断线的设备点「开始推流」。
    card({ state: "failed" }, { canStart: true });

    expect((screen.getByRole("button", { name: t("peer.start") }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("降级中也算推流中：给「停止推流」和音量入口，而不是「开始推流」", () => {
    // 改坏：streaming 判定只写 state === "streaming" → 降级会话既停不掉也没有音量入口，
    // 用户唯一的办法是等它自己恢复。
    card({ state: "degraded" }, { canStart: true });

    expect(screen.getByRole("button", { name: t("peer.stop") })).toBeTruthy();
    expect(screen.queryByRole("button", { name: t("peer.start") })).toBeNull();
    expect(screen.getByRole("slider", { name: t("peer.volume_label") })).toBeTruthy();
  });

  it("正在 start/stop 时按钮禁用、文案变「启动中…」（防连点）", () => {
    card({ state: "idle" }, { canStart: true, busy: true });

    const starting = screen.getByRole("button", { name: t("peer.starting") });
    expect((starting as HTMLButtonElement).disabled).toBe(true);
  });

  it("音量滑块把 0–2 的增益交给上层（是数字，不是字符串）", () => {
    const handlers = card({ state: "streaming" }, { canStart: true });

    fireEvent.change(screen.getByRole("slider", { name: t("peer.volume_label") }), {
      target: { value: "0.5" },
    });
    expect(handlers.onGain).toHaveBeenCalledWith(0.5);
  });
});

describe("能力协商的展示（§13）", () => {
  const capabilities = {
    local: "Opus 编码、音频采集",
    peer: "Opus 编码、音频播放",
    agreed: "Opus 编码",
    missingOnPeer: ["音频采集"],
    agreedKeys: ["opus"],
    missingKeys: ["capture"],
  };

  it("对端缺能力时逐条列出来（缺什么一眼看得见）", () => {
    card({ capabilities });

    expect(
      screen.getAllByText(t("peer.caps_missing", { list: "音频采集" })).length,
    ).toBeGreaterThan(0);
  });

  it("对端什么都不缺时不显示那句缺能力的提示（好链路不该被标黄）", () => {
    card({ capabilities: { ...capabilities, missingOnPeer: [], missingKeys: [] } });

    expect(screen.queryByText(t("peer.caps_missing", { list: "" }))).toBeNull();
    expect(
      screen.getAllByText(t("peer.caps_agreed", { list: "Opus 编码" })).length,
    ).toBeGreaterThan(0);
  });
});

describe("协商帧长（帧长联动的直接读数）", () => {
  const quality = { rttUs: 12500, lossPctX100: 0, underruns: 0, bufferLevelUs: 40000 };

  it("协商为 10 ms 时显示「10 ms」（而不是本机档位）", () => {
    card({ receiving: true, quality, negotiatedFrameMs: 10 });

    expect(screen.getByText(t("diag.frame_ms"))).toBeTruthy();
    expect(screen.getByText("10 ms")).toBeTruthy();
  });

  it("协商为 20 ms 时显示「20 ms」", () => {
    card({ receiving: true, quality, negotiatedFrameMs: 20 });

    expect(screen.getByText("20 ms")).toBeTruthy();
  });

  it("还没协商（null）→ 显示占位符，绝不伪装成某个帧长", () => {
    // 改坏：把 null 兜底成 20 → 界面上永远是一个正常数字，
    // 而「帧长不一致」的症状正是「遥测全绿但声音发闷」，正好被这个假数字盖住。
    card({ receiving: true, quality, negotiatedFrameMs: null });

    expect(screen.getByText(t("diag.frame_ms"))).toBeTruthy();
    expect(screen.queryByText("10 ms")).toBeNull();
    expect(screen.queryByText("20 ms")).toBeNull();
    expect(screen.getByText("—")).toBeTruthy();
  });

  it("老内核不给这个字段（undefined）→ 同样显示占位符，不崩", () => {
    card({ receiving: true, quality });

    expect(screen.getByText("—")).toBeTruthy();
  });
});
