/**
 * 接收端入口（`ConnectToHost`）—— 「去听另一台设备」这一格。
 *
 * 钉住的是**方向**，不是"渲染不崩"：主机永不主动请求连接，所以屏幕上"去连别人"只能以
 * 接收端角色出现（标题 + 角色标签 + 代价说明），而且不能长得和「本机地址」卡一样平级 ——
 * 一旦它看起来像第二个主行动，用户就会以为 PC 也得去连手机，然后去填一个不该由他填的地址。
 */
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { t } from "../i18n";
import { ConnectToHost } from "./ConnectToHost";

vi.mock("./DiscoveredHosts", () => ({ DiscoveredHosts: () => null }));

function entry(over: { connecting?: boolean } = {}) {
  const onConnect = vi.fn(async () => true);
  const { container } = render(
    <ConnectToHost connecting={over.connecting ?? false} onConnect={onConnect} />,
  );
  return { onConnect, container };
}

describe("接收端入口：本机切换成接收端，而不是「添加设备」", () => {
  it("标题、角色标签、代价说明都在：一眼看出这是角色切换", () => {
    entry();

    expect(screen.getByRole("heading", { name: t("connect.manual") })).toBeTruthy();
    expect(screen.getByText(t("connect.role"))).toBeTruthy();
    expect(screen.getByText(t("connect.hint"))).toBeTruthy();
  });

  it("旧措辞「手动添加设备」不再出现", () => {
    // 改坏：标题换回 manual.title → 界面又说回"主机去添加设备"那个错误方向。
    entry();

    // 用字面量而不是 t()：那套"主机视角的手动添加设备"文案已从 i18n 表整体删除，
    // 取不到键就断言不了；钉在界面文本上，才真的防得住有人把这套说法写回来。
    expect(screen.queryByText(/手动添加/)).toBeNull();
  });

  it("空输入通过 Enter 提交也不会发出连接请求", () => {
    const { onConnect } = entry();
    fireEvent.submit(screen.getByRole("form", { name: t("connect.manual") }));
    expect(onConnect).not.toHaveBeenCalled();
  });

  it("输入框的标签就是「主机地址」：方向写在明面上，不藏进 sr-only", () => {
    entry();

    const input = screen.getByLabelText(t("connect.address")) as HTMLInputElement;
    expect(input.placeholder).toBe(t("manual.placeholder"));
  });

  it("空输入时按钮禁用，且悬停能解释为什么点不动", () => {
    entry();

    const button = screen.getByRole("button") as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.title).toBe(t("manual.address_hint"));
  });

  it("填了地址就能提交，并把地址原样交给上层", async () => {
    const { onConnect } = entry();

    fireEvent.change(screen.getByLabelText(t("connect.address")), {
      target: { value: "192.168.1.23:58290" },
    });
    fireEvent.click(screen.getByRole("button", { name: t("connect.submit") }));

    await waitFor(() => expect(onConnect).toHaveBeenCalledWith("192.168.1.23:58290"));
  });

  it("格式不对只提示、不拦提交：能不能连由内核回答", () => {
    // 前端判据比内核窄，拿它禁用提交会把用户锁在一个更严的规则之外（types.isPeerAddrWellFormed 的取舍）。
    entry();
    const input = screen.getByLabelText(t("connect.address")) as HTMLInputElement;

    fireEvent.change(input, { target: { value: "192.168.1.23:abc" } });

    expect(input.getAttribute("aria-invalid")).toBe("true");
    expect(screen.getByText(t("manual.address_hint"))).toBeTruthy();
    expect((screen.getByRole("button", { name: t("connect.submit") }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("连接中：按钮改成「连接中…」并禁用（避免连点两次）", () => {
    entry({ connecting: true });

    const button = screen.getByRole("button", { name: t("manual.connecting") }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(screen.queryByRole("button", { name: t("connect.submit") })).toBeNull();
  });
});
