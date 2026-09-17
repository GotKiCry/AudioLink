import { useState } from "react";

import type { GroupView, PeerView } from "../types";
import { t } from "../i18n";

/**
 * 临时同步组面板（M3 交付物 4 / FR-22）。
 *
 * 边界：**勾选成组 + 动态加入退出**。成员同步质量展示与真机三台同时出声的验收属 M3 后续项；
 * 引擎侧的组账本与事件已经就绪，本组件只做展示与动作。
 */
interface GroupPanelProps {
  peers: PeerView[];
  groups: GroupView[];
  busy: boolean;
  onRefresh: () => void;
  onCreate: (idShorts: string[], leadMs: number) => void;
  onJoin: (idShort: string, groupId: number) => void;
  onLeave: (idShort: string, groupId: number) => void;
}

/**
 * 成员同步质量 → 颜色 + 文案。
 *
 * 三档都走 `t()`：早先只有 poor 有人话（「同步质量差」），而 fair / good 直接印英文原词，
 * 于是同一个 chip 行里一英一中 —— 中文界面上尤其刺眼。
 *
 * **为什么「不认识」的档位不涂绿**：契约里 `quality` 是 `string`（引擎目前穷举三档，
 * 见 `engine_bridge.rs` 的 `quality_name`），但"将来引擎加一档"是迟早的事：
 * 那时若 `else` 兜到绿色，界面会把一个**未知**质量显示成"良好"，而且不报错、不留痕。
 * 所以兜底是中性色 + **把原值显式写出来**（排查时一眼看到引擎到底给了什么）。
 *
 * 写成函数而不是常量表：文案在渲染期求值，切语言时才会跟着变（同 `types.ts::peerStateLabel`)。
 */
function qualityStyle(quality: string): { className: string; text: string } {
  switch (quality) {
    case "good":
      return { className: "text-emerald-400", text: t("group.good") };
    case "fair":
      return { className: "text-amber-400", text: t("group.fair") };
    case "poor":
      return { className: "text-red-400", text: t("group.poor") };
    default:
      return { className: "text-neutral-400", text: t("group.quality_unknown", { quality }) };
  }
}

export function GroupPanel({
  peers,
  groups,
  busy,
  onRefresh,
  onCreate,
  onJoin,
  onLeave,
}: GroupPanelProps) {
  const [selected, setSelected] = useState<string[]>([]);
  const [leadMs, setLeadMs] = useState(120);

  const selectable = peers.filter((peer) => peer.state !== "idle");

  /**
   * §13：建组依赖「组内排播」（`GROUP_EPOCH`）—— 对端不支持就该**选不了**，而不是选完再失败。
   *
   * 返回 `null` 表示还没协商完（握手中）：这时也禁用，但说的是「等一下」而不是「不支持」——
   * 两句话的处置方式完全不同（一个等，一个别等）。
   */
  const epochReady = (peer: (typeof peers)[number]): boolean | null =>
    peer.capabilities === null
      ? null
      : peer.capabilities.agreedKeys.includes("group_epoch");

  const toggle = (idShort: string) => {
    setSelected((current) =>
      current.includes(idShort)
        ? current.filter((id) => id !== idShort)
        : [...current, idShort],
    );
  };

  return (
    <section className="rounded-lg border border-neutral-800 bg-neutral-900/50 p-4">
      <header className="mb-3 flex items-center justify-between">
        <h2 className="text-sm font-semibold text-neutral-200">
          {t("group.title")}
        </h2>
        <button
          type="button"
          className="rounded border border-neutral-700 px-2 py-1 text-xs text-neutral-300 hover:bg-neutral-800"
          onClick={onRefresh}
        >
          {t("group.refresh")}
        </button>
      </header>

      <div className="mb-3 flex flex-wrap items-center gap-3 text-xs text-neutral-300">
        <span>{t("group.lead")}</span>
        <input
          type="number"
          min={0}
          max={2000}
          value={leadMs}
          onChange={(event) => setLeadMs(Number(event.target.value))}
          className="w-20 rounded border border-neutral-700 bg-neutral-950 px-2 py-1 text-right"
        />
        <span>ms</span>
        <button
          type="button"
          disabled={busy || selected.length === 0}
          className="rounded bg-emerald-700 px-3 py-1 text-xs font-medium text-white disabled:opacity-40"
          onClick={() => onCreate(selected, leadMs)}
        >
          {busy ? t("group.busy") : t("group.create", { count: selected.length })}
        </button>
      </div>

      {selectable.length === 0 ? (
        <p className="text-xs text-neutral-500">{t("group.no_peers")}</p>
      ) : (
        <ul className="mb-3 space-y-1 text-xs">
          {selectable.map((peer) => {
            const ready = epochReady(peer);
            return (
              <li key={peer.idShort} className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={selected.includes(peer.idShort)}
                  disabled={ready !== true}
                  onChange={() => toggle(peer.idShort)}
                />
                <span className={ready === true ? "text-neutral-200" : "text-neutral-500"}>
                  {peer.name}
                </span>
                <span className="text-neutral-500">fp:{peer.idShort}</span>
                <span className="text-neutral-500">{peer.state}</span>
                {ready === true ? null : (
                  <span className="text-amber-500">
                    {ready === null ? t("group.peer_negotiating") : t("group.peer_no_epoch")}
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      )}

      {groups.length === 0 ? (
        <p className="text-xs text-neutral-500">{t("group.none")}</p>
      ) : (
        <ul className="space-y-2">
          {groups.map((group) => (
            <li
              key={group.groupId}
              className="rounded border border-neutral-800 bg-neutral-950/60 p-2 text-xs"
            >
              <div className="mb-1 flex items-center justify-between">
                <span className="font-medium text-neutral-200">
                  {t("group.summary", { id: group.groupId, count: group.members.length, lead: group.leadMs })}
                </span>
                <span className="text-neutral-500">epoch {group.epochId}</span>
              </div>
              <ul className="flex flex-wrap gap-2">
                {group.members.map((member) => {
                  const badge = qualityStyle(member.quality);
                  return (
                    <li
                      key={member.idShort}
                      className="flex items-center gap-1 rounded bg-neutral-800 px-2 py-0.5"
                    >
                      <span className="text-neutral-200">fp:{member.idShort}</span>
                      <span
                        className={badge.className}
                        title={
                          member.offsetUs === null
                            ? t("group.no_clock")
                            : t("group.offset", { us: member.offsetUs })
                        }
                      >
                        {badge.text}
                      </span>
                      <button
                        type="button"
                        className="text-neutral-400 hover:text-red-400"
                        onClick={() => onLeave(member.idShort, group.groupId)}
                      >
                        {t("group.leave")}
                      </button>
                    </li>
                  );
                })}
              </ul>
              <div className="mt-1 flex flex-wrap gap-2">
                {selectable
                  .filter(
                    (peer) =>
                      !group.members.some((member) => member.idShort === peer.idShort),
                  )
                  .map((peer) => (
                    <button
                      key={peer.idShort}
                      type="button"
                      className="rounded border border-neutral-700 px-2 py-0.5 text-neutral-300 hover:bg-neutral-800"
                      onClick={() => onJoin(peer.idShort, group.groupId)}
                    >
                      + {peer.name}
                    </button>
                  ))}
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
