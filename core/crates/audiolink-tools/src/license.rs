//! 依赖许可审计（M5 合规清单的第一步）。
//!
//! 输入是两份**现成**的 JSON：`cargo metadata`（Rust 侧）与 `pnpm licenses list --json`（前端侧）。
//! 本模块只做一件事：把许可表达式判成「能不能发布」，并渲染成人能读的报告。
//!
//! 判定语义（SPDX 表达式的三种连接词）：
//! - `A OR B`：**任一**选项可接受即整条可接受 —— 选择权在我们手里；
//! - `A AND B`：**全部**必须可接受 —— 义务叠加；
//! - `A WITH B`：把 `X WITH Y` 当**原子**（例外会改写义务，不能拆开看）。
//!
//! 1. **未知许可进 `notice`，不进 `denied`** —— 未知不等于安全，也不等于不能用；当允许会漏，当禁止会误杀。
//! 2. **`AND` 与 `OR` 混用的表达式一律进 `notice`** —— 完整 SPDX 解析（优先级/括号）不在本审计的范围内，
//!    与其猜，不如交给人看。
//! 3. 旧式写法 `MIT/Apache-2.0`（用 `/` 表示 OR）与 `Apache-2.0/MIT` 按历史约定当 `OR` 处理。

use serde_json::Value;

/// 许可判定三档。声明顺序即严格程度：`Allowed < Notice < Denied`（`Ord` 由此派生）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// 可直接发布（宽松许可，保留声明即可）。
    Allowed,
    /// 需要人看一眼：弱 copyleft，或**未知**许可。
    Notice,
    /// 不能发布（强 copyleft / 非商业 / 有争议的条款）。
    Denied,
}

impl Verdict {
    /// 报告里的标签。
    pub fn name(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Notice => "notice",
            Self::Denied => "denied",
        }
    }
}

/// 单条依赖的结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// 包名。
    pub name: String,
    /// 版本（前端聚合里可能是多版本拼接）。
    pub version: String,
    /// 原始许可表达式（未声明时是 `(未声明)`）。
    pub license: String,
    /// 判定。
    pub verdict: Verdict,
    /// 本仓库自己的 crate（`cargo metadata` 里 `source` 为空）。
    pub own: bool,
}

/// 可接受的许可原子（宽松：保留声明即可）。
const ALLOWED_ATOMS: &[&str] = &[
    "MIT",
    "MIT-0",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "0BSD",
    "Unlicense",
    "CC0-1.0",
    "CC-BY-4.0",
    "Unicode-3.0",
    "BSL-1.0",
    "Python-2.0",
    "OpenSSL",
    "NCSA",
    "WTFPL",
];

/// 需注意的许可原子（弱 copyleft：要提供修改后的源码与声明，但可以静态链接进闭源产品）。
const NOTICE_ATOMS: &[&str] = &[
    "MPL-2.0",
    "EPL-2.0",
    "CDDL-1.0",
    "LGPL-2.1-only",
    "LGPL-2.1-or-later",
    "LGPL-3.0-only",
    "LGPL-3.0-or-later",
    "CC-BY-SA-4.0",
];

/// 禁止的许可原子（强 copyleft / 非商业 / 条款有争议）。
const DENIED_ATOMS: &[&str] = &[
    "GPL-2.0",
    "GPL-2.0-only",
    "GPL-2.0-or-later",
    "GPL-3.0",
    "GPL-3.0-only",
    "GPL-3.0-or-later",
    "AGPL-3.0",
    "AGPL-3.0-only",
    "AGPL-3.0-or-later",
    "SSPL-1.0",
    "BUSL-1.1",
    "JSON",
    "Commons-Clause",
    "CC-BY-NC-4.0",
    "CC-BY-NC-SA-4.0",
];

fn in_list(list: &[&str], atom: &str) -> bool {
    let normalized = atom.trim().to_ascii_lowercase();
    list.iter()
        .any(|item| item.to_ascii_lowercase() == normalized)
}

/// 单个许可原子（或 `X WITH Y` 组合）的判定；未知一律 `Notice`。
#[must_use]
pub fn atom_verdict(atom: &str) -> Verdict {
    if in_list(ALLOWED_ATOMS, atom) {
        return Verdict::Allowed;
    }
    if in_list(NOTICE_ATOMS, atom) {
        return Verdict::Notice;
    }
    if in_list(DENIED_ATOMS, atom) {
        return Verdict::Denied;
    }
    Verdict::Notice
}

/// 把表达式切成原子与连接词：返回 `(atoms, ops)`，`ops[i]` 表示 `atoms[i]` 与 `atoms[i+1]` 之间是 `AND`。
fn split_expression(cleaned: &str) -> (Vec<String>, Vec<bool>) {
    let mut atoms: Vec<String> = Vec::new();
    let mut ops: Vec<bool> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for token in cleaned.split_whitespace() {
        let upper = token.to_ascii_uppercase();
        if upper == "AND" || upper == "OR" {
            if !pending.is_empty() {
                atoms.push(pending.join(" "));
                pending.clear();
            }
            ops.push(upper == "AND");
        } else {
            // `WITH` 与前后的许可名一起构成一个原子。
            pending.push(token.to_string());
        }
    }
    if !pending.is_empty() {
        atoms.push(pending.join(" "));
    }
    (atoms, ops)
}

/// 把一条 SPDX 表达式判成三档。
#[must_use]
pub fn classify(expression: &str) -> Verdict {
    // 括号只影响优先级；旧式 `/` 按历史约定表示 OR。
    let cleaned = expression.replace(['(', ')'], " ").replace('/', " OR ");
    let (atoms, ops) = split_expression(&cleaned);
    if atoms.is_empty() {
        return Verdict::Notice;
    }
    let has_and = ops.iter().any(|op| *op);
    let has_or = ops.iter().any(|op| !*op);
    if has_and && has_or {
        // 混用 = 优先级问题：不自动放行，也不自动拒绝。
        return Verdict::Notice;
    }
    let verdicts: Vec<Verdict> = atoms.iter().map(|atom| atom_verdict(atom)).collect();
    if has_and {
        // AND：全部必须可接受 → 取最严。
        verdicts.into_iter().max().unwrap_or(Verdict::Notice)
    } else {
        // OR：任一可用即可 → 取最宽。
        verdicts.into_iter().min().unwrap_or(Verdict::Notice)
    }
}

/// 从 `cargo metadata` 的 JSON 收集 Rust 侧依赖。
pub fn audit_cargo_metadata(json: &str) -> Result<Vec<Entry>, String> {
    let value: Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    let packages = value
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "cargo metadata 缺少 packages 字段".to_string())?;
    let mut entries = Vec::with_capacity(packages.len());
    for package in packages {
        let name = package
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        let version = package
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        let license = package
            .get("license")
            .and_then(Value::as_str)
            .or_else(|| package.get("license_file").and_then(Value::as_str))
            .unwrap_or("(未声明)")
            .to_string();
        // 本仓库自己的 crate：cargo metadata 里 source 为 null。
        let own = package.get("source").is_none_or(Value::is_null);
        let verdict = classify(&license);
        entries.push(Entry {
            name,
            version,
            license,
            verdict,
            own,
        });
    }
    Ok(entries)
}

/// 从 `pnpm licenses list --json` 的输出收集前端依赖。
///
/// 那份输出是**按许可分组**的：`{ "MIT": [ { name, versions: [...] }, ... ] }`。
pub fn audit_npm_licenses(json: &str) -> Result<Vec<Entry>, String> {
    let value: Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    let groups = value
        .as_object()
        .ok_or_else(|| "pnpm licenses 输出不是对象".to_string())?;
    let mut entries = Vec::new();
    for (license, packages) in groups {
        let verdict = classify(license);
        let Some(list) = packages.as_array() else {
            continue;
        };
        for package in list {
            let name = package
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            let version = package
                .get("versions")
                .and_then(Value::as_array)
                .map(|versions| {
                    versions
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            entries.push(Entry {
                name,
                version,
                license: license.clone(),
                verdict,
                own: false,
            });
        }
    }
    Ok(entries)
}

fn count(entries: &[Entry], verdict: Verdict) -> usize {
    entries
        .iter()
        .filter(|entry| entry.verdict == verdict)
        .count()
}

/// 渲染 Markdown 报告。
///
/// **幂等**：不含时间戳、不含路径 —— CI 里跑不会因为「什么时候跑的」产生 diff。
#[must_use]
pub fn render_report(rust: &[Entry], frontend: &[Entry]) -> String {
    let mut out = String::new();
    out.push_str("# 依赖许可审计报告（自动生成）\n\n");
    out.push_str("> 由 `tools/license-audit.ps1` 生成，**请勿手改**：改策略去改 `core/crates/audiolink-tools/src/license.rs` 的白/黑名单。\n");
    out.push_str("> 判定语义：`OR` 任一可用即可、`AND` 全部必须可用、`WITH` 当原子；\n");
    out.push_str("> 未声明的许可以及 `AND`/`OR` 混用的表达式一律进 `notice`（交给人看，不自动放行也不自动拒绝）。\n\n");
    out.push_str("| 来源 | 包数 | allowed | notice | denied |\n|---|---|---|---|---|\n");
    out.push_str(&format!(
        "| Rust（cargo metadata） | {} | {} | {} | {} |\n",
        rust.len(),
        count(rust, Verdict::Allowed),
        count(rust, Verdict::Notice),
        count(rust, Verdict::Denied)
    ));
    out.push_str(&format!(
        "| 前端（pnpm licenses） | {} | {} | {} | {} |\n\n",
        frontend.len(),
        count(frontend, Verdict::Allowed),
        count(frontend, Verdict::Notice),
        count(frontend, Verdict::Denied)
    ));

    for (label, entries) in [("Rust", rust), ("前端", frontend)] {
        let denied: Vec<&Entry> = entries
            .iter()
            .filter(|e| e.verdict == Verdict::Denied)
            .collect();
        if denied.is_empty() {
            out.push_str(&format!("**{label}：无 `denied` 依赖。**\n\n"));
        } else {
            out.push_str(&format!("### {label} · denied（{} 个，发布前必须处理）\n\n| 包 | 版本 | 许可 |\n|---|---|---|\n", denied.len()));
            for entry in denied {
                out.push_str(&format!(
                    "| {} | {} | {} |\n",
                    entry.name, entry.version, entry.license
                ));
            }
            out.push('\n');
        }

        let mut notice: Vec<&Entry> = entries
            .iter()
            .filter(|e| e.verdict == Verdict::Notice)
            .collect();
        if !notice.is_empty() {
            notice.sort_by(|a, b| a.name.cmp(&b.name));
            out.push_str(&format!("### {label} · notice（{} 个，需要人看一眼）\n\n| 包 | 版本 | 许可 |\n|---|---|---|\n", notice.len()));
            for entry in notice {
                out.push_str(&format!(
                    "| {} | {} | {} |\n",
                    entry.name, entry.version, entry.license
                ));
            }
            out.push('\n');
        }
    }

    out.push_str("## 覆盖面（诚实清单）\n\n");
    out.push_str("- **已覆盖**：Rust workspace 的全部依赖（`cargo metadata`）、桌面前端依赖（`pnpm licenses`）；\n");
    out.push_str("  Android（Gradle/Maven）依赖见**文末专节**（该节存在与否取决于是否采集过）。\n");
    out.push_str("- **未覆盖**：随包分发的二进制（.exe / .apk 内的第三方库）、字体与图标资源、以及 Android 侧的**投放位置**（声明入口）。\n");
    out.push_str(
        "- 本报告回答的是「许可是否允许这样分发」；署名/免责文本的**实际投放位置**是另一件事。\n",
    );
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn permissive_expressions_are_allowed() {
        assert_eq!(classify("MIT"), Verdict::Allowed);
        assert_eq!(classify("MIT OR Apache-2.0"), Verdict::Allowed);
        assert_eq!(classify("Apache-2.0 OR MIT"), Verdict::Allowed);
        assert_eq!(classify("MIT AND Apache-2.0"), Verdict::Allowed);
        assert_eq!(classify("Apache-2.0 WITH LLVM-exception"), Verdict::Allowed);
        assert_eq!(
            classify("Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT"),
            Verdict::Allowed
        );
        assert_eq!(classify("BSD-3-Clause"), Verdict::Allowed);
    }

    #[test]
    fn or_takes_the_most_permissive_option() {
        // 选择权在我们手里：MIT OR GPL-3.0 可以走 MIT。
        assert_eq!(classify("MIT OR GPL-3.0-only"), Verdict::Allowed);
        assert_eq!(classify("GPL-3.0-only OR MPL-2.0"), Verdict::Notice);
    }

    #[test]
    fn and_demands_every_option() {
        // 义务叠加：只要有一项不可用，整条不可用。
        assert_eq!(classify("MIT AND GPL-3.0-only"), Verdict::Denied);
        assert_eq!(classify("MIT AND MPL-2.0"), Verdict::Notice);
    }

    #[test]
    fn strong_copyleft_is_denied() {
        assert_eq!(classify("GPL-3.0-only"), Verdict::Denied);
        assert_eq!(classify("AGPL-3.0-or-later"), Verdict::Denied);
        assert_eq!(classify("SSPL-1.0"), Verdict::Denied);
    }

    #[test]
    fn weak_copyleft_needs_a_human_look() {
        assert_eq!(classify("MPL-2.0"), Verdict::Notice);
        assert_eq!(classify("LGPL-2.1-or-later"), Verdict::Notice);
    }

    #[test]
    fn unknown_is_notice_not_denied() {
        // 未知不等于安全，也不等于不能用 —— 两种错的代价不对称，所以交给人看。
        assert_eq!(classify("Frobnicate-1.0"), Verdict::Notice);
        assert_eq!(classify("(未声明)"), Verdict::Notice);
        assert_eq!(classify(""), Verdict::Notice);
    }

    #[test]
    fn legacy_slash_means_or() {
        // 数据里真实存在：`MIT/Apache-2.0`、`Unlicense/MIT`。
        assert_eq!(classify("MIT/Apache-2.0"), Verdict::Allowed);
        assert_eq!(classify("Unlicense/MIT"), Verdict::Allowed);
        assert_eq!(classify("MIT/GPL-3.0-only"), Verdict::Allowed);
    }

    #[test]
    fn parenthesised_expressions_are_flattened() {
        assert_eq!(classify("(MIT OR Apache-2.0)"), Verdict::Allowed);
        assert_eq!(classify("(MIT) AND (Apache-2.0)"), Verdict::Allowed);
    }

    #[test]
    fn mixed_and_or_needs_a_human_look() {
        // 混用意味着优先级问题：本审计不做完整 SPDX 解析。
        assert_eq!(classify("MIT OR (Apache-2.0 AND MPL-2.0)"), Verdict::Notice);
    }

    #[test]
    fn cargo_metadata_is_read_package_by_package() {
        let json = r#"{"packages":[
            {"name":"good","version":"1.0.0","license":"MIT","source":"registry+https://x"},
            {"name":"local","version":"0.1.0","license":"MPL-2.0","source":null}
        ]}"#;
        let entries = audit_cargo_metadata(json).expect("解析");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].verdict, Verdict::Allowed);
        assert!(!entries[0].own);
        assert!(entries[1].own, "source 为 null = 本仓库自己的 crate");
        assert_eq!(entries[1].verdict, Verdict::Notice);
    }

    #[test]
    fn missing_license_field_is_notice() {
        let json =
            r#"{"packages":[{"name":"mystery","version":"2.0.0","source":"registry+https://x"}]}"#;
        let entries = audit_cargo_metadata(json).expect("解析");
        assert_eq!(entries[0].license, "(未声明)");
        assert_eq!(entries[0].verdict, Verdict::Notice);
    }

    #[test]
    fn npm_groups_become_entries() {
        let json = r#"{"MIT":[{"name":"react","versions":["19.0.0","19.1.0"]}],"MPL-2.0":[{"name":"weird","versions":["1.0.0"]}]}"#;
        let mut entries = audit_npm_licenses(json).expect("解析");
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "react");
        assert_eq!(entries[0].version, "19.0.0, 19.1.0");
        assert_eq!(entries[0].verdict, Verdict::Allowed);
        assert_eq!(entries[1].verdict, Verdict::Notice);
    }

    #[test]
    fn report_is_idempotent_and_lists_both_sides() {
        let rust = vec![Entry {
            name: "bad".to_string(),
            version: "1.0.0".to_string(),
            license: "GPL-3.0-only".to_string(),
            verdict: Verdict::Denied,
            own: false,
        }];
        let frontend = vec![Entry {
            name: "warn".to_string(),
            version: "2.0.0".to_string(),
            license: "MPL-2.0".to_string(),
            verdict: Verdict::Notice,
            own: false,
        }];
        let first = render_report(&rust, &frontend);
        let second = render_report(&rust, &frontend);
        assert_eq!(
            first, second,
            "报告必须幂等（CI 里不该因为时间戳产生 diff）"
        );
        assert!(first.contains("| bad | 1.0.0 | GPL-3.0-only |"));
        assert!(first.contains("| warn | 2.0.0 | MPL-2.0 |"));
        assert!(first.contains("未覆盖"), "覆盖面必须写在报告里");
    }
}

// ---------------------------------------------------------------------------
// THIRD-PARTY-NOTICES 生成（M5 合规第二块）
// ---------------------------------------------------------------------------

use std::collections::BTreeMap;
use std::hash::{Hash as _, Hasher as _};
use std::path::{Path, PathBuf};

/// 一份**去重后**的许可全文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeText {
    /// 人类可读来源标签（取自文件名，如 `LICENSE-MIT`）。
    pub label: String,
    /// 全文。
    pub body: String,
}

/// 一个第三方组件在声明里的条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeEntry {
    /// 包名。
    pub name: String,
    /// 版本。
    pub version: String,
    /// 许可表达式。
    pub license: String,
    /// 指向 [`NoticeText`] 的下标（一个包可能带两份：如 `LICENSE-MIT` + `LICENSE-APACHE`）。
    pub texts: Vec<usize>,
}

/// 这个文件名看起来是不是许可/声明文件。
fn is_license_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let upper = name.to_ascii_uppercase();
    ["LICENSE", "LICENCE", "COPYING", "NOTICE"]
        .iter()
        .any(|prefix| upper.starts_with(prefix))
}

fn body_key(body: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    body.hash(&mut hasher);
    hasher.finish()
}

/// 从 `cargo metadata` 收集「组件 + 许可全文」。
///
/// 全文从每个包的 `manifest_path` **同目录**下读（`LICENSE*` / `LICENCE*` / `COPYING*` / `NOTICE*`），
/// 按**内容**去重：实测 635 个包里有 570 个带文件，但只有 148 份不同内容 —— 这正是
/// 「清单 + 全文」能塞进一个文档的原因。
///
/// 读不到文件不是错误：包可能只在 `Cargo.toml` 里写了 SPDX 标识。
pub fn collect_notices(json: &str) -> Result<(Vec<NoticeEntry>, Vec<NoticeText>), String> {
    let value: Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    let packages = value
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "cargo metadata 缺少 packages 字段".to_string())?;

    let mut texts: Vec<NoticeText> = Vec::new();
    let mut seen: BTreeMap<u64, usize> = BTreeMap::new();
    let mut entries = Vec::with_capacity(packages.len());

    for package in packages {
        // 本仓库自己的 crate 不进第三方声明（`cargo metadata` 里 source 为 null）——
        // 它们不是「第三方」，而且它们的许可文件在仓库根，不在各自目录下。
        if package.get("source").is_none_or(Value::is_null) {
            continue;
        }
        let name = package
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        let version = package
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        let license = package
            .get("license")
            .and_then(Value::as_str)
            .or_else(|| package.get("license_file").and_then(Value::as_str))
            .unwrap_or("(未声明)")
            .to_string();

        let mut indices: Vec<usize> = Vec::new();
        if let Some(directory) = package
            .get("manifest_path")
            .and_then(Value::as_str)
            .and_then(|manifest| Path::new(manifest).parent())
        {
            let mut files: Vec<PathBuf> = std::fs::read_dir(directory)
                .map(|read| {
                    read.filter_map(Result::ok)
                        .map(|item| item.path())
                        .filter(|path| path.is_file() && is_license_file(path))
                        .collect()
                })
                .unwrap_or_default();
            files.sort();
            for file in files {
                let Ok(body) = std::fs::read_to_string(&file) else {
                    continue;
                };
                if body.trim().is_empty() {
                    continue;
                }
                let key = body_key(&body);
                let index = if let Some(existing) = seen.get(&key) {
                    *existing
                } else {
                    let label = file
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("LICENSE")
                        .to_string();
                    texts.push(NoticeText { label, body });
                    let index = texts.len() - 1;
                    seen.insert(key, index);
                    index
                };
                indices.push(index);
            }
        }
        indices.sort_unstable();
        indices.dedup();
        entries.push(NoticeEntry {
            name,
            version,
            license,
            texts: indices,
        });
    }

    Ok((entries, texts))
}

/// 渲染 `THIRD-PARTY-NOTICES.md`。
///
/// **全量口径**：清单包含整棵依赖图（含构建期依赖），它是分发包实际内容集的**超集** ——
/// 多列不构成合规问题，漏列才是。收窄到「运行时子集」是可选的优化，不是必需。
#[must_use]
pub fn render_notices(entries: &[NoticeEntry], texts: &[NoticeText], frontend: &[Entry]) -> String {
    let mut out = String::new();
    out.push_str("# 第三方组件声明（THIRD-PARTY NOTICES）\n\n");
    out.push_str("> 由 `tools/license-audit.ps1 -Notices` 自动生成，**请勿手改**。\n");
    out.push_str(
        "> 口径：**整棵依赖图的超集**（含构建期依赖）—— 多列不构成合规问题，漏列才是。\n\n",
    );

    out.push_str("## 1. 组件清单（按许可分组）\n\n");
    let mut groups: BTreeMap<&str, Vec<&NoticeEntry>> = BTreeMap::new();
    for entry in entries {
        groups
            .entry(entry.license.as_str())
            .or_default()
            .push(entry);
    }
    for (license, list) in &mut groups {
        list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.version.cmp(&b.version)));
        out.push_str(&format!("### {} （{} 个）\n\n", license, list.len()));
        for entry in list.iter() {
            let marker = if entry.texts.is_empty() {
                " ⚠️无全文"
            } else {
                ""
            };
            out.push_str(&format!("- {} {}{}\n", entry.name, entry.version, marker));
        }
        out.push('\n');
    }

    if !frontend.is_empty() {
        out.push_str("### 前端依赖（pnpm 不提供许可文件路径，只列清单）\n\n");
        let mut list: Vec<&Entry> = frontend.iter().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        for entry in list {
            out.push_str(&format!(
                "- {} {}（{}）\n",
                entry.name, entry.version, entry.license
            ));
        }
        out.push('\n');
    }

    out.push_str("## 2. 许可全文（按内容去重）\n\n");
    for (index, text) in texts.iter().enumerate() {
        let users = entries
            .iter()
            .filter(|entry| entry.texts.contains(&index))
            .count();
        out.push_str(&format!(
            "### 文本 {} · {}（被 {} 个包引用）\n\n",
            index + 1,
            text.label,
            users
        ));
        out.push_str("```text\n");
        out.push_str(text.body.trim_end());
        out.push_str("\n```\n\n");
    }
    out.push_str(&format!("（共 {} 份去重后的许可全文）\n", texts.len()));
    out
}

#[cfg(test)]
mod notice_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// 本仓库自己的 crate 不该出现在第三方声明里。
    #[test]
    fn own_packages_are_not_listed() {
        let json = serde_json::json!({
            "packages": [
                {"name": "mine", "version": "0.1.0", "license": "Apache-2.0", "source": null},
                {"name": "theirs", "version": "1.0.0", "license": "MIT",
                 "source": "registry+https://github.com/rust-lang/crates.io-index"},
            ]
        })
        .to_string();
        let (entries, _texts) = collect_notices(&json).expect("收集");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "theirs");
    }

    /// 造一个最小 workspace：两个包共用同一份许可文本 → 只应留一份全文。
    #[test]
    fn identical_license_bodies_are_deduplicated() {
        let dir = tempfile::TempDir::new().expect("临时目录");
        let a = dir.path().join("alpha-1.0.0");
        let b = dir.path().join("beta-2.0.0");
        std::fs::create_dir_all(&a).expect("建目录");
        std::fs::create_dir_all(&b).expect("建目录");
        let body = "MIT License\n\nPermission is hereby granted, free of charge...\n";
        std::fs::write(a.join("LICENSE"), body).expect("写");
        std::fs::write(b.join("LICENSE"), body).expect("写");

        let json = serde_json::json!({
            "packages": [
                {"name": "alpha", "version": "1.0.0", "license": "MIT", "source": "registry+https://x",
                 "manifest_path": a.join("Cargo.toml").to_string_lossy()},
                {"name": "beta", "version": "2.0.0", "license": "MIT", "source": "registry+https://x",
                 "manifest_path": b.join("Cargo.toml").to_string_lossy()},
            ]
        })
        .to_string();

        let (entries, texts) = collect_notices(&json).expect("收集");
        assert_eq!(entries.len(), 2);
        assert_eq!(texts.len(), 1, "同一份文本只应留一份");
        assert_eq!(entries[0].texts, vec![0]);
        assert_eq!(entries[1].texts, vec![0]);
        assert!(texts[0].body.contains("Permission is hereby granted"));
    }

    /// 双许可包（LICENSE-MIT + LICENSE-APACHE）两份都要收，且非许可文件不收。
    #[test]
    fn dual_licensed_packages_keep_both_texts() {
        let dir = tempfile::TempDir::new().expect("临时目录");
        let pkg = dir.path().join("dual-1.0.0");
        std::fs::create_dir_all(&pkg).expect("建目录");
        std::fs::write(pkg.join("LICENSE-MIT"), "MIT 文本\n").expect("写");
        std::fs::write(pkg.join("LICENSE-APACHE"), "Apache 文本\n").expect("写");
        std::fs::write(pkg.join("README.md"), "不是许可文件\n").expect("写");

        let json = serde_json::json!({
            "packages": [
                {"name": "dual", "version": "1.0.0", "license": "MIT OR Apache-2.0", "source": "registry+https://x",
                 "manifest_path": pkg.join("Cargo.toml").to_string_lossy()},
            ]
        })
        .to_string();

        let (entries, texts) = collect_notices(&json).expect("收集");
        assert_eq!(entries[0].texts.len(), 2);
        assert_eq!(texts.len(), 2);
        assert!(texts.iter().any(|text| text.label == "LICENSE-APACHE"));
    }

    /// 只有 SPDX 标识、没有文件的包：条目照收，但必须标出「没有全文」。
    #[test]
    fn packages_without_files_are_still_listed() {
        let dir = tempfile::TempDir::new().expect("临时目录");
        let pkg = dir.path().join("bare-1.0.0");
        std::fs::create_dir_all(&pkg).expect("建目录");
        let json = serde_json::json!({
            "packages": [
                {"name": "bare", "version": "1.0.0", "license": "MIT", "source": "registry+https://x",
                 "manifest_path": pkg.join("Cargo.toml").to_string_lossy()},
            ]
        })
        .to_string();

        let (entries, texts) = collect_notices(&json).expect("收集");
        assert!(texts.is_empty());
        assert!(entries[0].texts.is_empty());
        let rendered = render_notices(&entries, &texts, &[]);
        assert!(rendered.contains("⚠️无全文"), "缺全文必须显式标出来");
    }

    #[test]
    fn notices_are_idempotent_and_grouped() {
        let entries = vec![
            NoticeEntry {
                name: "zeta".to_string(),
                version: "1.0.0".to_string(),
                license: "MIT".to_string(),
                texts: vec![0],
            },
            NoticeEntry {
                name: "alpha".to_string(),
                version: "2.0.0".to_string(),
                license: "MIT".to_string(),
                texts: vec![0],
            },
        ];
        let texts = vec![NoticeText {
            label: "LICENSE".to_string(),
            body: "MIT 全文".to_string(),
        }];
        let first = render_notices(&entries, &texts, &[]);
        let second = render_notices(&entries, &texts, &[]);
        assert_eq!(first, second, "声明必须幂等");
        assert!(first.contains("### MIT （2 个）"));
        let alpha_at = first.find("alpha 2.0.0").expect("列在清单里");
        let zeta_at = first.find("zeta 1.0.0").expect("列在清单里");
        assert!(alpha_at < zeta_at, "组内按名排序，输出才稳定");
        assert!(first.contains("被 2 个包引用"));
    }
}

// ---------------------------------------------------------------------------
// Android（Gradle/Maven）依赖：把 POM 里的许可名纳入同一套判定
// ---------------------------------------------------------------------------

/// 把 Maven POM 里的许可**名字**规范化成 SPDX 标识（认不出的原样保留）。
///
/// 为什么需要：Gradle 缓存的 POM 写的是 `Apache License, Version 2.0` 这类**自然语言名**，
/// 而不是 SPDX 标识。直接丢给 [`classify`] 会整片掉进 `notice` —— 把「真未知」和「只是写法不同」
/// 混成一类，报告就失去分辨力了。
#[must_use]
pub fn normalize_license_name(name: &str) -> String {
    let trimmed = name.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.is_empty() {
        return "(未声明)".to_string();
    }
    if lower == "mit" {
        return "MIT".to_string();
    }
    // 顺序有意义：`lesser general public license` 必须排在 `general public license` 之前，
    // 否则 LGPL 会被误判成 GPL（一个可分发、一个不可分发，差得很远）。
    const RULES: &[(&str, &str)] = &[
        ("apache software license, version 2.0", "Apache-2.0"),
        ("apache license, version 2.0", "Apache-2.0"),
        ("apache license 2.0", "Apache-2.0"),
        ("apache 2.0", "Apache-2.0"),
        ("apache-2.0", "Apache-2.0"),
        ("mit license", "MIT"),
        ("the mit license", "MIT"),
        ("3-clause bsd", "BSD-3-Clause"),
        ("bsd 3-clause", "BSD-3-Clause"),
        ("2-clause bsd", "BSD-2-Clause"),
        ("bsd 2-clause", "BSD-2-Clause"),
        ("eclipse public license - v 2.0", "EPL-2.0"),
        ("eclipse public license v. 2.0", "EPL-2.0"),
        ("eclipse public license - v 1.0", "EPL-1.0"),
        ("mozilla public license version 2.0", "MPL-2.0"),
        ("lesser general public license", "LGPL-2.1-or-later"),
        ("general public license", "GPL-2.0"),
        ("json license", "JSON"),
        ("isc license", "ISC"),
        ("the unlicense", "Unlicense"),
        ("cc0 1.0", "CC0-1.0"),
    ];
    for (pattern, spdx) in RULES {
        if lower.contains(pattern) {
            return (*spdx).to_string();
        }
    }
    // 认不出就原样返回：它会进 `notice`，而「未知不等于安全」这条规则照旧生效。
    trimmed.to_string()
}

/// 从 Android 依赖清单（JSON 数组：`[{name, version, license}]`）收集条目。
///
/// 采集在 Gradle 侧完成（解析依赖树 + 读缓存里的 POM），这里只做「规范化 + 判定」——
/// 于是它可以离线单测，也不需要在构建里引入第三方 Gradle 插件。
pub fn audit_android(json: &str) -> Result<Vec<Entry>, String> {
    let value: Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    let list = value
        .as_array()
        .ok_or_else(|| "Android 依赖清单不是数组".to_string())?;
    let mut entries = Vec::with_capacity(list.len());
    for item in list {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        let version = item
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        let raw = item
            .get("license")
            .and_then(Value::as_str)
            .unwrap_or("(未声明)");
        let license = normalize_license_name(raw);
        let verdict = classify(&license);
        entries.push(Entry {
            name,
            version,
            license,
            verdict,
            own: false,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod android_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn pom_names_are_normalised_to_spdx() {
        assert_eq!(
            normalize_license_name("Apache License, Version 2.0"),
            "Apache-2.0"
        );
        assert_eq!(
            normalize_license_name("The Apache Software License, Version 2.0"),
            "Apache-2.0"
        );
        assert_eq!(normalize_license_name("MIT"), "MIT");
        assert_eq!(normalize_license_name("MIT License"), "MIT");
        assert_eq!(
            normalize_license_name("3-Clause BSD License"),
            "BSD-3-Clause"
        );
        assert_eq!(normalize_license_name("The Unlicense"), "Unlicense");
    }

    #[test]
    fn lgpl_is_not_mistaken_for_gpl() {
        let lgpl = normalize_license_name("GNU Lesser General Public License");
        assert_eq!(lgpl, "LGPL-2.1-or-later");
        assert_eq!(
            classify(&lgpl),
            Verdict::Notice,
            "弱 copyleft：要人看，不是禁止"
        );
        let gpl = normalize_license_name("GNU General Public License, version 2");
        assert_eq!(gpl, "GPL-2.0");
        assert_eq!(classify(&gpl), Verdict::Denied);
    }

    #[test]
    fn unknown_pom_names_stay_unknown() {
        assert_eq!(
            normalize_license_name("Frobnicate License"),
            "Frobnicate License"
        );
        assert_eq!(normalize_license_name("   "), "(未声明)");
        assert_eq!(
            classify(&normalize_license_name("Frobnicate License")),
            Verdict::Notice
        );
    }

    #[test]
    fn android_manifest_becomes_entries() {
        let json = r#"[
            {"name": "androidx.compose.ui:ui", "version": "1.12.1", "license": "Apache License, Version 2.0"},
            {"name": "net.java.dev.jna:jna", "version": "5.19.1", "license": "LGPL-2.1-or-later"}
        ]"#;
        let entries = audit_android(json).expect("解析");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].license, "Apache-2.0");
        assert_eq!(entries[0].verdict, Verdict::Allowed);
        assert_eq!(entries[1].verdict, Verdict::Notice);
    }
}

/// 渲染 Android 依赖一节（追加到报告 / 声明尾部）。
///
/// 单独成函数而不是塞进 `render_report` 的签名：那份报告已经有两个来源参数，再加一个会把
/// 「谁是谁」挤没；Android 这一节作为**追加段**更清楚，也避免改签名带动一堆调用点。
#[must_use]
pub fn render_android_section(entries: &[Entry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("## Android（Gradle/Maven）依赖\n\n");
    out.push_str("> 采集：解析 `gradlew :app:dependencies` 的依赖树 + 读 Gradle 缓存里 POM 的 `<licenses>`；\n");
    out.push_str(
        "> POM 写的是自然语言许可名，规范化成 SPDX 与判定都在本 crate 里完成（可离线单测）。\n\n",
    );
    out.push_str(&format!(
        "| 指标 | 值 |\n|---|---|\n| 组件 | {} |\n| allowed | {} |\n| notice | {} |\n| denied | {} |\n\n",
        entries.len(),
        count(entries, Verdict::Allowed),
        count(entries, Verdict::Notice),
        count(entries, Verdict::Denied)
    ));
    let mut notable: Vec<&Entry> = entries
        .iter()
        .filter(|entry| entry.verdict != Verdict::Allowed)
        .collect();
    if !notable.is_empty() {
        notable.sort_by(|a, b| a.name.cmp(&b.name));
        out.push_str("| 包 | 版本 | 许可 | 判定 |\n|---|---|---|---|\n");
        for entry in notable {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                entry.name,
                entry.version,
                entry.license,
                entry.verdict.name()
            ));
        }
        out.push('\n');
    }
    out
}
