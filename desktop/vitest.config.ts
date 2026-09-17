import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

/**
 * 测试配置（vitest）。
 *
 * 为什么不复用 `vite.config.ts`：那份配置带着 tailwind 插件与 Tauri dev server 的固定端口
 * （1420，`strictPort`）—— 测试既不渲染样式、也不起 dev server，带上只会更慢更容易互相绊。
 * 两边共享的是**同一套 tsconfig 与同一份源码**，那才是要紧的部分。
 *
 * 为什么要 jsdom：组件测试要真实 DOM（`getByRole` 的可访问名、`disabled` 属性、
 * `role="alert"` 都要浏览器语义）。纯函数测试跑在 jsdom 上也不吃亏（它是进程内的）。
 */
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["src/test/setup.ts"],
    // 用例里 spy 的还原由 vitest 负责，避免 "上一个用例的 spy 还在" 这类串味
    restoreMocks: true,
  },
});
