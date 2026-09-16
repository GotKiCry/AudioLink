import { useState } from "react";

import type { GroupView, PeerView } from "../types";

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
          同步组（临时组 · 组内对齐靠 epoch 排播）
        </h2>
        <button
          type="button"
          className="rounded border border-neutral-700 px-2 py-1 text-xs text-neutral-300 hover:bg-neutral-800"
          onClick={onRefresh}
        >
          刷新
        </button>
      </header>

      <div className="mb-3 flex flex-wrap items-center gap-3 text-xs text-neutral-300">
        <span>提前量</span>
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
          {busy ? "处理中…" : `建组（已选 ${selected.length} 台）`}
        </button>
      </div>

      {selectable.length === 0 ? (
        <p className="text-xs text-neutral-500">还没有可加入的设备：先连接并对端进入会话。</p>
      ) : (
        <ul className="mb-3 space-y-1 text-xs">
          {selectable.map((peer) => (
            <li key={peer.idShort} className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={selected.includes(peer.idShort)}
                onChange={() => toggle(peer.idShort)}
              />
              <span className="text-neutral-200">{peer.name}</span>
              <span className="text-neutral-500">fp:{peer.idShort}</span>
              <span className="text-neutral-500">{peer.state}</span>
            </li>
          ))}
        </ul>
      )}

      {groups.length === 0 ? (
        <p className="text-xs text-neutral-500">当前没有同步组。</p>
      ) : (
        <ul className="space-y-2">
          {groups.map((group) => (
            <li
              key={group.groupId}
              className="rounded border border-neutral-800 bg-neutral-950/60 p-2 text-xs"
            >
              <div className="mb-1 flex items-center justify-between">
                <span className="font-medium text-neutral-200">
                  组 #{group.groupId} · {group.members.length} 台 · 提前量 {group.leadMs} ms
                </span>
                <span className="text-neutral-500">epoch {group.epochId}</span>
              </div>
              <ul className="flex flex-wrap gap-2">
                {group.members.map((member) => (
                  <li
                    key={member.idShort}
                    className="flex items-center gap-1 rounded bg-neutral-800 px-2 py-0.5"
                  >
                    <span className="text-neutral-200">fp:{member.idShort}</span>
                    <span
                      className={
                        member.quality === "poor"
                          ? "text-red-400"
                          : member.quality === "fair"
                            ? "text-amber-400"
                            : "text-emerald-400"
                      }
                      title={
                        member.offsetUs === null
                          ? "还没有时钟估计"
                          : `偏移 ${member.offsetUs} µs`
                      }
                    >
                      {member.quality === "poor" ? "同步质量差" : member.quality}
                    </span>
                    <button
                      type="button"
                      className="text-neutral-400 hover:text-red-400"
                      onClick={() => onLeave(member.idShort, group.groupId)}
                    >
                      退出
                    </button>
                  </li>
                ))}
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
