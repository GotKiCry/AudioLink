//! soak-runner —— 回环长跑 + 指标采集 + 异常快照（`docs/05-roadmap.md` M2 的 8 h soak 验收）。
//!
//! # 它解决什么问题
//!
//! 「跑 8 h 不出故障」以前只能靠人盯着看日志。本工具把这件事变成**可判定的产物**：
//!
//! - 同一进程里起两个**真实 Engine**，走 127.0.0.1 上的真实 QUIC（与 `link-loop` 同一条链路）；
//! - 用合成源 + NullPlayout，**不出声也不采集声卡**，因此可以无人值守长跑；
//! - 每 1 s 采一次**接收侧**遥测，任何「回环稳态不该出现」的增量（欠载 / PCM 掩盖 / 丢包 /
//!   迟到丢弃 / NACK 重传）都会留下一份**异常快照**（含当时的完整遥测）；
//! - 结束写一份 JSON 报告（summary + violations + 每分钟粗采样），退出码即判定：
//!   `0` = 无异常，`1` = 有异常，`2` = 用法 / 初始化失败。
//!
//! ```text
//! soak-runner run [--seconds 28800] [--frame-ms 20] [--report PATH] [--expected-bps 320000] [--warmup-seconds 3] [--quiet]
//! ```
//!
//! 短时自检（CI 冒烟）：`soak-runner run --seconds 60`。真正的 8 h 长跑用 `--seconds 28800`。

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use audiolink_audio::{NullPlayout, PlayoutSink, SyntheticCapture};
use audiolink_engine::session::SessionState;
use audiolink_engine::{Engine, EngineConfig, EngineEvent};
use audiolink_tools::soak::{SoakMeta, SoakMonitor, SoakSample, SoakThresholds};
use audiolink_types::{ErrorCode, NodeId, StreamStats};

const USAGE: &str = "\
soak-runner —— 回环长跑 + 指标采集 + 异常快照

用法：
  soak-runner run [--seconds 28800] [--frame-ms 20] [--report PATH] [--expected-bps 320000] [--warmup-seconds 3] [--quiet]

参数：
  --seconds         观测时长（秒），默认 28800（8 h）；冒烟用 60
  --frame-ms        帧长（10 / 20 / 40 / 60），默认 20
  --report          报告路径，默认 target/evidence/soak/soak-<unix 秒>.json
  --expected-bps    目标码率（bps），默认 320000（冗余双发后的期望值）；0 = 不判码率
  --warmup-seconds  预热秒数（不参与判定），默认 3
  --quiet           不打印每秒进度

说明：
  两个真实 Engine（node-a 发送 / node-b 接收）走 127.0.0.1 的真实 QUIC；
  node-b 之前不认识 node-a，因此会实跑一遍 §5 的 PIN 配对流程。
  退出码：0 = 无异常；1 = 有异常；2 = 用法或初始化失败。
";

fn main() -> ExitCode {
    match real_main() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("soak-runner: {error:#}");
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

    let mut seconds: u64 = 28_800;
    let mut frame_ms: u32 = 20;
    let mut warmup_seconds: u64 = 3;
    let mut expected_bps: u32 = 320_000;
    let mut report: Option<PathBuf> = None;
    let mut quiet = false;

    let mut index = 1;
    while index < args.len() {
        let key = args[index].as_str();
        match key {
            "--seconds" => {
                seconds = next_u64(&args, index, key)?;
                index += 2;
            }
            "--frame-ms" => {
                frame_ms = u32::try_from(next_u64(&args, index, key)?)
                    .map_err(|_| anyhow!("{key} 超出范围"))?;
                index += 2;
            }
            "--warmup-seconds" => {
                warmup_seconds = next_u64(&args, index, key)?;
                index += 2;
            }
            "--expected-bps" => {
                expected_bps = u32::try_from(next_u64(&args, index, key)?)
                    .map_err(|_| anyhow!("{key} 超出范围"))?;
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

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("创建 tokio 运行时失败")?;
    runtime.block_on(run(
        seconds,
        frame_ms,
        warmup_seconds,
        expected_bps,
        report,
        quiet,
    ))
}

fn next_u64(args: &[String], index: usize, key: &str) -> Result<u64> {
    let raw = args
        .get(index + 1)
        .ok_or_else(|| anyhow!("{key} 缺少取值"))?;
    raw.parse::<u64>()
        .with_context(|| format!("{key} 的取值不是整数：{raw}"))
}

async fn run(
    seconds: u64,
    frame_ms: u32,
    warmup_seconds: u64,
    expected_bps: u32,
    report: Option<PathBuf>,
    quiet: bool,
) -> Result<ExitCode> {
    println!("=== AudioLink soak-runner（真实 QUIC 回环 · 不出声）===");
    println!(
        "计划 {seconds} s · 帧长 {frame_ms} ms · 预热 {warmup_seconds} s · 目标码率 {expected_bps} bps\n"
    );

    let dir = tempfile::TempDir::new().context("创建临时目录失败")?;
    let dir_a = dir.path().join("node-a");
    let dir_b = dir.path().join("node-b");
    std::fs::create_dir_all(&dir_a).context("创建 node-a 目录失败")?;
    std::fs::create_dir_all(&dir_b).context("创建 node-b 目录失败")?;

    let mut config_a = EngineConfig::new("soak-sender", &dir_a);
    config_a.listen = "127.0.0.1:0".parse().context("解析监听地址失败")?;
    config_a.codec.frame_ms = frame_ms;
    config_a.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));

    let mut config_b = EngineConfig::new("soak-receiver", &dir_b);
    config_b.listen = "127.0.0.1:0".parse().context("解析监听地址失败")?;
    config_b.codec.frame_ms = frame_ms;
    config_b.playout = Some(Arc::new(move || {
        Ok(Box::new(NullPlayout::new(60)) as Box<dyn PlayoutSink>)
    }));

    let engine_a = Engine::start(config_a)
        .await
        .map_err(link_error)
        .context("启动 node-a 失败")?;
    let engine_b = Engine::start(config_b)
        .await
        .map_err(link_error)
        .context("启动 node-b 失败")?;

    let _accept_a = engine_a.spawn_accept_loop();
    let _accept_b = engine_b.spawn_accept_loop();
    println!(
        "引擎就绪：node-a {} / node-b {}",
        engine_a.local_addr(),
        engine_b.local_addr()
    );

    // ---- §5 握手 + PIN 配对 ----
    let mut events_b = engine_b.subscribe();
    let peer_on_a = match engine_a.connect(engine_b.local_addr()).await {
        Ok(_) => first_peer(&engine_a).ok_or_else(|| anyhow!("node-a 侧没有建立会话"))?,
        Err(error) if error.code() == ErrorCode::NotPaired => {
            let peer_on_a =
                first_peer(&engine_a).ok_or_else(|| anyhow!("node-a 侧没有建立会话"))?;
            let pin = wait_for_pin(&mut events_b, Duration::from_secs(5)).await?;
            engine_a
                .submit_pin(peer_on_a, &pin)
                .await
                .map_err(link_error)
                .context("提交 PIN 失败")?;
            peer_on_a
        }
        Err(error) => return Err(link_error(error)).context("连接 node-b 失败"),
    };
    wait_for_streaming(&engine_a, peer_on_a, Duration::from_secs(5)).await?;
    let peer_on_b = first_peer(&engine_b).ok_or_else(|| anyhow!("node-b 侧没有建立会话"))?;
    println!(
        "配对完成：node-a 侧 peer {} / node-b 侧 peer {}",
        peer_on_a.short(),
        peer_on_b.short()
    );

    // ---- 开流并观测 ----
    engine_a
        .start_send(peer_on_a)
        .await
        .map_err(link_error)
        .context("开流失败")?;
    tokio::time::sleep(Duration::from_millis(600)).await;

    let started_at_unix = unix_now();
    let mut monitor = SoakMonitor::new(SoakThresholds::default(), expected_bps);
    let started = std::time::Instant::now();
    let mut next_tick = tokio::time::Instant::now();
    println!("开始观测（预热 {warmup_seconds} s 不参与判定）…");

    while started.elapsed() < Duration::from_secs(seconds) {
        next_tick += Duration::from_secs(1);
        tokio::time::sleep_until(next_tick).await;
        let at_secs = started.elapsed().as_secs();
        let stats = engine_b.telemetry(peer_on_b).unwrap_or_default();
        let state = peer_state(&engine_b, peer_on_b);

        if at_secs >= warmup_seconds {
            monitor.observe(SoakSample {
                at_secs,
                state,
                stats,
            });
        }

        if !quiet {
            print_progress(at_secs, state, stats, monitor.violations().len());
        }
    }
    if !quiet {
        println!();
    }

    // ---- 报告 ----
    let meta = SoakMeta {
        planned_seconds: seconds,
        frame_ms,
        expected_bitrate_bps: expected_bps,
        started_at_unix,
    };
    let path = report.unwrap_or_else(|| default_report_path(started_at_unix));
    write_report(&path, &monitor.report_json(&meta)).context("写报告失败")?;

    println!("{}", monitor.summary_text(&meta));
    println!("报告：{}", path.display());

    engine_a.shutdown().await;
    engine_b.shutdown().await;

    let verdict = monitor.summary().verdict;
    println!("退出码对应判定：{verdict}");
    Ok(if verdict == "ok" {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
// ---------------------------------------------------------------------------
// 辅助
// ---------------------------------------------------------------------------

fn first_peer(engine: &Engine) -> Option<NodeId> {
    engine.peers().first().map(|peer| peer.id)
}

/// 当前会话状态名；对端已消失 → `"gone"`（监视器会把它记成 not_streaming 异常）。
fn peer_state(engine: &Engine, peer: NodeId) -> &'static str {
    engine
        .peers()
        .iter()
        .find(|status| status.id == peer)
        .map_or("gone", |status| status.state.name())
}

async fn wait_for_pin(
    events: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
    timeout: Duration,
) -> Result<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!("等 PIN 超时（{timeout:?}）");
        }
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(EngineEvent::DisplayPin { pin, .. })) => return Ok(pin),
            Ok(Ok(_)) => continue,
            Ok(Err(error)) => bail!("事件通道关闭：{error}"),
            Err(_) => bail!("等 PIN 超时（{timeout:?}）"),
        }
    }
}

async fn wait_for_streaming(engine: &Engine, peer: NodeId, timeout: Duration) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if engine
            .peers()
            .iter()
            .any(|status| status.id == peer && status.state == SessionState::Streaming)
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    bail!("等会话进入 streaming 超时（{timeout:?}）")
}

fn print_progress(at_secs: u64, state: &str, stats: StreamStats, violations: usize) {
    print!(
        "\r    t={}s  {:<13} 码率 {:>7} bps  丢包 {:>3}  欠载 {:>3}  掩盖 {:>3}  迟到 {:>3}  NACK {:>3}  异常 {}   ",
        at_secs,
        state,
        stats.bitrate_bps,
        stats.loss_pct_x100,
        stats.underruns,
        stats.plc_count,
        stats.late_drops,
        stats.nack_count,
        violations,
    );
    // 长跑里进度行必须及时可见：stdout 默认行缓冲，这里显式冲刷。
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
}

fn default_report_path(started_at_unix: u64) -> PathBuf {
    PathBuf::from("target")
        .join("evidence")
        .join("soak")
        .join(format!("soak-{started_at_unix}.json"))
}

fn write_report(path: &Path, json: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建报告目录 {} 失败", parent.display()))?;
    }
    std::fs::write(path, json).with_context(|| format!("写 {} 失败", path.display()))
}

fn link_error(error: audiolink_types::AudioLinkError) -> anyhow::Error {
    anyhow!("{} {}", error.code().as_u16(), error.context())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}
