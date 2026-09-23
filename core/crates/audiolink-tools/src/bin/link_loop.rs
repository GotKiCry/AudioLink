//! link-loop —— PC↔PC 真 QUIC 端到端验收（M1 验收主工具）
//!
//! 规格：`docs/05-roadmap.md` M1 验收表、`docs/11-m1-contract.md` §5/§9。
//!
//! # 这个工具比 `self-loop` 多了什么
//!
//! `self-loop` 量的是**单进程内**的采集 → 编码 → 解码 → 播放，没有任何网络。
//! 本工具在同一台机器上起**两个真实的 `Engine` 实例**，让它们之间跑真实的 QUIC 会话：
//! 真实的 TLS 握手、真实的证书指纹互认、真实的数据报收发、真实的会话状态机（连接即建立，没有配对步骤）。
//! 换句话说，除了「对端是另一台设备」这一件事，M1 的整条链路都在被测。
//!
//! # 端到端延迟的口径（重要：别把模型值读成实测值）
//!
//! 探针量的是 **「帧封口 → sink 写出」**（见 [`MeasurementTap`] 的文档）。
//! 它**不含**「采集半周期」与「组帧」这两段 —— 那两段无法在同一个时间戳上量到，
//! 只能按帧长建模。报告里两者分开列，**不会**把模型值混进实测数字里。
//!
//! # 本机可测 vs 不可测
//!
//! - **可测**：PC↔PC 全链路（含真实 QUIC 网络栈、真实 Opus、真实会话）。
//! - **不可测**：PC→Android。本机 `adb devices` 为空、也没有可用的 AVD；
//!   因此 `getPerformanceMode()`、Android 出声延迟、30 min 无断流**本轮无法验证**。
//!
//! 用法：`cargo run -q -p audiolink-tools --bin link-loop -- run --seconds 20`

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use audiolink_audio::{CaptureSource, NullPlayout, PlayoutSink, Summary, SyntheticCapture};
use audiolink_engine::session::SessionState;
use audiolink_engine::{CaptureFactory, Engine, EngineConfig, MeasurementTap, PlayoutFactory};

const USAGE: &str = "\
link-loop —— PC↔PC 真 QUIC 端到端验收

用法：
  link-loop run [--seconds 20] [--frame-ms 20] [--prime-ms 20]

参数：
  --seconds   观测时长（秒），默认 20
  --frame-ms  帧长（10 / 20 / 40 / 60），默认 20
  --prime-ms  采集源的缓冲时长（ms），默认 20

说明：
  本工具在同一进程里起两个真实 Engine（node-a / node-b），走 127.0.0.1 上的真实 QUIC。
  连接即建立：一次 connect 就完成 §5 握手，没有任何配对步骤。
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        print!("{USAGE}");
        return Ok(());
    };

    if command == "-h" || command == "--help" || command == "help" {
        print!("{USAGE}");
        return Ok(());
    }
    if command != "run" {
        bail!("未知子命令 {command}（用 --help 看用法）");
    }

    let mut seconds: u64 = 20;
    let mut frame_ms: u32 = 20;
    let mut prime_ms: u32 = 20;

    let mut index = 1;
    while index < args.len() {
        let key = args[index].as_str();
        let value = || -> Result<u32> {
            let raw = args
                .get(index + 1)
                .ok_or_else(|| anyhow!("{key} 缺少取值"))?;
            raw.parse::<u32>()
                .with_context(|| format!("{key} 的取值不是整数：{raw}"))
        };
        match key {
            "--seconds" => {
                seconds = u64::from(value()?);
                index += 2;
            }
            "--frame-ms" => {
                frame_ms = value()?;
                index += 2;
            }
            "--prime-ms" => {
                prime_ms = value()?;
                index += 2;
            }
            other => bail!("未知参数 {other}（用 --help 看用法）"),
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("创建 tokio 运行时失败")?;

    runtime.block_on(run(seconds, frame_ms, prime_ms))
}

async fn run(seconds: u64, frame_ms: u32, prime_ms: u32) -> Result<()> {
    println!("=== AudioLink PC↔PC 端到端验收（真实 QUIC · 127.0.0.1）===");
    println!("参数：观测 {seconds} s · 帧长 {frame_ms} ms · 采集缓冲 {prime_ms} ms\n");

    // ------------------------------------------------------------------
    // [1] 零重采样断言：先证明两端设备格式就是内核统一格式
    // ------------------------------------------------------------------
    println!("[1] 零重采样断言（NFR-13）");
    report_device_formats(frame_ms, prime_ms)?;

    // ------------------------------------------------------------------
    // [2] 起两台真实 Engine
    // ------------------------------------------------------------------
    let dir = tempfile::TempDir::new().context("创建临时目录失败")?;
    let dir_a = dir.path().join("node-a");
    let dir_b = dir.path().join("node-b");
    std::fs::create_dir_all(&dir_a).context("创建 node-a 目录失败")?;
    std::fs::create_dir_all(&dir_b).context("创建 node-b 目录失败")?;

    // 同一个探针实例同时给两端：这样「发送侧封口」与「接收侧播出」用的是同一个时钟基准。
    let tap = Arc::new(MeasurementTap::new(16_384));

    let mut config_a = EngineConfig::new("node-a", &dir_a);
    config_a.listen = "127.0.0.1:0".parse().context("解析监听地址失败")?;
    config_a.codec.frame_ms = frame_ms;
    config_a.capture = Some(synthetic_capture(prime_ms));
    config_a.measurement = Some(Arc::clone(&tap));

    let mut config_b = EngineConfig::new("node-b", &dir_b);
    config_b.listen = "127.0.0.1:0".parse().context("解析监听地址失败")?;
    config_b.codec.frame_ms = frame_ms;
    config_b.playout = Some(null_playout(prime_ms * 3));
    config_b.measurement = Some(Arc::clone(&tap));

    let engine_a = Engine::start(config_a)
        .await
        .map_err(link_error)
        .context("启动 node-a 失败")?;
    let engine_b = Engine::start(config_b)
        .await
        .map_err(link_error)
        .context("启动 node-b 失败")?;

    let addr_a = engine_a.local_addr();
    let addr_b = engine_b.local_addr();
    println!("[2] 引擎就绪");
    println!("    node-a  {addr_a}  指纹 {}", engine_a.info().id.short());
    println!("    node-b  {addr_b}  指纹 {}", engine_b.info().id.short());

    let _accept_a = engine_a.spawn_accept_loop();
    let _accept_b = engine_b.spawn_accept_loop();

    // ------------------------------------------------------------------
    // [3] §5 握手（连接即建立，没有配对步骤）
    // ------------------------------------------------------------------
    println!("\n[3] 握手与连接建立（§5）");

    engine_a
        .connect(addr_b)
        .await
        .map_err(link_error)
        .context("连接 node-b 失败")?;
    let peer_on_a = first_peer(&engine_a).ok_or_else(|| anyhow!("node-a 侧没有建立会话"))?;
    wait_for_streaming(&engine_a, peer_on_a, Duration::from_secs(5)).await?;
    println!(
        "    已连接：node-a {} ↔ node-b {}",
        peer_on_a.short(),
        addr_b
    );

    // ------------------------------------------------------------------
    // [4] 推流
    // ------------------------------------------------------------------
    println!("\n[4] 开流并观测 {seconds} s");
    engine_a
        .start_send(peer_on_a)
        .await
        .map_err(link_error)
        .context("开流失败")?;

    // 等接收端把播放线程拉起来
    tokio::time::sleep(Duration::from_millis(600)).await;

    let started = std::time::Instant::now();
    let mut last_report = 0u64;
    while started.elapsed() < Duration::from_secs(seconds) {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let elapsed = started.elapsed().as_secs();
        if elapsed > last_report {
            last_report = elapsed;
            if let (Some(sender), Some(receiver)) =
                (engine_a.telemetry(peer_on_a), peer_stats(&engine_b))
            {
                print!(
                    "\r    t={elapsed:>3}s  已配对 {:>2} 个样本  发送 {:.1} kbps  接收欠载 {}  迟到丢弃 {}   ",
                    tap.matched(),
                    receiver.bitrate_bps as f64 / 1000.0,
                    receiver.underruns,
                    receiver.late_drops
                );
                let _ = sender;
            }
        }
    }
    println!();

    // ------------------------------------------------------------------
    // [5] 报告
    // ------------------------------------------------------------------
    report(
        seconds, frame_ms, prime_ms, &tap, &engine_a, &engine_b, peer_on_a,
    )
}

// ---------------------------------------------------------------------------
// 报告
// ---------------------------------------------------------------------------

fn report_device_formats(frame_ms: u32, prime_ms: u32) -> Result<()> {
    let capture = SyntheticCapture::new(prime_ms, 440.0).context("构造合成采集源失败")?;
    let playout = NullPlayout::new(prime_ms * 3);

    let capture_format = capture.device_format();
    let playout_format = playout.device_format();

    println!(
        "    采集端  {:<18} {} Hz / {} ch / {:?}   → {}",
        capture.backend_name(),
        capture_format.sample_rate,
        capture_format.channels,
        capture_format.sample_format,
        verdict(capture_format.is_unified())
    );
    println!(
        "    播放端  {:<18} {} Hz / {} ch / {:?}   → {}",
        playout.backend_name(),
        playout_format.sample_rate,
        playout_format.channels,
        playout_format.sample_format,
        verdict(playout_format.is_unified())
    );
    println!("    内核    48 kHz / f32 / 2ch（统一格式）· 帧长 {frame_ms} ms · 全链路无重采样环节");
    println!("    判据    audiolink-audio 的格式守卫在**线程启动时**断言，不达标即拒绝推流");

    if !capture_format.is_unified() || !playout_format.is_unified() {
        bail!("合成设备格式不是统一格式，验收前提不成立");
    }
    Ok(())
}

fn verdict(ok: bool) -> &'static str {
    if ok { "断言通过" } else { "断言失败" }
}

fn synthetic_capture(prime_ms: u32) -> CaptureFactory {
    Arc::new(move || {
        SyntheticCapture::new(prime_ms, 440.0)
            .map(|capture| Box::new(capture) as Box<dyn CaptureSource>)
    })
}

fn null_playout(buffer_ms: u32) -> PlayoutFactory {
    Arc::new(move || Ok(Box::new(NullPlayout::new(buffer_ms)) as Box<dyn PlayoutSink>))
}

fn first_peer(engine: &Arc<Engine>) -> Option<audiolink_types::NodeId> {
    engine.peers().into_iter().next().map(|peer| peer.id)
}

fn peer_stats(engine: &Arc<Engine>) -> Option<audiolink_types::StreamStats> {
    first_peer(engine).and_then(|id| engine.telemetry(id))
}

async fn wait_for_streaming(
    engine: &Arc<Engine>,
    peer: audiolink_types::NodeId,
    timeout: Duration,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if engine
            .peers()
            .into_iter()
            .any(|status| status.id == peer && status.state == SessionState::Streaming)
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            bail!("等待会话进入 Streaming 超时");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[allow(clippy::too_many_arguments)]
fn report(
    seconds: u64,
    frame_ms: u32,
    prime_ms: u32,
    tap: &Arc<MeasurementTap>,
    engine_a: &Arc<Engine>,
    engine_b: &Arc<Engine>,
    peer_on_a: audiolink_types::NodeId,
) -> Result<()> {
    println!("\n=== 验收报告 ===");

    println!("\n[5] 端到端延迟（探针实测：帧封口 → sink 写出）");
    match tap.summary() {
        Some(summary) => {
            print_summary(&summary);
            println!("    注意：这一段**不含**采集半周期与组帧，两者只能建模，见下一节");
        }
        None => {
            println!("    未配对上任何样本 —— 链路可能没真正跑起来");
        }
    }

    let (orphan_sealed, orphan_played) = tap.orphan_counts();
    println!("    孤儿记录：发送侧 {orphan_sealed} / 接收侧 {orphan_played}（正常应接近 0）");

    println!("\n[6] 分段账本（模型段 + 实测段）");
    let half_buffer_us = prime_ms as f64 * 1000.0 / 2.0;
    let assemble_us = f64::from(frame_ms) * 1000.0;
    let measured_p50 = tap.summary().map_or(0.0, |s| f64::from(s.p50));
    println!(
        "    采集半周期（模型，缓冲 {prime_ms} ms / 2）   {:>8.1} ms",
        half_buffer_us / 1000.0
    );
    println!(
        "    组帧（模型，{frame_ms} ms 帧）                {:>8.1} ms",
        assemble_us / 1000.0
    );
    println!(
        "    帧封口 → 出声（实测 P50）                {:>8.1} ms",
        measured_p50 / 1000.0
    );
    println!("    ─────────────────────────────────────────────────────");
    println!(
        "    合计估计（2 段模型 + 1 段实测）            {:>8.1} ms",
        (half_buffer_us + assemble_us + measured_p50) / 1000.0
    );

    println!("\n[7] 接收侧遥测（node-b 视角）");
    if let Some(stats) = peer_stats(engine_b) {
        println!(
            "    实际码率        {:>8.1} kbps",
            f64::from(stats.bitrate_bps) / 1000.0
        );
        println!(
            "    RTT（QUIC 平滑）{:>8.1} ms",
            f64::from(stats.rtt_us) / 1000.0
        );
        println!(
            "    丢包率          {:>8.2} %",
            f64::from(stats.loss_pct_x100) / 100.0
        );
        println!(
            "    播放环水位      {:>8.1} ms",
            f64::from(stats.buffer_level_us) / 1000.0
        );
        println!("    欠载次数        {:>8}", stats.underruns);
        println!("    迟到丢弃        {:>8}", stats.late_drops);
        println!("    PLC 次数        {:>8}", stats.plc_count);
        println!(
            "    抖动 P50 / P95  {:>6.2} / {:.2} ms",
            f64::from(stats.jitter_us) / 1000.0,
            f64::from(stats.jitter_p95_us) / 1000.0
        );
        println!(
            "    对端遥测 e2e    {:>8.1} ms（接收侧聚合口径）",
            f64::from(stats.e2e_latency_us) / 1000.0
        );
    } else {
        println!("    无接收侧遥测（会话没起来？）");
    }

    println!("\n[8] 发送侧遥测（node-a 视角）");
    if let Some(stats) = engine_a.telemetry(peer_on_a) {
        println!(
            "    实际码率        {:>8.1} kbps",
            f64::from(stats.bitrate_bps) / 1000.0
        );
        println!(
            "    RTT（QUIC 平滑）{:>8.1} ms",
            f64::from(stats.rtt_us) / 1000.0
        );
        println!("    采样率断言      48000 Hz（设备格式守卫）");
    }

    println!("\n[9] 与 M1 验收线的对照（docs/05-roadmap.md）");
    if let Some(summary) = tap.summary() {
        let p50_ms = f64::from(summary.p50) / 1000.0;
        let p95_ms = f64::from(summary.p95) / 1000.0;
        println!(
            "    P50 {p50_ms:.1} ms ≤ 110 ms  {}",
            if p50_ms <= 110.0 { "✅" } else { "❌" }
        );
        println!(
            "    P95 {p95_ms:.1} ms ≤ 150 ms  {}",
            if p95_ms <= 150.0 { "✅" } else { "❌" }
        );
        println!("    口径提醒：以上是**探针实测段**（帧封口 → 出声），不含采集半周期与组帧；");
        println!(
            "              按上一节的合计估计口径，端到端应为 {:.1} ms。",
            (prime_ms as f64 / 2.0) + f64::from(frame_ms) + p50_ms
        );
    }

    println!("\n[10] 本轮**未验证**的部分（不许推断）");
    println!("    · PC → Android 真机链路：本机 adb 无设备、无可用 AVD，**未验证**");
    println!("    · Android getPerformanceMode() 是否真的返回 LOW_LATENCY：**未验证**");
    println!("    · 30 min 连续无断流（本轮只跑了 {seconds} s）：**未验证**");
    println!("    · 真实 Wi-Fi 下的丢包/抖动表现（本轮走 127.0.0.1）：**未验证**");

    Ok(())
}

fn print_summary(summary: &Summary) {
    println!("    样本数          {:>8}", summary.count);
    println!(
        "    min             {:>8.2} ms",
        f64::from(summary.min) / 1000.0
    );
    println!(
        "    P50             {:>8.2} ms",
        f64::from(summary.p50) / 1000.0
    );
    println!(
        "    P95             {:>8.2} ms",
        f64::from(summary.p95) / 1000.0
    );
    println!(
        "    P99             {:>8.2} ms",
        f64::from(summary.p99) / 1000.0
    );
    println!(
        "    max             {:>8.2} ms",
        f64::from(summary.max) / 1000.0
    );
    println!("    mean            {:>8.2} ms", summary.mean / 1000.0);
}

/// `AudioLinkError` → `anyhow::Error`（保留错误码，方便报告里定位）。
fn link_error(error: audiolink_types::AudioLinkError) -> anyhow::Error {
    anyhow!(
        "{} {}（{}）",
        error.code().as_u16(),
        error.code().name(),
        error.context()
    )
}
