/**
 * 连接到主机：本机**作为接收端**时的入口（另一侧的镜像）。
 *
 * 为什么它不抢主位：这台机器的常态是「PC 当音源」，主机只需要等别人来连；
 * 只有反向使用时（手机内录推给 PC / PC 去接另一台 PC）才需要在这里输地址。
 * 但它是**一键可达**的（原则 2），所以直接摆在设备列表下方，不藏进二级菜单。
 */

import { useState } from "react";

import { t } from "../i18n";
import { IconLink } from "./icons";

interface ConnectToHostProps {
  connecting: boolean;
  /** 返回 true 表示命令已被接受（失败原因由上层统一展示，避免同一错误显示两遍）。 */
  onConnect: (addr: string) => Promise<boolean>;
}

export function ConnectToHost({ connecting, onConnect }: ConnectToHostProps) {
  const [addr, setAddr] = useState("");

  return (
    <form
      aria-labelledby="connect-host-heading"
      className="plate flex flex-wrap items-end gap-3 p-4"
      onSubmit={(event) => {
        event.preventDefault();
        void onConnect(addr);
      }}
    >
      <div className="min-w-0">
        <h2 id="connect-host-heading" className="silk">{t("manual.title")}</h2>
        <p className="mt-1 t-cap leading-relaxed text-text-2">{t("manual.hint")}</p>
      </div>

      <label className="sr-only" htmlFor="peer-addr">{t("manual.address")}</label>
      <input
        id="peer-addr"
        name="addr"
        value={addr}
        onChange={(event) => setAddr(event.target.value)}
        placeholder={t("manual.placeholder")}
        inputMode="url"
        autoComplete="off"
        spellCheck={false}
        className="num ml-auto h-9 w-[248px] border border-line px-3 t-body text-text"
      />

      <button
        type="submit"
        // 空输入不发请求：让内核少一次必然失败的往返（原因可见性由返回错误保证）
        disabled={connecting || addr.trim() === ""}
        className="key h-9 gap-2 px-4 t-body"
      >
        <IconLink className="h-4 w-4" />
        {connecting ? t("manual.connecting") : t("manual.connect")}
      </button>
    </form>
  );
}
