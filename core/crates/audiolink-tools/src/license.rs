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
    out.push_str("- **已覆盖**：Rust workspace 的全部依赖（`cargo metadata`）、桌面前端依赖（`pnpm licenses`）。\n");
    out.push_str("- **未覆盖**：Android（Gradle）依赖、随包分发的二进制（.exe / .apk 内的第三方库）、字体与图标资源。\n");
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
