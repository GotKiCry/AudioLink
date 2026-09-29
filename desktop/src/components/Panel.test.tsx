/**
 * 面板外壳（`Panel`）—— 钉住两件「7 个面板之间必须只有一个答案」的事。
 *
 * ① **标题排版**：字号一处、颜色一处。报告 §一.3 记的正是这里出的问题 —— 同一个元素挂
 *    `text-caption … text-body` 两个字号类时，实际渲染由 Tailwind 产物里的**类出现顺序**
 *    决定（`.text-caption` 排在 `.text-body` 之后 → 12px 胜），写在最后的那份"意图"是死代码。
 *    仓库没有 eslint/prettier，也没有类冲突检查，所以这条断言就是那个兜底。
 * ② **id 契约**：`aria-labelledby` 指向的 h2 必须存在、且页面内唯一（重复 id 会让它解析到
 *    第一个元素上 —— 读屏用户听到的标题可能不是这块面板的）。
 *
 * 断言的是「渲染出来的 DOM 长什么样」，不是「源码里怎么写的」。
 */
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AlignmentPanel } from "./AlignmentPanel";
import { GroupPanel } from "./GroupPanel";
import { Panel } from "./Panel";
import { TelemetryPanel } from "./TelemetryPanel";

/** 字号档：一个元素只该挂一个，多了就由产物顺序决定。 */
const SIZE_CLASS = /^text-(caption|body|lead|title)$/;
/** 颜色档：同理（语义色与 text-* 文字色都算）。 */
const COLOR_CLASS = /^text-(text-[a-z-]+|success|caution|critical|accent)$/;

function classes(element: Element): string[] {
  return (element.getAttribute("class") ?? "").split(/\s+/).filter(Boolean);
}

describe("标题排版只有一个答案", () => {
  it("h2 只挂一个字号类 + 一个颜色类，且是书写意图值（text-body 14px + text-text-secondary）", () => {
    render(
      <Panel id="probe" title="探针">
        <p>body</p>
      </Panel>,
    );

    const names = classes(screen.getByRole("heading", { level: 2 }));

    // 改坏：再挂一个 text-caption（或 text-text-tertiary）→ 下面两条会红，
    // 而界面上只表现为"标题小了一号、淡了一档"这种说不清哪里不对的样子。
    expect(names.filter((name) => SIZE_CLASS.test(name))).toEqual(["text-body"]);
    expect(names.filter((name) => COLOR_CLASS.test(name))).toEqual(["text-text-secondary"]);
  });

  it("真实面板的标题与外壳给出的类名逐字相同（面板不再各写各的标题排版）", () => {
    const seen: string[][] = [];

    const shell = render(
      <Panel id="probe" title="探针">
        <p />
      </Panel>,
    );
    seen.push(classes(screen.getByRole("heading", { level: 2 })));
    shell.unmount();

    const align = render(<AlignmentPanel alignment={null} busy={false} onBroadcast={vi.fn()} />);
    seen.push(classes(screen.getByRole("heading", { level: 2 })));
    align.unmount();

    const group = render(
      <GroupPanel
        peers={[]}
        groups={[]}
        busy={false}
        onRefresh={vi.fn()}
        onCreate={vi.fn()}
        onJoin={vi.fn()}
        onLeave={vi.fn()}
      />,
    );
    seen.push(classes(screen.getByRole("heading", { level: 2 })));
    group.unmount();

    const telemetry = render(
      <TelemetryPanel telemetry={null} history={[]} open exporting={false} onExport={vi.fn()} />,
    );
    seen.push(classes(screen.getByRole("heading", { level: 2 })));
    telemetry.unmount();

    expect(seen).toHaveLength(4);
    for (const names of seen) {
      expect(names).toEqual(seen[0]);
    }
  });
});

describe("id 契约（aria-labelledby 指向的标题必须真实存在）", () => {
  it("section 的 aria-labelledby = `${id}-heading`，且那个 h2 在页面里唯一", () => {
    const { container } = render(
      <Panel id="probe" title="探针" hint="说明">
        <p>body</p>
      </Panel>,
    );

    const section = container.querySelector("section");
    expect(section?.getAttribute("aria-labelledby")).toBe("probe-heading");
    expect(container.querySelectorAll("#probe-heading")).toHaveLength(1);
    expect(container.querySelector("#probe-heading")?.textContent).toBe("探针");
  });

  it("sectionId 只在显式给出时才落到 section 上（状态条要用 aria-controls 指过来）", () => {
    const named = render(
      <Panel id="probe" title="探针" sectionId="probe-readout">
        <p />
      </Panel>,
    );
    expect(named.container.querySelector("section")?.id).toBe("probe-readout");
    named.unmount();

    const plain = render(
      <Panel id="probe" title="探针">
        <p />
      </Panel>,
    );
    expect(plain.container.querySelector("section")?.hasAttribute("id")).toBe(false);
  });
});

describe("槽位与外壳材质", () => {
  it("meta 与标题同排、hint 在标题下方、right 自带 ml-auto + shrink-0", () => {
    const { container } = render(
      <Panel
        id="probe"
        title="探针"
        meta={<span data-testid="meta">3</span>}
        hint="说明"
        right={<button type="button">按钮</button>}
      >
        <p>body</p>
      </Panel>,
    );

    const heading = screen.getByRole("heading", { level: 2 });
    // 改坏：把 meta 挪到 header 之外 → 读数会掉到标题下方，换行时与标题脱开。
    expect(heading.parentElement?.contains(screen.getByTestId("meta"))).toBe(true);
    expect(classes(screen.getByText("说明"))).toContain("mt-0.5");

    const right = container.querySelector("button")?.parentElement;
    expect(classes(right as Element)).toContain("ml-auto");
    expect(classes(right as Element)).toContain("shrink-0");
  });

  it("align 决定 header 的垂直对齐：多行头部（标题 + 说明）用 start，单行用 center", () => {
    const multi = render(
      <Panel id="probe" title="探针" hint="说明" align="start">
        <p />
      </Panel>,
    );
    expect(classes(multi.container.querySelector("header") as Element)).toContain("items-start");
    multi.unmount();

    const single = render(
      <Panel id="probe" title="探针">
        <p />
      </Panel>,
    );
    expect(classes(single.container.querySelector("header") as Element)).toContain("items-center");
  });

  it("卡片材质沿用 .al-card（外壳不新增 index.css 类）", () => {
    const { container } = render(
      <Panel id="probe" title="探针">
        <p>body</p>
      </Panel>,
    );

    const section = container.querySelector("section");
    expect(classes(section as Element)).toEqual(["al-card", "p-3"]);
    expect(screen.getByText("body")).toBeTruthy();
  });
});
