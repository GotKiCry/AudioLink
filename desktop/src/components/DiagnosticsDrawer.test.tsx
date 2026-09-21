/**
 * 诊断抽屉（`DiagnosticsDrawer`）—— 背板的第一格是**本机身份**。
 *
 * 这一组同时钉两个相反方向的事：
 * ① 抽屉没打开时整块不渲染 —— 指纹与监听地址不上第一屏（去噪的成果要保住）；
 * ② 打开后机器名 / 指纹 / 监听地址 / 可达地址一个都不少 —— 去噪是"折起来"，不是"删掉"。
 * 取不到的项写「未知」，不写空白：空白看起来像界面坏了，而"取不到"只是一条读数。
 */
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { LocalStatus } from "../types";
import { t } from "../i18n";
import { DiagnosticsDrawer } from "./DiagnosticsDrawer";

const local: LocalStatus = {
  idShort: "3f9a1c0b",
  name: "客厅主机",
  addr: "0.0.0.0:58290",
  platform: "windows",
  lanAddrs: ["192.168.1.10:58290"],
  displayAddr: "192.168.1.10:58290",
};

function drawer(over: { open?: boolean; local?: LocalStatus | null } = {}) {
  const onClose = vi.fn();
  const { container } = render(
    <DiagnosticsDrawer
      open={over.open ?? true}
      onClose={onClose}
      local={over.local === undefined ? local : over.local}
    >
      <p>child-panel</p>
    </DiagnosticsDrawer>,
  );
  return { onClose, container };
}

describe("本机身份在诊断区可得", () => {
  it("指纹、监听地址、可达地址都能查到（排障要抄的就是这三条）", () => {
    // 改坏：把身份格删掉 → 用户报障时全界面查不到指纹与端口（这正是被修回来的那次回归）。
    drawer({});

    expect(screen.getByText(t("diag.identity"))).toBeTruthy();
    expect(screen.getByText(local.idShort)).toBeTruthy();
    expect(screen.getByText(local.addr)).toBeTruthy();
    expect(screen.getByText("192.168.1.10:58290")).toBeTruthy();
  });

  it("可达地址取不到 → 写「未知」，不留空白格", () => {
    drawer({ local: { ...local, displayAddr: null, lanAddrs: [] } });

    expect(screen.getByText(t("diag.identity_none"))).toBeTruthy();
  });

  it("本机身份还没水合 → 给占位符，不把 undefined 印在背板上", () => {
    const { container } = drawer({ local: null });

    expect(container.textContent ?? "").not.toContain("undefined");
    // 机器名 + 三个字段都该是占位符
    expect(screen.getAllByText("…").length).toBeGreaterThanOrEqual(4);
  });

  it("子面板照旧渲染（身份格只是背板上的第一格，不顶掉 children）", () => {
    drawer({});

    expect(screen.getByText("child-panel")).toBeTruthy();
  });
});

describe("首屏依旧干净", () => {
  it("抽屉关着时整块不渲染 —— 指纹与监听地址都查不到", () => {
    // 改坏：把 `if (!open) return null` 拿掉 → 首屏又出现 fp/0.0.0.0 这些只有开发者看得懂的东西。
    const { container } = drawer({ open: false });

    expect(container.textContent ?? "").toBe("");
    expect(screen.queryByText(local.idShort)).toBeNull();
    expect(screen.queryByText(local.addr)).toBeNull();
  });

  it("关闭按钮把动作交给上层（本组件不自己管展开状态）", () => {
    const { onClose } = drawer({});

    fireEvent.click(screen.getByRole("button", { name: t("banner.close") }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
