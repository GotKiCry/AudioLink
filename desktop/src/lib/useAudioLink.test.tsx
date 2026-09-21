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
  vi.resetAllMocks();
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
  vi.mocked(api.listCaptureDevices).mockResolvedValue([{
    id: "speakers", name: "Speakers", isDefault: true, isVirtual: false,
    sampleRate: 48000, channels: 2, unavailableReason: null,
  }]);
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

  it("暂停是全局意图：新设备接入也不会自动推流", async () => {
    const { result } = await mount();
    await emit([peer({ idShort: "aaaa1111", state: "streaming" })]);
    await act(async () => {
      await result.current.stopBroadcast();
    });

    await emit([
      peer({ idShort: "aaaa1111", state: "idle" }),
      peer({ idShort: "bbbb2222" }),
    ]);

    expect(startSend).not.toHaveBeenCalled();
    expect(result.current.broadcastPaused).toBe(true);
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

  it("切换自动偏好不会取消暂停，必须主动恢复", async () => {
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

    expect(startSend).not.toHaveBeenCalled();
    await act(async () => { result.current.startBroadcast(["aaaa1111"]); });
    await waitFor(() => expect(startSend).toHaveBeenCalledTimes(1));
    expect(result.current.broadcastPaused).toBe(false);
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe("自动共享的时序与恢复", () => {
  it("设置尚未读完不能抢跑，已保存关闭偏好时保持静音", async () => {
    const saved = deferred<boolean>();
    vi.mocked(api.autoBroadcastState).mockReturnValue(saved.promise);
    await mount();
    await emit([peer()]);
    expect(startSend).not.toHaveBeenCalled();
    await act(async () => { saved.resolve(false); });
    expect(startSend).not.toHaveBeenCalled();
  });
  it("等待音源加载完毕后自动开始", async () => {
    const sources = deferred<Awaited<ReturnType<typeof api.listCaptureDevices>>>();
    vi.mocked(api.listCaptureDevices).mockReturnValueOnce(sources.promise);
    await mount();
    await emit([peer()]);
    expect(startSend).not.toHaveBeenCalled();
    await act(async () => { sources.resolve([{
      id: "speakers", name: "Speakers", isDefault: true, isVirtual: false,
      sampleRate: 48000, channels: 2, unavailableReason: null,
    }]); });
    await waitFor(() => expect(startSend).toHaveBeenCalledOnce());
  });
  it("启动尚未完成时暂停，先等启动完成再停止，且不重复停止", async () => {
    const starting = deferred<Awaited<ReturnType<typeof api.startSend>>>();
    startSend.mockReturnValueOnce(starting.promise);
    const { result } = await mount();
    await emit([peer()]);
    let stopping!: Promise<void>;
    act(() => { stopping = result.current.stopBroadcast(); });
    expect(result.current.broadcastPaused).toBe(true);
    expect(stopSend).not.toHaveBeenCalled();
    act(() => { void result.current.stopBroadcast(); });
    await act(async () => { starting.resolve({ stream_id: 1 }); await stopping; });
    expect(stopSend).toHaveBeenCalledOnce();
    await emit([peer()]);
    expect(startSend).toHaveBeenCalledOnce();
  });
  it("无设备时先暂停，接入不触发；恢复后自动推流", async () => {
    const { result } = await mount();
    await act(async () => { await result.current.stopBroadcast(); });
    await emit([peer()]);
    expect(startSend).not.toHaveBeenCalled();
    await act(async () => { result.current.startBroadcast(["aaaa1111"]); });
    expect(startSend).toHaveBeenCalledOnce();
  });
  it("断开后同一设备再次连接可以重新自动开始", async () => {
    await mount();
    await emit([peer()]);
    await waitFor(() => expect(startSend).toHaveBeenCalledOnce());
    await emit([peer({ state: "failed" })]);
    await emit([peer()]);
    await waitFor(() => expect(startSend).toHaveBeenCalledTimes(2));
  });
  it("启动失败不会伪装成推流或循环重试，用户可以重试", async () => {
    startSend.mockRejectedValueOnce({ code: 2001, message: "Capture unavailable", detail: "capture" });
    const { result } = await mount();
    await emit([peer()]);
    await waitFor(() => expect(result.current.error).not.toBeNull());
    expect(result.current.broadcastRequested).toBe(false);
    await emit([peer()]);
    expect(startSend).toHaveBeenCalledOnce();
    await act(async () => { result.current.startBroadcast(["aaaa1111"]); });
    expect(startSend).toHaveBeenCalledTimes(2);
  });
  it("单路外壳只启动一个目标，避免多设备接入造成 busy 错误", async () => {
    await mount();
    await emit([peer(), peer({ idShort: "bbbb2222" })]);
    await waitFor(() => expect(startSend).toHaveBeenCalledOnce());
    await emit([peer(), peer({ idShort: "bbbb2222" })]);
    expect(startSend).toHaveBeenCalledOnce();
  });
});


describe("设备快照与音源选择", () => {
  it("晚到的初始快照不能覆盖刚连接的设备", async () => {
    const initial = deferred<PeerView[]>();
    vi.mocked(api.listPeers).mockReturnValueOnce(initial.promise);
    const { result } = await mount();
    await emit([peer()]);
    await waitFor(() => expect(startSend).toHaveBeenCalledOnce());
    await act(async () => { initial.resolve([]); });
    expect(result.current.peers.map((p) => p.idShort)).toEqual(["aaaa1111"]);
  });
  it("无可用音源时不消耗自动启动机会，刷新后可以开始", async () => {
    vi.mocked(api.listCaptureDevices).mockResolvedValueOnce([]);
    const { result } = await mount();
    await emit([peer()]);
    expect(startSend).not.toHaveBeenCalled();
    await act(async () => { await result.current.refreshCaptureDevices(); });
    await waitFor(() => expect(startSend).toHaveBeenCalledOnce());
  });
  it("暂停后选择另一音源，恢复使用新设备", async () => {
    const { result } = await mount();
    await emit([peer()]);
    await act(async () => { await result.current.stopBroadcast(); });
    vi.mocked(api.listCaptureDevices).mockResolvedValueOnce([{
      id: "headphones", name: "Headphones", isDefault: true, isVirtual: false,
      sampleRate: 48000, channels: 2, unavailableReason: null,
    }]);
    await act(async () => { await result.current.refreshCaptureDevices(); });
    act(() => { result.current.selectCapture("headphones"); });
    await act(async () => { result.current.startBroadcast(["aaaa1111"]); });
    expect(startSend).toHaveBeenLastCalledWith("aaaa1111", "headphones");
  });
});


it("停止回执失败但会话已回到 idle 时仍能手动恢复", async () => {
  const { result } = await mount();
  await emit([peer()]);
  await waitFor(() => expect(startSend).toHaveBeenCalledOnce());
  stopSend.mockRejectedValueOnce({ code: 2001, message: "Stop reply lost", detail: "stop" });
  await act(async () => { await result.current.stopBroadcast(); });
  await emit([peer()]);
  expect(startSend).toHaveBeenCalledOnce();
  await act(async () => { result.current.startBroadcast(["aaaa1111"]); });
  expect(startSend).toHaveBeenCalledTimes(2);
});


it("快速重复恢复只启动一次，之后断开重连仍可自动发送", async () => {
  const starting = deferred<Awaited<ReturnType<typeof api.startSend>>>();
  startSend.mockReturnValueOnce(starting.promise);
  const { result } = await mount();
  await act(async () => { await result.current.stopBroadcast(); });
  await emit([peer()]);
  act(() => {
    result.current.startBroadcast(["aaaa1111"]);
    result.current.startBroadcast(["aaaa1111"]);
  });
  expect(startSend).toHaveBeenCalledOnce();
  await act(async () => { starting.resolve({ stream_id: 1 }); });
  await emit([]);
  await emit([peer()]);
  await waitFor(() => expect(startSend).toHaveBeenCalledTimes(2));
});

it("收听主机不会自动向主机回传声音，入站接收设备仍自动推流", async () => {
  await mount();
  await emit([peer({ receiving: true })]);
  expect(startSend).not.toHaveBeenCalled();
  await emit([peer({ receiving: true }), peer({ idShort: "receiver", receiving: false })]);
  await waitFor(() => expect(startSend).toHaveBeenCalledTimes(1));
  expect(startSend.mock.calls[0]?.[0]).toBe("receiver");
});
