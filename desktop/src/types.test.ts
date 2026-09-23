/**
 * 契约层的纯函数（`src/types.ts`）。
 *
 * 为什么先测它：这一层是**口径转换处** —— 单位换算、错误归一化、CSV 导出列名、状态文案。
 * 它们写错不会崩、不会抛异常，只会静默给出「差不多但不对」的数字与文案，
 * 而用户与支持者都只能看到结果。每一组用例的注释都写了「改坏什么会让它红」。
 */
import { describe, expect, it } from "vitest";

import { t } from "./i18n";
import {
  PEER_STATE_STYLE,
  bpsToKbps,
  peerStateLabel,
  telemetryRowOf,
  toCommandError,
  usToMs,
  type PeerState,
  type TelemetryView,
} from "./types";

describe("toCommandError（任何 rejection 都要变成可显示的人话）", () => {
  it("完整形状原样带过来（code / message / context 一个不少）", () => {
    expect(toCommandError({ code: 1002, message: "指定的对端不存在", context: "connect" })).toEqual({
      code: 1002,
      message: "指定的对端不存在",
      context: "connect",
    });
  });

  it("只有 message 时也要能显示（更新链路只给一句人话）", () => {
    const failure = toCommandError({ message: "连不上更新服务器" });
    expect(failure.message).toBe("连不上更新服务器");
    expect(failure.code).toBe(0);
    expect(failure.context).toBe("");
  });

  it("message 不是字符串时不把 undefined 塞进界面（UI 规格 §4：错误必说人话）", () => {
    // 改坏：删掉 `typeof candidate.message === "string"` 判断 → 横幅上出现空白原因。
    const failure = toCommandError({ code: 7 });
    expect(failure.message).toBe(t("err.ipc"));
    expect(failure.context.length).toBeGreaterThan(0);
  });

  it("字符串 / 非对象 rejection：原文进 context，人话进 message", () => {
    const fromString = toCommandError("boom");
    expect(fromString.message).toBe(t("err.ipc"));
    expect(fromString.context).toBe("boom");

    const fromNull = toCommandError(null);
    expect(fromNull.message).toBe(t("err.ipc"));
    expect(fromNull.context).toBe("null");
  });
});

describe("单位换算（界面数字的口径）", () => {
  it("usToMs：µs → ms 且保留一位小数（不是整数截断）", () => {
    // 改坏：toFixed(0) 或去掉小数 → 端到端延迟一律显示成整数 ms，1.5 ms 的抖动看不见。
    expect(usToMs(0)).toBe("0.0");
    expect(usToMs(1500)).toBe("1.5");
    expect(usToMs(333)).toBe("0.3");
    expect(usToMs(19999)).toBe("20.0");
  });

  it("bpsToKbps：四舍五入到整数 kbps（截断会让码率系统性偏小）", () => {
    // 改坏：Math.floor / 整除 → 3500 bps 会显示 3 kbps、999 bps 显示 0 kbps。
    expect(bpsToKbps(0)).toBe("0");
    expect(bpsToKbps(320000)).toBe("320");
    expect(bpsToKbps(3500)).toBe("4");
    expect(bpsToKbps(999)).toBe("1");
  });
});

describe("telemetryRowOf（CSV 导出行的形状）", () => {
  const view: TelemetryView = {
    peers: 2,
    rttUs: 12000,
    jitterUs: 800,
    lossPct: 0.15,
    bitrateBps: 320000,
    bufferLevelUs: 40000,
    underruns: 3,
    e2eLatencyUs: 45000,
    e2eP50Us: 42000,
    e2eP95Us: 61000,
  };

  it("键集合恰好是契约列名（改名或漏字段 = 用户导出的 CSV 少一列）", () => {
    // 列名是**跨版本持久化**的东西：写进用户文件后再改就晚了，所以在这里钉死。
    expect(Object.keys(telemetryRowOf(view, 1758000000000)).sort()).toEqual(
      [
        "atUnixMs",
        "bitrateBps",
        "bufferLevelUs",
        "e2eLatencyUs",
        "e2eP50Us",
        "e2eP95Us",
        "jitterUs",
        "lossPct",
        "peers",
        "rttUs",
        "underruns",
      ].sort(),
    );
  });

  it("值与快照一一对应，且不共享引用（导出的行不该被后续快照改动污染）", () => {
    const row = telemetryRowOf(view, 1758000000000);
    expect(row).toEqual({ atUnixMs: 1758000000000, ...view });
    expect(row).not.toBe(view);
  });
});

describe("状态文案与样式表（UI 规格 §5：不允许只靠颜色传达状态）", () => {
  const states: PeerState[] = ["idle", "handshaking", "streaming", "degraded", "failed"];

  it("每个状态都有非空文案，且不是把 i18n 键名当文案显示", () => {
    // 改坏：映射表漏一个状态（`t(undefined)` 会让整句变 undefined），
    // 或把 key 直接当文案（界面上出现 "state.streaming"）。
    for (const state of states) {
      const label = peerStateLabel(state);
      expect(label.length).toBeGreaterThan(0);
      expect(label).not.toContain("state.");
    }
  });

  it("每个状态都有非空图标（色觉障碍用户也要能分辨状态）", () => {
    for (const state of states) {
      expect(PEER_STATE_STYLE[state].icon.trim().length).toBeGreaterThan(0);
    }
  });

  it("样式表覆盖全部状态（漏一个 → `undefined` 会被拼进 className）", () => {
    expect(Object.keys(PEER_STATE_STYLE).sort()).toEqual([...states].sort());
  });
});
