/**
 * 面板外壳：一块「卡片 + 标题 + 可选操作区」的格子（诊断抽屉里的那些面板都用它）。
 *
 * 为什么收成一个组件：7 个面板此前各自复制同一套壳 —— `<section className="al-card p-3">`
 * + `<header>` + `<h2 id="…-heading">` + `aria-labelledby` 三件套抄了 7 遍。复制走样的代价
 * 不只是多打几个字：标题排版写在 7 个地方，于是出现了同一个元素同时挂两个字号类
 * （`text-caption … text-body`）的写法 —— 实际渲染由 Tailwind 产物里的**类出现顺序**决定，
 * 而不是书写意图。收进一处之后，「面板标题长什么样」只有一个答案。
 *
 * 契约：
 * - `id` 是**标题 id 的前缀**：h2 的 id = `` `${id}-heading` ``，section 用 `aria-labelledby` 指向它。
 *   每个 id 在页面内必须唯一 —— 重复 id 会让 `aria-labelledby` 解析到第一个元素上。
 * - `title` / `hint` 传**已翻译的文案**（外壳不碰 i18n，也不做拼接）。
 * - `sectionId` 只在外部需要引用这块 section 时才给（例：状态条按钮的 `aria-controls="telemetry-panel"`）。
 * - 标题排版只有**一处字号 + 一处颜色**：`text-body`(14px) + `text-text-secondary`。
 *   不要在这里再追加第二套字号/颜色类：utilities 之间没有"谁更具体"，只有"谁在后面"。
 *
 * 材质沿用 `.al-card`（填充、描边、模糊都由 index.css 的语义 token 决定）；本组件不新增
 * index.css 类，也不判断明暗 —— 明暗是 index.css 变量的事。
 */

import type { ReactNode } from "react";

interface PanelProps {
  /** 标题 id 前缀：h2 的 id = `${id}-heading`，并作为 section 的 aria-labelledby。 */
  id: string;
  /** 面板标题（已翻译）。 */
  title: string;
  /** 标题**同一行**的读数/标记（对端数、文件大小、结论灯…）。 */
  meta?: ReactNode;
  /** 标题下方的说明段（与标题同一缩进块）。整行独占的提示写在 children 里。 */
  hint?: ReactNode;
  /** 靠右的操作区（按钮、忙碌灯…）。容器已带 `ml-auto shrink-0`，调用方不必再写。 */
  right?: ReactNode;
  /**
   * header 的垂直对齐。
   *
   * `center`（默认）给单行头部（标题行 + 一个按钮）；
   * `start` 给**左侧不止一行**的头部（标题 + hint 段）—— 那时按钮要跟标题顶对齐，
   * 否则它会浮在说明段的垂直中点，看起来像另一组的元素。
   */
  align?: "start" | "center";
  /** section 自身的 id（仅当外部要引用这块 section 时给）。 */
  sectionId?: string;
  children: ReactNode;
}

export function Panel({
  id,
  title,
  meta,
  hint,
  right,
  align = "center",
  sectionId,
  children,
}: PanelProps) {
  const headingId = `${id}-heading`;

  return (
    <section id={sectionId} aria-labelledby={headingId} className="al-card p-3">
      <header
        className={`flex flex-wrap gap-x-3 gap-y-2 ${
          align === "start" ? "items-start" : "items-center"
        }`}
      >
        <div className="min-w-0">
          {/* 标题与它的读数同排（`items-baseline`）：换行时读数跟着走，不会掉到标题下方 */}
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
            <h2 id={headingId} className="text-body font-semibold text-text-secondary">
              {title}
            </h2>
            {meta}
          </div>
          {hint === undefined ? null : (
            <p className="mt-0.5 text-caption leading-4 text-text-tertiary">{hint}</p>
          )}
        </div>
        {right === undefined ? null : (
          <div className="ml-auto flex shrink-0 items-center gap-2">{right}</div>
        )}
      </header>
      {children}
    </section>
  );
}
