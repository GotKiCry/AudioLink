/**
 * 本机地址卡（HostAddressCard）—— 「对方照着输的那串字」。
 *
 * 为什么三条分支都要钉住：这张卡是整个接入流程里唯一把地址交到用户手上的地方。
 * 印错一个字符，用户就会在手机上照着敲、连不上，然后开始怀疑 Wi-Fi 与防火墙 ——
 * 而界面这边永远看不出问题。它的历史版本印的恰好是**监听地址**（0.0.0.0:58290），
 * 那是一个「在任何设备上都指向那台设备自己」的地址，照着输必然失败。
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { LocalStatus } from "../types";
import { t } from "../i18n";
import { HostAddressCard } from "./HostAddressCard";

/** 夹具里的 addr 固定是监听地址：这正是「卡片不许显示它」要钉的那个值。 */
function local(over: Partial<LocalStatus> = {}): LocalStatus {
  return {
    idShort: "a1b2c3",
    name: "Studio-PC",
    addr: "0.0.0.0:58290",
    platform: "windows",
    ...over,
  };
}

function copyButton(): HTMLButtonElement {
  return screen.getByRole("button") as HTMLButtonElement;
}

describe("本机地址卡：只给真能连上的地址，给不出来就说清楚", () => {
  it("有地址：大字显示 displayAddr，配「输这个」的话，复制可用", () => {
    render(
      <HostAddressCard
        local={local({ displayAddr: "192.168.1.23:58290", lanAddrs: ["192.168.1.23:58290"] })}
      />,
    );

    expect(screen.getByText("192.168.1.23:58290")).toBeTruthy();
    expect(screen.getByText(t("host.role_hint"))).toBeTruthy();
    expect(screen.getByText(t("host.hint"))).toBeTruthy();
    expect(copyButton().disabled).toBe(false);
    // 有地址时不许同时出现「没检测到」——两句话同屏，用户不知道该信哪一句
    expect(screen.queryByText(t("host.addr_missing"))).toBeNull();
    expect(document.body.textContent).not.toContain("0.0.0.0");
  });

  it("一条都没有：改成「没检测到」+ 该去查什么，复制按钮禁用，且绝不印监听地址", () => {
    render(<HostAddressCard local={local()} />);

    expect(screen.getByText(t("host.addr_missing"))).toBeTruthy();
    expect(screen.getByText(t("host.hint_missing"))).toBeTruthy();
    // 不能一边说没地址、一边挂着「在手机上输入这个地址即可接入」
    expect(screen.queryByText(t("host.hint"))).toBeNull();
    expect(copyButton().disabled).toBe(true);

    const text = document.body.textContent ?? "";
    expect(text).not.toContain("0.0.0.0");
    expect(text).not.toContain("58290");
  });

  it("有多条：补一句「逐个试一次」，主显示不变", () => {
    render(
      <HostAddressCard
        local={{
          ...local(),
          displayAddr: "192.168.1.23:58290",
          lanAddrs: ["192.168.1.23:58290", "10.0.0.5:58290"],
        }}
      />,
    );

    expect(screen.getByText(t("host.multi_hint"))).toBeTruthy();
    expect(screen.getByText("192.168.1.23:58290")).toBeTruthy();
    expect(copyButton().disabled).toBe(false);
  });

  it("只有一条、或者一条都没有时不提「多条」（没有可试的对象时那句话是空话）", () => {
    const one = render(
      <HostAddressCard
        local={local({ displayAddr: "192.168.1.23:58290", lanAddrs: ["192.168.1.23:58290"] })}
      />,
    );
    expect(screen.queryByText(t("host.multi_hint"))).toBeNull();
    one.unmount();

    render(<HostAddressCard local={local({ lanAddrs: ["192.168.1.23:58290", "10.0.0.5:58290"] })} />);
    expect(screen.queryByText(t("host.multi_hint"))).toBeNull();
    expect(screen.getByText(t("host.addr_missing"))).toBeTruthy();
  });

  it.each([
    ["字段缺失（后端还没上线）", undefined],
    ["明确给 null", null],
    ["空串", ""],
    ["只有空白", "   "],
    ["拿监听地址兜底", "0.0.0.0:58290"],
    ["回环地址", "127.0.0.1:58290"],
  ])("displayAddr 是「%s」时一律按没检测到处理", (_label, value) => {
    render(<HostAddressCard local={local({ displayAddr: value })} />);

    expect(screen.getByText(t("host.addr_missing"))).toBeTruthy();
    expect(copyButton().disabled).toBe(true);
    expect(document.body.textContent).not.toContain("0.0.0.0");
  });

  it("local 还没读到（null）：也是「没检测到」，不许印空串", () => {
    render(<HostAddressCard local={null} />);

    expect(screen.getByText(t("host.addr_missing"))).toBeTruthy();
    expect(copyButton().disabled).toBe(true);
  });

  it("复制成功给一次「已复制」反馈", async () => {
    const writeText = vi.fn(async () => undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });

    render(<HostAddressCard local={local({ displayAddr: "192.168.1.23:58290" })} />);
    fireEvent.click(copyButton());

    await waitFor(() => {
      expect(copyButton().textContent).toContain(t("host.copied"));
    });
    expect(writeText).toHaveBeenCalledWith("192.168.1.23:58290");
  });

  it("规则句与有没有地址无关；卡上没有任何输入控件（主机不向对方请求连接）", () => {
    // 这条守的是方向：主机是被连的一方，"去连别人"只能发生在 ConnectToHost 那个角色动作里。
    // 改坏：在地址卡上加一个"对方地址"输入框 → 屏幕立刻又变成"PC 也得去连手机"。
    const addressView = render(
      <HostAddressCard local={local({ displayAddr: "192.168.1.23:58290" })} />,
    );
    expect(screen.getByText(t("host.role_hint"))).toBeTruthy();
    expect(addressView.container.querySelector("input")).toBeNull();
    expect(addressView.container.querySelector("form")).toBeNull();
    addressView.unmount();

    render(<HostAddressCard local={local()} />);
    expect(screen.getByText(t("host.role_hint"))).toBeTruthy();
  });
});
