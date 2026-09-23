/**
 * i18n 的运行时语义（`src/i18n.ts`）。
 *
 * 键集合与占位符一致性已经由 `scripts/check-i18n.mjs` 护栏把关 —— 这里**不重复**它，
 * 只钉护栏看不到的三件事：
 *   ① 占位符的**运行时**替换语义（缺变量时保留占位符，而不是显示 undefined）；
 *   ② 切语言能真的驱动界面重渲染（`t` 是模块级函数，靠 useLocale 订阅）；
 *   ③ 系统语言探测的回落规则。
 */
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { LOCALES, currentLocale, detectLocale, setLocale, t, useLocale } from "./i18n";

describe("t 的占位符语义", () => {
  it("缺变量时把占位符原样留在文案里（绝不显示 undefined）", () => {
    // 改坏：把 hasOwnProperty 判断删掉 → 中文用户看到「本机「undefined」」这种整句崩坏。
    setLocale("zh-CN");
    expect(t("status.local")).toContain("{name}");
    expect(t("status.local", {})).toContain("{name}");
    expect(t("status.local", {})).not.toContain("undefined");

    setLocale("en-US");
    expect(t("status.local", {})).toContain("{name}");
  });

  it("给全变量时替换干净（结果里不残留花括号）", () => {
    const text = t("status.streaming", { count: 12 });
    expect(text).toContain("12");
    expect(text).not.toContain("{count}");
  });

  it("数字变量按十进制字符串化（不出现 0.30000000000000004 之类）", () => {
    expect(t("status.streaming", { count: 59 })).toContain("59");
    expect(t("toast.gain", { peer: "fp-1", pct: Math.round(0.35 * 100) })).toContain("35");
  });
});

describe("语言切换", () => {
  it("切语言后同一个键给出另一种语言的文案", () => {
    setLocale("zh-CN");
    const zh = t("state.streaming");
    setLocale("en-US");
    const en = t("state.streaming");
    expect(zh).not.toBe(en);
    expect(currentLocale()).toBe("en-US");
  });

  it("useLocale 会收到切换通知（否则界面切了语言、文案全都不动）", () => {
    setLocale("zh-CN");
    const { result } = renderHook(() => useLocale());
    expect(result.current).toBe("zh-CN");

    act(() => setLocale("en-US"));
    expect(result.current).toBe("en-US");
  });

  it("切到当前语言是空操作：不通知订阅者（避免整棵树白重渲染一次）", () => {
    setLocale("en-US");
    let renders = 0;
    renderHook(() => {
      renders += 1;
      return useLocale();
    });
    const before = renders;

    act(() => setLocale("en-US"));
    expect(renders).toBe(before);
  });
});

describe("系统语言探测", () => {
  it("以 zh 开头 → zh-CN；其余语言一律回落 en-US", () => {
    const setNavigatorLanguage = (value: string): void => {
      Object.defineProperty(navigator, "language", { value, configurable: true });
    };
    try {
      setNavigatorLanguage("zh-Hans-CN");
      expect(detectLocale()).toBe("zh-CN");
      setNavigatorLanguage("zh");
      expect(detectLocale()).toBe("zh-CN");
      setNavigatorLanguage("en-GB");
      expect(detectLocale()).toBe("en-US");
      setNavigatorLanguage("ja-JP");
      expect(detectLocale()).toBe("en-US");
    } finally {
      setNavigatorLanguage("en-US");
    }
  });

  it("探测出的语言一定在语言表里有对应项（否则连回落都没有）", () => {
    const available = LOCALES.map((item) => item.value);
    expect(available).toContain(detectLocale());
    expect(available).toContain(currentLocale());
  });
});
