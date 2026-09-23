/**
 * `useAudioLink` 里两个**纯归约**（hook 的状态推进全靠它们）。
 *
 * 为什么单测这两个函数、而不是整个 hook：整个 hook 的其余部分都是"调 ipc → setState"，
 * 那种测试只能断言"某个 mock 被调用过"，是虚的（而且要把 Tauri 运行时整个假一遍）。
 * 这两个函数不一样 —— 它们只吃数据、只吐数据，错了的后果都是**界面上看不出来**的：
 * 曲线冻结、卡片不更新。
 */
import { describe, expect, it } from "vitest";

import type { PeerView, TelemetryRow, TelemetryView } from "../types";
import { TELEMETRY_HISTORY_LIMIT, appendTelemetryRow, upsert } from "./useAudioLink";

function view(e2eLatencyUs: number): TelemetryView {
  return {
    peers: 1,
    rttUs: 1000,
    jitterUs: 100,
    lossPct: 0,
    bitrateBps: 320000,
    bufferLevelUs: 20000,
    underruns: 0,
    e2eLatencyUs,
    e2eP50Us: e2eLatencyUs,
    e2eP95Us: e2eLatencyUs,
  };
}

function row(e2eLatencyUs: number): TelemetryRow {
  return { atUnixMs: e2eLatencyUs, ...view(e2eLatencyUs) };
}

describe("appendTelemetryRow（曲线与 CSV 导出的数据源）", () => {
  it("新采样点追加在尾部：曲线从左到右是「越来越近」", () => {
    const history = [row(1), row(2)];
    const next = appendTelemetryRow(history, view(3));

    expect(next.map((item) => item.e2eLatencyUs)).toEqual([1, 2, 3]);
    // 不改输入：hook 里直接改旧数组，React 拿不到新引用就不会重渲染
    expect(history.map((item) => item.e2eLatencyUs)).toEqual([1, 2]);
  });

  it("超过上限后丢的是**最旧**的点（丢错方向 = 曲线冻结在最初 5 分钟）", () => {
    // 改坏：把 slice(next.length - LIMIT) 写成 slice(0, LIMIT) → 曲线永远停在最开始那段，
    // 新数据进不来，而界面一切"正常"（不报错、不空白，只是不动）。
    const full = Array.from({ length: TELEMETRY_HISTORY_LIMIT }, (_, index) => row(index));
    const next = appendTelemetryRow(full, view(9999));

    expect(next).toHaveLength(TELEMETRY_HISTORY_LIMIT);
    expect(next[0]?.e2eLatencyUs).toBe(1); // 第 0 个（最旧）被丢掉
    expect(next.at(-1)?.e2eLatencyUs).toBe(9999); // 新点必须在
  });

  it("窗口长度 = 600 点 × 500 ms = 5 分钟（改窗口要连同注释与界面文案一起改）", () => {
    // 这是**用户能感知的契约**（docs 里写明"最多 5 分钟曲线"）：改这个数字必须是有意的。
    expect(TELEMETRY_HISTORY_LIMIT).toBe(600);
  });
});

describe("upsert（对端列表的乐观更新）", () => {
  function peer(idShort: string, over: Partial<PeerView> = {}): PeerView {
    return {
      idShort,
      name: "peer-" + idShort,
      addr: "192.168.1.23:58290",
      state: "idle",
      capabilities: null,
      ...over,
    };
  }

  it("同一台设备覆盖、不追加（否则卡片会重复出现）", () => {
    const before = [peer("aaaa", { state: "streaming" })];
    const after = upsert(before, peer("aaaa", { state: "idle" }));

    expect(after).toHaveLength(1);
    expect(after[0]?.state).toBe("idle");
  });

  it("就地覆盖：位置不变（新状态不该让卡片在网格里跳来跳去）", () => {
    const before = [peer("aaaa"), peer("bbbb"), peer("cccc")];
    const after = upsert(before, peer("bbbb", { state: "streaming" }));

    expect(after.map((item) => item.idShort)).toEqual(["aaaa", "bbbb", "cccc"]);
    expect(after[1]?.state).toBe("streaming");
  });

  it("新设备追加到末尾", () => {
    const after = upsert([peer("aaaa")], peer("zzzz"));
    expect(after.map((item) => item.idShort)).toEqual(["aaaa", "zzzz"]);
  });

  it("返回新数组、绝不改输入（原地改 = 点了没反应，且不报错）", () => {
    // 改坏：写成 peers[index] = peer 或 peers.push(...) → 引用不变，
    // React 认为状态没变 → 卡片停在旧状态（"点了连接没反应"这一类最难查的现象）。
    const before = [peer("aaaa", { state: "idle" })];
    const after = upsert(before, peer("aaaa", { state: "streaming" }));

    expect(after).not.toBe(before);
    expect(before[0]?.state).toBe("idle");
  });
});
