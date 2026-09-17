/**
 * 配对对话框（`PairDialog`）—— 两个方向共用一个事件、靠载荷里的 `pin` 区分。
 *
 * 这里钉的是「方向判断」与「提交门槛」：判反了会让用户输一个自己屏幕上的码；
 * 门槛放水会让一次必然失败的配对走完 15 s 死线。
 */
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { PairRequiredPayload } from "../types";
import { t } from "../i18n";
import { PairDialog } from "./PairDialog";

function request(pin: string): PairRequiredPayload {
  return { idShort: "3f9a1c0b", name: "客厅 R1", pin };
}

function dialog(pin: string, over: { reason?: string; onSubmit?: (idShort: string, pin: string) => Promise<boolean> } = {}) {
  const onSubmit = over.onSubmit ?? vi.fn(async () => true);
  const onDismiss = vi.fn();
  render(<PairDialog request={request(pin)} reason={over.reason ?? ""} onSubmit={onSubmit} onDismiss={onDismiss} />);
  return { onSubmit, onDismiss };
}

describe("方向判断（pin 非空 = 本机亮码）", () => {
  it("接收端：只把 6 位码亮出来，**没有**输入框", () => {
    // 改坏：两个方向共用一个输入框 → 接收端用户被要求输入一个自己屏幕上就有的码。
    dialog("123456");

    expect(screen.getByLabelText(t("pair.code")).textContent).toBe("123456");
    expect(screen.queryByLabelText(t("pair.input"))).toBeNull();
    expect(screen.getByRole("button", { name: t("pair.got_it") })).toBeTruthy();
  });

  it("发起端：给输入框，且粘贴带空格/连字符的码会被收敛成 6 位数字", () => {
    // 改坏：删掉 `replace(/\D/g, "")` → 用户从聊天窗口复制来的 "12 34-56" 被判成非法码。
    dialog("");

    const input = screen.getByLabelText(t("pair.input")) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "12 34-56x7" } });
    expect(input.value).toBe("123456");
  });
});

describe("提交门槛", () => {
  it("不足 6 位时「确认配对」禁用，填满才放行", () => {
    dialog("");
    const input = screen.getByLabelText(t("pair.input"));
    const confirm = screen.getByRole("button", { name: t("pair.confirm") }) as HTMLButtonElement;

    expect(confirm.disabled).toBe(true);
    fireEvent.change(input, { target: { value: "12345" } });
    expect(confirm.disabled).toBe(true);
    fireEvent.change(input, { target: { value: "123456" } });
    expect(confirm.disabled).toBe(false);
  });

  it("提交时把 (idShort, pin) 交给上层：idShort 决定是哪条会话", async () => {
    const onSubmit = vi.fn(async () => true);
    dialog("", { onSubmit });

    fireEvent.change(screen.getByLabelText(t("pair.input")), { target: { value: "958403" } });
    fireEvent.click(screen.getByRole("button", { name: t("pair.confirm") }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith("3f9a1c0b", "958403"));
  });

  it("提交中按钮变「校验中…」并禁用（防连点提交同一个码）", async () => {
    let settle: (value: boolean) => void = () => undefined;
    const onSubmit = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          settle = resolve;
        }),
    );
    dialog("", { onSubmit });

    fireEvent.change(screen.getByLabelText(t("pair.input")), { target: { value: "123456" } });
    fireEvent.click(screen.getByRole("button", { name: t("pair.confirm") }));

    const busy = await screen.findByRole("button", { name: t("pair.verifying") });
    expect((busy as HTMLButtonElement).disabled).toBe(true);

    settle(true);
    await waitFor(() => expect(screen.getByRole("button", { name: t("pair.confirm") })).toBeTruthy());
  });

  it("倒计时归零后不能再提交（60 s 后这个码在内核侧也已失效）", () => {
    vi.useFakeTimers();
    try {
      dialog("");
      fireEvent.change(screen.getByLabelText(t("pair.input")), { target: { value: "123456" } });
      const confirm = screen.getByRole("button", { name: t("pair.confirm") }) as HTMLButtonElement;
      expect(confirm.disabled).toBe(false);

      act(() => {
        vi.advanceTimersByTime(60_000);
      });

      expect(confirm.disabled).toBe(true);
      expect(screen.getByText(t("pair.expired_send"))).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("失败原因的展示", () => {
  it("引擎给的人话原样显示（前端不加工、不替换）", () => {
    dialog("", { reason: "配对码不对，请让对方重新发起连接" });

    expect(screen.getByRole("alert").textContent).toContain("配对码不对，请让对方重新发起连接");
  });

  it("还没失败过时不出现空的警告框", () => {
    dialog("");

    expect(screen.queryByRole("alert")).toBeNull();
  });
});
