/**
 * 提前量输入的解析（`parseLeadMs`）—— 两个面板共用的那一条规则。
 *
 * 这里钉的是**规则本身**（纯函数），两个面板的 bounds 只是它的两个实例：
 * 对齐面板 0–5000 / 默认 200，建组面板 0–2000 / 默认 120。
 * 覆盖矩阵里的每一格都对应一个真实的用户动作，注释写在每一格旁边。
 */
import { describe, expect, it } from "vitest";

import { parseLeadMs, type LeadBounds } from "./leadInput";

/** 对齐面板的域。 */
const ALIGN: LeadBounds = { min: 0, max: 5000, fallback: 200 };
/** 同步组面板的域（与对齐不同，这正是"规则共用、取值域各自独立"的验证点）。 */
const GROUP: LeadBounds = { min: 0, max: 2000, fallback: 120 };

describe("未填：空串与非数字", () => {
  it("空串是「未填」而不是 0（清空输入框的那一刻）", () => {
    // 改坏：让空串回落成 0（或 fallback）→ 用户删掉内容后立刻冒出一个数字，
    // 而且那个数会被当成合法提前量提交（0 提前量 = 发送端来不及等到那个时刻，整段丢掉）。
    expect(parseLeadMs("", ALIGN)).toEqual({ value: 200, valid: false });
    expect(parseLeadMs("", GROUP)).toEqual({ value: 120, valid: false });
  });

  it("只有空白字符也算未填（用户粘进来一串空格）", () => {
    expect(parseLeadMs("   ", ALIGN).valid).toBe(false);
    expect(parseLeadMs("\t\n", GROUP).valid).toBe(false);
  });

  it("非数字不合法（`type=number` 输入框在编辑中途会留下这种值）", () => {
    expect(parseLeadMs("-", ALIGN).valid).toBe(false);
    expect(parseLeadMs("1e", ALIGN).valid).toBe(false);
    expect(parseLeadMs("十二", GROUP).valid).toBe(false);
  });

  it("`valid: false` 时给的是 fallback，但它只是回填候选，不是可提交的值", () => {
    // 这条是语义边界：值里带着 fallback 是为了"恢复默认"这类用途，
    // 调用方必须看 `valid` 决定要不要提交 —— 所以两者在同一个结果里显式分开。
    const parse = parseLeadMs("", ALIGN);
    expect(parse.value).toBe(ALIGN.fallback);
    expect(parse.valid).toBe(false);
  });
});

describe("合法值：原样通过（含 0）", () => {
  it("正常值原样通过", () => {
    expect(parseLeadMs("450", ALIGN)).toEqual({ value: 450, valid: true });
    expect(parseLeadMs("600", GROUP)).toEqual({ value: 600, valid: true });
  });

  it("0 是合法提前量（清空 ≠ 填 0）", () => {
    // 改坏：把 0 与空串合并处理（都用 falsy 判断）→ "填 0" 会被当成"没填"，
    // 用户明确想给 0 的时候反而被拦住。
    expect(parseLeadMs("0", ALIGN)).toEqual({ value: 0, valid: true });
    expect(parseLeadMs("0", GROUP)).toEqual({ value: 0, valid: true });
  });

  it("两侧边界值都在域内", () => {
    expect(parseLeadMs(String(ALIGN.min), ALIGN)).toEqual({ value: 0, valid: true });
    expect(parseLeadMs(String(ALIGN.max), ALIGN)).toEqual({ value: 5000, valid: true });
    expect(parseLeadMs(String(GROUP.max), GROUP)).toEqual({ value: 2000, valid: true });
  });

  it("带前后空格的原文照样解析（从别处粘进来的数）", () => {
    expect(parseLeadMs("  450  ", ALIGN)).toEqual({ value: 450, valid: true });
  });
});

describe("超范围：收敛到各自的域（`min`/`max` 属性拦不住手打）", () => {
  it("超上限 → clamp 到 max", () => {
    expect(parseLeadMs("99999", ALIGN)).toEqual({ value: 5000, valid: true });
    expect(parseLeadMs("99999", GROUP)).toEqual({ value: 2000, valid: true });
  });

  it("负数 → clamp 到 min", () => {
    expect(parseLeadMs("-50", ALIGN)).toEqual({ value: 0, valid: true });
    expect(parseLeadMs("-50", GROUP)).toEqual({ value: 0, valid: true });
  });

  it("小数先四舍五入再 clamp（不会把 199.6 变成 199）", () => {
    expect(parseLeadMs("199.6", ALIGN)).toEqual({ value: 200, valid: true });
    expect(parseLeadMs("4999.9", ALIGN)).toEqual({ value: 5000, valid: true });
  });
});

describe("两个面板的域互相独立", () => {
  it("同一个输入在两套 bounds 下得到各自的结果（规则共用 ≠ 约束共用）", () => {
    // 改坏：把两个面板的域强行统一成一套 → 对齐面板能填 5000，建组面板也会跟着放开到 5000，
    // 而建组面板的域是建组契约的一部分。
    expect(parseLeadMs("3000", ALIGN)).toEqual({ value: 3000, valid: true });
    expect(parseLeadMs("3000", GROUP)).toEqual({ value: 2000, valid: true });
  });

  it("fallback 也各自独立（回填候选值来自调用方给的域）", () => {
    expect(parseLeadMs("", ALIGN).value).toBe(200);
    expect(parseLeadMs("", GROUP).value).toBe(120);
  });
});
