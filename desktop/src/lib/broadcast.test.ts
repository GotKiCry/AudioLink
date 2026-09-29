/**
 * `deriveBroadcastStatus` 与 `deriveBroadcastAffordances` 的规则测试。
 *
 * 这两段逻辑原先散在两处、**都没有单测**：状态链在 `App.tsx:98-105`，
 * 按钮语义在 `Sidebar.tsx:29-32` —— 也就是说"侧栏显示什么、按钮能不能点"以前只能靠肉眼看。
 * 搬进 `broadcast.ts` 后，这里钉四类事实：
 *
 * 1. **逐分支**：每条分支都有一条最小输入能到达（不是死代码）；
 * 2. **优先级**：多个条件同时成立时谁赢 —— 顺序即优先级，重排顺序必须是有人故意的；
 * 3. **穷举等价**：状态链对 7 个开关 × 计数 × busyPeer × error 的 1536 种组合、
 *    按钮语义对 9 个状态 × canResume 两态，逐一与搬迁前的原实现对齐 ——
 *    这是"搬迁没有改变行为"的硬证据（波次 2 把调用点切过来之后，那两份对照实现即可删）；
 * 4. **边界**：全 false / 全 true、`captureLoading` 与 `busyPeer` 同真、空串 `busyPeer` 等。
 */
import { describe, expect, it } from "vitest";

import type { CommandError } from "../types";
import {
  deriveBroadcastAffordances,
  deriveBroadcastStatus,
  type BroadcastStatus,
  type BroadcastStatusInput,
} from "./broadcast";

const ERROR: CommandError = { code: 2001, message: "Capture unavailable", context: "capture" };

/**
 * 全门放行的基线：每个字段都取"不遮蔽任何分支"的值，所以它的结果是 `manual`。
 *
 * 逐分支用例都在它上面**只改一个字段** —— 这样"哪条分支赢了"是唯一的变量，
 * 断言失败时不用再猜是哪个输入造成的。
 */
const CLEAR: BroadcastStatusInput = {
  broadcastStopping: false,
  streamingCount: 0,
  broadcastPaused: false,
  autoBroadcastReady: true,
  captureLoading: false,
  canStartCapture: true,
  busyPeer: null,
  broadcastError: null,
  autoBroadcast: false,
  broadcastRequested: false,
};

/** 把基线加上若干字段的覆盖值（不改原对象）。 */
function input(patch: Partial<BroadcastStatusInput> = {}): BroadcastStatusInput {
  return { ...CLEAR, ...patch };
}

describe("逐分支：每条分支都有最小输入能到达", () => {
  const cases: ReadonlyArray<readonly [string, Partial<BroadcastStatusInput>, BroadcastStatus]> = [
    ["pausing —— 停止已发起、回执未到", { broadcastStopping: true }, "pausing"],
    ["streaming —— 有一台对端在推（1 台就够）", { streamingCount: 1 }, "streaming"],
    ["paused —— 用户主动暂停了会话", { broadcastPaused: true }, "paused"],
    ["loading —— 设置还没读回（不是'开关关着'）", { autoBroadcastReady: false }, "loading"],
    ["loading —— 采集设备还在枚举", { captureLoading: true }, "loading"],
    ["unavailable —— 没有可用音源", { canStartCapture: false }, "unavailable"],
    ["starting —— 正在 start/stop 某台对端", { busyPeer: "aaaa1111" }, "starting"],
    ["error —— 上一次动作失败了", { broadcastError: ERROR }, "error"],
    ["waiting —— 自动推流开关开着，等设备接入", { autoBroadcast: true }, "waiting"],
    ["waiting —— 已请求共享，等对端真的开始", { broadcastRequested: true }, "waiting"],
    ["manual —— 什么都不成立（完全手动）", {}, "manual"],
  ];

  it.each(cases)("%s", (_name, patch, expected) => {
    expect(deriveBroadcastStatus(input(patch))).toBe(expected);
  });
});

describe("优先级：stopping > streaming > paused > loading > unavailable > starting > error > waiting > manual", () => {
  it("全部条件成立 → pausing（停止中压倒一切）", () => {
    expect(deriveBroadcastStatus({
      broadcastStopping: true,
      streamingCount: 5,
      broadcastPaused: true,
      autoBroadcastReady: false,
      captureLoading: true,
      canStartCapture: false,
      busyPeer: "aaaa1111",
      broadcastError: ERROR,
      autoBroadcast: true,
      broadcastRequested: true,
    })).toBe("pausing");
  });

  it("停止中但对端还报着 streaming → 仍是 pausing（不能退回「停止共享」，否则用户以为没点上）", () => {
    expect(deriveBroadcastStatus(input({ broadcastStopping: true, streamingCount: 1 }))).toBe("pausing");
  });

  it("streaming 盖过 paused / loading / unavailable / starting / error", () => {
    expect(deriveBroadcastStatus(input({
      streamingCount: 2,
      broadcastPaused: true,
      autoBroadcastReady: false,
      captureLoading: true,
      canStartCapture: false,
      busyPeer: "aaaa1111",
      broadcastError: ERROR,
    }))).toBe("streaming");
  });

  it("paused 盖过 loading / unavailable / starting / error（暂停意图必须看得见）", () => {
    expect(deriveBroadcastStatus(input({
      broadcastPaused: true,
      autoBroadcastReady: false,
      captureLoading: true,
      canStartCapture: false,
      busyPeer: "aaaa1111",
      broadcastError: ERROR,
    }))).toBe("paused");
  });

  it("loading 盖过 unavailable / starting / error（还没准备完，谈不上音源不可用）", () => {
    expect(deriveBroadcastStatus(input({
      captureLoading: true,
      canStartCapture: false,
      busyPeer: "aaaa1111",
      broadcastError: ERROR,
    }))).toBe("loading");
  });

  it("unavailable 盖过 starting / error", () => {
    expect(deriveBroadcastStatus(input({
      canStartCapture: false,
      busyPeer: "aaaa1111",
      broadcastError: ERROR,
    }))).toBe("unavailable");
  });

  it("starting 盖过 error（启动中不显示上一次的失败）", () => {
    expect(deriveBroadcastStatus(input({ busyPeer: "aaaa1111", broadcastError: ERROR }))).toBe("starting");
  });

  it("error 盖过 waiting / manual（失败要说出来，不能退回'等待接入'）", () => {
    expect(deriveBroadcastStatus(input({
      broadcastError: ERROR,
      autoBroadcast: true,
      broadcastRequested: true,
    }))).toBe("error");
  });

  it("waiting 盖过 manual", () => {
    expect(deriveBroadcastStatus(input({ autoBroadcast: true, broadcastRequested: true }))).toBe("waiting");
  });
});

describe("边界", () => {
  it("全 false / null / 0 → manual", () => {
    expect(deriveBroadcastStatus(CLEAR)).toBe("manual");
  });

  it("captureLoading 与 busyPeer 同时为真 → loading（设备还没枚举完，没有'启动中'可言）", () => {
    expect(deriveBroadcastStatus(input({ captureLoading: true, busyPeer: "aaaa1111" }))).toBe("loading");
  });

  it("autoBroadcastReady 与 captureLoading 是「或」关系：任一为真就 loading", () => {
    expect(deriveBroadcastStatus(input({ autoBroadcastReady: false, captureLoading: false }))).toBe("loading");
    expect(deriveBroadcastStatus(input({ autoBroadcastReady: true, captureLoading: true }))).toBe("loading");
    expect(deriveBroadcastStatus(input({ autoBroadcastReady: false, captureLoading: true }))).toBe("loading");
  });

  it("streamingCount 恰好 1 就算在发送（判据是 > 0，含端点）", () => {
    expect(deriveBroadcastStatus(input({ streamingCount: 1 }))).toBe("streaming");
  });

  it("streamingCount 为 0 / 负数都不算在发送（纯函数对垃圾输入不炸）", () => {
    expect(deriveBroadcastStatus(input({ streamingCount: 0 }))).toBe("manual");
    expect(deriveBroadcastStatus(input({ streamingCount: -1 }))).toBe("manual");
  });

  it("busyPeer 的空串也算「有值」→ starting（判据是 !== null；只有 null 才表示没有在忙）", () => {
    expect(deriveBroadcastStatus(input({ busyPeer: "" }))).toBe("starting");
  });
});

describe("与搬迁前 App.tsx:98-105 的三元链穷举等价", () => {
  /**
   * `App.tsx` 搬迁前的原文（逐字复制，含先 stopping 后 streaming 的顺序）。
   * 只在本测试里作为对照实现，波次 2 把 App 切到 `deriveBroadcastStatus` 后即可删除。
   */
  const legacyChain = (input: BroadcastStatusInput): BroadcastStatus => input.broadcastStopping ? "pausing"
    : input.streamingCount > 0 ? "streaming"
    : input.broadcastPaused ? "paused"
    : !input.autoBroadcastReady || input.captureLoading ? "loading"
    : !input.canStartCapture ? "unavailable"
    : input.busyPeer !== null ? "starting"
    : input.broadcastError !== null ? "error"
    : input.autoBroadcast || input.broadcastRequested ? "waiting" : "manual";

  /** 参与穷举的 7 个布尔开关（其余三个轴向单独展开）。 */
  const SWITCHES = [
    "broadcastStopping", "broadcastPaused", "autoBroadcastReady",
    "captureLoading", "canStartCapture", "autoBroadcast", "broadcastRequested",
  ] as const;

  it("7 个开关 × 计数 × busyPeer × error 的全部 1536 种组合结果完全相同", () => {
    let checked = 0;
    for (let mask = 0; mask < 1 << SWITCHES.length; mask += 1) {
      for (const streamingCount of [0, 1, 3]) {
        for (const busyPeer of [null, "aaaa1111"] as const) {
          for (const broadcastError of [null, ERROR] as const) {
            const candidate: BroadcastStatusInput = { ...CLEAR, streamingCount, busyPeer, broadcastError };
            SWITCHES.forEach((field, index) => {
              candidate[field] = (mask & (1 << index)) !== 0;
            });

            expect(deriveBroadcastStatus(candidate)).toBe(legacyChain(candidate));
            checked += 1;
          }
        }
      }
    }
    expect(checked).toBe(1536);
  });
});

describe("deriveBroadcastAffordances：侧栏主按钮的三项", () => {
  /**
   * `Sidebar.tsx:29-32` 搬迁前的原文（`t(...)` 换成键名，其余逐字）。
   * 只在本测试里作为对照实现；波次 2 把 Sidebar 切过来后即可删除。
   */
  const legacyAffordances = (status: BroadcastStatus, canResume: boolean) => {
    const canPause = status === "streaming" || status === "starting" || status === "waiting";
    const disabled = status === "loading" || status === "pausing" || (!canPause && !canResume);
    const label = status === "pausing" ? "peer.stopping" : canPause ? "side.stop_broadcast"
      : status === "error" ? "share.retry" : status === "manual" ? "share.start" : "side.broadcast";
    return { canPause, disabled, label };
  };

  const STATUSES: BroadcastStatus[] = [
    "loading", "waiting", "starting", "streaming", "pausing", "paused", "unavailable", "error", "manual",
  ];

  it("9 个状态 × canResume 两态 = 18 种组合与 Sidebar 原三元链完全一致", () => {
    let checked = 0;
    for (const status of STATUSES) {
      for (const canResume of [false, true]) {
        expect(deriveBroadcastAffordances(status, canResume)).toEqual(legacyAffordances(status, canResume));
        checked += 1;
      }
    }
    expect(checked).toBe(18);
  });

  it("能暂停的三个状态：按钮可点、语义是「停止」、文案走 side.stop_broadcast（canResume 不参与）", () => {
    for (const status of ["streaming", "starting", "waiting"] as const) {
      const expected = { canPause: true, disabled: false, label: "side.stop_broadcast" };
      expect(deriveBroadcastAffordances(status, true)).toEqual(expected);
      expect(deriveBroadcastAffordances(status, false)).toEqual(expected);
    }
  });

  it("过渡态一律禁用：loading / pausing 与 canResume 无关", () => {
    for (const status of ["loading", "pausing"] as const) {
      expect(deriveBroadcastAffordances(status, true).disabled).toBe(true);
      expect(deriveBroadcastAffordances(status, false).disabled).toBe(true);
    }
    // pausing 的文案是「进行中」，不是「停止」—— 按钮已经点不动了，不能再说"点我停止"
    expect(deriveBroadcastAffordances("pausing", true)).toEqual({
      canPause: false, disabled: true, label: "peer.stopping",
    });
  });

  it("canResume 是「开始/恢复」侧的准入：paused / unavailable / error / manual 都看它", () => {
    for (const status of ["paused", "unavailable", "error", "manual"] as const) {
      expect(deriveBroadcastAffordances(status, true).disabled).toBe(false);
      expect(deriveBroadcastAffordances(status, false).disabled).toBe(true);
    }
  });

  it("文案优先级：pausing > 能暂停 > error > manual > 兜底", () => {
    expect(deriveBroadcastAffordances("pausing", true).label).toBe("peer.stopping");
    expect(deriveBroadcastAffordances("streaming", true).label).toBe("side.stop_broadcast");
    expect(deriveBroadcastAffordances("error", true).label).toBe("share.retry");
    expect(deriveBroadcastAffordances("manual", true).label).toBe("share.start");
    expect(deriveBroadcastAffordances("paused", true).label).toBe("side.broadcast");
    expect(deriveBroadcastAffordances("unavailable", true).label).toBe("side.broadcast");
    expect(deriveBroadcastAffordances("loading", true).label).toBe("side.broadcast");
  });
});
