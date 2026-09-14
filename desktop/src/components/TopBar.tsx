/**
 * 顶栏：应用名 + 版本号。
 *
 * M1 刻意只保留这些：UI 规格 §2.1 的"采集设备 / 采样率 / 编码档"选择器需要
 * `SET_GAIN`、采集源枚举等 command —— 契约 §6 里**不存在**，做了就是没数据的假控件。
 * 那些属 M2/M5（`docs/11-m1-contract.md` §6「不做」清单）。
 */

export function TopBar({ version }: { version: string }) {
  return (
    <header className="flex h-12 shrink-0 items-center gap-3 border-b border-slate-200 px-4 dark:border-slate-800">
      <span className="font-semibold tracking-tight">AudioLink</span>
      <span className="text-xs text-slate-400">局域网低延迟音频分发</span>
      {/* 版本号来自 `version` command（唯一来源是 Cargo workspace 版本） */}
      <span className="ml-auto font-mono text-xs tabular-nums text-slate-500">
        {version === "" ? "…" : `v${version}`}
      </span>
    </header>
  );
}
