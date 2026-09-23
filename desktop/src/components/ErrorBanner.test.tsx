/**
 * 失败横幅（`ErrorBanner`）—— 错误呈现链路的**末端**。
 *
 * 为什么这里也要测（第 96 轮点名"建组失败时错误如实显示"）：
 * `GroupPanel` 自己**不**处理错误（它只派发 `onCreate`），失败一路是
 * `api.createGroup` → hook 的 `catch` → `setError(CommandError)` → `App` → 本组件。
 * 要钉住"用户真的能看到那句人话"，能测的就是这条链的显示端：**三件必须出现的东西**
 * （人话原因 / 错误码 / 机械上下文）。在 GroupPanel 那端编一个错误状态只会是空壳测试 ——
 * 它压根没有 error 入参。
 */
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ErrorBanner } from "./ErrorBanner";

describe("错误必说人话（UI 规格 §4）且可对照协议文档", () => {
  it("人话原因、错误码、机械上下文三件都在（用户看第一件、排查看后两件）", () => {
    // 改坏：只显示 message 不显示 code → 用户报障时无法对照协议文档的那一行；
    // 或反过来只显示 code → 用户看到的是 1009 这种机器话。
    render(
      <ErrorBanner
        error={{ code: 1009, message: "加入同步组失败：对端已离线", context: "join_group: peer offline" }}
        onDismiss={vi.fn()}
      />,
    );

    const banner = screen.getByRole("alert");
    expect(banner.textContent).toContain("加入同步组失败：对端已离线");
    expect(banner.textContent).toContain("code=1009");
    expect(banner.textContent).toContain("join_group: peer offline");
  });

  it("上下文为空时不留下一个孤零零的分隔符", () => {
    render(<ErrorBanner error={{ code: 0, message: "界面与内核通信失败，请重试", context: "" }} onDismiss={vi.fn()} />);

    const banner = screen.getByRole("alert");
    expect(banner.textContent).toContain("code=0");
    expect(banner.textContent).not.toContain(" · ");
  });

  it("是人话而不是同一句兜底文案（组件不加工、不替换上层给的句子）", () => {
    // 改坏：把 message 换成一句统一的"操作失败" → 这条会红，而用户唯一能拿到的线索就没了。
    render(
      <ErrorBanner error={{ code: 1002, message: "指定的对端不存在", context: "start_send" }} onDismiss={vi.fn()} />,
    );

    expect(screen.getByRole("alert").textContent).toContain("指定的对端不存在");
    expect(screen.getByRole("alert").textContent).not.toContain("操作失败");
  });
});
