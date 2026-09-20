/**
 * 本机地址卡：「音源即主机」架构里最重要的一块 ——
 * 主机不需要知道对方的地址，它只需要把自己的地址说清楚，让对方输进去。
 *
 * 所以这里把地址做得最大、最可复制，并给出下一步动作的人话（PRODUCT.md 原则 3）。
 *
 * 卡片上还写着一句 host.role_hint，把规则本身说白（"本机是主机：只等对方接入"）：
 * 它是"为什么这张卡上只有本机地址、没有对方地址输入框"的答案。规则要写在用户看得见的地方 ——
 * 不写，用户就会照着直觉（PC 得去连手机）去找一个并不存在的入口。
 *
 * 为什么不用 local.addr：那是**监听地址**（QUIC bind 的 0.0.0.0:58290）。
 * 0.0.0.0 在别人的手机上指的就是那部手机自己，照着输必然连不上 ——
 * 一张「教人输地址」的卡片印一个注定失败的地址，比承认没地址更糟：
 * 用户会先去怀疑 Wi-Fi、手机、防火墙，最后才怀疑界面。所以这里只显示后端的
 * displayAddr，一条都挑不出来就如实说「未检测到」。
 */

import { useState } from "react";

import type { LocalStatus } from "../types";
import { t } from "../i18n";
import { IconLink } from "./icons";

/** 通配与回环地址：在本机是合法的监听目标，在**对方设备**上却指向对方自己。 */
const UNREACHABLE_HOSTS = new Set(["0.0.0.0", "::", "::1", "0:0:0:0:0:0:0:0", "localhost"]);

/**
 * 这条地址能不能交给对方设备去输；不能就给空串。
 *
 * 为什么展示层还要自己拦一道：displayAddr 由后端算出来，而「拿监听地址兜底」是一类
 * 很容易复发的退化。界面是最后的出口 —— 宁可说「没检测到局域网地址」，
 * 也不要在屏幕上印一串手机永远连不上的字符。
 */
export function peerUsableAddr(raw: string | null | undefined): string {
  const addr = (raw ?? "").trim();
  if (addr === "") return "";

  // 取主机部分：带方括号的是 IPv6（[fe80::1]:58290），否则冒号前就是主机。
  const bracket = addr.startsWith("[") ? addr.indexOf("]") : -1;
  const head = bracket === -1 ? addr.split(":")[0] ?? "" : addr.slice(1, bracket);
  const host = head.toLowerCase();

  // 0.0.0.0/8 整段是「本网络」、127.0.0.0/8 是回环，两者都到不了对方。
  const unreachable =
    host === "" ||
    UNREACHABLE_HOSTS.has(host) ||
    host.startsWith("0.") ||
    host.startsWith("127.");
  return unreachable ? "" : addr;
}

export function HostAddressCard({ local }: { local: LocalStatus | null }) {
  const [copied, setCopied] = useState(false);
  const address = peerUsableAddr(local?.displayAddr);
  const ready = address !== "";
  // 多地址提示只在「确实有得试」时出现：一条都没有的时候，它后面没有可试的对象。
  const multi = ready && (local?.lanAddrs?.length ?? 0) > 1;

  const copy = async () => {
    if (!ready) return;
    try {
      await navigator.clipboard.writeText(address);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      // 剪贴板被拒（无权限 / 非安全上下文）：地址本来就印在屏幕上，用户可手抄。
    }
  };

  return (
    <section className="al-card flex flex-wrap items-center gap-x-4 gap-y-3 p-4" aria-labelledby="host-address-heading">
      <div className="min-w-0">
        <h2 id="host-address-heading" className="text-caption font-semibold text-text-secondary">{t("host.address")}</h2>
        {/* 先规则、后动作：规则解释这张卡为什么长这样，动作才是用户接下来要做的事 */}
        <p className="mt-1 text-caption leading-relaxed text-text-tertiary">{t("host.role_hint")}</p>
        <p className="mt-1 text-caption leading-relaxed text-text-secondary">
          {ready ? t("host.hint") : t("host.hint_missing")}
        </p>
        {multi ? (
          <p className="mt-1 text-caption leading-relaxed text-text-tertiary">{t("host.multi_hint")}</p>
        ) : null}
      </div>

      <div className="ml-auto flex items-center gap-2">
        {/*
          没地址时把大字号让给一句人话：num 是给地址串准备的数宽样式，
          套在「未检测到局域网地址」上只会更难读。
        */}
        <span
          className={
            ready
              ? "num select-all text-body tracking-tight text-text-primary"
              : "max-w-[22rem] text-caption leading-relaxed text-text-tertiary"
          }
        >
          {ready ? address : t("host.addr_missing")}
        </span>
        <button
          type="button"
          onClick={() => void copy()}
          disabled={!ready}
          // 禁用的按钮不可聚焦，鼠标悬停是它唯一能解释「为什么点不动」的渠道。
          title={ready ? undefined : t("host.addr_missing")}
          className="al-btn h-8 gap-1.5 px-3 text-caption"
        >
          <IconLink className="h-3.5 w-3.5" />
          {/* 地址在 1.6 秒里失效（拔网线）时不该还挂着「已复制」 */}
          {copied && ready ? t("host.copied") : t("host.copy")}
        </button>
      </div>
    </section>
  );
}
