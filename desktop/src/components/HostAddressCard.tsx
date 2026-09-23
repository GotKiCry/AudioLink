import { useEffect, useRef, useState } from "react";

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
  const [copyFailed, setCopyFailed] = useState(false);
  const copyTimer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(copyTimer.current), []);
  const address = peerUsableAddr(local?.displayAddr);
  const ready = address !== "";
  // 多地址提示只在「确实有得试」时出现：一条都没有的时候，它后面没有可试的对象。
  const multi = ready && (local?.lanAddrs?.length ?? 0) > 1;

  const copy = async () => {
    if (!ready) return;
    try {
      await navigator.clipboard.writeText(address);
      setCopied(true);
      setCopyFailed(false);
      window.clearTimeout(copyTimer.current);
      copyTimer.current = window.setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopyFailed(true);
    }
  };

  return (
    <section className="al-connect-panel" aria-labelledby="host-address-heading">
      <div className="al-address-block">
        <h2 id="host-address-heading" className="text-base font-semibold">{t("host.address")}</h2>
        <p className="mt-2 text-caption leading-relaxed text-text-secondary">{t("host.role_hint")}</p>
        <div className="al-address-value">
          <span className={ready ? "num select-all break-all text-xl font-medium" : "text-body text-text-secondary"}>
            {ready ? address : t("host.addr_missing")}
          </span>
          <button type="button" onClick={() => void copy()} disabled={!ready}
            title={ready ? undefined : t("host.addr_missing")}
            className="al-btn h-8 gap-1.5 px-3 text-caption">
            <IconLink className="h-3.5 w-3.5" />
            {copied && ready ? t("host.copied") : t("host.copy")}
          </button>
        </div>
        {multi ? <p className="mt-2 text-caption leading-relaxed text-text-secondary">{t("host.multi_hint")}</p> : null}
        <p role="status" className="text-caption text-caution">{copyFailed ? t("host.copy_failed") : null}</p>
      </div>
      {ready ? (
        <ol className="al-connect-steps">
          <li><span aria-hidden="true">1</span><p>{t("host.hint")}</p></li>
        </ol>
      ) : <p className="text-caption leading-relaxed text-caution">{t("host.hint_missing")}</p>}
    </section>
  );
}
