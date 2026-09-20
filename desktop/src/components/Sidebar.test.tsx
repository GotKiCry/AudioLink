/**
 * 侧边栏（`Sidebar`）—— 主按钮是全屏唯一的「开始 / 停止推流」入口。
 *
 * 这一组钉的是**准入判断**，不是"渲染不崩"：
 *   无候选 → 禁用 + 说清怎么办（禁用的按钮自己解释不了自己）；
 *   有候选未发起 → 开始；
 *   已发起（哪怕一台设备都还没接上、哪怕 busy）→ 停止，且**永远可点**。
 * 最后那条是这个文件存在的理由：按钮语义一旦由 streamingCount 推导，
 * 没有设备在听时用户就永远找不到「停止」。
 */
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { t } from "../i18n";
import { Sidebar } from "./Sidebar";

type SidebarProps = Parameters<typeof Sidebar>[0];

function sidebar(over: Partial<SidebarProps> = {}) {
  const handlers = {
    onSelectCapture: vi.fn(),
    onRefreshCapture: vi.fn(async () => undefined),
    onBroadcast: vi.fn(),
    onStopBroadcast: vi.fn(),
  };
  const props: SidebarProps = {
    version: "0.1.0",
    local: { idShort: "3f9a1c0b", name: "本机", addr: "192.168.1.10:58290", platform: "windows" },
    onAir: false,
    waitingForDevice: false,
    streamingCount: 0,
    broadcastTargets: 0,
    busy: false,
    captureDevices: [],
    selectedCaptureId: "",
    activeCapture: null,
    captureLoading: false,
    captureLocked: false,
    captureError: null,
    ...handlers,
    ...over,
  };
  const { container } = render(<Sidebar {...props} />);
  return { ...handlers, container };
}

describe("主按钮三态", () => {
  it("一台候选设备都没有、也没推流 → 禁用 + 说清怎么办", () => {
    // 改坏：去掉 blocked → 用户点得动，但列表为空、什么也不会发生（原来的"纹丝不动"）。
    const handlers = sidebar({ broadcastTargets: 0 });

    const button = screen.getByRole("button", { name: t("side.broadcast_blocked") }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(screen.getByText(t("side.broadcast_blocked_hint"))).toBeTruthy();

    fireEvent.click(button);
    expect(handlers.onBroadcast).not.toHaveBeenCalled();
  });

  it("有候选、还没发起 → 「开始推流」可点，并把动作交给上层", () => {
    const handlers = sidebar({ broadcastTargets: 2 });

    const button = screen.getByRole("button", { name: t("side.broadcast") }) as HTMLButtonElement;
    expect(button.disabled).toBe(false);
    expect(screen.queryByText(t("side.broadcast_blocked_hint"))).toBeNull();

    fireEvent.click(button);
    expect(handlers.onBroadcast).toHaveBeenCalledTimes(1);
    expect(handlers.onStopBroadcast).not.toHaveBeenCalled();
  });

  it("已发起（还没有设备接上、且 busy）→ 「停止推流」仍然可点 —— 停止入口必须常在", () => {
    // 改坏：disabled 里带上 busy，或按 streamingCount 推导 → 握手期间/没有设备时按钮被禁用，用户停不下来。
    const handlers = sidebar({
      onAir: true,
      waitingForDevice: true,
      broadcastTargets: 0,
      streamingCount: 0,
      busy: true,
    });

    const button = screen.getByRole("button", { name: t("side.stop_broadcast") }) as HTMLButtonElement;
    expect(button.disabled).toBe(false);

    fireEvent.click(button);
    expect(handlers.onStopBroadcast).toHaveBeenCalledTimes(1);
    expect(handlers.onBroadcast).not.toHaveBeenCalled();
  });

  it("未发起时 busy 仍然禁用（避免连点两次开始）", () => {
    const handlers = sidebar({ broadcastTargets: 1, busy: true });

    const button = screen.getByRole("button", { name: t("side.broadcast") }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);

    fireEvent.click(button);
    expect(handlers.onBroadcast).not.toHaveBeenCalled();
  });
});

describe("身份区的状态行", () => {
  it("已发起但还没有设备接上 → 说「推流中 · 等待设备接入」，不装作没事", () => {
    const { container } = sidebar({ onAir: true, waitingForDevice: true, streamingCount: 0 });

    expect(screen.getByText(t("side.broadcasting_waiting"))).toBeTruthy();
    expect(screen.queryByText(t("console.off_air"))).toBeNull();
    expect(container.querySelector(".al-lamp")?.getAttribute("data-on")).toBe("warn");
  });

  it("有设备真的在听 → 报台数", () => {
    sidebar({ onAir: true, waitingForDevice: false, streamingCount: 2, broadcastTargets: 1 });

    expect(screen.getByText(t("console.channels", { count: 2 }))).toBeTruthy();
    expect(screen.queryByText(t("side.broadcasting_waiting"))).toBeNull();
    expect(screen.getByRole("button", { name: t("side.stop_broadcast") })).toBeTruthy();
  });

  it("没推流 → 「未推流」", () => {
    sidebar({});

    expect(screen.getByText(t("console.off_air"))).toBeTruthy();
  });

  it("指纹行已从侧栏移除（开发信息不占首屏）", () => {
    const { container } = sidebar({});

    expect(container.textContent ?? "").not.toContain("fp:");
  });
});
