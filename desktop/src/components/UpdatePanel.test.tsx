/**
 * 软件更新面板（`UpdatePanel`）—— 三条结果路径 + **两条安全边界**。
 *
 * 这里必须 mock `../lib/ipc`（组件只通过它说话），但断言的不是"某个 mock 被调用过"，
 * 而是**用户看到什么**与**不该发生什么**：
 *   · 渲染本身不会发起检查（更新只能由用户主动发起）；
 *   · 点了「下载并安装」在二次确认之前**不会**真的调用安装命令。
 * 这两条是这个产品的硬边界：用户可能正在推流/录音，静默安装与静默重启不可接受。
 */
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/ipc", () => ({
  api: {
    checkUpdate: vi.fn(),
    installUpdate: vi.fn(),
  },
}));

import { api } from "../lib/ipc";
import { t } from "../i18n";
import { UpdatePanel } from "./UpdatePanel";

const checkUpdate = vi.mocked(api.checkUpdate);
const installUpdate = vi.mocked(api.installUpdate);

beforeEach(() => {
  checkUpdate.mockReset();
  installUpdate.mockReset();
  installUpdate.mockResolvedValue("0.1.1");
});

describe("安全边界", () => {
  it("渲染不会自动检查更新（更新只能由用户发起）", () => {
    // 改坏：加一个 useEffect 里自动检查 → 每次打开界面都会打一次网络请求，
    // 这正是"静默更新"的开端。
    render(<UpdatePanel />);

    expect(checkUpdate).not.toHaveBeenCalled();
  });

  it("点「下载并安装」只进入确认态：在用户二次确认之前**不调用**安装命令", () => {
    // 改坏：把确认态去掉、点一下就 installUpdate → 用户点一次就装+重启，
    // 正在推流的会话被无声打断。
    checkUpdate.mockResolvedValue({ currentVersion: "0.1.0", version: "0.1.1", notes: null, pubDate: null });
    render(<UpdatePanel />);

    fireEvent.click(screen.getByRole("button", { name: t("upd.check") }));
    return screen
      .findByRole("button", { name: t("upd.install", { version: "0.1.1" }) })
      .then((installButton) => {
        fireEvent.click(installButton);

        expect(installUpdate).not.toHaveBeenCalled();
        expect(screen.getByText(t("upd.confirm_hint"))).toBeTruthy();
      });
  });
});

describe("三种结果", () => {
  it("已是最新：报告当前版本，且不给任何安装入口", async () => {
    checkUpdate.mockResolvedValue({ currentVersion: "0.1.0", version: null, notes: null, pubDate: null });
    render(<UpdatePanel />);

    fireEvent.click(screen.getByRole("button", { name: t("upd.check") }));

    expect(await screen.findByText(t("upd.up_to_date", { version: "0.1.0" }))).toBeTruthy();
    expect(screen.queryByRole("button", { name: t("upd.install", { version: "0.1.0" }) })).toBeNull();
  });

  it("发现新版本：显示版本号、发布日期与更新说明；确认后带着**那个**版本号调安装", async () => {
    checkUpdate.mockResolvedValue({
      currentVersion: "0.1.0",
      version: "0.1.1",
      notes: "修了连接超时",
      pubDate: "2026-09-17",
    });
    render(<UpdatePanel />);

    fireEvent.click(screen.getByRole("button", { name: t("upd.check") }));

    expect(await screen.findByText(t("upd.available", { version: "0.1.1", current: "0.1.0" }))).toBeTruthy();
    expect(screen.getByText(t("upd.published", { date: "2026-09-17" }))).toBeTruthy();
    expect(screen.getByText("修了连接超时")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: t("upd.install", { version: "0.1.1" }) }));
    fireEvent.click(screen.getByRole("button", { name: t("upd.confirm") }));

    await waitFor(() => expect(installUpdate).toHaveBeenCalledWith("0.1.1"));
  });

  it("出错：原样显示后端翻译好的人话原因，且不给出任何版本结论", async () => {
    // 改坏：在前端按字符串拼原因（例如统一显示"检查更新失败"）→ 这条会红，
    // 而那正是用户唯一能拿到的线索（网络不通 vs 签名不对，处置完全不同）。
    const message = "连不上更新服务器（网络不通或被拦）。检查网络后重试，或到官网手动下载。";
    checkUpdate.mockRejectedValue({ code: 1009, message, context: "check_update: request failed" });
    render(<UpdatePanel />);

    fireEvent.click(screen.getByRole("button", { name: t("upd.check") }));

    expect(await screen.findByText(message)).toBeTruthy();
    expect(screen.queryByText(t("upd.up_to_date", { version: "0.1.0" }))).toBeNull();
  });

  it("检查进行中：按钮禁用并显示「检查中…」（防连点发两次请求）", async () => {
    let settle: (value: { currentVersion: string; version: string | null; notes: string | null; pubDate: string | null }) => void =
      () => undefined;
    checkUpdate.mockReturnValue(
      new Promise((resolve) => {
        settle = resolve;
      }),
    );
    render(<UpdatePanel />);

    fireEvent.click(screen.getByRole("button", { name: t("upd.check") }));

    const busy = screen.getByRole("button", { name: t("upd.checking") }) as HTMLButtonElement;
    expect(busy.disabled).toBe(true);

    settle({ currentVersion: "0.1.0", version: null, notes: null, pubDate: null });
    expect(await screen.findByText(t("upd.up_to_date", { version: "0.1.0" }))).toBeTruthy();
  });
});
