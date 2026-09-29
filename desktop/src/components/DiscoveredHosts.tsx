import { useEffect, useRef, useState } from "react";
import { t } from "../i18n";
import { api } from "../lib/ipc";
import type { DiscoveredHost } from "../types";
import { IconRefresh } from "./icons";

interface Props {
  connecting: boolean;
  connectedIds: string[];
  /**
   * 所在的折叠区是否展开（默认展开）。
   *
   * 收起时**暂停轮询但保留状态**：组件并没有卸载（已发现的主机、刷新状态都还在），
   * 只是不再每秒问一次 —— 每次轮询都会 `setHosts` 一个新数组，白跑一轮就是白重渲染一次。
   */
  active?: boolean;
  onConnect: (addr: string) => Promise<boolean>;
}

export function DiscoveredHosts({ connecting, connectedIds, active = true, onConnect }: Props) {
  const [hosts, setHosts] = useState<DiscoveredHost[]>([]);
  const [failed, setFailed] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [refreshVersion, setRefreshVersion] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const pending = useRef(false);

  useEffect(() => {
    // 收起时不轮询（状态留着，理由见 active 的说明）。
    if (!active) {
      return;
    }
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const settled = setTimeout(() => { if (!cancelled) setRefreshing(false); }, 3500);
    const poll = async (restart = false) => {
      try {
        const found = await api.discoveredHosts(restart);
        if (!cancelled) {
          setHosts(found); setFailed(false);
          if (found.length > 0) setRefreshing(false);
        }
      } catch {
        if (!cancelled) { setHosts([]); setFailed(true); setRefreshing(false); }
      } finally {
        if (!cancelled) { setLoaded(true); timer = setTimeout(() => void poll(), 1000); }
      }
    };
    void poll(refreshVersion > 0);
    return () => { cancelled = true; clearTimeout(timer); clearTimeout(settled); };
  }, [refreshVersion, active]);

  function refresh() {
    setHosts([]); setFailed(false); setLoaded(false); setRefreshing(true);
    setRefreshVersion((version) => version + 1);
  }

  async function connect(host: DiscoveredHost) {
    if (pending.current || connecting || !host.compatible || connectedIds.includes(host.idShort)) return;
    pending.current = true;
    setSelected(host.addr);
    try { await onConnect(host.addr); }
    finally { pending.current = false; setSelected(null); }
  }

  return (
    <section className="pt-4" aria-labelledby="discovery-heading">
      <div className="flex items-center justify-between gap-3">
        <h3 id="discovery-heading" className="text-body font-semibold">{t("discovery.title")}</h3>
        <button type="button" className="al-btn al-btn-lg gap-2"
          disabled={refreshing || connecting || selected !== null} onClick={refresh}
          aria-label={t("discovery.refresh")}>
          <IconRefresh className="h-3.5 w-3.5" />
          {refreshing ? t("discovery.refreshing") : t("discovery.refresh")}
        </button>
      </div>
      <p className="mt-1 text-caption text-text-secondary" role="status">
        {failed ? t("discovery.error") : !loaded || refreshing ? t("discovery.searching") :
          hosts.length === 0 ? t("discovery.empty") : t("discovery.hint")}
      </p>
      {hosts.length > 0 && (
        <ul className="mt-3 max-h-64 overflow-y-auto divide-y divide-stroke-control">
          {hosts.map((host) => {
            const connected = connectedIds.includes(host.idShort);
            return <li key={`${host.idShort}:${host.addr}`} className="flex items-center gap-4 py-3">
              <div className="min-w-0 flex-1">
                <p className="truncate text-body font-semibold" title={host.name}>{host.name}</p>
                <p className="num break-all text-caption text-text-secondary">{host.addr}</p>
                {!host.compatible && <p className="mt-1 text-caption text-caution">{t("discovery.incompatible")}</p>}
              </div>
              <button type="button" className="al-btn al-btn-xl shrink-0"
                aria-label={`${t("connect.submit")} ${host.name}`}
                disabled={connecting || selected !== null || connected || !host.compatible}
                onClick={() => void connect(host)}>
                {connected ? t("discovery.connected") : selected === host.addr ? t("manual.connecting") : t("connect.submit")}
              </button>
            </li>;
          })}
        </ul>
      )}
    </section>
  );
}
