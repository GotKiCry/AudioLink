/**
 * 多源对齐面板（`AlignmentPanel`）—— 提前量输入框的**边界行为**。
 *
 * 这块面板上唯一能填错的东西就是那个数字输入框，所以测试盯着它的几个边界：清空、0、超范围。
 * 报告 §二 记的正是"清空即变 0"：`Number("")` 是 0，于是用户没法"先删掉再输入"，
 * 而且一个他没打算填的 0 会被当成合法提前量广播出去 —— 0 提前量意味着发送端来不及等到
 * 那个时刻，整段丢掉，正是这个面板最不该让用户无意中做出来的事。
 */
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { t } from "../i18n";
import { AlignmentPanel } from "./AlignmentPanel";

/** 对齐读数给 null（空态）就够了：这里测的是输入框，不是读数表。 */
function panel(over: { busy?: boolean } = {}) {
  const onBroadcast = vi.fn();
  render(
    <AlignmentPanel alignment={null} busy={over.busy ?? false} onBroadcast={onBroadcast} />,
  );
  return { onBroadcast };
}

/** 提前量输入框（面板上唯一的 spinbutton）。 */
function lead(): HTMLInputElement {
  return screen.getByRole("spinbutton") as HTMLInputElement;
}

/** 「广播共同基准」按钮（忙碌时文案换成 align.busy）。 */
function broadcastButton(label: string = t("align.broadcast")): HTMLButtonElement {
  return screen.getByRole("button", { name: label }) as HTMLButtonElement;
}

describe("提前量输入框的边界", () => {
  it("清空输入框就只是空的：不会自己变成 0，也不会放行一次广播", () => {
    // 改坏：state 改回 `Number(event.target.value)` → 这里会变成 "0"，
    // 用户再也"删不干净"（删掉一个 0 又冒出一个 0），而且那个 0 还会被合法提交。
    const { onBroadcast } = panel();

    fireEvent.change(lead(), { target: { value: "" } });

    expect(lead().value).toBe("");
    expect(broadcastButton().disabled).toBe(true);

    fireEvent.click(broadcastButton());
    expect(onBroadcast).not.toHaveBeenCalled();
  });

  it("0 仍然是合法提前量（清空 ≠ 0：手打 0 允许，空着不允许）", () => {
    const { onBroadcast } = panel();

    fireEvent.change(lead(), { target: { value: "0" } });
    expect(broadcastButton().disabled).toBe(false);

    fireEvent.click(broadcastButton());
    expect(onBroadcast).toHaveBeenCalledWith(0);
  });

  it("原样提交用户填的值（不偷偷换成默认值）", () => {
    const { onBroadcast } = panel();

    fireEvent.change(lead(), { target: { value: "450" } });
    fireEvent.click(broadcastButton());

    expect(onBroadcast).toHaveBeenCalledWith(450);
  });

  it("超出上限收敛到 5000（输入框的 max 只是提示，真正执行约束的是提交那一刻）", () => {
    const { onBroadcast } = panel();

    fireEvent.change(lead(), { target: { value: "99999" } });
    fireEvent.click(broadcastButton());

    expect(onBroadcast).toHaveBeenCalledWith(5000);
  });

  it("负数收敛到 0（下限不是只写在 min 属性上）", () => {
    const { onBroadcast } = panel();

    fireEvent.change(lead(), { target: { value: "-50" } });
    fireEvent.click(broadcastButton());

    expect(onBroadcast).toHaveBeenCalledWith(0);
  });

  it("广播进行中按钮禁用（防连点重复发）", () => {
    panel({ busy: true });

    expect(broadcastButton(t("align.busy")).disabled).toBe(true);
  });
});
