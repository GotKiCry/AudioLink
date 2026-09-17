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
//! soak-runner run [--seconds 28800] [--frame-ms 20] [--report PATH] [--expected-bps auto] [--warmup-seconds 3] [--quiet]
//! ```
//!
//! 短时自检（CI 冒烟）：`soak-runner run --seconds 60`。真正的 8 h 长跑用 `--seconds 28800`。
//!
//! # 期望码率默认不写死
//!
//! `--expected-bps` 的默认值是 `auto`：期望码率 = 本次**真实使用**的 codec 目标码率
//! （`EngineConfig::codec.bitrate_bps`）× 冗余双发份数，并且随 §8 自适应的升 / 降级跟着走。
//! 理由是实测出来的：干净回环上自适应会把编码器目标从 160 kbps 推到上限 320 kbps，
//! 接收侧于是从 320 kbps 长到 640.8 kbps —— 钉死一个数字（旧默认 320000）必然把整条干净链路
//! 判成全红（第 82 轮 900 s：878 条 `bitrate_out_of_range`，其余七类全 0）。
//! 判据本身没松：它判的是「**收到的**码率有没有跟上发送侧实际想发的量」。

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use audiolink_audio::{NullPlayout, PlayoutSink, SyntheticCapture};
use audiolink_engine::session::SessionState;
use audiolink_engine::{Engine, EngineConfig, EngineEvent};
use audiolink_tools::netem::{NetemConfig, NetemRelay};
use audiolink_tools::soak::{
    BitrateExpectationSource, REDUNDANT_COPIES, SoakMeta, SoakMonitor, SoakSample, SoakThresholds,
    expected_bitrate_bps_from_codec,
};
use audiolink_types::{ErrorCode, NodeId, StreamStats};
use tokio::sync::broadcast::error::TryRecvError;

const USAGE: &str = "\
soak-runner —— 回环长跑 + 指标采集 + 异常快照

用法：
  soak-runner run [--seconds 28800] [--frame-ms 20] [--report PATH] [--expected-bps auto] [--warmup-seconds 3] [--quiet]

参数：
  --seconds         观测时长（秒），默认 28800（8 h）；冒烟用 60
  --frame-ms        帧长（10 / 20 / 40 / 60），默认 20
  --report          报告路径，默认 target/evidence/soak/soak-<unix 秒>.json
  --expected-bps    期望码率（bps）：auto（默认）= 由本次实际 codec 配置推导，并跟随自适应升降；
                    数字 = 钉死在这个值上（判定链路自检用）；0 = 不判码率
  --warmup-seconds  预热秒数（不参与判定），默认 3
  --quiet           不打印每秒进度
  --tolerant        弱网档：只钉「会话不断 + 掩盖比例 ≤ 1%」，不判欠载/迟到/NACK/瞬时丢包/码率

弱网注入（可选，M2 验收口径见 docs/05-roadmap.md）：
  --netem-loss-pct        丢包率（百分比，可小数），默认 0
  --netem-delay-ms        单向延迟（ms），默认 0
  --netem-jitter-ms       抖动幅度（ms），默认 0
  --netem-bandwidth-kbps  带宽上限（kbps），默认 0（不限）
  --netem-seed            注入随机种子，默认 1（同种子 = 同一条注入序列）

说明：
  两个真实 Engine（node-a 发送 / node-b 接收）走真实 QUIC；
  node-b 之前不认识 node-a，因此会实跑一遍 §5 的 PIN 配对流程。
  给了任一 --netem-* 参数时，中间会插入一个弱网中继（node-a 连中继、中继转给 node-b），
  于是「弱网下的长跑」也是一条命令。
  五个参数写全即 M2 的验收口径：--netem-loss-pct 2 --netem-delay-ms 15 --netem-jitter-ms 15 --netem-bandwidth-kbps 5000
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
    let mut expected_bps = ExpectedBps::Auto;
    let mut report: Option<PathBuf> = None;
    let mut quiet = false;
    let mut tolerant = false;
    let mut netem_loss_pct_x100: u16 = 0;
    let mut netem_delay_ms: u32 = 0;
    let mut netem_jitter_ms: u32 = 0;
    let mut netem_bandwidth_kbps: u32 = 0;
    let mut netem_seed: u64 = 1;

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
                expected_bps = ExpectedBps::parse(&args, index, key)?;
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
            "--tolerant" => {
                tolerant = true;
                index += 1;
            }
            "--netem-loss-pct" => {
                let raw = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("{key} 缺少取值"))?;
                let value: f64 = raw
                    .parse()
                    .with_context(|| format!("{key} 的取值不是数字：{raw}"))?;
                if !(0.0..=100.0).contains(&value) {
                    bail!("{key} 必须在 0..=100 之间：{raw}");
                }
                netem_loss_pct_x100 = (value * 100.0).round() as u16;
                index += 2;
            }
            "--netem-delay-ms" => {
                netem_delay_ms = u32::try_from(next_u64(&args, index, key)?)
                    .map_err(|_| anyhow!("{key} 超出范围"))?;
                index += 2;
            }
            "--netem-jitter-ms" => {
                netem_jitter_ms = u32::try_from(next_u64(&args, index, key)?)
                    .map_err(|_| anyhow!("{key} 超出范围"))?;
                index += 2;
            }
            "--netem-bandwidth-kbps" => {
                netem_bandwidth_kbps = u32::try_from(next_u64(&args, index, key)?)
                    .map_err(|_| anyhow!("{key} 超出范围"))?;
                index += 2;
            }
            "--netem-seed" => {
                netem_seed = next_u64(&args, index, key)?;
                index += 2;
            }
            other => bail!("未知参数 {other}（用 --help 看用法）"),
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("创建 tokio 运行时失败")?;
    let netem = NetemOptions {
        loss_pct_x100: netem_loss_pct_x100,
        delay_ms: netem_delay_ms,
        jitter_ms: netem_jitter_ms,
        bandwidth_kbps: netem_bandwidth_kbps,
        seed: netem_seed,
    };
    runtime.block_on(run(
        seconds,
        frame_ms,
        warmup_seconds,
        expected_bps,
        report,
        quiet,
        tolerant,
        netem,
    ))
}

/// 弱网注入参数（全 0 = 不加中继）。
#[derive(Debug, Clone, Copy)]
struct NetemOptions {
    loss_pct_x100: u16,
    delay_ms: u32,
    jitter_ms: u32,
    bandwidth_kbps: u32,
    seed: u64,
}

impl NetemOptions {
    /// 是否启用注入。
    const fn enabled(&self) -> bool {
        self.loss_pct_x100 > 0 || self.delay_ms > 0 || self.jitter_ms > 0 || self.bandwidth_kbps > 0
    }

    /// 转成注入内核的参数。
    const fn config(&self) -> NetemConfig {
        NetemConfig {
            loss_pct_x100: self.loss_pct_x100,
            delay_ms: self.delay_ms,
            jitter_ms: self.jitter_ms,
            bandwidth_kbps: self.bandwidth_kbps,
        }
    }
}

/// `--expected-bps` 的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpectedBps {
    /// 默认：期望值由本次**真实使用**的 codec 配置推出，并跟随自适应的升 / 降级。
    Auto,
    /// 钉死在给定数字上（`0` = 不判码率）：
    /// 「故意把目标写错就该红」这条判定链路自检要的正是这种语义。
    Fixed(u32),
}

impl ExpectedBps {
    /// 解析取值：`auto` 或非负整数。
    fn parse(args: &[String], index: usize, key: &str) -> Result<Self> {
        let raw = args
            .get(index + 1)
            .ok_or_else(|| anyhow!("{key} 缺少取值"))?;
        if raw.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        let value = raw
            .parse::<u64>()
            .with_context(|| format!("{key} 的取值既不是 auto 也不是整数：{raw}"))?;
        u32::try_from(value)
            .map(Self::Fixed)
            .map_err(|_| anyhow!("{key} 超出范围：{raw}"))
    }
}

/// 期望码率的来路（启动横幅用），把「这次拿什么在判」说清楚。
fn describe_expected_source(source: BitrateExpectationSource, codec_bitrate_bps: i32) -> String {
    match source {
        BitrateExpectationSource::Off => "命令行钉死：不判码率".to_owned(),
        BitrateExpectationSource::FixedCli => "命令行钉死（不跟随自适应）".to_owned(),
        BitrateExpectationSource::FollowCodecTarget => format!(
            "由实际 codec 配置推导：{codec_bitrate_bps} bps × {REDUNDANT_COPIES} 份冗余，并跟随自适应升降"
        ),
    }
}

fn next_u64(args: &[String], index: usize, key: &str) -> Result<u64> {
    let raw = args
        .get(index + 1)
        .ok_or_else(|| anyhow!("{key} 缺少取值"))?;
    raw.parse::<u64>()
        .with_context(|| format!("{key} 的取值不是整数：{raw}"))
}

/// 编排入口：参数就是命令行选项本身，不再为它们造一个只有名字的包装结构。
#[allow(clippy::too_many_arguments)]
async fn run(
    seconds: u64,
    frame_ms: u32,
    warmup_seconds: u64,
    expected: ExpectedBps,
    report: Option<PathBuf>,
    quiet: bool,
    tolerant: bool,
    netem: NetemOptions,
) -> Result<ExitCode> {
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

    // 期望码率的来路：默认跟着**本次真实使用的** codec 配置走。
    // 引擎每帧主帧 + 冗余帧各发一次，接收侧遥测按收到的字节记账 ⇒ 期望值 = 目标 × 冗余份数。
    let codec_bitrate_bps = config_a.codec.bitrate_bps;
    let (expected_bps, expected_source) = match expected {
        ExpectedBps::Auto => (
            expected_bitrate_bps_from_codec(codec_bitrate_bps),
            BitrateExpectationSource::FollowCodecTarget,
        ),
        ExpectedBps::Fixed(0) => (0, BitrateExpectationSource::Off),
        ExpectedBps::Fixed(bps) => (bps, BitrateExpectationSource::FixedCli),
    };
    // 钉死的期望值**不**跟随自适应：判定链路自检要的正是「目标写错就该红」的语义。
    let follow_encoder_target = expected_source == BitrateExpectationSource::FollowCodecTarget;

    println!("=== AudioLink soak-runner（真实 QUIC 回环 · 不出声）===");
    println!(
        "计划 {seconds} s · 帧长 {frame_ms} ms · 预热 {warmup_seconds} s · 期望码率 {expected_bps} bps（{}）\n",
        describe_expected_source(expected_source, codec_bitrate_bps)
    );

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

    // ---- 可选：弱网中继（M2 验收口径）----
    let relay = if netem.enabled() {
        let relay = NetemRelay::start(engine_b.local_addr(), netem.config(), netem.seed)
            .await
            .context("起弱网中继失败")?;
        println!(
            "弱网注入：{}（种子 {}）· 中继 {} → {}\n",
            netem.config().describe(),
            netem.seed,
            relay.addr(),
            engine_b.local_addr()
        );
        Some(relay)
    } else {
        None
    };
    // 发起方连的地址：有中继就连中继，否则直连（这才是「注入」的定义）。
    let rendezvous = relay
        .as_ref()
        .map_or_else(|| engine_b.local_addr(), |relay| relay.addr());

    // ---- §5 握手 + PIN 配对 ----
    let mut events_b = engine_b.subscribe();
    let peer_on_a = match engine_a.connect(rendezvous).await {
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
    // 判据档位：默认是**回环稳态**（任何非零欠载/掩盖/丢包都是缺陷信号）；
    // 弱网档只钉三件事 —— 会话不断、修复后仍丢包 ≤ 1%、（可选）码率不跑偏。
    // 理由：弱网下欠载/迟到本来就是给定条件的一部分，把它们算成「故障」等于永远红。
    // 弱网档的判据是「**可听**」，不是「零异常」：会话必须一直在 streaming，
    // 掩盖比例 ≤ 1%（偶发掩盖可以，成片掩盖不行）—— 这是「可听」的量化版本。
    // 其余口径集中在 `SoakThresholds::weak_network`，那里同时记着为什么放宽。
    let planned_secs = if seconds == 0 { 28_800 } else { seconds };
    let thresholds = if tolerant {
        SoakThresholds::weak_network(planned_secs, frame_ms)
    } else {
        SoakThresholds::default()
    };
    println!(
        "判据档位：{}\n",
        if tolerant {
            "弱网（宽容）：只钉会话不断与掩盖比例 ≤ 1%"
        } else {
            "回环稳态（零容忍）"
        }
    );
    let mut monitor = SoakMonitor::new(thresholds, expected_bps);
    // 自适应改的是**发送侧**的目标码率：订阅 node-a 的事件，每个采样 tick 捞一次，
    // 让判据的期望值跟着走（`try_recv` 不阻塞；每秒清一次，广播通道容量 256 够用）。
    let mut events_a = engine_a.subscribe();
    let started = std::time::Instant::now();
    let mut next_tick = tokio::time::Instant::now();
    println!("开始观测（预热 {warmup_seconds} s 不参与判定）…");

    while started.elapsed() < Duration::from_secs(seconds) {
        next_tick += Duration::from_secs(1);
        tokio::time::sleep_until(next_tick).await;
        let at_secs = started.elapsed().as_secs();
        let stats = engine_b.telemetry(peer_on_b).unwrap_or_default();
        let state = peer_state(&engine_b, peer_on_b);

        // 期望值要**先**跟上这一拍的目标码率，再判这一拍的采样。
        if follow_encoder_target {
            loop {
                match events_a.try_recv() {
                    Ok(EngineEvent::CodecAdapted { to_bps, .. }) => {
                        monitor.follow_encoder_target(to_bps);
                    }
                    // 追不上（旧消息被覆盖）不是错误：继续取后面那一条 ——
                    // 期望值宁可晚一拍，也不能停在旧值上。
                    Err(TryRecvError::Lagged(_)) => continue,
                    Err(_) => break,
                    Ok(_) => {}
                }
            }
        }

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
        expected_bitrate_source: expected_source,
        started_at_unix,
    };
    let path = report.unwrap_or_else(|| default_report_path(started_at_unix));
    write_report(&path, &monitor.report_json(&meta)).context("写报告失败")?;

    println!("{}", monitor.summary_text(&meta));
    println!("报告：{}", path.display());

    // 弱网注入的自账：丢了多少、按哪个原因丢的 —— 否则「这次卡顿是不是注入造成的」说不清。
    if let Some(relay) = relay.as_ref() {
        let netem_path = path.with_extension("netem.json");
        let elapsed_secs = started.elapsed().as_secs_f64();
        write_report(&netem_path, &relay.report_json(elapsed_secs)).context("写弱网报告失败")?;
        let stats = relay.stats();
        println!(
            "弱网注入自账：观察 {} · 转发 {} · 按丢包率丢 {} · 限速丢 {} → {}\n",
            stats.observed,
            stats.forwarded,
            stats.dropped_loss,
            stats.dropped_bandwidth,
            netem_path.display()
        );
    }

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
