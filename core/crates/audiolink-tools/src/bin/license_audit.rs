//! `license-audit` —— 读两份 JSON（`cargo metadata` / `pnpm licenses list`），判定许可并渲染报告。
//!
//! 为什么采集不在这里做：`cargo` 与 `pnpm` 的调用方式属于**环境知识**，放在 `tools/license-audit.ps1`
//! 一处即可；本程序只负责「判定 + 渲染 + 退出码」，于是它可以离线单测（见 `audiolink_tools::license`）。
//!
//! 退出码：0 = 没有 denied；1 = 有 denied（发布前必须处理）；2 = 用法或解析错误。

use std::path::PathBuf;

use anyhow::Context;
use audiolink_tools::license::{
    Verdict, audit_android, audit_cargo_metadata, audit_npm_licenses, collect_notices,
    render_android_section, render_notices, render_report,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("license-audit: {error:#}");
        std::process::exit(2);
    }
}

fn run() -> anyhow::Result<()> {
    let mut cargo_path: Option<PathBuf> = None;
    let mut npm_path: Option<PathBuf> = None;
    let mut write_path: Option<PathBuf> = None;
    let mut notices_path: Option<PathBuf> = None;
    let mut android_path: Option<PathBuf> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--cargo" => cargo_path = args.next().map(PathBuf::from),
            "--npm" => npm_path = args.next().map(PathBuf::from),
            "--write" => write_path = args.next().map(PathBuf::from),
            "--notices" => notices_path = args.next().map(PathBuf::from),
            "--android" => android_path = args.next().map(PathBuf::from),
            "--help" | "-h" => {
                usage();
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other}（--help 看用法）"),
        }
    }

    let cargo_path = cargo_path.context("--cargo <path> 是必需的")?;
    let rust_json = std::fs::read_to_string(&cargo_path)
        .with_context(|| format!("读取 {}", cargo_path.display()))?;
    let rust = audit_cargo_metadata(&rust_json).map_err(anyhow::Error::msg)?;

    let frontend = match npm_path {
        Some(path) => {
            let json = std::fs::read_to_string(&path)
                .with_context(|| format!("读取 {}", path.display()))?;
            audit_npm_licenses(&json).map_err(anyhow::Error::msg)?
        }
        None => Vec::new(),
    };

    // Android（Gradle/Maven）依赖：采集在 Gradle 侧，这里只做规范化 + 判定。
    let android = match android_path {
        Some(path) => {
            let json = std::fs::read_to_string(&path)
                .with_context(|| format!("读取 {}", path.display()))?;
            audit_android(&json).map_err(anyhow::Error::msg)?
        }
        None => Vec::new(),
    };

    let mut report = render_report(&rust, &frontend);
    report.push_str(&render_android_section(&android));
    if let Some(path) = write_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建 {}", parent.display()))?;
        }
        std::fs::write(&path, &report).with_context(|| format!("写入 {}", path.display()))?;
        println!("license-audit: 报告已写入 {}", path.display());
    }

    if let Some(path) = notices_path {
        let (entries, texts) = collect_notices(&rust_json).map_err(anyhow::Error::msg)?;
        let mut notices = render_notices(&entries, &texts, &frontend);
        notices.push_str(&render_android_section(&android));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建 {}", parent.display()))?;
        }
        std::fs::write(&path, &notices).with_context(|| format!("写入 {}", path.display()))?;
        println!(
            "license-audit: 第三方声明已写入 {}（{} 个组件 / {} 份去重全文）",
            path.display(),
            entries.len(),
            texts.len()
        );
    }

    let denied: Vec<_> = rust
        .iter()
        .chain(frontend.iter())
        .filter(|entry| entry.verdict == Verdict::Denied)
        .collect();
    let notice = rust
        .iter()
        .chain(frontend.iter())
        .filter(|entry| entry.verdict == Verdict::Notice)
        .count();
    println!(
        "license-audit: Rust {} 包 / 前端 {} 包 / Android {} 包 · denied {} · notice {}",
        rust.len(),
        frontend.len(),
        android.len(),
        denied.len(),
        notice
    );
    for entry in &denied {
        println!(
            "  DENIED  {} {} ({})",
            entry.name, entry.version, entry.license
        );
    }
    if !denied.is_empty() {
        anyhow::bail!("{} 个依赖的许可不允许这样分发", denied.len());
    }
    Ok(())
}

fn usage() {
    println!(
        "license-audit --cargo <cargo-metadata.json> [--npm <pnpm-licenses.json>] \\
         [--write <report.md>] [--notices <THIRD-PARTY-NOTICES.md>]"
    );
}
