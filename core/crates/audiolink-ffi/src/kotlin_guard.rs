//! 生成物护栏：挡住「源文件注释里的字符改变了另一门语言的代码语义」这一类**只有编译器才看得见**的缺陷。
//!
//! # 背景（这不是假想，是踩过的坑）
//!
//! UniFFI 把本 crate 的 rustdoc **原样**搬进生成的 Kotlin KDoc。Kotlin 的块注释**可以嵌套**，
//! 于是 rustdoc 里出现一次「斜杠紧跟星号」的字符序列，就会在 KDoc 里再开一层注释，
//! 需要两个结束标记才能闭合 —— 上一条路径通配符把生成物第 1779–2343 行整段吞掉，
//! `PcmFeed` / `PcmPull` 的 callback object 与全部顶层函数一起消失，
//! 编译器只报 `Unclosed comment`，**review 完全看不出来**（Android 侧跑真编译器才暴露）。
//!
//! 所以这里用两条机械断言把它钉死：
//!
//! 1. **源码侧**：[`hostile_doc_lines`] 扫 `src` 下每个 `.rs` 的文档注释行，不许出现那两个序列；
//! 2. **生成物侧**：[`scan_kotlin`] 按 Kotlin 的语法（嵌套块注释、字符串、字符字面量）走一遍
//!    `bindings/kotlin` 下的 `.kt`，必须以「正常状态」收尾、没有游离的结束标记，
//!    并断言导出面（§8 的十个函数 + 两个回调接口 + 错误类型）真的还在文件里。
//!
//! 第 2 条同时兜住「源码改了却没重新生成绑定」——那份 `.kt` 是**入库交付物**，
//! 它坏了必须在 `cargo test` 就红，而不是等 Android 侧编译。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

/// 扫描器状态（Kotlin 的块注释可以嵌套，所以深度不能只用一个 bool）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    LineComment,
    /// 块注释嵌套深度（进一层 / 出一层）。
    BlockComment(u32),
    String,
    /// 三引号原始字符串。
    RawString,
    Char,
}

/// 一次扫描的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Scan {
    /// 收尾时仍在的状态；`Normal` 才算闭合。
    final_state: State,
    /// 仍处于打开状态的块注释起始行（1 起）。
    unclosed_comment_at: Option<usize>,
    /// 在正常状态下遇到的游离结束标记所在行。
    stray_close_at: Option<usize>,
}

impl Scan {
    fn is_clean(&self) -> bool {
        self.final_state == State::Normal
            && self.unclosed_comment_at.is_none()
            && self.stray_close_at.is_none()
    }

    fn describe(&self) -> String {
        format!(
            "final_state={:?} unclosed_comment_at={:?} stray_close_at={:?}",
            self.final_state, self.unclosed_comment_at, self.stray_close_at
        )
    }
}

/// 按 Kotlin 词法走一遍源码：块注释**嵌套**、字符串/字符字面量里的符号不算结构。
fn scan_kotlin(source: &str) -> Scan {
    let chars: Vec<char> = source.chars().collect();
    let mut state = State::Normal;
    let mut line = 1usize;
    let mut index = 0usize;
    let mut block_stack: Vec<usize> = Vec::new();
    let mut stray_close_at = None;

    while index < chars.len() {
        let current = chars[index];
        let next = chars.get(index + 1).copied();
        let after_next = chars.get(index + 2).copied();

        match state {
            State::Normal => {
                if current == '/' && next == Some('/') {
                    state = State::LineComment;
                    index += 2;
                    continue;
                }
                if current == '/' && next == Some('*') {
                    block_stack.push(line);
                    state = State::BlockComment(1);
                    index += 2;
                    continue;
                }
                if current == '*' && next == Some('/') {
                    stray_close_at = Some(line);
                    index += 2;
                    continue;
                }
                if current == '"' && next == Some('"') && after_next == Some('"') {
                    state = State::RawString;
                    index += 3;
                    continue;
                }
                if current == '"' {
                    state = State::String;
                } else if current == '\'' {
                    state = State::Char;
                }
            }
            State::LineComment => {
                if current == '\n' {
                    state = State::Normal;
                }
            }
            State::BlockComment(depth) => {
                if current == '/' && next == Some('*') {
                    block_stack.push(line);
                    state = State::BlockComment(depth + 1);
                    index += 2;
                    continue;
                }
                if current == '*' && next == Some('/') {
                    block_stack.pop();
                    state = if depth <= 1 {
                        State::Normal
                    } else {
                        State::BlockComment(depth - 1)
                    };
                    index += 2;
                    continue;
                }
            }
            State::String => {
                if current == '\\' {
                    index += 2;
                    if current == '\n' {
                        line += 1;
                    }
                    continue;
                }
                if current == '"' {
                    state = State::Normal;
                }
            }
            State::RawString => {
                if current == '"' && next == Some('"') && after_next == Some('"') {
                    state = State::Normal;
                    index += 3;
                    continue;
                }
            }
            State::Char => {
                if current == '\\' {
                    index += 2;
                    continue;
                }
                if current == '\'' {
                    state = State::Normal;
                }
            }
        }

        if current == '\n' {
            line += 1;
        }
        index += 1;
    }

    Scan {
        final_state: state,
        unclosed_comment_at: block_stack.last().copied(),
        stray_close_at,
    }
}

/// 扫一份 Rust 源码里的**文档注释行**（`///` / `//!`），返回含危险序列的行号与内容。
///
/// 只查文档注释：UniFFI 只把 rustdoc 搬进 KDoc，代码里的字符串不受影响。
/// 行内相邻才算数 —— KDoc 渲染时每行独立加前缀，跨行不会拼出那两个序列。
fn hostile_doc_lines(source: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (offset, raw) in source.lines().enumerate() {
        let trimmed = raw.trim_start();
        let body = if let Some(rest) = trimmed.strip_prefix("///") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("//!") {
            rest
        } else {
            continue;
        };
        let chars: Vec<char> = body.chars().collect();
        let hostile = chars
            .iter()
            .enumerate()
            .any(|(index, c)| match chars.get(index + 1) {
                Some(next) => (*c == '/' && *next == '*') || (*c == '*' && *next == '/'),
                None => false,
            });
        if hostile {
            found.push((offset + 1, raw.trim().to_string()));
        }
    }
    found
}

/// 递归收集目录下的 `.kt` 文件（不引第三方 walker）。
fn collect_kotlin_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_kotlin_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "kt") {
            out.push(path);
        }
    }
}

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `bindings/kotlin` 下生成的 `.kt`（入库交付物）。
fn generated_bindings() -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_kotlin_files(&crate_root().join("bindings").join("kotlin"), &mut files);
    files.sort();
    files
}

/// 递归收集 `src` 下的 `.rs`。
fn source_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![crate_root().join("src")];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §8 冻结的导出面：这些符号必须真的出现在生成物里。
    ///
    /// 断言的是「函数还在不在」，而不是它们的签名细节 —— 签名有 Rust 侧的类型检查盯着，
    /// 这里盯的是「被注释吞掉 / 生成链路漏了」这种**整段消失**的故障。
    const REQUIRED_SYMBOLS: &[&str] = &[
        "fun `engineStart`",
        "fun `engineStop`",
        "fun `connect`",
        "fun `startSend`",
        "fun `stopSend`",
        "fun `submitPin`",
        "fun `localStatus`",
        "fun `peers`",
        "fun `telemetry`",
        "fun `displayedPin`",
        "fun `protocolSelfTest`",
        "interface PcmFeed",
        "interface PcmPull",
        "sealed class FfiException",
        "val `messageText`",
        "uniffiCallbackInterfacePcmFeed.register",
        "uniffiCallbackInterfacePcmPull.register",
    ];

    /// 护栏 1：源码侧的 rustdoc 不许含会让 KDoc 走样的字符序列。
    #[test]
    fn 源码文档注释不含_kotlin_危险序列() {
        let mut offenders: Vec<String> = Vec::new();
        for file in source_files() {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            for (line, content) in hostile_doc_lines(&text) {
                let name = file
                    .strip_prefix(crate_root())
                    .unwrap_or(&file)
                    .to_string_lossy()
                    .to_string();
                offenders.push(format!("{name}:{line}: {content}"));
            }
        }
        assert!(
            offenders.is_empty(),
            "文档注释里的「斜杠紧跟星号 / 星号紧跟斜杠」会被原样带进 Kotlin KDoc，\
             而 Kotlin 块注释可嵌套 —— 它会把生成物从那里一直吞到文件尾。\n\
             改写方式：去掉通配符或用文字绕开（例如「`bindings/kotlin/` 目录下的 `.kt` 文件」）。\n{}",
            offenders.join("\n")
        );
    }

    /// 护栏 2：生成物必须在正常状态下收尾（Kotlin 块注释嵌套计数归零、无游离结束标记）。
    #[test]
    fn 生成物块注释闭合() {
        let files = generated_bindings();
        assert!(
            !files.is_empty(),
            "找不到生成的 Kotlin 绑定：先跑 bindings/README.md 里的生成命令（`.kt` 是入库交付物）。"
        );

        let mut broken: Vec<String> = Vec::new();
        for file in &files {
            let Ok(text) = std::fs::read_to_string(file) else {
                continue;
            };
            let scan = scan_kotlin(&text);
            if !scan.is_clean() {
                broken.push(format!("{}: {}", file.display(), scan.describe()));
            }
        }
        assert!(
            broken.is_empty(),
            "生成物没有闭合（多半是 rustdoc 里的危险序列污染了 KDoc）:\n{}",
            broken.join("\n")
        );
    }

    /// 护栏 3：§8 的导出面真的还在生成物里（防「整段被吞掉」）。
    #[test]
    fn 生成物包含契约_s8_的导出面() {
        let files = generated_bindings();
        assert!(!files.is_empty(), "找不到生成的 Kotlin 绑定");
        let text: String = files
            .iter()
            .filter_map(|file| std::fs::read_to_string(file).ok())
            .collect();

        let missing: Vec<&str> = REQUIRED_SYMBOLS
            .iter()
            .copied()
            .filter(|symbol| !text.contains(symbol))
            .collect();
        assert!(
            missing.is_empty(),
            "生成物里缺这些符号（绑定没重新生成 / 导出面被吞）: {missing:?}"
        );
    }

    /// 扫描器自证：嵌套块注释必须数得准，否则护栏 2 是装饰。
    #[test]
    fn 扫描器认得嵌套块注释与字符串() {
        // 合法的单层块注释 + 行注释 + 字符串里的假符号
        let ok = "// line\n/* block */\nval s = \"/* not a comment */\"\n";
        assert!(scan_kotlin(ok).is_clean(), "{:?}", scan_kotlin(ok));

        // 嵌套：内层 `/*` 让外层多要一个结束标记 → 必须以非正常状态收尾
        let nested = "/**\n * 见 `bindings/kotlin/** 通配`。\n */\nfun x() {}\n";
        let scan = scan_kotlin(nested);
        assert!(!scan.is_clean(), "嵌套块注释必须被判定为未闭合: {scan:?}");
        assert!(scan.unclosed_comment_at.is_some());

        // 游离结束标记同样是坏味道
        let stray = "val s = 1\n*/\n";
        assert!(scan_kotlin(stray).stray_close_at.is_some());
    }

    /// 源码扫描器自证：真值表要对（否则护栏 1 是装饰）。
    #[test]
    fn 危险序列判定真值表() {
        assert!(hostile_doc_lines("/// 见 `a/**` 通配").len() == 1);
        assert!(hostile_doc_lines("//! 见 `a/**` 通配").len() == 1);
        assert!(hostile_doc_lines("/// 收尾 `*/`").len() == 1);
        // 「`/` 紧跟 `*`」用引号隔开就不构成序列
        assert!(hostile_doc_lines("/// 不许出现「`/` 紧跟 `*`」").is_empty());
        // 代码里的字符串与普通注释不受约束（UniFFI 不搬它们）
        assert!(hostile_doc_lines("let x = \"/*\";").is_empty());
        assert!(hostile_doc_lines("// 普通注释 /* 无所谓 */").is_empty());
    }

    /// 护栏 4：生成物里的 **record 字段**与 Rust 侧逐字段一致（含声明顺序）。
    ///
    /// # 为什么必须有它（实测结论，不是推测）
    ///
    /// UniFFI 0.29 只生成**函数 / 方法级** checksum（`checksum_func_*` / `checksum_method_*`），
    /// **没有 record 级 checksum**。实测（2026-09-17）：给 `EngineStartConfig` 加一个字段，
    /// `uniffi_audiolink_ffi_checksum_func_engine_start` 的值**一字不变**（前后都是 32272）。
    ///
    /// 于是「改了 Rust 的 record 字段却忘了重新生成绑定」既不会被 `cargo test` 拦住
    /// （护栏 2 的块注释闭合、护栏 3 的导出面都仍然成立），也不会在第一次 FFI 调用时被 checksum
    /// 拦住 —— `.kt` 的 `FfiConverter` 少写一个字段，而 `.so` 按新结构读，两边**静默错位**。
    /// 所以这里做纯文本比对：Rust 的 `pub struct <Name>` 与生成物的 `data class <Name>`。
    #[test]
    fn 生成物记录字段与源码一致() {
        const RECORDS: &[&str] = &[
            "EngineStartConfig",
            "LocalStatus",
            "PeerView",
            "TelemetryView",
        ];

        let source = std::fs::read_to_string(crate_root().join("src").join("engine_bridge.rs"))
            .expect("读 engine_bridge.rs");
        let files = generated_bindings();
        assert!(!files.is_empty(), "找不到生成的 Kotlin 绑定");
        let kotlin: String = files
            .iter()
            .filter_map(|file| std::fs::read_to_string(file).ok())
            .collect();

        let mut problems = Vec::new();
        for name in RECORDS {
            let rust_fields = rust_record_fields(&source, name);
            assert!(
                !rust_fields.is_empty(),
                "Rust 侧找不到 record {name} —— 护栏自身失效了（改了名字就得同步这里）"
            );
            let kotlin_fields: Vec<String> = kotlin_record_fields(&kotlin, name)
                .iter()
                .map(|field| camel_to_snake(field))
                .collect();
            if rust_fields != kotlin_fields {
                problems.push(format!(
                    "{name}：Rust {rust_fields:?} ≠ 生成物 {kotlin_fields:?}（多半是改了字段却忘了重新生成绑定）"
                ));
            }
        }
        assert!(
            problems.is_empty(),
            "生成物与源码的 record 字段不一致:\n{}",
            problems.join("\n")
        );
    }

    /// 取 Rust 里 `pub struct <name> { … }` 的字段名（声明顺序）。
    fn rust_record_fields(source: &str, name: &str) -> Vec<String> {
        let marker = format!("pub struct {name} {{");
        let Some(start) = source.find(&marker) else {
            return Vec::new();
        };
        let body = &source[start + marker.len()..];
        let end = body.find("\n}").unwrap_or(body.len());
        body[..end]
            .lines()
            .filter_map(|line| {
                let rest = line.trim().strip_prefix("pub ")?;
                let (field, _) = rest.split_once(':')?;
                Some(field.trim().to_string())
            })
            .collect()
    }

    /// 取生成物里 `data class <name> ( … )` 的字段名（声明顺序）。
    fn kotlin_record_fields(source: &str, name: &str) -> Vec<String> {
        let marker = format!("data class {name} (");
        let Some(start) = source.find(&marker) else {
            return Vec::new();
        };
        let body = &source[start + marker.len()..];
        let end = body.find("\n) {").unwrap_or(body.len());
        body[..end]
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let rest = line
                    .strip_prefix("var `")
                    .or_else(|| line.strip_prefix("val `"))?;
                let (field, _) = rest.split_once('`')?;
                Some(field.to_string())
            })
            .collect()
    }

    /// `dataDir` → `data_dir`（UniFFI 的 Kotlin 命名规则）。
    fn camel_to_snake(camel: &str) -> String {
        let mut out = String::with_capacity(camel.len() + 4);
        for ch in camel.chars() {
            if ch.is_ascii_uppercase() {
                out.push('_');
                out.push(ch.to_ascii_lowercase());
            } else {
                out.push(ch);
            }
        }
        out
    }
}
