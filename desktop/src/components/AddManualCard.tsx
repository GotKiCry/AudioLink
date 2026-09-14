/**
 * 手工添加设备（UI 规格 §2.1 的 `AddManualCard`，需求 FR-17）。
 *
 * 为什么 M1 只有这一个入口：设备自动发现（mDNS/广播）属 M2 —— 契约 §6「不做」清单里
 * 明确排除"设备列表"。发现被 AP 隔离时的手工 IP 兜底，恰好也是本轮唯一能端到端跑通的路。
 */

import { useState } from "react";

interface AddManualCardProps {
  connecting: boolean;
  /** 返回 true 表示命令已被接受（失败原因由上层统一展示，避免同一错误显示两遍）。 */
  onConnect: (addr: string) => Promise<boolean>;
}

export function AddManualCard({ connecting, onConnect }: AddManualCardProps) {
  const [addr, setAddr] = useState("");

  return (
    <form
      className="rounded-xl border border-dashed border-slate-300 p-4 dark:border-slate-700"
      onSubmit={(event) => {
        event.preventDefault();
        void onConnect(addr);
      }}
    >
      <div className="text-sm font-medium">手动添加设备</div>
      <p className="mt-1 text-xs text-slate-500">
        两台设备需在同一 Wi-Fi/局域网；若自动发现失效（AP 客户端隔离），用对方「本机状态条」上的地址直连。
      </p>

      <label className="mt-3 block text-xs text-slate-500" htmlFor="peer-addr">
        对方地址
      </label>
      <input
        id="peer-addr"
        name="addr"
        value={addr}
        onChange={(event) => setAddr(event.target.value)}
        placeholder="192.168.1.23 或 192.168.1.23:58290"
        inputMode="url"
        autoComplete="off"
        spellCheck={false}
        className="mt-1 h-9 w-full rounded-lg border border-slate-200 bg-transparent px-2 font-mono text-sm dark:border-slate-700"
      />

      <button
        type="submit"
        // 空输入不发请求：让内核少一次必然失败的往返（原因可见性由返回错误保证）
        disabled={connecting || addr.trim() === ""}
        className="mt-3 h-9 w-full rounded-lg bg-indigo-600 text-sm font-medium text-white disabled:opacity-50"
      >
        {connecting ? "连接中…" : "连接"}
      </button>
    </form>
  );
}
