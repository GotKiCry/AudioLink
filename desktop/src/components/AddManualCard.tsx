/**
 * 空槽位：手工添加设备（UI 规格 §2.1 的 `AddManualCard`，需求 FR-17）。
 *
 * 世界语汇：机架上还没有插卡的槽位 —— 虚线边框、下沉的机箱底、外加一条等宽地址输入。
 * 为什么 M1 只有这一个入口：设备自动发现（mDNS/广播）属 M2 —— 契约 §6「不做」清单里
 * 明确排除"设备列表"。发现被 AP 隔离时的手工 IP 兜底，恰好也是本轮唯一能端到端跑通的路。
 */

import { useState } from "react";
import { t } from "../i18n";
import { IconLink } from "./icons";

interface AddManualCardProps {
  connecting: boolean;
  /** 返回 true 表示命令已被接受（失败原因由上层统一展示，避免同一错误显示两遍）。 */
  onConnect: (addr: string) => Promise<boolean>;
}

export function AddManualCard({ connecting, onConnect }: AddManualCardProps) {
  const [addr, setAddr] = useState("");

  return (
    <form
      className="flex h-full w-full flex-col border border-dashed border-stroke-control bg-surface-sunken/70 p-3"
      onSubmit={(event) => {
        event.preventDefault();
        void onConnect(addr);
      }}
    >
      <div className="text-caption font-semibold text-text-secondary">{t("manual.title")}</div>
      <p className="mt-2 text-caption leading-relaxed text-text-tertiary">{t("manual.hint")}</p>

      <label className="text-caption font-semibold text-text-tertiary mt-4" htmlFor="peer-addr">
        {t("manual.address")}
      </label>
      <input
        id="peer-addr"
        name="addr"
        value={addr}
        onChange={(event) => setAddr(event.target.value)}
        placeholder={t("manual.placeholder")}
        inputMode="url"
        autoComplete="off"
        spellCheck={false}
        className="num mt-1.5 h-10 w-full border border-stroke-control bg-surface-card-solid px-2 text-body text-text-primary placeholder:text-text-tertiary"
      />

      <button
        type="submit"
        // 空输入不发请求：让内核少一次必然失败的往返（原因可见性由返回错误保证）
        disabled={connecting || addr.trim() === ""}
        className="al-btn al-btn-accent mt-auto h-11 w-full gap-2 text-body"
      >
        <IconLink className="h-4 w-4" />
        {connecting ? t("manual.connecting") : t("manual.connect")}
      </button>
    </form>
  );
}
