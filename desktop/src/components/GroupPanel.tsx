import { useState } from "react";

import type { GroupView, PeerView } from "../types";
import { t } from "../i18n";
import { parseLeadMs, type LeadBounds } from "../lib/leadInput";
import { Panel } from "./Panel";

/**
 * 临时同步组面板（M3 交付物 4 / FR-22）。
 *
 * 边界：**勾选成组 + 动态加入退出**。成员同步质量展示与真机三台同时出声的验收属 M3 后续项；
 * 引擎侧的组账本与事件已经就绪，本组件只做展示与动作。
 *
 * 视觉：这块面板原先整块硬编码深色（`bg-neutral-900/50` + `text-neutral-*`）—— 面板之间没有
 * 材质差别，所以它和其它面板一样只消费语义 token，深色/浅色由 `index.css` 的变量决定，
 * 组件里没有任何一处写死的明暗假设。
 * 按钮尺寸一律用 `.al-btn-*` 档位类，且不留 `h-* / px-* / text-*`：utilities 层在
 * components 层之上，留着会把档位高度盖回去。
 */
interface GroupPanelProps {
  peers: PeerView[];
  groups: GroupView[];
  busy: boolean;
  /**
   * 刷新组列表。返回 promise 时按钮会保持禁用直到刷新结束；返回 `void` 时退化为
   * "点完即解锁" —— 调用方要做真防连点，把 promise 交出来即可
   * （`onRefresh={() => al.refreshGroups()}`，而不是 `() => void al.refreshGroups()`）。
   */
  onRefresh: () => void | Promise<void>;
  onCreate: (idShorts: string[], leadMs: number) => void;
  onJoin: (idShort: string, groupId: number) => void;
  onLeave: (idShort: string, groupId: number) => void;
}

/**
 * 成员同步质量 → 颜色 + 文案。
 *
 * 三档都走 t()：早先只有 poor 有人话（「同步质量差」），而 fair / good 直接印英文原词，
 * 于是同一个 chip 行里一英一中 —— 中文界面上尤其刺眼。
 *
 * **为什么「不认识」的档位不涂绿**：契约里 quality 是 string（引擎目前穷举三档，
 * 见 engine_bridge.rs 的 quality_name），但"将来引擎加一档"是迟早的事：
 * 那时若 else 兜到绿色，界面会把一个**未知**质量显示成"良好"，而且不报错、不留痕。
 * 所以兜底是中性色 + **把原值显式写出来**（排查时一眼看到引擎到底给了什么）。
 *
 * 写成函数而不是常量表：文案在渲染期求值，切语言时才会跟着变（同 types.ts::peerStateLabel）。
 *
 * 颜色一律走语义令牌（text-success / text-caution / text-critical / text-text-tertiary）：
 * 档位靠这三档语义色区分，界面里没有任何一处写死的 Tailwind 调色板色值。
 */
function qualityStyle(quality: string): { className: string; text: string } {
  switch (quality) {
    case "good":
      return { className: "text-success", text: t("group.good") };
    case "fair":
      return { className: "text-caution", text: t("group.fair") };
    case "poor":
      return { className: "text-critical", text: t("group.poor") };
    default:
      return { className: "text-text-tertiary", text: t("group.quality_unknown", { quality }) };
  }
}

/** 建组时的默认提前量（ms）：给排播留余量（见 `group.summary` 的 lead 字段）。 */
const DEFAULT_LEAD_MS = 120;

/** 输入框的取值域：与 `min` / `max` 属性是同一条约束（属性只是提示，真正执行的是 parseLeadMs）。 */
const MIN_LEAD_MS = 0;
const MAX_LEAD_MS = 2000;

/**
 * 本面板的取值域。
 *
 * 解析规则本身（空串 = 未填、超范围收敛、0 合法）在 `lib/leadInput.ts`，与对齐面板共用一份 ——
 * 两个面板之间只有这里的数字不同（建组的域是建组契约的一部分，比对齐面板窄）。
 */
const LEAD_BOUNDS: LeadBounds = { min: MIN_LEAD_MS, max: MAX_LEAD_MS, fallback: DEFAULT_LEAD_MS };

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
  // 存用户看到的原文；`lead` 是**提交时**才算出来的（valid: false = 还没填好）
  const [leadText, setLeadText] = useState(String(DEFAULT_LEAD_MS));
  const lead = parseLeadMs(leadText, LEAD_BOUNDS);
  /** 「刷新」正在飞（与 `busy` 是两件事：建组 / 加入 / 退出也走 busy）。 */
  const [refreshing, setRefreshing] = useState(false);

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

  /**
   * 刷新按钮：连点不重复触发。
   *
   * 两道锁各管一段 —— `busy`（建组 / 加入 / 退出进行中：它们内部本来就会重拉一次列表，
   * 这时再点刷新纯属重复）与 `refreshing`（单纯点刷新时的那次飞行）。
   * `refreshing` 等的是 `onRefresh` 返回的 promise：调用方交出来就锁到刷新结束；
   * 只给 `void` 的话它锁不住跨点击的连点（那时防连点只能靠调用方自己）。
   */
  const refresh = (): void => {
    if (busy || refreshing) {
      return;
    }
    setRefreshing(true);
    void (async () => {
      try {
        await onRefresh();
      } finally {
        setRefreshing(false);
      }
    })();
  };

  return (
    <Panel
      id="group"
      title={t("group.title")}
      meta={
        <span className="flex items-baseline gap-1.5">
          <span className="text-caption font-semibold text-text-secondary">{t("tm.peers")}</span>
          <span className="num text-caption text-text-secondary">{peers.length}</span>
        </span>
      }
      right={
        <button
          type="button"
          disabled={busy || refreshing}
          className="al-btn al-btn-md text-text-primary disabled:cursor-not-allowed"
          onClick={refresh}
        >
          {t("group.refresh")}
        </button>
      }
    >
      {/* 提前量：建组与加入都按它排播，所以放在按钮旁边、同一条基线上 */}
      <div className="mt-2 mb-2 flex flex-wrap items-center gap-2 text-caption text-text-secondary">
        <label htmlFor="group-lead" className="text-caption font-semibold text-text-secondary">
          {t("group.lead")}
        </label>
        <input
          id="group-lead"
          type="number"
          min={MIN_LEAD_MS}
          max={MAX_LEAD_MS}
          value={leadText}
          onChange={(event) => setLeadText(event.target.value)}
          className="al-well num w-20 rounded-control px-2 py-1 text-right text-caption text-text-primary"
        />
        <span className="num text-text-tertiary">ms</span>
        <button
          type="button"
          // 提前量没填（空串 / 非数字）时不给建组：按钮灰着，比发一个用户没看见的值诚实
          disabled={busy || selected.length === 0 || !lead.valid}
          className="al-btn al-btn-accent al-btn-md ml-auto shrink-0 disabled:cursor-not-allowed"
          onClick={() => {
            if (lead.valid) {
              onCreate(selected, lead.value);
            }
          }}
        >
          {busy ? t("group.busy") : t("group.create", { count: selected.length })}
        </button>
      </div>

      {selectable.length === 0 ? (
        <p className="text-caption text-text-tertiary">{t("group.no_peers")}</p>
      ) : (
        <ul className="mb-3 divide-y divide-stroke-divider border-y border-stroke-control text-caption">
          {selectable.map((peer) => {
            const ready = epochReady(peer);
            return (
              <li key={peer.idShort} className="flex flex-wrap items-center gap-x-2 gap-y-1 py-1">
                <input
                  type="checkbox"
                  checked={selected.includes(peer.idShort)}
                  disabled={ready !== true}
                  onChange={() => toggle(peer.idShort)}
                  className="accent-caution disabled:cursor-not-allowed"
                />
                <span className={ready === true ? "text-text-primary" : "text-text-tertiary"}>
                  {peer.name}
                </span>
                <span className="num text-text-tertiary">fp:{peer.idShort}</span>
                <span className="text-text-tertiary">{peer.state}</span>
                {ready === true ? null : (
                  <span className="ml-auto text-caution">
                    {ready === null ? t("group.peer_negotiating") : t("group.peer_no_epoch")}
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      )}

      {groups.length === 0 ? (
        <p className="text-caption text-text-tertiary">{t("group.none")}</p>
      ) : (
        <ul className="space-y-2">
          {groups.map((group) => (
            <li
              key={group.groupId}
              className="al-well p-2 text-caption"
            >
              <div className="mb-1.5 flex flex-wrap items-baseline gap-x-3 gap-y-0.5">
                <span className="text-text-primary">
                  {t("group.summary", { id: group.groupId, count: group.members.length, lead: group.leadMs })}
                </span>
                <span className="num text-caption text-text-tertiary">epoch {group.epochId}</span>
              </div>
              <ul className="flex flex-wrap gap-1.5">
                {group.members.map((member) => {
                  const badge = qualityStyle(member.quality);
                  return (
                    <li
                      key={member.idShort}
                      className="flex items-center gap-1.5 rounded-control border border-stroke-control bg-surface-control px-1.5 py-0.5"
                    >
                      <span className="num text-caption text-text-secondary">fp:{member.idShort}</span>
                      <span
                        className={`text-caption ${badge.className}`}
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
                        className="al-btn al-btn-danger al-btn-xs text-text-secondary"
                        onClick={() => onLeave(member.idShort, group.groupId)}
                      >
                        {t("group.leave")}
                      </button>
                    </li>
                  );
                })}
              </ul>
              <div className="mt-1.5 flex flex-wrap gap-1.5">
                {selectable
                  .filter(
                    (peer) =>
                      !group.members.some((member) => member.idShort === peer.idShort),
                  )
                  .map((peer) => (
                    <button
                      key={peer.idShort}
                      type="button"
                      className="al-btn al-btn-xs text-text-secondary"
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
    </Panel>
  );
}
