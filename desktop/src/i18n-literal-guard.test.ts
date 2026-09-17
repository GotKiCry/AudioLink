/// <reference types="vite/client" />

/**
 * 护栏补齐：**组件里的裸中文字面量**。
 *
 * `scripts/check-i18n.mjs` 只看 `i18n.ts` 里两张表的键与占位符是否一致 —— 它看不见
 * "某个组件把中文直接写死在 JSX 里"。那种写法在中文界面下完全正常，一切到英文就露馅
 * （那一句永远是中文），而这正是评审最容易漏掉的一类回归。
 *
 * 判据：`src/**` 里的源码（除 `i18n.ts` 与测试文件）**剥掉注释后不允许出现汉字**。
 * 顿号、引号、"…"、"—" 这类标点不在 CJK 统一汉字区（U+4E00–U+9FFF），本来就不需要翻译。
 */
import { describe, expect, it } from "vitest";

/** 源码原文（vite 的 raw glob，不需要碰文件系统）。 */
const SOURCES = import.meta.glob("./**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const CJK = /[\u4e00-\u9fff]/;

/**
 * 剥掉 `//` 与 `/* *\/` 注释。
 *
 * 为什么要手写状态机而不是用正则：字符串里的 `//`（例如 `"https://…"`）不是注释开头，
 * 而模板串里的换行也不是。认错会导致"注释剥不干净"或"代码被吃掉"，两种情况都会让这条护栏失真。
 */
function stripComments(source: string): string {
  let out = "";
  let index = 0;
  let quote: string | null = null;

  while (index < source.length) {
    const char = source.charAt(index);
    const next = source.charAt(index + 1);

    if (quote !== null) {
      out += char;
      if (char === "\\") {
        out += next;
        index += 2;
        continue;
      }
      if (char === quote) {
        quote = null;
      }
      index += 1;
      continue;
    }

    if (char === '"' || char === "'" || char === "`") {
      quote = char;
      out += char;
      index += 1;
      continue;
    }
    if (char === "/" && next === "/") {
      while (index < source.length && source.charAt(index) !== "\n") {
        index += 1;
      }
      continue;
    }
    if (char === "/" && next === "*") {
      index += 2;
      while (index < source.length && !(source.charAt(index) === "*" && source.charAt(index + 1) === "/")) {
        index += 1;
      }
      index += 2;
      continue;
    }

    out += char;
    index += 1;
  }

  return out;
}

describe("用户可见文案只能来自 i18n（组件里不许写死中文）", () => {
  const guarded = Object.entries(SOURCES).filter(
    ([path]) => !path.includes(".test.") && !path.endsWith("i18n.ts"),
  );

  it("真的扫到了源码文件（glob 写错时这条测试不能悄悄变绿）", () => {
    expect(guarded.length).toBeGreaterThanOrEqual(15);
  });

  it("剥掉注释后没有任何汉字（有的话，说明它永远是中文，切到英文也不变）", () => {
    const offenders: string[] = [];
    for (const [path, source] of guarded) {
      stripComments(source)
        .split("\n")
        .forEach((line, index) => {
          if (CJK.test(line)) {
            offenders.push(`${path}:${index + 1} ${line.trim().slice(0, 60)}`);
          }
        });
    }

    expect(offenders).toEqual([]);
  });

  it("注释剥离器本身可信：注释里的中文要被剥掉，字符串里的 // 不能被当注释", () => {
    // 这三条是给上面那条判据兜底的 —— 剥离器认错方向，护栏就会一路变绿。
    expect(stripComments("const a = 1; // 中文注释")).not.toContain("中文注释");
    expect(stripComments("/* 中文注释 */ const a = 1;")).not.toContain("中文注释");
    expect(stripComments('const url = "https://example.com"; const b = 2;')).toContain("const b = 2;");
    expect(stripComments('const s = "还有汉字";')).toContain("还有汉字");
  });
});
