// M5 · 双语护栏：中英表的键集合与占位符必须一致。
//
// `tsc` 已经能保证「英文表键齐全」（`Record<MessageKey, string>`），但它管不了占位符：
// 英文译文里漏写一个 `{name}`，界面上就会静默少一个值，编译期一声不吭。这个脚本补上那一段。
// 由 `pnpm build` 调用，所以本地构建与 CI 都会跑。
//
// M5+ 追加 §5「生产源码引用检查」：前面四条查的都是"翻译得对不对"，
// 查不出"有没有人用" —— 一份从旧界面淘汰下来的表，中英齐全、占位符一致，一条都不会红。

import { readFileSync, readdirSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

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

// §5 生产源码引用检查 ─────────────────────────────────────────────────────────
//
// 判据是「键名有没有以字面量形式出现在生产源码里」，而不是「是不是 t("…") 的实参」：
// 界面里有一批准静态引用把键名写在**映射表的值**上（types.ts:220 的状态表、
// BackgroundPanel 的 MODE_LABEL/DIR_LABEL、lib/broadcast.ts 的 broadcastLabels/broadcastHints），
// 只认 t() 实参会把它们全部误判成死键 —— 而误判的代价是删掉还在用的文案。
// 判据刻意放宽：宁可漏报，不误报。

/** 死键即失败：收尾波次已把无引用的键删干净，此后新冒出来的未引用键要当场拦住。 */
const BLOCK_ON_UNUSED_KEYS = true;

/**
 * 运行时拼出来的键：表里有、源码里查不到是正常的。前缀匹配，加一条必须写清"谁在拼"。
 */
const DYNAMIC_KEY_PREFIXES = [
  // src/components/BackgroundPanel.tsx:225
  //   t(("bg.preset." + preset.key) as MessageKey)
  // 后缀来自 lib/background.ts 的 BACKGROUND_PRESETS（12 个预设色名），拼不出静态字面量。
  "bg.preset.",
];

const SRC_DIR = fileURLToPath(new URL("../src", import.meta.url));
const I18N_FILE = resolve(SRC_DIR, "i18n.ts");

/** src/ 下所有 *.ts / *.tsx（含测试）。 */
function walk(dir, out = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) walk(full, out);
    else if (/\.tsx?$/.test(entry.name)) out.push(full);
  }
  return out;
}

/** 文件里出现过的所有字符串字面量（够宽松：键名写在哪儿都算数）。 */
function literals(file) {
  const found = new Set();
  for (const match of readFileSync(file, "utf8").matchAll(/["'`]([A-Za-z][\w.]*)["'`]/g)) {
    found.add(match[1]);
  }
  return found;
}

const isTest = (file) => /\.test\.tsx?$/.test(file);
const referenced = new Set(); // 生产源码引用过
const testOnly = new Map(); // 键 → 只剩谁在测试里用（删键前必须先改它）

for (const file of walk(SRC_DIR)) {
  if (resolve(file) === I18N_FILE) continue; // 定义处不算"使用"
  const found = literals(file);
  if (isTest(file)) {
    for (const key of found) testOnly.set(key, [...(testOnly.get(key) ?? []), relative(SRC_DIR, file)]);
  } else {
    for (const key of found) referenced.add(key);
  }
}

const isDynamicKey = (key) => DYNAMIC_KEY_PREFIXES.some((prefix) => key.startsWith(prefix));
const unused = [...zh.keys()].filter((key) => !referenced.has(key) && !isDynamicKey(key));

if (BLOCK_ON_UNUSED_KEYS) for (const key of unused) errors.push(`没有被任何生产源码引用：${key}`);

console.log(`i18n：zh=${zh.size} en=${en.size} 条`);
if (errors.length > 0) {
  for (const error of errors) console.error(`  ✗ ${error}`);
  console.error(`i18n 检查未通过（${errors.length} 项）`);
  process.exit(1);
}
console.log("  ✓ 键集合一致、占位符一致、无空文案");

if (unused.length === 0) {
  console.log("  ✓ 每个键都有生产源码引用");
} else {
  const tag = BLOCK_ON_UNUSED_KEYS ? "✗" : "⚠";
  console.log(`  ${tag} 没有被生产源码引用：${unused.length} 个${BLOCK_ON_UNUSED_KEYS ? "" : "（只提示，不阻断）"}`);
  for (const key of unused) {
    console.log(`      ${key}   「${zh.get(key)}」`);
    const users = testOnly.get(key);
    if (users) console.log(`          ↑ 只剩测试在用（${users.join("、")}）—— 删键前先改测试，否则红的是它`);
  }
  console.log("    多半是旧界面的残骸；删之前先确认没人靠动态拼键用它（白名单见本文件 DYNAMIC_KEY_PREFIXES）。");
}
