/**
 * 测试启动环境（`vitest.config.ts` 的 `setupFiles`）。
 *
 * 这里只做两件让断言**确定**的事，不塞任何"方便"的工具：
 *
 * 1. **手动 cleanup**：`globals: false` 时 RTL 不会自动卸载上一次渲染的组件，
 *    不清理就会出现「上一个用例的按钮还在 DOM 里」→ `getByRole` 撞成多个匹配，红得莫名其妙。
 * 2. **把界面语言复位成 en-US**：i18n 是模块级单例，默认跟随 `navigator.language`
 *    （jsdom 恰好给 en-US，但那是环境默认值，不该被当成契约）。
 *    用例里断言文案一律用 `t(key)` 生成期望值 —— 这样翻译改了不会假红，
 *    而**选错键**（例如该说"等待协商"却说成"不支持"）一定红。
 */
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

import { setLocale } from "../i18n";

afterEach(() => {
  cleanup();
  setLocale("en-US");
});
