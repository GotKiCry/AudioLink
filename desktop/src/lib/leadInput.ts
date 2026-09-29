/**
 * 提前量输入的解析（对齐面板与同步组面板共用）。
 *
 * 为什么单独一个文件：这两个面板此前各有一份**逐字相同**的解析逻辑，只有取值域不同
 * （对齐 0–5000 / 建组 0–2000）。同一领域规则写两遍的代价不是"多打几行"，而是漂移：
 * 改了一处忘了另一处，两个面板对「空串」「99999」「-50」的处置就会分家 ——
 * 而它们是同一件事（给排播留多少余量），用户没有理由期待两处不一样。
 *
 * 三条语义（两个面板必须完全一致，差异只在 `bounds`）：
 *
 * ① **空串 / 非数字 = 未填**（`valid: false`），既不是 0 也不是默认值。
 *    界面显示的值与真正提交的值必须一致 —— 框里空着却按默认值发出去，正是这套面板最不该有的
 *    那种"看着正常"。所以调用方拿到 `valid: false` 时应当**禁用提交**，而不是拿 `value` 顶上。
 * ② **超范围收敛到 [min, max]**（`valid: true`）：`min` / `max` 属性只参与原生校验，
 *    用户手打 99999 照样进得来，真正执行约束的是这里。
 * ③ **`0` 是合法值**（它在取值域内）——"空着"和"填 0"是两件完全不同的事，
 *    两者都会被当成 0 提交的那种实现，正是这个函数存在的理由。
 *
 * `fallback` **不是"非法输入的兜底值"**：它只是初始显示 / "恢复默认"这类用途的候选值，
 * 由调用方决定什么时候用（本仓库两个面板都把它用作输入框的初始文本）。把它拿去填非法输入，
 * 就等于把 ① 重新破坏掉。
 */

/** 提前量输入的取值域与默认值。 */
export interface LeadBounds {
  min: number;
  max: number;
  /** 默认值：只用于初始显示或"恢复默认"，**不会**用来填补非法输入。 */
  fallback: number;
}

/** 解析结果：`valid` 为 false 时 `value` 只是回填候选，**不可提交**。 */
export interface LeadParse {
  value: number;
  valid: boolean;
}

/**
 * 把输入框里的**原文**解析成提前量（ms）。
 *
 * 纯函数：同样的输入永远给同样的结果，不碰 DOM、不碰 state —— 所以它上面的每条语义
 * 都能被单测钉死，而不是靠在两个组件里各读一遍代码去确认。
 */
export function parseLeadMs(text: string, bounds: LeadBounds): LeadParse {
  const trimmed = text.trim();
  if (trimmed === "") {
    return { value: bounds.fallback, valid: false };
  }
  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed)) {
    return { value: bounds.fallback, valid: false };
  }
  return {
    value: Math.min(bounds.max, Math.max(bounds.min, Math.round(parsed))),
    valid: true,
  };
}
