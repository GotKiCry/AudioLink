/**
 * 本机地址卡：「音源即主机」架构里最重要的一块 ——
 * 主机不需要知道对方的地址，它只需要把自己的地址说清楚，让对方输进去。
 *
 * 所以这里把地址做得最大、最可复制，并给出下一步动作的人话（PRODUCT.md 原则 3）。
 */

import { useState } from "react";

import type { LocalStatus } from "../types";
import { t } from "../i18n";
import { IconLink } from "./icons";

export function HostAddressCard({ local }: { local: LocalStatus | null }) {
  const [copied, setCopied] = useState(false);
  const address = local?.addr ?? "";
  const ready = address !== "";

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
    <section className="plate flex flex-wrap items-center gap-x-4 gap-y-3 p-4" aria-labelledby="host-address-heading">
      <div className="min-w-0">
        <h2 id="host-address-heading" className="silk">{t("host.address")}</h2>
        <p className="mt-1 t-cap leading-relaxed text-text-2">{t("host.hint")}</p>
      </div>

      <div className="ml-auto flex items-center gap-2">
        <span className="num select-all t-lead tracking-tight text-text">
          {ready ? address : "…"}
        </span>
        <button
          type="button"
          onClick={() => void copy()}
          disabled={!ready}
          className="key h-8 gap-1.5 px-3 t-cap"
        >
          <IconLink className="h-3.5 w-3.5" />
          {copied ? t("host.copied") : t("host.copy")}
        </button>
      </div>
    </section>
  );
}
