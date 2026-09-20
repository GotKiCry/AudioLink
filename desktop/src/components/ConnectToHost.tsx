/**
 * 接收端入口：本机**切换成接收端**，去听另一台主机（架构规则的另一半）。
 *
 * 为什么它不再叫「手动添加设备」、也不再是一张主卡片：
 * 这台机器的常态是主机 —— 主机只出示自己的地址等接入，**永不主动请求连接**。
 * 旧措辞（"添加设备"）与旧视觉（与地址卡平级的主卡片）传达的恰好相反："PC 也得去连手机"，
 * 于是用户会在两张卡之间挑一个填地址 —— 而主机这边根本没有该由他填的地址。
 *
 * 现在的表达是一个**角色动作**：标题说清要做什么（去听另一台设备），标签说清代价
 * （本机将作为接收端：只收别人的声音，不把自己的声音送出去），说明给出前提
 * （地址来自主机屏幕），容器下沉一级（`.al-well` 而不是 `.al-card`）——
 * 主路径上因此仍然只有一条线："出示地址 → 等接入"。
 *
 * 但它依旧一键可达（PRODUCT 原则 2）：摆在设备列表下方，不藏进二级菜单。
 */

import { useState } from "react";

import { t } from "../i18n";
import { isPeerAddrWellFormed } from "../types";
import { IconLink } from "./icons";

interface ConnectToHostProps {
  connecting: boolean;
  /** 返回 true 表示命令已被接受（失败原因由上层统一展示，避免同一错误显示两遍）。 */
  onConnect: (addr: string) => Promise<boolean>;
}

export function ConnectToHost({ connecting, onConnect }: ConnectToHostProps) {
  const [addr, setAddr] = useState("");
  const filled = addr.trim() !== "";
  // 只提示、不拦截：前端判据（IP / IP:端口）比内核窄，拿它禁用提交等于用一个更严的规则
  // 把用户锁在门外 —— 能不能连由内核回答，这里只负责把"该怎么写"说清楚。
  const illFormed = filled && !isPeerAddrWellFormed(addr);

  return (
    <form
      aria-labelledby="connect-host-heading"
      // al-well 而不是 al-card：这是角色切换，不是与「本机地址」平级的第二个主行动
      className="al-well flex flex-wrap items-end gap-3 p-4"
      onSubmit={(event) => {
        event.preventDefault();
        void onConnect(addr);
      }}
    >
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          <h2 id="connect-host-heading" className="text-caption font-semibold text-text-secondary">
            {t("connect.title")}
          </h2>
          {/* 角色标签：这一格改的是本机的角色，不是"往列表里加一台设备" */}
          <span className="rounded-full border border-stroke-control px-2 py-0.5 text-caption text-text-tertiary">
            {t("connect.role")}
          </span>
        </div>
        <p className="mt-1 max-w-[38rem] text-caption leading-relaxed text-text-secondary">
          {t("connect.hint")}
        </p>
      </div>

      {/* 说明贴着输入框走：用户填错时眼睛在输入框上，把它放到卡片另一头就等于没写 */}
      <div className="ml-auto flex flex-col gap-1">
        {/* 标签可见：这一格要填的是**主机**的地址，方向本身就是信息，藏进 sr-only 等于丢掉了它 */}
        <label className="text-caption font-semibold text-text-tertiary" htmlFor="connect-peer-addr">
          {t("connect.address")}
        </label>
        <input
          id="connect-peer-addr"
          name="addr"
          value={addr}
          onChange={(event) => setAddr(event.target.value)}
          placeholder={t("manual.placeholder")}
          inputMode="url"
          autoComplete="off"
          spellCheck={false}
          aria-describedby="connect-peer-addr-hint"
          aria-invalid={illFormed || undefined}
          className={
            illFormed
              ? "num h-9 w-[248px] border border-caution px-3 text-body text-text-primary"
              : "num h-9 w-[248px] border border-stroke-control px-3 text-body text-text-primary"
          }
        />
        <p
          id="connect-peer-addr-hint"
          className={
            illFormed
              ? "text-caption leading-relaxed text-caution"
              : "text-caption leading-relaxed text-text-tertiary"
          }
        >
          {t("manual.address_hint")}
        </p>
      </div>

      <button
        type="submit"
        // 空输入不发请求：让内核少一次必然失败的往返（原因可见性由返回错误保证）
        disabled={connecting || !filled}
        // 禁用的按钮不可聚焦，悬停是它唯一能解释"为什么点不动"的渠道
        title={filled ? undefined : t("manual.address_hint")}
        className="al-btn h-9 gap-2 px-4 text-body"
      >
        <IconLink className="h-4 w-4" />
        {connecting ? t("manual.connecting") : t("connect.submit")}
      </button>
    </form>
  );
}
