import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { t } from "../i18n";
import { Sidebar } from "./Sidebar";

type Props = Parameters<typeof Sidebar>[0];
function sidebar(over: Partial<Props> = {}) {
  const handlers = { onSelectCapture: vi.fn(), onRefreshCapture: vi.fn(async () => undefined),
    onBroadcast: vi.fn(), onStopBroadcast: vi.fn() };
  render(<Sidebar version="0.1.0" local={null} status="waiting" streamingCount={0}
    autoBroadcast canResume captureDevices={[]} selectedCaptureId="" activeCapture={null}
    captureLoading={false} captureLocked={false} captureError={null} {...handlers} {...over} />);
  return handlers;
}

describe("共享控制", () => {
  it("没有设备也可暂停自动共享，等待状态不冒充推流", () => {
    const handlers = sidebar();
    expect(screen.getByText(t("share.waiting"))).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: t("side.stop_broadcast") }));
    expect(handlers.onStopBroadcast).toHaveBeenCalledOnce();
    expect(handlers.onBroadcast).not.toHaveBeenCalled();
  });
  it("启动过程中仍可暂停", () => {
    const handlers = sidebar({ status: "starting", canResume: false, captureLocked: true });
    fireEvent.click(screen.getByRole("button", { name: t("side.stop_broadcast") }));
    expect(handlers.onStopBroadcast).toHaveBeenCalledOnce();
  });
  it("已暂停可恢复，且恢复不需要已有设备", () => {
    const handlers = sidebar({ status: "paused" });
    expect(screen.getByText(t("share.paused_hint"))).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: t("side.broadcast") }));
    expect(handlers.onBroadcast).toHaveBeenCalledOnce();
  });
  it("暂停尚未完成时不能抢先恢复", () => {
    const handlers = sidebar({ status: "pausing" });
    const button = screen.getByRole("button", { name: t("peer.stopping") }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    fireEvent.click(button);
    expect(handlers.onBroadcast).not.toHaveBeenCalled();
  });
  it("没有可用音源时不能发起恢复", () => {
    sidebar({ status: "unavailable", canResume: false });
    expect((screen.getByRole("button", { name: t("side.broadcast") }) as HTMLButtonElement).disabled).toBe(true);
  });
  it("自动启动失败时给出可执行的重试入口", () => {
    const handlers = sidebar({ status: "error" });
    fireEvent.click(screen.getByRole("button", { name: t("share.retry") }));
    expect(handlers.onBroadcast).toHaveBeenCalledOnce();
  });
});
