import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { WindowControls } from "./WindowControls";

describe("窗口控件（沉浸式顶栏）", () => {
  it("非 Tauri 环境（浏览器预览 / jsdom）一个按钮都不渲染 —— 那里窗口归浏览器管", () => {
    // 改坏：去掉 isTauriRuntime 判断 → jsdom 里渲染出三个点了也没反应的死按钮。
    const { container } = render(<WindowControls />);
    expect(container.querySelector(".al-caption")).toBeNull();
  });
});
