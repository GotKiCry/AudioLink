/**
 * 临时同步组面板（`GroupPanel`）—— M3 交付物 4 的 UI 侧。
 *
 * 引擎侧（group_management / group_join / group_multi / group_sync_under_loss）已经把
 * "组怎么建、成员怎么进"测透了；这里要钉的是**界面这一层自己的判断**：
 *   · 谁有资格进组（没会话的不行、没协商完的不行、不支持排播的不行）；
 *   · 建组时到底提交了哪些成员、提前量取的是哪个值；
 *   · 成员质量分级的**上色与文案**（尤其"还没有时钟估计"这种"看着正常其实最差"的情形）。
 *
 * 每条用例都写了"改坏什么会让它红"；说不出来的断言一律没写。
 */
import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { GroupMemberView, GroupView, PeerCapabilitiesView, PeerView } from "../types";
import { t } from "../i18n";
import { GroupPanel } from "./GroupPanel";

/** §13 能力协商结果：`group_epoch` 是「组内排播」，建组的硬前提。 */
function caps(groupEpoch: boolean): PeerCapabilitiesView {
  return {
    local: "组内排播",
    peer: groupEpoch ? "组内排播" : "无",
    agreed: groupEpoch ? "组内排播" : "无",
    missingOnPeer: groupEpoch ? [] : ["组内排播"],
    agreedKeys: groupEpoch ? ["group_epoch"] : [],
    missingKeys: groupEpoch ? [] : ["group_epoch"],
  };
}

function peer(idShort: string, over: Partial<PeerView> = {}): PeerView {
  return {
    idShort,
    name: "device-" + idShort,
    addr: "192.168.1.23:58290",
    state: "streaming",
    capabilities: caps(true),
    ...over,
  };
}

function member(idShort: string, quality: string, offsetUs: number | null): GroupMemberView {
  return { idShort, quality, offsetUs };
}

function group(groupId: number, members: GroupMemberView[], over: Partial<GroupView> = {}): GroupView {
  return { groupId, epochId: "9007199254740993", leadMs: 120, members, ...over };
}

function panel(over: { peers?: PeerView[]; groups?: GroupView[]; busy?: boolean } = {}) {
  const handlers = { onRefresh: vi.fn(), onCreate: vi.fn(), onJoin: vi.fn(), onLeave: vi.fn() };
  render(
    <GroupPanel
      peers={over.peers ?? []}
      groups={over.groups ?? []}
      busy={over.busy ?? false}
      {...handlers}
    />,
  );
  return handlers;
}

/** 按设备名/短码找到它所在的那一行（不依赖列表顺序）。 */
function rowOf(label: string): HTMLElement {
  const row = screen.getByText(label, { exact: false }).closest("li");
  if (row === null) {
    throw new Error("找不到这一行：" + label);
  }
  return row;
}

describe("谁有资格进组（可勾选列表的准入）", () => {
  it("还没有会话的设备（state=idle）不出现在可选列表里", () => {
    // 改坏：去掉 `state !== "idle"` 过滤 → 用户能把一台没建立会话的设备勾进组，
    // 提交后引擎只能回一句失败，而界面上看不出为什么。
    panel({ peers: [peer("aaaa0001", { state: "idle", name: "idle-device" })] });

    expect(screen.queryByText("idle-device")).toBeNull();
    expect(screen.getByText(t("group.no_peers"))).toBeTruthy();
  });

  it("能力还没协商完（capabilities=null）→ 勾选框禁用，并说「等待能力协商」（等 ≠ 不支持）", () => {
    // 改坏：把两类"不能建"合并成同一句文案 → 用户分不清"等一下就好"还是"这台永远不行"。
    panel({ peers: [peer("aaaa0001", { capabilities: null })] });

    const row = rowOf("device-aaaa0001");
    expect((within(row).getByRole("checkbox") as HTMLInputElement).disabled).toBe(true);
    expect(within(row).getByText(t("group.peer_negotiating"))).toBeTruthy();
  });

  it("对端不支持组内排播 → 勾选框禁用，并说「对端不支持组内排播」", () => {
    panel({ peers: [peer("aaaa0001", { capabilities: caps(false) })] });

    const row = rowOf("device-aaaa0001");
    expect((within(row).getByRole("checkbox") as HTMLInputElement).disabled).toBe(true);
    expect(within(row).getByText(t("group.peer_no_epoch"))).toBeTruthy();
    expect(within(row).queryByText(t("group.peer_negotiating"))).toBeNull();
  });

  it("支持组内排播 → 可勾选，且不给任何「用不了」的提示", () => {
    panel({ peers: [peer("aaaa0001")] });

    const row = rowOf("device-aaaa0001");
    expect((within(row).getByRole("checkbox") as HTMLInputElement).disabled).toBe(false);
    expect(within(row).queryByText(t("group.peer_no_epoch"))).toBeNull();
    expect(within(row).queryByText(t("group.peer_negotiating"))).toBeNull();
  });
});

describe("勾选成组", () => {
  it("一台都没勾时「建组」禁用（不让用户打一次必然失败的往返）", () => {
    // 改坏：去掉 `selected.length === 0` → 空组也会发一次 create_group，失败原因还要占一条错误横幅。
    panel({ peers: [peer("aaaa0001"), peer("bbbb0002")] });

    const create = screen.getByRole("button", { name: t("group.create", { count: 0 }) }) as HTMLButtonElement;
    expect(create.disabled).toBe(true);
  });

  it("勾选两台后建组：提交的**恰好是这两台**（不是全部可用设备）与当前提前量", () => {
    // 改坏：onClick 里传 `selectable`（全部可用）或 `peers` → 用户只想拉两台进组，
    // 结果是所有在线设备都被拉进来。
    const handlers = panel({ peers: [peer("aaaa0001"), peer("bbbb0002"), peer("cccc0003")] });

    fireEvent.click(within(rowOf("device-aaaa0001")).getByRole("checkbox"));
    fireEvent.click(within(rowOf("device-cccc0003")).getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: t("group.create", { count: 2 }) }));

    expect(handlers.onCreate).toHaveBeenCalledWith(["aaaa0001", "cccc0003"], 120);
  });

  it("提前量取用户输入的值（不是写死 120）", () => {
    // 改坏：onClick 里传常量 → 提前量输入框变成摆设，排播余量永远按默认值给。
    const handlers = panel({ peers: [peer("aaaa0001")] });

    fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "450" } });
    fireEvent.click(within(rowOf("device-aaaa0001")).getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: t("group.create", { count: 1 }) }));

    expect(handlers.onCreate).toHaveBeenCalledWith(["aaaa0001"], 450);
  });

  it("取消勾选后回到「建组禁用」（选择是可见、可逆的）", () => {
    const handlers = panel({ peers: [peer("aaaa0001")] });

    const checkbox = within(rowOf("device-aaaa0001")).getByRole("checkbox");
    fireEvent.click(checkbox);
    fireEvent.click(checkbox);

    expect((screen.getByRole("button", { name: t("group.create", { count: 0 }) }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: t("group.create", { count: 0 }) }));
    expect(handlers.onCreate).not.toHaveBeenCalled();
  });

  it("建组进行中（busy）→ 按钮禁用并显示「处理中…」（防连点建出两个组）", () => {
    panel({ peers: [peer("aaaa0001")], busy: true });

    const create = screen.getByRole("button", { name: t("group.busy") }) as HTMLButtonElement;
    expect(create.disabled).toBe(true);
  });
});

describe("成员退出与加入", () => {
  it("逐成员退出：把**这台成员**与**这个组**一起交上去", () => {
    // 改坏：onLeave 少传/传错 groupId（例如传下标）→ 用户点了 A 组的设备，B 组掉了一台。
    const handlers = panel({
      peers: [peer("aaaa0001")],
      groups: [group(7, [member("aaaa0001", "good", 42), member("bbbb0002", "good", 43)])],
    });

    fireEvent.click(within(rowOf("fp:bbbb0002")).getByRole("button", { name: t("group.leave") }));
    expect(handlers.onLeave).toHaveBeenCalledWith("bbbb0002", 7);
  });

  it("「+ 设备名」只列**还没进这个组**的可选设备，点了就带着组号加入", () => {
    // 改坏：过滤条件写反 → 已经在组里的设备也出现「+」按钮，点下去是重复加入。
    const handlers = panel({
      peers: [peer("aaaa0001"), peer("bbbb0002")],
      groups: [group(7, [member("aaaa0001", "good", 42)])],
    });

    const groupRow = screen.getByText(t("group.summary", { id: 7, count: 1, lead: 120 })).closest("li");
    if (groupRow === null) {
      throw new Error("找不到组条目");
    }
    expect(within(groupRow).queryByRole("button", { name: "+ device-aaaa0001" })).toBeNull();

    fireEvent.click(within(groupRow).getByRole("button", { name: "+ device-bbbb0002" }));
    expect(handlers.onJoin).toHaveBeenCalledWith("bbbb0002", 7);
  });

  it("没有可加入的设备时，组里不出现任何「+」按钮", () => {
    panel({ peers: [], groups: [group(7, [member("aaaa0001", "good", 42)])] });

    expect(screen.queryByRole("button", { name: /^\+ / })).toBeNull();
  });
});

describe("成员同步质量分级的上色（§6.5）", () => {
  it("good：绿字 + 人话「同步质量良好」+ 时钟偏移 tooltip", () => {
    // 改坏：把 t("group.good") 换回 member.quality → 中文界面里蹦出一个英文原词 good，红。
    panel({ peers: [], groups: [group(7, [member("aaaa0001", "good", 42)])] });

    const badge = screen.getByTitle(t("group.offset", { us: 42 }));
    expect(badge.className).toContain("text-success");
    expect(badge.textContent).toBe(t("group.good"));
    expect(badge.textContent).not.toBe("good");
  });

  it("fair：琥珀色 + 人话「同步质量一般」", () => {
    panel({ peers: [], groups: [group(7, [member("aaaa0001", "fair", 900)])] });

    const badge = screen.getByTitle(t("group.offset", { us: 900 }));
    expect(badge.className).toContain("text-caution");
    expect(badge.textContent).toBe(t("group.fair"));
    expect(badge.textContent).not.toBe("fair");
  });

  it("poor + 没有时钟估计：红字 + **人话**「同步质量差」+ tooltip「还没有时钟估计」", () => {
    // 这条钉的是第 96 轮点名的那种"看着正常其实最差"的情形：
    // 引擎侧对没有时钟估计的成员**按 Poor 处理**（core/…/runtime.rs:1040 map_or(Poor)），
    // 而 UI 必须如实呈现 —— 红字、人话文案、并把"为什么还没有偏移量"说出来。
    // 改坏：① 直接把 member.quality 印在界面上 → 中文用户看到裸词 "poor"；
    //      ② 无估计时把手艺当"正常"（换个颜色/换个词）→ 用户以为链路没问题。
    panel({ peers: [], groups: [group(7, [member("aaaa0001", "poor", null)])] });

    const badge = screen.getByTitle(t("group.no_clock"));
    expect(badge.className).toContain("text-critical");
    expect(badge.textContent).toBe(t("group.poor"));
    expect(badge.textContent).not.toBe("poor");
  });

  it("三档颜色互斥（不许「全红」或「全绿」这种一刀切）", () => {
    panel({
      peers: [],
      groups: [group(7, [member("aaaa0001", "good", 10), member("bbbb0002", "fair", 20), member("cccc0003", "poor", 30)])],
    });

    const good = screen.getByTitle(t("group.offset", { us: 10 }));
    const fair = screen.getByTitle(t("group.offset", { us: 20 }));
    const poor = screen.getByTitle(t("group.offset", { us: 30 }));

    expect(good.className).not.toContain("text-critical");
    expect(good.className).not.toContain("text-caution");
    expect(fair.className).not.toContain("text-success");
    expect(poor.className).not.toContain("text-success");
    // 分级词本身也要能区分（不只靠颜色 —— UI 规格 §5）
    expect([good.textContent, fair.textContent, poor.textContent]).toEqual([
      t("group.good"),
      t("group.fair"),
      t("group.poor"),
    ]);
  });

  it("未知档位：不涂绿、不冒充 good，而是中性色 + 把原值显式写出来", () => {
    // 契约里 quality 是 string（引擎目前穷举 good/fair/poor），但"将来引擎加一档"是迟早的事。
    // 改坏：把兜底改回绿色（或沿用 t("group.good")）→ 一个**未知**质量被显示成"良好"，
    // 不报错、不留痕。这条断言就是那个兜底的钉子。
    panel({ peers: [], groups: [group(7, [member("aaaa0001", "unknown", 42)])] });

    const badge = screen.getByTitle(t("group.offset", { us: 42 }));
    expect(badge.className).not.toContain("text-success");
    expect(badge.className).toContain("text-text-tertiary");
    expect(badge.textContent).toBe(t("group.quality_unknown", { quality: "unknown" }));
    expect(badge.textContent).toContain("unknown");
    expect(badge.textContent).not.toBe(t("group.good"));
  });

  it("未知档位不是对某个特定值的特判：换个没见过的词也一样，且照常给「还没有时钟估计」", () => {
    panel({ peers: [], groups: [group(7, [member("aaaa0001", "excellent", null)])] });

    const badge = screen.getByTitle(t("group.no_clock"));
    expect(badge.className).not.toContain("text-success");
    expect(badge.textContent).toContain("excellent");
  });

  it("组摘要与 epoch 原样展示：u64 的 epochId 不做数字转换（不丢精度）", () => {
    // 改坏：把 epochId 转成 Number → 9007199254740993 会显示成 …92（对不上引擎侧账本，
    // 而这正是 types.ts 把 epochId 定成 string 的原因）。
    panel({ peers: [], groups: [group(7, [member("aaaa0001", "good", 42)], { epochId: "9007199254740993" })] });

    expect(screen.getByText(t("group.summary", { id: 7, count: 1, lead: 120 }))).toBeTruthy();
    expect(screen.getByText("epoch 9007199254740993")).toBeTruthy();
  });
});

describe("空态（用户唯一的引导）", () => {
  it("既没有可加入的设备、也还没有组 → 两句引导都要出现", () => {
    panel({});

    expect(screen.getByText(t("group.no_peers"))).toBeTruthy();
    expect(screen.getByText(t("group.none"))).toBeTruthy();
  });

  it("有可用设备但还没建组 → 不该说「还没有可加入的设备」", () => {
    // 改坏：两个空态的条件写反/合并 → 有设备时也喊"没有设备"，用户会去查连接。
    panel({ peers: [peer("aaaa0001")] });

    expect(screen.queryByText(t("group.no_peers"))).toBeNull();
    expect(screen.getByText(t("group.none"))).toBeTruthy();
  });
});
