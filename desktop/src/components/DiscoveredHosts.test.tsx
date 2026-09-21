import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "../lib/ipc";
import { t } from "../i18n";
import { DiscoveredHosts } from "./DiscoveredHosts";

vi.mock("../lib/ipc", () => ({ api: { discoveredHosts: vi.fn() } }));
const host = { idShort: "0123456789abcdef", name: "Studio", addr: "192.168.1.20:58291", platform: "win", protoVersion: 0x0201, compatible: true };
beforeEach(() => { vi.resetAllMocks(); vi.mocked(api.discoveredHosts).mockResolvedValue([host]); });
afterEach(() => { vi.useRealTimers(); });

it("uses the discovered QUIC address and prevents duplicate clicks", async () => {
  const onConnect = vi.fn(() => new Promise<boolean>(() => {}));
  render(<DiscoveredHosts connecting={false} connectedIds={[]} onConnect={onConnect} />);
  const button = await screen.findByRole("button", { name: `${t("connect.submit")} Studio` });
  fireEvent.click(button);
  fireEvent.click(button);
  expect(onConnect).toHaveBeenCalledExactlyOnceWith(host.addr);
});

it("keeps incompatible and connected hosts visible without connecting", async () => {
  vi.mocked(api.discoveredHosts).mockResolvedValue([host, { ...host, idShort: "future", name: "Future", compatible: false }]);
  const onConnect = vi.fn(async () => true);
  render(<DiscoveredHosts connecting={false} connectedIds={[host.idShort]} onConnect={onConnect} />);
  await screen.findByText(t("discovery.incompatible"));
  for (const button of screen.getAllByRole("button", { name: /^连接主机 |^Connect to host / })) {
    expect((button as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(button);
  }
  expect(onConnect).not.toHaveBeenCalled();
});

it("refreshes expired hosts, recovers from scan failure, and stops polling on unmount", async () => {
  vi.useFakeTimers();
  const scan = vi.mocked(api.discoveredHosts);
  const { unmount } = render(<DiscoveredHosts connecting={false} connectedIds={[]} onConnect={vi.fn()} />);
  await act(async () => {});
  expect(screen.getByText("Studio")).toBeTruthy();
  scan.mockRejectedValueOnce(new Error("socket unavailable"));
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  expect(screen.getByText(t("discovery.error"))).toBeTruthy();
  expect(screen.queryByText("Studio")).toBeNull();
  scan.mockResolvedValue([]);
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  expect(screen.getByText(t("discovery.empty"))).toBeTruthy();
  unmount();
  await vi.advanceTimersByTimeAsync(5000);
  expect(scan).toHaveBeenCalledTimes(3);
});

it("ignores a scan response received after unmount", async () => {
  let resolve!: (hosts: typeof host[]) => void;
  vi.mocked(api.discoveredHosts).mockReturnValue(new Promise((done) => { resolve = done; }));
  const { unmount } = render(<DiscoveredHosts connecting={false} connectedIds={[]} onConnect={vi.fn()} />);
  unmount();
  await act(async () => resolve([host]));
  await waitFor(() => expect(api.discoveredHosts).toHaveBeenCalledTimes(1));
});

it("refresh restarts the browser and ignores the previous scan response", async () => {
  vi.useFakeTimers();
  let oldResponse!: (hosts: typeof host[]) => void;
  const scan = vi.mocked(api.discoveredHosts);
  scan.mockReturnValueOnce(new Promise((resolve) => { oldResponse = resolve; })).mockResolvedValue([]);
  render(<DiscoveredHosts connecting={false} connectedIds={[]} onConnect={vi.fn()} />);
  const refresh = screen.getByRole("button", { name: t("discovery.refresh") });
  fireEvent.click(refresh);
  fireEvent.click(refresh);
  await act(async () => {});
  expect(scan).toHaveBeenNthCalledWith(1, false);
  expect(scan).toHaveBeenNthCalledWith(2, true);
  expect(scan).toHaveBeenCalledTimes(2);
  expect((refresh as HTMLButtonElement).disabled).toBe(true);
  await act(async () => oldResponse([host]));
  expect(screen.queryByText("Studio")).toBeNull();
  await act(async () => { await vi.advanceTimersByTimeAsync(3500); });
  expect((refresh as HTMLButtonElement).disabled).toBe(false);
  expect(screen.getByText(t("discovery.empty"))).toBeTruthy();
  expect(scan).toHaveBeenLastCalledWith(false);
});

it("refresh can recover a failed discovery scan", async () => {
  const scan = vi.mocked(api.discoveredHosts);
  scan.mockRejectedValueOnce(new Error("socket error"));
  render(<DiscoveredHosts connecting={false} connectedIds={[]} onConnect={vi.fn()} />);
  await screen.findByText(t("discovery.error"));
  fireEvent.click(screen.getByRole("button", { name: t("discovery.refresh") }));
  await screen.findByText("Studio");
  expect(scan).toHaveBeenLastCalledWith(true);
});
