/**
 * 设置面板的「已配对设备」区（FR-18 的读侧）。
 *
 * 这一区的存在理由很具体：**没有会话的已配对设备**（换机后残留的旧记录、很久没连过的设备）
 * 在界面上没有对端卡片，只能从**信任库**读出来 —— 所以每条用例都在钉
 * 「它们能被看见、能被移除、移除后列表立刻更新」。
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
    listTrustedPeers: vi.fn(),
    revokeTrust: vi.fn(),
  },
}));

import { api } from "../lib/ipc";
import { t } from "../i18n";
import type { TrustedPeerView } from "../types";
import { SettingsPanel } from "./SettingsPanel";

const autostartEnabled = vi.mocked(api.autostartEnabled);
const autoConnectState = vi.mocked(api.autoConnectState);
const listTrustedPeers = vi.mocked(api.listTrustedPeers);
const revokeTrust = vi.mocked(api.revokeTrust);

function trusted(idShort: string, name: string): TrustedPeerView {
  return { idShort, name, platform: "windows", pairedAtUnix: 1_700_000_000 };
}

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
  listTrustedPeers.mockResolvedValue([trusted("aaaa1111", "旧手机")]);
  revokeTrust.mockResolvedValue({ removed: true, forgotLastPeer: false });
});

describe("已配对设备（信任库读侧）", () => {
  it("列出没有会话的已配对设备（名字 + 指纹短码）", async () => {
    panel();

    expect(await screen.findByText("旧手机")).toBeTruthy();
    expect(screen.getByText("aaaa1111")).toBeTruthy();
  });

  it("空列表给空态文案，而不是永远停在「读取中」", async () => {
    listTrustedPeers.mockResolvedValue([]);
    panel();

    expect(await screen.findByText(t("set.trusted_empty"))).toBeTruthy();
  });

  it("点「移除」只进确认态：二次确认之前**不**调用撤销", async () => {
    // 改坏：把 revokeTrust 直接挂在第一个按钮上 → 一次误点就断掉一台设备的信任，
    // 且它下次连接必须重新配对（不可逆）。
    panel();

    fireEvent.click(await screen.findByRole("button", { name: t("set.trusted_revoke") }));

    expect(revokeTrust).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: t("set.trusted_confirm") })).toBeTruthy();
  });

  it("确认后按**指纹短码**撤销，并立刻重拉列表（条目当场消失）", async () => {
    // 短码传错 = 移除另一台设备的信任；不重拉 = 列表留着已经删掉的条目（用户以为没生效）。
    panel();
    fireEvent.click(await screen.findByRole("button", { name: t("set.trusted_revoke") }));
    // 撤销成功后信任库少一条：第二次拉取返回空列表。
    listTrustedPeers.mockResolvedValue([]);
    fireEvent.click(screen.getByRole("button", { name: t("set.trusted_confirm") }));

    await waitFor(() => expect(revokeTrust).toHaveBeenCalledWith("aaaa1111"));
    expect(await screen.findByText(t("set.trusted_empty"))).toBeTruthy();
    expect(screen.queryByText("旧手机")).toBeNull();
  });

  it("取消后收起确认，且不发起撤销", async () => {
    panel();
    fireEvent.click(await screen.findByRole("button", { name: t("set.trusted_revoke") }));
    fireEvent.click(screen.getByRole("button", { name: t("set.trusted_cancel") }));

    expect(revokeTrust).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: t("set.trusted_revoke") })).toBeTruthy();
  });
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
