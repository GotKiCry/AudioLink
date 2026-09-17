/**
 * 采集源面板（`CaptureSourcePanel`）—— 「问题提示」的**优先级链**。
 *
 * 这块面板的全部价值就在于"当下到底是哪一种毛病"：命令失败 / 真没设备 / 所选设备掉了 /
 * 格式不支持。提示写错一条，用户就会去修一个不存在的问题（这在音频工具里代价很具体：
 * 他会去拔插耳机、重启播放器）。
 */
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { CaptureDeviceView, CommandError } from "../types";
import { t } from "../i18n";
import { CaptureSourcePanel } from "./CaptureSourcePanel";

function device(over: Partial<CaptureDeviceView> = {}): CaptureDeviceView {
  return {
    id: "dev-1",
    name: "扬声器 (Realtek)",
    isDefault: true,
    isVirtual: false,
    sampleRate: 48000,
    channels: 2,
    unavailableReason: null,
    ...over,
  };
}

function panel(over: {
  devices?: CaptureDeviceView[];
  selectedId?: string;
  active?: CaptureDeviceView | null;
  loading?: boolean;
  locked?: boolean;
  error?: CommandError | null;
} = {}) {
  render(
    <CaptureSourcePanel
      devices={over.devices ?? []}
      selectedId={over.selectedId ?? ""}
      active={over.active ?? null}
      loading={over.loading ?? false}
      locked={over.locked ?? false}
      error={over.error ?? null}
      onSelect={vi.fn()}
      onRefresh={vi.fn(async () => undefined)}
    />,
  );
  return {
    select: document.querySelector("select") as HTMLSelectElement,
  };
}

describe("问题提示的优先级", () => {
  it("读取中且还没有设备时不报「未找到输出设备」（启动瞬间不能闪一条假错误）", () => {
    // 改坏：去掉 `!loading` 条件 → 界面每次启动都先喊一句"没找到设备"，再自我更正。
    panel({ loading: true, devices: [] });

    expect(screen.queryByText(t("cap.no_device"))).toBeNull();
  });

  it("读取完成且确实没有设备 → 报「未找到输出设备」并禁用选择器", () => {
    const { select } = panel({ loading: false, devices: [] });

    expect(screen.getByText(t("cap.no_device"))).toBeTruthy();
    expect(select.disabled).toBe(true);
  });

  it("命令失败时显示后端给的人话原因，而不是「没有设备」", () => {
    // 改坏：把 error 分支的顺序放到 noDevices 之后 → 一次 IPC 失败会被说成"你没插设备"。
    panel({
      error: { code: 1009, message: "读取输出设备失败：设备忙", context: "list_capture_devices" },
      devices: [],
    });

    expect(screen.getByText("读取输出设备失败：设备忙")).toBeTruthy();
    expect(screen.queryByText(t("cap.no_device"))).toBeNull();
  });

  it("所选设备已从列表消失 → 报「所选设备已断开」，并在下拉里保留一个占位项", () => {
    // 占位项的意义：不保留的话 select 会静默回落到"系统默认"，用户以为选择生效了。
    panel({ devices: [device()], selectedId: "gone-device" });

    expect(screen.getByText(t("cap.device_lost"))).toBeTruthy();
    expect(screen.getByRole("option", { name: t("cap.unavailable") })).toBeTruthy();
  });

  it("所选设备格式不支持 → 显示设备自己给出的原因", () => {
    const broken = device({ id: "dev-2", unavailableReason: "该设备是 8 kHz 单声道，需要调整格式" });
    panel({ devices: [broken], selectedId: "dev-2" });

    expect(screen.getByText("该设备是 8 kHz 单声道，需要调整格式")).toBeTruthy();
  });
});

describe("推流中的锁定（不打扰 vs 错误必须可见）", () => {
  it("锁定时不显示「设备不可用」这类可自我修复的提示（推流中不该催用户去动设备）", () => {
    panel({
      locked: true,
      devices: [device({ id: "dev-2", unavailableReason: "格式不支持" })],
      selectedId: "dev-2",
    });

    expect(screen.queryByText("格式不支持")).toBeNull();
    expect(screen.getByText(t("cap.locked_hint"))).toBeTruthy();
  });

  it("锁定中但命令**失败**时，失败原因仍然要显示（错误例外）", () => {
    panel({
      locked: true,
      error: { code: 1009, message: "读取输出设备失败：设备忙", context: "list_capture_devices" },
    });

    expect(screen.getByText("读取输出设备失败：设备忙")).toBeTruthy();
  });
});
