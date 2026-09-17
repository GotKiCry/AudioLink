/**
 * 对端卡片（`PeerCard`）—— 界面上**唯一**能发起/停止推流与配对的地方。
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
    trusted: false,
    capabilities: null,
    ...over,
  };
}

function card(over: Partial<PeerView> = {}, props: Partial<Parameters<typeof PeerCard>[0]> = {}) {
  const handlers = {
    onStart: vi.fn(async () => undefined),
    onStop: vi.fn(async () => undefined),
    onBeginPair: vi.fn(),
    onGain: vi.fn(),
    ...props,
  };
  render(
    <PeerCard
      peer={peer(over)}
      busy={props.busy === true}
      canStart={props.canStart !== false}
      canInputPin={props.canInputPin === true}
      {...handlers}
    />,
  );
  return handlers;
}

describe("配对入口（两个方向的按钮必须不同）", () => {
  it("未受信 + 不是本机该输码（接收端场景）→ 只有一个禁用的「等待配对」，没有输入入口", () => {
    // 改坏：忽略 canInputPin → 接收端也会出现可点的「输入配对码」，
    // 用户点进去只会看到一个自己屏幕上已有的码（点了没反应）。
    card({ trusted: false }, { canInputPin: false });

    const waiting = screen.getByRole("button", { name: t("peer.waiting") });
    expect((waiting as HTMLButtonElement).disabled).toBe(true);
    expect(screen.queryByRole("button", { name: t("peer.pin_entry") })).toBeNull();
  });

  it("未受信 + 本机该输码 → 「输入配对码」可点，并把**这台**对端的短码交上去", () => {
    // 多台对端可能同时在配对：idShort 传错 = 把码提交到另一条会话上。
    const handlers = card({ trusted: false, idShort: "aaaa1111" }, { canInputPin: true });

    fireEvent.click(screen.getByRole("button", { name: t("peer.pin_entry") }));
    expect(handlers.onBeginPair).toHaveBeenCalledWith("aaaa1111");
  });
});

describe("推流按钮的准入", () => {
  it("已受信 + 空闲 + 可用采集设备 → 「开始推流」可点", () => {
    card({ trusted: true, state: "idle" }, { canStart: true });

    const start = screen.getByRole("button", { name: t("peer.start") });
    expect((start as HTMLButtonElement).disabled).toBe(false);
  });

  it("本机没有可用采集设备（canStart=false）→ 禁用", () => {
    // 改坏：把 canStart 从 disabled 里去掉 → 用户点了必然失败（内核会拒），
    // 却要走一次 IPC 才看到失败原因。
    card({ trusted: true, state: "idle" }, { canStart: false });

    expect((screen.getByRole("button", { name: t("peer.start") }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("会话已断开（state=failed）→ 无论如何都禁用推流", () => {
    // 改坏：删掉 `peer.state === "failed"` → 能对着一台已经断线的设备点「开始推流」。
    card({ trusted: true, state: "failed" }, { canStart: true });

    expect((screen.getByRole("button", { name: t("peer.start") }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("降级中也算推流中：给「停止推流」和音量入口，而不是「开始推流」", () => {
    // 改坏：streaming 判定只写 state === "streaming" → 降级会话既停不掉也没有音量入口，
    // 用户唯一的办法是等它自己恢复。
    card({ trusted: true, state: "degraded" }, { canStart: true });

    expect(screen.getByRole("button", { name: t("peer.stop") })).toBeTruthy();
    expect(screen.queryByRole("button", { name: t("peer.start") })).toBeNull();
    expect(screen.getByRole("slider", { name: t("peer.volume_label") })).toBeTruthy();
  });

  it("正在 start/stop 时按钮禁用、文案变「启动中…」（防连点）", () => {
    card({ trusted: true, state: "idle" }, { canStart: true, busy: true });

    const starting = screen.getByRole("button", { name: t("peer.starting") });
    expect((starting as HTMLButtonElement).disabled).toBe(true);
  });

  it("音量滑块把 0–2 的增益交给上层（是数字，不是字符串）", () => {
    const handlers = card({ trusted: true, state: "streaming" }, { canStart: true });

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
    card({ trusted: true, capabilities });

    expect(
      screen.getAllByText(t("peer.caps_missing", { list: "音频采集" })).length,
    ).toBeGreaterThan(0);
  });

  it("对端什么都不缺时不显示那句缺能力的提示（好链路不该被标黄）", () => {
    card({ trusted: true, capabilities: { ...capabilities, missingOnPeer: [], missingKeys: [] } });

    expect(screen.queryByText(t("peer.caps_missing", { list: "" }))).toBeNull();
    expect(
      screen.getAllByText(t("peer.caps_agreed", { list: "Opus 编码" })).length,
    ).toBeGreaterThan(0);
  });
});
