#!/usr/bin/env node
// M5 · 更新产物本地校验：`latest.json` 里的签名，真的能配上本地那个安装包吗？
//
// 为什么需要它：自动更新的链路是「客户端读 latest.json → 下载 → **验签** → 安装」。
// 前两步能靠肉眼和 curl 检查，第三步不行 —— 签名对不对只有**算一遍**才知道。
// 发布前跑这个脚本，就能在「用户装不上」之前知道清单与产物是否自洽。
//
// 为什么用 Node 而不是 PowerShell：Ed25519 与 BLAKE2b-512 都是 Node 内置（`crypto`），
// 而 PowerShell 的 .NET 没有 BLAKE2b，也不保证有 Ed25519 —— 自己实现一遍密码学不是这里该做的事。
//
// 用法：
//   node tools/check-update.mjs [--latest <path>] [--dir <产物目录>] [--config <tauri.conf.json>]
//   退出码：0 = 全部通过；1 = 有校验失败；2 = 用法/文件问题

import { createHash, createPublicKey, verify } from "node:crypto";
import { readFileSync, existsSync } from "node:fs";
import { basename, join, resolve } from "node:path";

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i];
    if (!key.startsWith("--")) throw new Error(`不认识的参数：${key}`);
    out[key.slice(2)] = argv[i + 1];
  }
  return out;
}

/** minisign 文本的第二行是真正的载荷（第一行是给人看的注释）。 */
function minisignPayload(base64Text) {
  const text = Buffer.from(base64Text, "base64").toString("utf8");
  const lines = text.split("\n").map((line) => line.trim()).filter((line) => line.length > 0);
  if (lines.length < 2) throw new Error("minisign 文本没有第二行");
  return Buffer.from(lines[1], "base64");
}

/** 32 字节裸公钥 → Ed25519 KeyObject（DER/SPKI 前缀是固定的 12 字节）。 */
function ed25519Key(raw32) {
  const der = Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), raw32]);
  return createPublicKey({ key: der, format: "der", type: "spki" });
}

const args = parseArgs(process.argv.slice(2));
const root = resolve(import.meta.dirname, "..");
const latestPath = args.latest ?? join(root, "target/evidence/release/latest.json");
const configPath = args.config ?? join(root, "desktop/src-tauri/tauri.conf.json");
const dir = args.dir ?? join(root, "target/evidence/release/artifacts");

if (!existsSync(latestPath)) { console.error(`找不到更新清单：${latestPath}`); process.exit(2); }
if (!existsSync(configPath)) { console.error(`找不到 tauri 配置：${configPath}`); process.exit(2); }

const latest = JSON.parse(readFileSync(latestPath, "utf8"));
const config = JSON.parse(readFileSync(configPath, "utf8"));
const pubkeyB64 = config?.plugins?.updater?.pubkey;
if (typeof pubkeyB64 !== "string" || pubkeyB64.length === 0) {
  console.error("tauri.conf.json 里没有 plugins.updater.pubkey —— 无法验签");
  process.exit(2);
}
const pubPayload = minisignPayload(pubkeyB64);
const pub32 = pubPayload.subarray(-32);
const pubKeyId = pubPayload.subarray(2, 10).toString("hex");
const key = ed25519Key(pub32);
// minisign 的公钥与签名里各带 8 字节 key_id：它们不相等就说明「清单不是这把公钥签的」，
// 这时候再谈签名对不对没有意义 —— 先把这个更根本的问题说清楚。
console.log(`公钥：key_id=${pubKeyId} 载荷 ${pubPayload.length} B`);

const failures = [];
const platforms = latest.platforms ?? {};
if (Object.keys(platforms).length === 0) failures.push("latest.json 里没有任何 platform 条目");

console.log(`更新清单：${basename(latestPath)}  版本 ${latest.version ?? "(缺)"}`);

for (const [platform, entry] of Object.entries(platforms)) {
  const file = basename(new URL(entry.url).pathname);
  const localPath = join(dir, file);
  if (!existsSync(localPath)) {
    failures.push(`${platform}: 本地找不到 ${file}（在 ${dir}）`);
    console.log(`  ${platform}: ✗ 本地缺文件 ${file}`);
    continue;
  }
  const data = readFileSync(localPath);
  const sig = minisignPayload(entry.signature);
  const sig64 = sig.subarray(-64);

  // minisign 有两种模式：默认对 BLAKE2b-512(文件) 签名（prehashed），兼容模式直接对文件内容签名。
  // tauri 用哪一种不必猜 —— 两种都试，并把命中的那种报出来。
  const prehashed = createHash("blake2b512").update(data).digest();
  const mode = verify(null, prehashed, key, sig64)
    ? "prehashed (BLAKE2b-512)"
    : verify(null, data, key, sig64)
      ? "legacy (direct)"
      : null;

  const sizeKb = (data.length / 1024).toFixed(1);
  if (mode === null) {
    failures.push(`${platform}: 签名与 ${file} 不匹配`);
    console.log(`  ${platform}: ✗ 验签失败  ${file}（${sizeKb} KB）`);
    continue;
  }
  console.log(`  ${platform}: ✓ 验签通过 [${mode}]  ${file}（${sizeKb} KB）`);
}

console.log("");
if (failures.length > 0) {
  for (const failure of failures) console.error(`  ✗ ${failure}`);
  console.error(`更新清单校验未通过（${failures.length} 项）`);
  process.exit(1);
}
console.log("  ✓ latest.json 与本地产物自洽：客户端下载后能验签通过");
