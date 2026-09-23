/**
 * 遥测面板（`TelemetryPanel`）+ 曲线的唯一数学（`sparklinePoints`）。
 *
 * docs/25 §3 自认「真实窗口里的曲线观感未验证」。窗口观感在这里验不了，但**曲线的数学**能：
 * 归一化基准、零点/峰值落点、点数与采样点的对应关系 —— 这些错了曲线只是"形状怪一点"，
 * 在屏幕上没人看得出来，正是最该用测试兜住的一类。
 */
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { TelemetryRow, TelemetryView } from "../types";
import { bpsToKbps, usToMs } from "../types";
import { t } from "../i18n";
import { TelemetryPanel, sparklinePoints } from "./TelemetryPanel";

function view(over: Partial<TelemetryView> = {}): TelemetryView {
  return {
    peers: 1,
    rttUs: 12000,
    jitterUs: 800,
    lossPct: 0.15,
    bitrateBps: 320000,
    bufferLevelUs: 40000,
    underruns: 0,
    e2eLatencyUs: 45000,
    e2eP50Us: 42000,
    e2eP95Us: 61000,
    ...over,
  };
}

function row(over: Partial<TelemetryView> = {}): TelemetryRow {
  return { atUnixMs: 1758000000000, ...view(over) };
}

function panel(over: { telemetry?: TelemetryView | null; history?: TelemetryRow[] } = {}) {
  const { container } = render(
    <TelemetryPanel
      telemetry={over.telemetry === undefined ? view() : over.telemetry}
      history={over.history ?? [row()]}
      open
      exporting={false}
      onExport={vi.fn()}
    />,
  );
  return { container };
}

describe("sparklinePoints（曲线唯一的数学）", () => {
  it("峰值贴顶（y=2）、零值贴底（y=30）、点数与采样点数一致", () => {
    // 改坏：把 30 - (value / peak) * 28 写成别的系数 / 忘了乘峰值比例 →
    // 曲线要么贴成一条直线，要么冲出画布（viewBox 之外看不见，界面不报错）。
    expect(sparklinePoints([0, 50, 100]).split(" ")).toEqual(["0.00,30.00", "50.00,16.00", "100.00,2.00"]);
  });

  it("横轴按序号均匀铺满 0–100（用比例，不是绝对值）", () => {
    const xs = sparklinePoints([1, 2, 3, 4, 5])
      .split(" ")
      .map((point) => point.split(",")[0]);
    expect(xs).toEqual(["0.00", "25.00", "50.00", "75.00", "100.00"]);
  });

  it("不足两个点、或窗口内全零 → 画不出来（空串），不画一条假线", () => {
    // 全零窗口在"没在推流"时很常见：画出来是一条贴底的直线，看着像"延迟 0 ms 很稳"。
    expect(sparklinePoints([])).toBe("");
    expect(sparklinePoints([5])).toBe("");
    expect(sparklinePoints([0, 0, 0])).toBe("");
  });

  it("负值/单点峰值不产生 NaN（NaN 会让整条 polyline 消失）", () => {
    expect(sparklinePoints([0, 10])).not.toContain("NaN");
  });
});

describe("遥测面板的关键路径", () => {
  it("端到端尚未测量不应隐藏已测到的网络延迟与音频丢包率", () => {
    panel({ telemetry: view({ e2eLatencyUs: 0, e2eP50Us: 0, e2eP95Us: 0, receiverReport: true }) });
    expect(screen.getByText("12.0")).toBeTruthy();
    expect(screen.getByText("0.15")).toBeTruthy();
    expect(screen.getByText(t("quality.e2e_unavailable"))).toBeTruthy();
    expect(screen.queryByText(t("tm.idle"))).toBeNull();
  });

  it("没有会话（peers=0）→ 显示「未推流」提示，且数字格子给「—」而不是 0.0 ms 的假数字", () => {
    // 改坏：去掉 idle 判断 → 空闲时显示 "0.0 ms / 0 kbps"，看着像"链路完美"，
    // 而实际上根本没有会话（这正是遥测面板最容易骗人的地方）。
    panel({ telemetry: view({ peers: 0, e2eLatencyUs: 0 }), history: [] });

    expect(screen.getByText(t("tm.idle"))).toBeTruthy();
    expect(screen.queryByText("0.0")).toBeNull();
  });

  it("有会话 → 数字按口径换算显示（µs→ms 一位小数、bps→kbps）", () => {
    panel({ telemetry: view({ e2eLatencyUs: 45000, bitrateBps: 320000 }) });

    expect(screen.getByText(usToMs(45000))).toBeTruthy();
    expect(screen.getByText(bpsToKbps(320000))).toBeTruthy();
  });

  it("没有历史采样点时导出按钮禁用（不让用户导出一个空 CSV）", () => {
    panel({ history: [] });

    const exportButton = screen.getByRole("button", { name: t("tm.export", { count: 0 }) }) as HTMLButtonElement;
    expect(exportButton.disabled).toBe(true);
  });

  it("历史 ≥2 点时四条曲线都画出来", () => {
    // 改坏：把 ready 门槛写成 length >= 1 → 单点也会画（x 除以 0 → NaN → 整条线消失）。
    const { container } = panel({ history: [row({ e2eLatencyUs: 30000 }), row({ e2eLatencyUs: 45000 })] });

    expect(container.querySelectorAll("polyline")).toHaveLength(4);
    expect(container.textContent).not.toContain("NaN");
  });

  it("只有 1 个采样点时曲线区显示「等待采样」而不是空白", () => {
    const { container } = panel({ history: [row()] });

    expect(container.querySelectorAll("polyline")).toHaveLength(0);
    expect(screen.getAllByText(t("tm.waiting")).length).toBeGreaterThan(0);
  });

  it("折叠（open=false）时整块不渲染（不占位、不留一个空面板）", () => {
    const { container } = render(
      <TelemetryPanel telemetry={view()} history={[row()]} open={false} exporting={false} onExport={vi.fn()} />,
    );

    expect(container.firstChild).toBeNull();
  });
});
