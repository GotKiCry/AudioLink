// M5 · 双语护栏：中英表的键集合与占位符必须一致。
//
// `tsc` 已经能保证「英文表键齐全」（`Record<MessageKey, string>`），但它管不了占位符：
// 英文译文里漏写一个 `{name}`，界面上就会静默少一个值，编译期一声不吭。这个脚本补上那一段。
// 由 `pnpm build` 调用，所以本地构建与 CI 都会跑。

import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/i18n.ts", import.meta.url), "utf8");

/** 从 i18n.ts 里抠出一张表（按块的起止标记，不引 TS 编译器）。 */
function parseTable(startMarker, endMarker) {
  const start = source.indexOf(startMarker);
  if (start < 0) throw new Error(`找不到表：${startMarker}`);
  const end = source.indexOf(endMarker, start);
  if (end < 0) throw new Error(`找不到表的结束标记：${endMarker}`);
  const table = new Map();
  for (const match of source.slice(start, end).matchAll(/^  "([\w.]+)": "(.*)",$/gm)) {
    table.set(match[1], match[2]);
  }
  return table;
}

const zh = parseTable("const zh = {", "} as const;");
const en = parseTable("const en: Record<MessageKey, string> = {", "\n};");

const placeholders = (text) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(",");
const errors = [];

if (zh.size === 0) errors.push("中文表没解析出任何条目（格式变了？）");
for (const key of zh.keys()) if (!en.has(key)) errors.push(`英文表缺键：${key}`);
for (const key of en.keys()) if (!zh.has(key)) errors.push(`英文表多出键：${key}`);

for (const [key, zhText] of zh) {
  const enText = en.get(key) ?? "";
  if (zhText.trim() === "") errors.push(`中文文案为空：${key}`);
  if (enText.trim() === "") errors.push(`英文文案为空：${key}`);
  const a = placeholders(zhText);
  const b = placeholders(enText);
  if (a !== b) errors.push(`占位符不一致：${key} zh=[${a}] en=[${b}]`);
  if (!/[\u4e00-\u9fff]/.test(zhText)) errors.push(`中文表里没有汉字（多半是抄错行）：${key}`);
}

console.log(`i18n：zh=${zh.size} en=${en.size} 条`);
if (errors.length > 0) {
  for (const error of errors) console.error(`  ✗ ${error}`);
  console.error(`i18n 检查未通过（${errors.length} 项）`);
  process.exit(1);
}
console.log("  ✓ 键集合一致、占位符一致、无空文案");