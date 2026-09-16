//! netem-sim —— 弱网测试台（`docs/05-roadmap.md` M2 交付物 6）。
//!
//! 路线图 M2 的验收条件是「弱网（**5 Mbps / 2% 丢包 / 30 ms 抖动**）可听」；
//! 本工具就是那句话的可执行版本：一个**透明 UDP 中继**，按参数丢包 / 限速 / 加延迟与抖动。
//!
//! ```text
//! netem-sim run --upstream 127.0.0.1:58290 [--loss-pct 2] [--delay-ms 15] [--jitter-ms 15]
//!               [--bandwidth-kbps 5000] [--seed 1] [--seconds 60] [--report PATH] [--quiet]
//! ```
//!
//! 用法：把**发起方**的「连接地址」填成本工具打印的监听地址，其余不变；
//! QUIC 是端到端加密的，所以中继只搬字节、不需要理解协议。
//!
//! 退出码：`0` = 正常结束；`2` = 用法 / 绑定失败。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use audiolink_tools::netem::{NetemConfig, NetemRelay};

const USAGE: &str = "\
netem-sim —— 弱网测试台（丢包 / 延迟 / 抖动 / 限速的透明 UDP 中继）

用法：
  netem-sim run --upstream HOST:PORT [--loss-pct 2] [--delay-ms 15] [--jitter-ms 15]
                [--bandwidth-kbps 5000] [--seed 1] [--seconds 60] [--report PATH] [--quiet]

参数：
  --upstream        真正的服务端地址（例如接收端 Engine 的监听地址），必填
  --loss-pct        丢包率（百分比，可小数：1.5），默认 2（M2 验收口径）
  --delay-ms        单向固定延迟（ms），默认 15
  --jitter-ms       抖动幅度（ms，延迟在 基准 ± 该值 间均匀取值），默认 15
  --bandwidth-kbps  带宽上限（kbps），默认 5000；0 = 不限速
  --seed            注入随机种子（同种子 = 同一条注入序列，便于复现），默认 1
  --seconds         运行秒数；0 = 一直跑到 Ctrl+C，默认 60
  --report          JSON 报告路径，默认 target/evidence/netem/netem-<unix>.json
  --quiet           不打印每秒统计
";

fn main() -> ExitCode {
    match real_main() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("netem-sim: {error:#}");
            ExitCode::from(2)
        }
    }
}

fn real_main() -> Result<ExitCode> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        print!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    };
    if command == "-h" || command == "--help" || command == "help" {
        print!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    if command != "run" {
        bail!("未知子命令 {command}（用 --help 看用法）");
    }

    let mut upstream: Option<SocketAddr> = None;
    let mut loss_pct_x100 = NetemConfig::default().loss_pct_x100;
    let mut delay_ms = NetemConfig::default().delay_ms;
    let mut jitter_ms = NetemConfig::default().jitter_ms;
    let mut bandwidth_kbps = NetemConfig::default().bandwidth_kbps;
    let mut seed: u64 = 1;
    let mut seconds: u64 = 60;
    let mut report: Option<PathBuf> = None;
    let mut quiet = false;

    let mut index = 1;
    while index < args.len() {
        let key = args[index].as_str();
        match key {
            "--upstream" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("{key} 缺少取值"))?;
                upstream = Some(
                    raw.parse()
                        .with_context(|| format!("{key} 解析失败：{raw}"))?,
                );
                index += 2;
            }
            "--loss-pct" => {
                loss_pct_x100 = parse_pct_x100(&args, index, key)?;
                index += 2;
            }
            "--delay-ms" => {
                delay_ms = parse_u32(&args, index, key)?;
                index += 2;
            }
            "--jitter-ms" => {
                jitter_ms = parse_u32(&args, index, key)?;
                index += 2;
            }
            "--bandwidth-kbps" => {
                bandwidth_kbps = parse_u32(&args, index, key)?;
                index += 2;
            }
            "--seed" => {
                seed = parse_u64(&args, index, key)?;
                index += 2;
            }
            "--seconds" => {
                seconds = parse_u64(&args, index, key)?;
                index += 2;
            }
            "--report" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("{key} 缺少取值"))?
                    .clone();
                report = Some(PathBuf::from(value));
                index += 2;
            }
            "--quiet" => {
                quiet = true;
                index += 1;
            }
            other => bail!("未知参数 {other}（用 --help 看用法）"),
        }
    }

    let upstream = upstream.ok_or_else(|| anyhow!("--upstream 必填（用 --help 看用法）"))?;
    let config = NetemConfig {
        loss_pct_x100,
        delay_ms,
        jitter_ms,
        bandwidth_kbps,
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("创建 tokio 运行时失败")?;
    runtime.block_on(run(upstream, config, seed, seconds, report, quiet))
}

fn parse_u32(args: &[String], index: usize, key: &str) -> Result<u32> {
    let raw = args
        .get(index + 1)
        .ok_or_else(|| anyhow!("{key} 缺少取值"))?;
    raw.parse::<u32>()
        .with_context(|| format!("{key} 的取值不是非负整数：{raw}"))
}

fn parse_u64(args: &[String], index: usize, key: &str) -> Result<u64> {
    let raw = args
        .get(index + 1)
        .ok_or_else(|| anyhow!("{key} 缺少取值"))?;
    raw.parse::<u64>()
        .with_context(|| format!("{key} 的取值不是非负整数：{raw}"))
}

/// 解析「百分比」：`--loss-pct 1.5` → `150`（×100 的整数表达）。
fn parse_pct_x100(args: &[String], index: usize, key: &str) -> Result<u16> {
    let raw = args
        .get(index + 1)
        .ok_or_else(|| anyhow!("{key} 缺少取值"))?;
    let value: f64 = raw
        .parse()
        .with_context(|| format!("{key} 的取值不是数字：{raw}"))?;
    if !(0.0..=100.0).contains(&value) {
        bail!("{key} 必须在 0..=100 之间：{raw}");
    }
    Ok((value * 100.0).round() as u16)
}

async fn run(
    upstream: SocketAddr,
    config: NetemConfig,
    seed: u64,
    seconds: u64,
    report: Option<PathBuf>,
    quiet: bool,
) -> Result<ExitCode> {
    let relay = NetemRelay::start(upstream, config, seed)
        .await
        .context("起弱网中继失败")?;

    println!("=== netem-sim 弱网测试台 ===");
    println!("监听 {} → 上游 {upstream}", relay.addr());
    println!("注入 {}（种子 {seed}）", config.describe());
    println!("把发起方的连接地址改成 {} 即可。\n", relay.addr());

    let started = Instant::now();
    let mut next_tick = tokio::time::Instant::now();
    loop {
        if seconds > 0 && started.elapsed() >= Duration::from_secs(seconds) {
            break;
        }
        next_tick += Duration::from_secs(1);
        tokio::time::sleep_until(next_tick).await;
        if !quiet {
            let stats = relay.stats();
            print!(
                "\r    t={:>5}s  观察 {:>7}  转发 {:>7}  丢包 {:>6}（按丢包率 {} / 限速 {}）  ",
                started.elapsed().as_secs(),
                stats.observed,
                stats.forwarded,
                stats.dropped_loss + stats.dropped_bandwidth,
                stats.dropped_loss,
                stats.dropped_bandwidth,
            );
            use std::io::Write as _;
            let _ = std::io::stdout().flush();
        }
    }
    if !quiet {
        println!();
    }

    let elapsed = started.elapsed().as_secs_f64();
    let json = relay.report_json(elapsed);
    let path = report.unwrap_or_else(default_report_path);
    write_report(&path, &json).context("写报告失败")?;
    println!("{json}");
    println!("报告：{}", path.display());
    Ok(ExitCode::SUCCESS)
}

/// 默认报告路径：`target/evidence/netem/netem-<unix>.json`。
fn default_report_path() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    PathBuf::from("target")
        .join("evidence")
        .join("netem")
        .join(format!("netem-{stamp}.json"))
}

fn write_report(path: &Path, json: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建报告目录 {} 失败", parent.display()))?;
    }
    std::fs::write(path, json).with_context(|| format!("写 {} 失败", path.display()))
}
