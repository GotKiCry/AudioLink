/**
 * useAudioLink 的「接入即自动推流」规则。
 *
 * 这一组不是状态机的白盒测试，而是**产品纪律**：自动行为不能打脸用户。
 * "接入就出声"只是底线，真正的考题是那些**不能做**的分支 ——
 * failed 不重试、用户停过的不重启、开关关掉就完全手动、同一台只自动发起一次。
 */
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./ipc", () => ({
  api: {
    version: vi.fn(),
    localStatus: vi.fn(),
    listPeers: vi.fn(),
    telemetry: vi.fn(),
    tryAutoConnect: vi.fn(),
    autoBroadcastState: vi.fn(),
    setAutoBroadcast: vi.fn(),
    listCaptureDevices: vi.fn(),
    activeCaptureDevice: vi.fn(),
    alignment: vi.fn(),
    startSend: vi.fn(),
    stopSend: vi.fn(),
  },
  subscribeEvents: vi.fn(),
}));

import type { PeerView } from "../types";
import { api, subscribeEvents } from "./ipc";
import { useAudioLink } from "./useAudioLink";

/** 最近一次注册进来的事件表（hook 只在挂载时订阅一次）。 */
let events: Parameters<typeof subscribeEvents>[0] | null = null;

const startSend = vi.mocked(api.startSend);
const stopSend = vi.mocked(api.stopSend);
const setAutoBroadcast = vi.mocked(api.setAutoBroadcast);

function peer(over: Partial<PeerView> = {}): PeerView {
  return {
    idShort: "aaaa1111",
    name: "手机",
    addr: "192.168.1.23:58290",
    state: "idle",
    trusted: true,
    capabilities: null,
    ...over,
  };
}

/** 挂载 hook 并等首屏水合跑完（否则后续 act 会撞上还没落的状态）。 */
async function mount() {
  const view = renderHook(() => useAudioLink());
  await waitFor(() => expect(vi.mocked(api.listPeers)).toHaveBeenCalled());
  return view;
}

/** 把一次对端列表变化喂进去（等价于 audiolink://peer 事件到达）。 */
async function emit(list: PeerView[]): Promise<void> {
  await act(async () => {
    events?.onPeers?.(list);
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  events = null;
  vi.mocked(subscribeEvents).mockImplementation((handlers) => {
    events = handlers;
    return () => undefined;
  });
  vi.mocked(api.version).mockResolvedValue("0.1.0");
  vi.mocked(api.localStatus).mockResolvedValue({
    idShort: "self0001",
    name: "本机",
    addr: "0.0.0.0:58290",
    platform: "windows",
  });
  vi.mocked(api.listPeers).mockResolvedValue([]);
  vi.mocked(api.telemetry).mockResolvedValue({
    peers: 0,
    rttUs: 0,
    jitterUs: 0,
    lossPct: 0,
    bitrateBps: 0,
    bufferLevelUs: 0,
    underruns: 0,
    e2eLatencyUs: 0,
    e2eP50Us: 0,
    e2eP95Us: 0,
  });
  vi.mocked(api.tryAutoConnect).mockResolvedValue(null);
  vi.mocked(api.autoBroadcastState).mockResolvedValue(true);
  vi.mocked(api.setAutoBroadcast).mockResolvedValue(null);
  vi.mocked(api.listCaptureDevices).mockResolvedValue([]);
  vi.mocked(api.activeCaptureDevice).mockResolvedValue(null);
  vi.mocked(api.alignment).mockResolvedValue(null as never);
  vi.mocked(api.startSend).mockResolvedValue({} as never);
  vi.mocked(api.stopSend).mockResolvedValue(null);
});

describe("接入即自动推流", () => {
  it("已配对设备接入并进入 idle → 自动发起，且「已发起」如实成立", async () => {
    // 意图不成立的话，侧栏会停在「开始推流」—— 自动拉起之后用户反而没有停止入口。
    const { result } = await mount();

    await emit([peer({ idShort: "bbbb2222" })]);

    await waitFor(() => expect(startSend).toHaveBeenCalledTimes(1));
    expect(startSend.mock.calls[0]?.[0]).toBe("bbbb2222");
    await waitFor(() => expect(result.current.broadcastRequested).toBe(true));
  });

  it("未配对的设备不自动推（那会必然失败）", async () => {
    await mount();

    await emit([peer({ trusted: false })]);

    expect(startSend).not.toHaveBeenCalled();
  });

  it("failed 不自动重试（自动重试会变成错误风暴）", async () => {
    await mount();

    await emit([peer({ state: "failed" })]);

    expect(startSend).not.toHaveBeenCalled();
  });

  it("握手中 / 其它状态不触发，等它自己走到 idle 再说", async () => {
    await mount();

    await emit([peer({ state: "handshaking" })]);
    expect(startSend).not.toHaveBeenCalled();

    await emit([peer({ state: "idle" })]);
    await waitFor(() => expect(startSend).toHaveBeenCalledTimes(1));
  });

  it("同一台设备这一轮只自动发起一次（事件抖动不去重会重复推同一路）", async () => {
    await mount();

    await emit([peer()]);
    await waitFor(() => expect(startSend).toHaveBeenCalledTimes(1));

    // 事件抖动：同一台又报了一次 idle；状态在 idle/handshaking 之间来回跳也不重复发起
    await emit([peer()]);
    await emit([peer({ state: "handshaking" })]);
    await emit([peer({ state: "idle" })]);

    expect(startSend).toHaveBeenCalledTimes(1);
  });

  it("用户手动停过的设备不会被自动重启", async () => {
    // 这是整个功能最重要的一条：自动化不许覆盖用户刚做的决定。
    const { result } = await mount();
    await emit([peer()]);
    await waitFor(() => expect(startSend).toHaveBeenCalledTimes(1));
    await emit([peer({ state: "streaming" })]);

    await act(async () => {
      await result.current.stopBroadcast();
    });
    expect(stopSend).toHaveBeenCalledTimes(1);
    expect(result.current.broadcastRequested).toBe(false);

    // 会话回落到 idle（用户停止后内核把状态改回去）——自动推流不许把它再拉起来
    await emit([peer({ state: "idle" })]);

    expect(startSend).toHaveBeenCalledTimes(1);
  });

  it("拒绝只针对「那一刻存在的设备」：之后新接入的设备仍按默认规则自动推", async () => {
    const { result } = await mount();
    await emit([peer({ idShort: "aaaa1111", state: "streaming" })]);
    await act(async () => {
      await result.current.stopBroadcast();
    });

    await emit([
      peer({ idShort: "aaaa1111", state: "idle" }),
      peer({ idShort: "bbbb2222" }),
    ]);

    await waitFor(() => expect(startSend).toHaveBeenCalledTimes(1));
    expect(startSend.mock.calls[0]?.[0]).toBe("bbbb2222");
  });

  it("开关关掉 = 完全手动：接入也不自动发起", async () => {
    const { result } = await mount();

    await act(async () => {
      result.current.setAutoBroadcast(false);
    });
    expect(setAutoBroadcast).toHaveBeenCalledWith(false);
    expect(result.current.autoBroadcast).toBe(false);

    await emit([peer()]);

    expect(startSend).not.toHaveBeenCalled();
    expect(result.current.broadcastRequested).toBe(false);
  });

  it("重新打开开关 = 一次明确的重新授权：清掉拒绝名单，让设备能再被自动拉起", async () => {
    const { result } = await mount();
    await emit([peer({ state: "streaming" })]);
    await act(async () => {
      await result.current.stopBroadcast();
    });

    await act(async () => {
      result.current.setAutoBroadcast(false);
      result.current.setAutoBroadcast(true);
    });
    await emit([peer({ state: "idle" })]);

    await waitFor(() => expect(startSend).toHaveBeenCalledTimes(1));
  });
});
