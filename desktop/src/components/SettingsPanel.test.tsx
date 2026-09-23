/**
 * 设置面板（语言 / 开机自启 / 启动时自动连接 / 接入即自动推流 / 低延迟档）。
 *
 * mock 的是组件唯一的对外通道 `../lib/ipc`，断言的是**用户看到什么**，而不是某次调用发生没发生。
 */
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/ipc", () => ({
  api: {
    autostartEnabled: vi.fn(),
    autoConnectState: vi.fn(),
    setAutostart: vi.fn(),
    setAutoConnect: vi.fn(),
    setLocale: vi.fn(),
    lowLatencyState: vi.fn(),
    setLowLatency: vi.fn(),
  },
}));

import { api } from "../lib/ipc";
import { t } from "../i18n";
import { SettingsPanel } from "./SettingsPanel";

const autostartEnabled = vi.mocked(api.autostartEnabled);
const autoConnectState = vi.mocked(api.autoConnectState);
const lowLatencyState = vi.mocked(api.lowLatencyState);
const setLowLatency = vi.mocked(api.setLowLatency);

/**
 * 渲染面板。
 *
 * 自动推流的**状态与执行**都在 useAudioLink（面板只做开关），所以这里给的是受控 props ——
 * 面板自己不该再记一份"开没开"，否则关掉开关之后自动推流还在跑，是最难查的那种 bug。
 */
function panel(over: Partial<Parameters<typeof SettingsPanel>[0]> = {}) {
  const onToggleAutoBroadcast = vi.fn();
  render(
    <SettingsPanel
      autoBroadcast={over.autoBroadcast ?? true}
      autoBroadcastBusy={over.autoBroadcastBusy ?? false}
      onToggleAutoBroadcast={over.onToggleAutoBroadcast ?? onToggleAutoBroadcast}
    />,
  );
  return { onToggleAutoBroadcast };
}

beforeEach(() => {
  vi.clearAllMocks();
  autostartEnabled.mockResolvedValue(true);
  autoConnectState.mockResolvedValue({ enabled: false, lastPeer: null });
  lowLatencyState.mockResolvedValue(false);
  setLowLatency.mockResolvedValue(null);
});

describe("接入即自动推流（开关）", () => {
  it("默认开着，并把新值交给上层（面板不自己改行为）", async () => {
    const { onToggleAutoBroadcast } = panel({ autoBroadcast: true });

    const toggle = (await screen.findByRole("checkbox", {
      name: t("set.auto_broadcast"),
    })) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
    expect(screen.getByText(t("set.auto_broadcast_hint"))).toBeTruthy();

    fireEvent.click(toggle);
    expect(onToggleAutoBroadcast).toHaveBeenCalledWith(false);
  });

  it("落盘中禁用开关（避免连点造成两次写盘）", async () => {
    panel({ autoBroadcastBusy: true });

    const toggle = (await screen.findByRole("checkbox", {
      name: t("set.auto_broadcast"),
    })) as HTMLInputElement;
    expect(toggle.disabled).toBe(true);
  });
});

describe("低延迟档（M6）", () => {
  it("默认关：从设置读到的 true 才点亮开关", async () => {
    lowLatencyState.mockResolvedValue(true);
    panel();

    const toggle = (await screen.findByRole("checkbox", {
      name: t("set.low_latency"),
    })) as HTMLInputElement;
    await waitFor(() => expect(toggle.checked).toBe(true));
    // 这句提示是契约的一部分，而且语义已经变了：本开关只管**发送方向**的帧长，
    // 接收端会自动跟随对端 —— 缺了它用户仍会以为「两端必须手动调成同档」。
    // 所以这里不仅盯「文案在不在」，还盯旧的「两端需同时开启」措辞确实被换掉了。
    const hint = t("set.low_latency_hint");
    expect(screen.getByText(hint)).toBeTruthy();
    expect(hint).not.toContain("两端设备需同时开启");
  });

  it("切换时把新值落盘（写 set_low_latency，不是前端自记）", async () => {
    panel();

    const toggle = (await screen.findByRole("checkbox", {
      name: t("set.low_latency"),
    })) as HTMLInputElement;
    await waitFor(() => expect(toggle.disabled).toBe(false));
    fireEvent.click(toggle);

    await waitFor(() => expect(setLowLatency).toHaveBeenCalledWith(true));
  });

  it("设置读不出来时开关停在禁用态，而不是假装「关」", async () => {
    // 假「关」会让用户以为标准档正在生效；禁用 + 读取中才是如实的状态。
    lowLatencyState.mockRejectedValue(new Error("boom"));
    panel();

    const toggle = (await screen.findByRole("checkbox", {
      name: t("set.low_latency"),
    })) as HTMLInputElement;
    expect(toggle.disabled).toBe(true);
  });
});
