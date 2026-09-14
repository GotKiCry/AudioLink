//! self-loop —— 桌面自环链路 + 四段延迟分解（`docs/05-roadmap.md` M1 P0 第三项）
//!
//! # 它做什么
//!
//! 把真实链路按 M1 的样子接起来，**不碰网络、不碰 Android**，只测量两个关键未知量：
//!
//! ```text
//! WASAPI loopback 采集 ──► 无锁环 ──► 组帧(20ms) ──► Opus 编码 ──► Opus 解码 ──► WASAPI render
//!        ▲                                                                             │
//!        └────────────────────── 标记脉冲往返（设备真实往返） ◄───────────────────────┘
//! ```
//!
//! # 四段（五段）延迟分解的口径
//!
//! | 段 | 口径 | 来源 |
//! |---|---|---|
//! | 采集 | 设备周期 ÷ 2（事件驱动下样本在设备缓冲里平均停留半个周期） | **模型值**（周期为实测） |
//! | 组帧 | 帧长本身（20 ms） | 结构性常量 |
//! | 编码 | `encode()` 实测耗时 + 编码器固有延迟标称值 | 实测 + 标称 |
//! | 解码 | `decode()` 实测耗时 | 实测 |
//! | 播放 | 写入前水位（帧数 ÷ 48 kHz）+ 半个帧长 | 水位实测 + 模型 |
//!
//! 另外用**标记脉冲**测一次真实往返：每 `--marker-ms` 在提交给 render 的帧上叠加一个短脉冲，
//! 再从 loopback 采集里检出它。往返 = 检出时刻 − 提交时刻，包含 render 缓冲 + 引擎周期 + 采集缓冲，
//! 不依赖任何模型，是本工具最有说服力的一列（也是校准「采集段半周期模型」的基准）。
//!
//! # 用法
//!
//! ```text
//! self-loop list                                   # 列出渲染端点（默认标记 / 混音格式 / 周期）
//! self-loop run [选项]
//!     --device <sel>        渲染端点：default | id:<子串> | name:<友好名>（默认 default）
//!     --sink-device <sel>   播放端点；默认与采集同一端点（不同端点测不到标记往返）
//!     --seconds <n>         运行时长（默认 20）
//!     --frame-ms <n>        帧长 10|20（默认 20）
//!     --buffer-ms <n>       请求的采集/播放缓冲（默认 20；实测下限 22 ms）
//!     --capture <wasapi|synth>   采集后端（默认 wasapi；synth 无需声卡，用于 CI/无声卡环境）
//!     --sink <wasapi|null>       播放后端（默认 wasapi）
//!     --gain <f>            自环回放增益（默认 0.02，避免啸叫）
//!     --marker-ms <n>       标记脉冲间隔（默认 500；0 = 关闭）
//!     --no-render           不写播放（= --sink null --marker-ms 0）
//!     --json <path>         导出 JSON 报告
//!     --quiet               只打印结论
//! ```
//!
//! # 已知事实（2026-09-14 本机实测，详见 `docs/06-dev-environment.md`）
//!
//! 1. 共享模式下**设备缓冲下限是 1056 帧（22 ms）**：请求 1–20 ms 都会被顶到 1056，请求 ≥ 30 ms 才精确；
//!    能压到 10 ms 的是**引擎周期**（事件驱动每 10 ms 交付 480 帧），不是缓冲大小。
//! 2. 空闲端点**零数据零事件**：没有程序在播放时 loopback 不交付任何包 —— 所以超时不是错误，本工具有计数器。
//! 3. 采集与播放对象都是 `!Send`：本工具在**各自线程内**初始化 COM 并构造对象。

use std::path::PathBuf;

fn main() {
    #[cfg(windows)]
    {
        if let Err(error) = imp::run() {
            eprintln!("错误：{error:#}");
            std::process::exit(1);
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("self-loop 需要 Windows（WASAPI）；当前平台不可用。");
        std::process::exit(2);
    }
}

/// 运行参数（两个平台都编译，便于参数解析的单测）。
#[derive(Debug, Clone, PartialEq)]
struct RunOptions {
    device: String,
    sink_device: Option<String>,
    seconds: f64,
    frame_ms: u32,
    buffer_ms: u32,
    capture_backend: CaptureBackend,
    sink_backend: SinkBackend,
    gain: f32,
    marker_ms: u64,
    json: Option<PathBuf>,
    quiet: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            device: "default".to_string(),
            sink_device: None,
            seconds: 20.0,
            frame_ms: 20,
            buffer_ms: 20,
            capture_backend: CaptureBackend::Wasapi,
            sink_backend: SinkBackend::Wasapi,
            gain: 0.02,
            marker_ms: 500,
            json: None,
            quiet: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureBackend {
    Wasapi,
    Synth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SinkBackend {
    Wasapi,
    Null,
}

const HELP: &str = "\
self-loop —— 桌面自环链路 + 四段延迟分解（M1 P0）

用法：
  self-loop list                                   # 列出渲染端点
  self-loop run [选项]                             # 跑自环并出报告

run 选项：
  --device <sel>        渲染端点：default | id:<子串> | name:<友好名>（默认 default）
  --sink-device <sel>   播放端点；默认与采集同一端点（不同端点测不到标记往返）
  --seconds <n>         运行时长（默认 20）
  --frame-ms <n>        帧长 10|20（默认 20）
  --buffer-ms <n>       请求的采集/播放缓冲（默认 20；本机实测下限 22 ms）
  --capture <backend>   采集后端：wasapi|synth（默认 wasapi）
  --sink <backend>      播放后端：wasapi|null（默认 wasapi）
  --gain <f>            自环回放增益（默认 0.02）
  --marker-ms <n>       标记脉冲间隔（默认 500；0 = 关闭）
  --no-render           不写播放（等于 --sink null --marker-ms 0）
  --json <path>         导出 JSON 报告
  --quiet               只打印结论
  -h, --help            本帮助

注意：自环会把采集到的声音再放回去（增益 --gain），请把系统音量调小或戴耳机。
";

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use anyhow::{Context, Result, anyhow};
    use audiolink_audio::codec::proto_budget;
    use audiolink_audio::format::{CHANNELS, SAMPLE_RATE_HZ};
    use audiolink_audio::latency::{LatencyLedger, SegmentSample, frame_assembly_us};
    use audiolink_audio::ring::{SampleRingReader, SampleRingWriter, sample_ring};
    use audiolink_audio::sink::{PlayoutSink, PlayoutStats};
    use audiolink_audio::source::{CaptureSource, CaptureStats};
    use audiolink_audio::wasapi::{
        DeviceSelector, LoopbackCapture, RenderDeviceInfo, RenderSink, list_render_devices,
    };
    use audiolink_audio::{CodecConfig, FrameChunker, NullPlayout, OpusDecoder, OpusEncoder};
    use audiolink_audio::{Summary, SyntheticCapture};

    use super::{CaptureBackend, RunOptions, SinkBackend};

    /// 环容量：8 帧（160 ms @20 ms）—— 足够吸收调度抖动，又不至于掩盖问题。
    const RING_FRAMES: usize = 8;
    /// 采集超时：超过这个时间没有数据就继续等（端点安静不是错误）。
    const CAPTURE_TIMEOUT: Duration = Duration::from_millis(100);
    /// 标记检出阈值（Goertzel 幅度，单位 ≈ 正弦幅度；见 [`MARKER_AMPLITUDE`]）。
    ///
    /// 取 0.05：即使端点音量只有 10%，0.45 的标记也还剩 0.045 —— 刚好在边缘，所以报告里会打印
    /// 实测到的最大标记幅度，便于判断这次测量是否可信。
    pub(super) const MARKER_MAGNITUDE_THRESHOLD: f32 = 0.05;
    /// 标记频率（Hz）：放在 19 kHz —— 音乐内容在这个频段的能量通常极低，
    /// 因此**在一屋子音乐里也能干净检出**（脉冲标记在同音量环境里会被淹没，实测过）。
    const MARKER_FREQ_HZ: f32 = 19_000.0;
    /// 标记脉冲长度（帧）：10 ms。
    const MARKER_FRAMES: usize = 480;
    /// 标记幅度（线性，满量程 1.0）。
    const MARKER_AMPLITUDE: f32 = 0.45;
    /// 标记淡入/淡出长度（帧）：1 ms，避免频谱泄漏与"咔哒"声。
    const MARKER_FADE_FRAMES: usize = 48;
    /// 检出窗口长度（帧）：5 ms。
    const MARKER_WINDOW_FRAMES: usize = 240;
    /// 检出窗口跳步（帧）：2.5 ms。
    const MARKER_HOP_FRAMES: usize = 120;
    /// 命中后的不应期（忽略回声与抖动）。
    const MARKER_REFRACTORY: Duration = Duration::from_millis(120);
    /// 默认设备周期（μs）—— 采集线程会用它覆盖成实测值。
    const DEFAULT_PERIOD_US: u32 = 10_000;

    pub fn run() -> Result<()> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.first().map(String::as_str) {
            Some("list") => run_list(),
            Some("run") => run_loop(&args[1..]),
            Some("-h" | "--help") | None => {
                print!("{}", super::HELP);
                Ok(())
            }
            Some(other) => Err(anyhow!("未知子命令 {other}（--help 看用法）")),
        }
    }

    fn run_list() -> Result<()> {
        let devices = list_render_devices().map_err(|error| anyhow!("{error}"))?;
        if devices.is_empty() {
            println!("没有活动的渲染端点（检查声音设置，或设备是否被禁用）");
            return Ok(());
        }
        println!("活动渲染端点 {} 个：", devices.len());
        for info in &devices {
            println!("  - {}", info.summary());
            if info.looks_virtual() {
                println!("      ⚠ 疑似虚拟声卡：选它做 loopback 可能采到静音（ADR-005 本机警示）");
            }
        }
        if let Some(info) = devices.iter().find(|info| info.is_default) {
            println!();
            println!("默认端点：{}", info.name);
            if info.looks_virtual() {
                println!("⚠ 默认输出是虚拟声卡：建议 --device id:{}", info.short_id());
            }
            if info.format.sample_rate != SAMPLE_RATE_HZ {
                println!(
                    "⚠ 默认端点混音格式不是 48 kHz（{} Hz）：请到「声音设置 → 设备属性 → 高级」改成 48000 Hz（本项目不做静默重采样）",
                    info.format.sample_rate
                );
            }
        }
        Ok(())
    }

    /// 标记命中记录。
    #[derive(Debug, Clone, Copy)]
    struct MarkerHit {
        at: Instant,
        amplitude: f32,
    }

    /// 采集线程产出。
    struct CaptureOutcome {
        stats: CaptureStats,
        hits: Vec<MarkerHit>,
        device: Option<RenderDeviceInfo>,
        requested_buffer_ms: u32,
        effective_buffer_ms: u32,
        period_ms: (f64, f64),
        backend: String,
        ring_dropped_samples: u64,
        /// 采集信号峰值（×1000，整数避免浮点破坏 `PartialEq`）。
        peak_milli: u32,
        /// 采集信号整体 RMS（×1000）。
        rms_milli: u32,
        /// 环境电平 RMS 的 EMA（×1000）——标记检出阈值的审计基准。
        ambient_rms_milli: u32,
        /// 采集侧实测到的标记音最大幅度（×1000）：判断这次标记测量是否可信。
        marker_magnitude_milli: u32,
    }

    /// 工作线程产出。
    struct WorkerOutcome {
        ledger: LatencyLedger,
        frames: u64,
        encoded_bytes: u64,
        plc_count: u64,
        chunker_dropped_frames: u64,
        sink_stats: PlayoutStats,
        sink_device: Option<RenderDeviceInfo>,
        sink_requested_buffer_ms: u32,
        sink_effective_buffer_ms: u32,
        backend: String,
        markers_sent: u64,
        marker_send_times: Vec<Instant>,
        /// 每次发出标记时「同口径模型预期」的往返（μs）：写后排队 + 标记在帧内的位置。
        marker_expected_us: Vec<u32>,
    }

    fn run_loop(args: &[String]) -> Result<()> {
        let options = parse_run_options(args)?;
        if !options.quiet {
            println!("AudioLink self-loop（M1 P0：自环链路 + 四段延迟分解）");
            println!(
                "  采集={} 播放={} 帧长={} ms 缓冲请求={} ms 时长={:.0} s 回放增益={}",
                backend_name(options.capture_backend),
                backend_name_sink(options.sink_backend),
                options.frame_ms,
                options.buffer_ms,
                options.seconds,
                options.gain
            );
            println!(
                "  注意：自环会把采集到的声音再放回去，请调小音量（增益 {:.2}）",
                options.gain
            );
            println!();
        }

        let codec_config = CodecConfig {
            frame_ms: options.frame_ms,
            ..CodecConfig::m1_default()
        };
        let frame_interleaved = codec_config.interleaved_frame();
        let ring_capacity = frame_interleaved * RING_FRAMES;
        let (writer, reader) = sample_ring(ring_capacity);
        let stop = Arc::new(AtomicBool::new(false));
        let capture_period_us = Arc::new(AtomicU32::new(DEFAULT_PERIOD_US));

        let same_endpoint = match &options.sink_device {
            None => true,
            Some(sink) => *sink == options.device,
        };
        let markers_enabled = options.marker_ms > 0
            && options.sink_backend == SinkBackend::Wasapi
            && options.capture_backend == CaptureBackend::Wasapi
            && same_endpoint;
        if options.marker_ms > 0 && !markers_enabled && !options.quiet {
            println!("⚠ 标记往返测量已关闭（需要 wasapi 双端 + 同一端点 + --marker-ms > 0）");
        }

        let capture_stop = Arc::clone(&stop);
        let capture_options = options.clone();
        let capture_period = Arc::clone(&capture_period_us);
        // 标记只在「wasapi 采集 + wasapi 播放 + 同一端点」时才有意义；两个线程共用同一个口径
        let marker_interval = if markers_enabled {
            Some(Duration::from_millis(options.marker_ms))
        } else {
            None
        };
        let capture_thread = std::thread::Builder::new()
            .name("self-loop-capture".to_string())
            .spawn(move || {
                capture_thread_main(
                    capture_stop,
                    capture_options,
                    writer,
                    capture_period,
                    marker_interval,
                )
            })
            .context("启动采集线程失败")?;

        let worker_stop = Arc::clone(&stop);
        let worker_options = options.clone();
        let worker_period = Arc::clone(&capture_period_us);
        let worker_thread = std::thread::Builder::new()
            .name("self-loop-worker".to_string())
            .spawn(move || {
                worker_thread_main(
                    worker_stop,
                    worker_options,
                    codec_config,
                    reader,
                    worker_period,
                    marker_interval,
                )
            })
            .context("启动工作线程失败")?;

        std::thread::sleep(Duration::from_secs_f64(options.seconds.max(0.5)));
        stop.store(true, Ordering::Relaxed);
        println!("…… 采集停止，等待线程收尾（约 1–2 个设备周期）");

        let capture_outcome = capture_thread
            .join()
            .map_err(|_| anyhow!("采集线程异常结束"))?
            .map_err(|error| anyhow!("采集线程失败：{error}"))?;
        let worker_outcome = worker_thread
            .join()
            .map_err(|_| anyhow!("工作线程异常结束"))?
            .map_err(|error| anyhow!("工作线程失败：{error}"))?;

        let report = Report::new(&options, &capture_outcome, &worker_outcome, markers_enabled);
        report.print(options.quiet);
        if let Some(path) = &options.json {
            let json = report.to_json()?;
            std::fs::write(path, json).with_context(|| format!("写 JSON 报告到 {path:?} 失败"))?;
            println!("JSON 报告：{}", path.display());
        }
        Ok(())
    }

    fn backend_name(backend: CaptureBackend) -> &'static str {
        match backend {
            CaptureBackend::Wasapi => "wasapi-loopback",
            CaptureBackend::Synth => "synth",
        }
    }

    fn backend_name_sink(backend: SinkBackend) -> &'static str {
        match backend {
            SinkBackend::Wasapi => "wasapi-render",
            SinkBackend::Null => "null",
        }
    }

    // ---------------------------------------------------------------------
    // 采集线程：COM 在本线程内初始化，对象在本线程内构造（wasapi 对象是 !Send）
    // ---------------------------------------------------------------------
    fn capture_thread_main(
        stop: Arc<AtomicBool>,
        options: RunOptions,
        mut writer: SampleRingWriter,
        period_slot: Arc<AtomicU32>,
        marker_interval: Option<Duration>,
    ) -> Result<CaptureOutcome, String> {
        let selector = DeviceSelector::parse(&options.device);
        let (mut source, device, period_ms): (
            Box<dyn CaptureSource>,
            Option<RenderDeviceInfo>,
            (f64, f64),
        ) = match options.capture_backend {
            CaptureBackend::Wasapi => {
                let capture = LoopbackCapture::open(&selector, options.buffer_ms)
                    .map_err(|error| format!("打开 loopback 采集失败：{error}"))?;
                let info = capture.device_info().clone();
                let period = capture.device_period_ms();
                (Box::new(capture), Some(info), period)
            }
            CaptureBackend::Synth => {
                let synth = SyntheticCapture::new(options.buffer_ms.clamp(1, 50), 440.0)
                    .map_err(|error| format!("构造合成采集源失败：{error}"))?;
                let buffer = f64::from(synth.effective_buffer_ms());
                (Box::new(synth), None, (buffer, buffer))
            }
        };
        if options.capture_backend == CaptureBackend::Wasapi {
            period_slot.store((period_ms.0 * 1_000.0).round() as u32, Ordering::Relaxed);
        }
        let requested_buffer_ms = source.requested_buffer_ms();
        let effective_buffer_ms = source.effective_buffer_ms();

        let mut buffer: Vec<f32> = Vec::with_capacity(48 * options.buffer_ms as usize * 4);
        let mut hits: Vec<MarkerHit> = Vec::new();
        let mut last_hit: Option<Instant> = None;
        let mut dropped_samples = 0u64;
        // 不应期：忽略标记自身的回声；同时不能大过发送间隔的一半（否则会漏掉每次标记）
        let refractory = match marker_interval {
            Some(interval) => MARKER_REFRACTORY
                .min(interval / 2)
                .max(Duration::from_millis(20)),
            None => MARKER_REFRACTORY,
        };
        // 采集电平（报告里要能审计检出阈值是否合理）+ 环境电平 EMA（自适应门限）
        let mut level = LevelProbe::default();

        while !stop.load(Ordering::Relaxed) {
            let packet = match source.read(&mut buffer, CAPTURE_TIMEOUT) {
                Ok(Some(packet)) => packet,
                Ok(None) => continue,
                Err(error) => {
                    if error.is_statistical() {
                        continue;
                    }
                    return Err(format!("采集失败：{error}"));
                }
            };
            let scan = scan_packet(
                &buffer,
                packet.frames,
                packet.read_at,
                last_hit,
                refractory,
                marker_interval.is_some(),
            );
            level.observe(scan.mean_square, scan.peak, scan.marker_magnitude);
            if let Some(hit) = scan.hit {
                last_hit = Some(hit.at);
                hits.push(hit);
            }
            dropped_samples += writer.push(&buffer) as u64;
        }
        source.stop();
        let stats = source.stats();
        Ok(CaptureOutcome {
            stats,
            hits,
            device,
            requested_buffer_ms,
            effective_buffer_ms,
            period_ms,
            backend: source.backend_name().to_string(),
            ring_dropped_samples: dropped_samples,
            peak_milli: level.peak_milli(),
            rms_milli: level.rms_milli(),
            ambient_rms_milli: level.ambient_rms_milli(),
            marker_magnitude_milli: level.marker_magnitude_milli(),
        })
    }

    /// 采集电平探针（峰值 / 整体 RMS / 环境 RMS 的指数滑动平均 / 标记幅度上限）。
    #[derive(Default)]
    struct LevelProbe {
        peak: f32,
        square_sum: f64,
        samples: u64,
        ambient: f32,
        marker_max: f32,
    }

    impl LevelProbe {
        fn observe(&mut self, mean_square: f32, peak: f32, marker_magnitude: f32) {
            self.peak = self.peak.max(peak);
            self.square_sum += f64::from(mean_square);
            self.samples += 1;
            self.marker_max = self.marker_max.max(marker_magnitude);
            let packet_rms = mean_square.max(0.0).sqrt();
            self.ambient = if self.samples <= 1 {
                packet_rms
            } else {
                self.ambient * 0.95 + packet_rms * 0.05
            };
        }

        fn peak_milli(&self) -> u32 {
            (self.peak * 1_000.0).round().clamp(0.0, 65_535.0) as u32
        }

        fn rms_milli(&self) -> u32 {
            if self.samples == 0 {
                return 0;
            }
            let mean = self.square_sum / self.samples as f64;
            (mean.sqrt() * 1_000.0).round().clamp(0.0, 65_535.0) as u32
        }

        fn ambient_rms_milli(&self) -> u32 {
            (self.ambient * 1_000.0).round().clamp(0.0, 65_535.0) as u32
        }

        fn marker_magnitude_milli(&self) -> u32 {
            (self.marker_max * 1_000.0).round().clamp(0.0, 65_535.0) as u32
        }
    }

    /// 一个采集包的扫描结果。
    struct PacketScan {
        peak: f32,
        mean_square: f32,
        /// 本次包里检出的标记幅度（Goertzel，>= 阈值才算命中；未命中为 0）。
        marker_magnitude: f32,
        hit: Option<MarkerHit>,
    }

    /// 扫描一个采集包：算电平，并按需检出**标记音**（19 kHz + Goertzel）。
    ///
    /// 为什么不用「脉冲 + 幅度阈值」：实测在默认端点（有音乐在响，RMS −15 dBFS、峰值超过 0 dBFS）
    /// 上，脉冲会被内容淹没，产出了 1.1 ms / 402 ms 这种不可能的往返值。换成窄带标记音后，
    /// 检出只依赖「19 kHz 这个频带里有没有能量」，与音乐内容基本解耦。
    ///
    /// 判据：
    /// 1. 5 ms 窗 Goertzel 幅度 > [`MARKER_MAGNITUDE_THRESHOLD`]；
    /// 2. 距上次命中超过不应期（挡掉回声与重复检出）。
    fn scan_packet(
        samples: &[f32],
        frames: usize,
        read_at: Instant,
        last_hit: Option<Instant>,
        refractory: Duration,
        detect: bool,
    ) -> PacketScan {
        let mut peak = 0.0f32;
        let mut square_sum = 0.0f64;
        for value in samples.iter() {
            let magnitude = value.abs();
            if magnitude > peak {
                peak = magnitude;
            }
            square_sum += f64::from(*value) * f64::from(*value);
        }
        let mean_square = if samples.is_empty() {
            0.0
        } else {
            (square_sum / samples.len() as f64) as f32
        };
        if !detect {
            return PacketScan {
                peak,
                mean_square,
                marker_magnitude: 0.0,
                hit: None,
            };
        }

        let window = MARKER_WINDOW_FRAMES * CHANNELS as usize;
        let hop = MARKER_HOP_FRAMES * CHANNELS as usize;
        let mut best = 0.0f32;
        let mut best_start = 0usize;
        let mut start = 0usize;
        while start + window <= samples.len() {
            if let Some(chunk) = samples.get(start..start + window) {
                let magnitude = goertzel_magnitude(
                    chunk,
                    MARKER_FREQ_HZ,
                    SAMPLE_RATE_HZ as f32,
                    CHANNELS as usize,
                );
                if magnitude > best {
                    best = magnitude;
                    best_start = start;
                }
            }
            start += hop;
        }
        if best <= MARKER_MAGNITUDE_THRESHOLD {
            return PacketScan {
                peak,
                mean_square,
                marker_magnitude: best,
                hit: None,
            };
        }
        // 命中时刻：窗口起点（再往前推窗口长度的一半，近似能量重心）
        let frame_index = (best_start / CHANNELS as usize).saturating_add(MARKER_WINDOW_FRAMES / 2);
        let behind_frames = frames.saturating_sub(frame_index);
        let at = read_at
            .checked_sub(Duration::from_secs_f64(
                behind_frames as f64 / f64::from(SAMPLE_RATE_HZ),
            ))
            .unwrap_or(read_at);
        if let Some(previous) = last_hit
            && at.duration_since(previous) < refractory
        {
            return PacketScan {
                peak,
                mean_square,
                marker_magnitude: best,
                hit: None,
            };
        }
        PacketScan {
            peak,
            mean_square,
            marker_magnitude: best,
            hit: Some(MarkerHit {
                at,
                amplitude: best,
            }),
        }
    }

    /// 单频点 Goertzel 幅度估计（只取左声道；返回值和「正弦幅度」同量纲）。
    pub(super) fn goertzel_magnitude(
        samples: &[f32],
        freq: f32,
        sample_rate: f32,
        channels: usize,
    ) -> f32 {
        if samples.is_empty() || channels == 0 {
            return 0.0;
        }
        let omega = std::f32::consts::TAU * freq / sample_rate;
        let coefficient = 2.0 * omega.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        let mut count = 0usize;
        for frame in samples.chunks_exact(channels) {
            let value = frame.first().copied().unwrap_or(0.0);
            let s0 = value + coefficient * s1 - s2;
            s2 = s1;
            s1 = s0;
            count += 1;
        }
        if count == 0 {
            return 0.0;
        }
        let power = s1 * s1 + s2 * s2 - coefficient * s1 * s2;
        (power.max(0.0).sqrt()) * 2.0 / count as f32
    }

    // ---------------------------------------------------------------------
    // 工作线程：组帧 → 编码 → 解码 → 播放（COM 同样在本线程内初始化）
    // ---------------------------------------------------------------------
    fn worker_thread_main(
        stop: Arc<AtomicBool>,
        options: RunOptions,
        codec_config: CodecConfig,
        mut reader: SampleRingReader,
        period_slot: Arc<AtomicU32>,
        marker_interval: Option<Duration>,
    ) -> Result<WorkerOutcome, String> {
        let selector = DeviceSelector::parse(
            options
                .sink_device
                .as_deref()
                .unwrap_or(options.device.as_str()),
        );
        let (mut sink, sink_device): (Box<dyn PlayoutSink>, Option<RenderDeviceInfo>) =
            match options.sink_backend {
                SinkBackend::Wasapi => {
                    let render = RenderSink::open(&selector, options.buffer_ms)
                        .map_err(|error| format!("打开 render 输出失败：{error}"))?;
                    let info = render.device_info().clone();
                    (Box::new(render), Some(info))
                }
                SinkBackend::Null => (Box::new(NullPlayout::new(options.buffer_ms)), None),
            };
        let sink_backend = sink.backend_name().to_string();
        let sink_requested_buffer_ms = sink.requested_buffer_ms();
        let sink_effective_buffer_ms = sink.effective_buffer_ms();

        let mut encoder =
            OpusEncoder::new(codec_config).map_err(|error| format!("建编码器失败：{error}"))?;
        let mut decoder =
            OpusDecoder::new(codec_config).map_err(|error| format!("建解码器失败：{error}"))?;
        let mut chunker = FrameChunker::new(codec_config.frame_ms, RING_FRAMES)
            .map_err(|error| format!("建组帧器失败：{error}"))?;

        let frame_interleaved = codec_config.interleaved_frame();
        let mut input: Vec<f32> = Vec::with_capacity(frame_interleaved * RING_FRAMES);
        let mut encoded: Vec<u8> = vec![0u8; proto_budget::SUGGESTED_OUTPUT];
        let mut decoded: Vec<f32> = vec![0.0f32; frame_interleaved];
        let mut output: Vec<f32> = Vec::with_capacity(frame_interleaved);
        let mut ledger = LatencyLedger::new(16_384);
        let mut frames = 0u64;
        let mut encoded_bytes = 0u64;
        let mut markers_sent = 0u64;
        let mut marker_send_times: Vec<Instant> = Vec::new();
        let mut next_marker = marker_interval.map(|interval| Instant::now() + interval);
        let mut marker_pending = false;
        let mut marker_expected_us: Vec<u32> = Vec::new();

        while !stop.load(Ordering::Relaxed) {
            let read = reader.drain_into(&mut input, frame_interleaved * RING_FRAMES);
            if read == 0 {
                std::thread::sleep(Duration::from_micros(500));
                continue;
            }
            chunker.push(&input);
            while let Some(frame) = chunker.next_frame() {
                // ---- 编码 ----
                let encode_started = Instant::now();
                let written = encoder
                    .encode_into(frame, &mut encoded)
                    .map_err(|error| format!("编码失败：{error}"))?;
                let encode_us = encode_started
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u32::MAX)) as u32;
                encoded_bytes += written as u64;

                // ---- 解码（失败按丢包隐藏处理：M2 的 PLC 路径在这里已有落点）----
                let decode_started = Instant::now();
                let decoded_len = match decoder.decode_into(&encoded[..written], &mut decoded) {
                    Ok(len) => len,
                    Err(_) => decoder
                        .conceal_into(&mut decoded)
                        .map_err(|error| format!("丢包隐藏失败：{error}"))?,
                };
                let decode_us = decode_started
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u32::MAX)) as u32;

                // ---- 自环回放：先压增益，再（可选）叠加标记音 ----
                output.clear();
                output.extend(
                    decoded[..decoded_len]
                        .iter()
                        .map(|value| *value * options.gain),
                );
                if let (Some(due), Some(interval)) = (next_marker, marker_interval) {
                    let now = Instant::now();
                    if now >= due {
                        overlay_marker(&mut output);
                        marker_pending = true;
                        next_marker = Some(now + interval);
                    }
                }

                let output_frames = (output.len() / CHANNELS as usize) as u64;
                if let Err(error) = sink.write(&output)
                    && !error.is_statistical()
                {
                    return Err(format!("播放失败：{error}"));
                }

                // 播放段口径：**写入之后**的水位减去刚写进去的这一帧 = 「我们这帧前面还排着多少音频」。
                // 早先在 write 之前读水位，量到的是「等空间前的排队量」，会系统性高估
                // （实测 22 ms vs 真实 ≈10 ms），也无法与标记法往返对上。
                let queued_after = u64::from(sink.buffered_frames());
                let ahead_frames = queued_after.saturating_sub(output_frames);
                let ahead_us = (ahead_frames * 1_000_000 / u64::from(SAMPLE_RATE_HZ)) as u32;
                let playout_us = ahead_us + codec_config.frame_ms * 500;

                // 发出时刻要取「帧真正进入设备缓冲之后」——`write` 可能为了等空间阻塞若干个周期，
                // 用之前的时刻会把这段等待算进往返里（实测会多出 10–40 ms 的假延迟）。
                if marker_pending {
                    marker_send_times.push(Instant::now());
                    // 同口径预期：写后排队 + 标记在帧内的位置（标记铺在输出帧起始 10 ms，
                    // 检出的「能量重心」≈ 中点，即 MARKER_FRAMES/2 = 240 帧 = 5 ms）
                    marker_expected_us.push(
                        ahead_us
                            + (MARKER_FRAMES as u64 / 2 * 1_000_000 / u64::from(SAMPLE_RATE_HZ))
                                as u32,
                    );
                    markers_sent += 1;
                    marker_pending = false;
                }

                let period_us = period_slot.load(Ordering::Relaxed).max(1);
                ledger.record(
                    &SegmentSample {
                        capture_us: period_us / 2,
                        assemble_us: frame_assembly_us(codec_config.frame_ms),
                        encode_us: encode_us.saturating_add(encoder.nominal_pipeline_delay_us()),
                        decode_us,
                        playout_us,
                    },
                    false, // 采集段是「半周期」模型值，不是设备时间戳实测
                );
                frames += 1;
            }
        }

        sink.stop();
        Ok(WorkerOutcome {
            ledger,
            frames,
            encoded_bytes,
            plc_count: decoder.plc_count(),
            chunker_dropped_frames: chunker.stats().dropped_frames,
            sink_stats: sink.stats(),
            sink_device,
            sink_requested_buffer_ms,
            sink_effective_buffer_ms,
            backend: sink_backend,
            markers_sent,
            marker_send_times,
            marker_expected_us,
        })
    }

    /// 在输出帧头部叠加**标记音**（19 kHz、10 ms、1 ms 淡入淡出）。
    ///
    /// 选 19 kHz 而不是脉冲：脉冲靠幅度门限，在「有音乐在响」的端点上会被淹没（实测过）；
    /// 19 kHz 窄带标记靠频带能量检出，与音乐内容基本解耦（见 [`scan_packet`] 的说明）。
    pub(super) fn overlay_marker(samples: &mut [f32]) {
        let frames = samples.len() / CHANNELS as usize;
        let count = MARKER_FRAMES.min(frames);
        for index in 0..count {
            let envelope = if index < MARKER_FADE_FRAMES {
                index as f32 / MARKER_FADE_FRAMES as f32
            } else if index + MARKER_FADE_FRAMES >= count {
                (count - index) as f32 / MARKER_FADE_FRAMES as f32
            } else {
                1.0
            };
            let value = (std::f32::consts::TAU * MARKER_FREQ_HZ * index as f32
                / SAMPLE_RATE_HZ as f32)
                .sin()
                * MARKER_AMPLITUDE
                * envelope;
            let left = index * CHANNELS as usize;
            if let Some(slot) = samples.get_mut(left) {
                *slot = (*slot + value).clamp(-1.0, 1.0);
            }
            if let Some(slot) = samples.get_mut(left + 1) {
                *slot = (*slot + value).clamp(-1.0, 1.0);
            }
        }
    }

    /// 复制并升序排序（`&[u32]` → `Vec<u32>`）。
    fn sorted_copy(values: &[u32]) -> Vec<u32> {
        let mut copy = values.to_vec();
        copy.sort_unstable();
        copy
    }

    /// 取最近秩分位数（`sorted` 必须已升序）。
    fn percentile_us(sorted: &[u32], percent: f64) -> Option<u32> {
        if sorted.is_empty() {
            return None;
        }
        let rank = (percent / 100.0 * sorted.len() as f64).ceil() as usize;
        sorted
            .get(rank.saturating_sub(1).min(sorted.len() - 1))
            .copied()
    }

    // ---------------------------------------------------------------------
    // 报告
    // ---------------------------------------------------------------------
    struct Report {
        options: RunOptions,
        capture: CaptureOutcome,
        worker: WorkerOutcome,
        measured: Option<MeasurementRow>,
        markers_enabled: bool,
    }

    struct MeasurementRow {
        count: usize,
        min_us: u64,
        p50_us: u64,
        p95_us: u64,
        max_us: u64,
    }

    impl Report {
        fn new(
            options: &RunOptions,
            capture: &CaptureOutcome,
            worker: &WorkerOutcome,
            markers_enabled: bool,
        ) -> Self {
            let mut hits = capture.hits.clone();
            hits.sort_by_key(|hit| hit.at);
            let mut sends = worker.marker_send_times.clone();
            sends.sort();
            // 一一配对：每个「发出」配**它之后最近的一次命中**（一次命中只能用一次）。
            // 早先的实现把「命中之前的全部发出」都算了一遍，会产出一堆假的小值（实测出现过 2.75 ms）。
            let mut deltas: Vec<u64> = Vec::new();
            let mut hit_index = 0usize;
            for send in &sends {
                while hit_index < hits.len() && hits[hit_index].at < *send {
                    hit_index += 1;
                }
                let Some(hit) = hits.get(hit_index) else {
                    break;
                };
                let delta = hit.at.duration_since(*send);
                if delta < Duration::from_millis(1_500) {
                    deltas.push(delta.as_micros() as u64);
                }
                hit_index += 1;
            }
            let measured = if deltas.is_empty() {
                None
            } else {
                deltas.sort_unstable();
                let pick = |percent: f64| -> u64 {
                    let rank = (percent / 100.0 * deltas.len() as f64).ceil() as usize;
                    deltas
                        .get(rank.saturating_sub(1).min(deltas.len() - 1))
                        .copied()
                        .unwrap_or(0)
                };
                Some(MeasurementRow {
                    count: deltas.len(),
                    min_us: deltas.first().copied().unwrap_or(0),
                    p50_us: pick(50.0),
                    p95_us: pick(95.0),
                    max_us: deltas.last().copied().unwrap_or(0),
                })
            };

            Self {
                options: options.clone(),
                capture: CaptureOutcome {
                    stats: capture.stats,
                    hits: hits.clone(),
                    device: capture.device.clone(),
                    requested_buffer_ms: capture.requested_buffer_ms,
                    effective_buffer_ms: capture.effective_buffer_ms,
                    period_ms: capture.period_ms,
                    backend: capture.backend.clone(),
                    ring_dropped_samples: capture.ring_dropped_samples,
                    peak_milli: capture.peak_milli,
                    rms_milli: capture.rms_milli,
                    ambient_rms_milli: capture.ambient_rms_milli,
                    marker_magnitude_milli: capture.marker_magnitude_milli,
                },
                worker: WorkerOutcome {
                    ledger: worker.ledger.clone(),
                    frames: worker.frames,
                    encoded_bytes: worker.encoded_bytes,
                    plc_count: worker.plc_count,
                    chunker_dropped_frames: worker.chunker_dropped_frames,
                    sink_stats: worker.sink_stats,
                    sink_device: worker.sink_device.clone(),
                    sink_requested_buffer_ms: worker.sink_requested_buffer_ms,
                    sink_effective_buffer_ms: worker.sink_effective_buffer_ms,
                    backend: worker.backend.clone(),
                    markers_sent: worker.markers_sent,
                    marker_send_times: Vec::new(),
                    marker_expected_us: sorted_copy(&worker.marker_expected_us),
                },
                measured,
                markers_enabled,
            }
        }

        fn bitrate_bps(&self) -> u64 {
            if self.worker.frames == 0 {
                return 0;
            }
            self.worker.encoded_bytes * 8 * 1_000
                / (self.worker.frames * u64::from(self.options.frame_ms))
        }

        fn print(&self, quiet: bool) {
            let latency = &self.worker.ledger.report();
            if quiet {
                if let (Some(row), Some(total)) = (&self.measured, &latency.e2e_estimated) {
                    println!(
                        "自环估算 P50={:.1} ms；实测往返 P50={:.1} ms P95={:.1} ms（{} 次）",
                        total.p50 as f64 / 1000.0,
                        row.p50_us as f64 / 1000.0,
                        row.p95_us as f64 / 1000.0,
                        row.count
                    );
                } else if let Some(total) = &latency.e2e_estimated {
                    println!("自环估算 P50={:.1} ms", total.p50 as f64 / 1000.0);
                } else {
                    println!("没有采集到足够的帧（时长太短或端点安静）");
                }
                return;
            }

            println!("\n================ 链路 ================");
            println!(
                "采集：{}{}",
                self.capture.backend,
                describe_device(&self.capture.device)
            );
            println!(
                "      缓冲请求 {} ms → 实际 {} ms；设备周期 默认 {:.1} ms / 最小 {:.1} ms",
                self.capture.requested_buffer_ms,
                self.capture.effective_buffer_ms,
                self.capture.period_ms.0,
                self.capture.period_ms.1
            );
            println!(
                "播放：{}{}",
                self.worker.backend,
                describe_device(&self.worker.sink_device)
            );
            println!(
                "      缓冲请求 {} ms → 实际 {} ms",
                self.worker.sink_requested_buffer_ms, self.worker.sink_effective_buffer_ms
            );
            println!(
                "编码：Opus 48 kHz / 立体声 / {} ms / 实测 {} kbps / 复杂度 7 / VBR / in-band FEC 关",
                self.options.frame_ms,
                self.bitrate_bps() / 1_000
            );

            println!("\n============ 五段延迟分解（μs）============");
            println!(
                "{:<12} {:>8} {:>8} {:>8} {:>8} {:>8}",
                "段", "P50", "P95", "P99", "MAX", "样本"
            );
            print_row("采集(模型)", latency.capture.as_ref());
            print_row("组帧", latency.assemble.as_ref());
            print_row("编码", latency.encode.as_ref());
            print_row("解码", latency.decode.as_ref());
            print_row("播放", latency.playout.as_ref());
            print_row("合计(估算)", latency.e2e_estimated.as_ref());

            println!("\n================ 标记法实测 ================");
            match &self.measured {
                Some(row) => {
                    let strongest = self
                        .capture
                        .hits
                        .iter()
                        .map(|hit| hit.amplitude)
                        .fold(0.0f32, f32::max);
                    println!(
                        "设备往返（render 提交 → loopback 检出）：P50 {:.2} ms / P95 {:.2} ms / min {:.2} ms / max {:.2} ms，n={}（发出 {}，最强峰值 {:.2}）",
                        row.p50_us as f64 / 1000.0,
                        row.p95_us as f64 / 1000.0,
                        row.min_us as f64 / 1000.0,
                        row.max_us as f64 / 1000.0,
                        row.count,
                        self.worker.markers_sent,
                        strongest
                    );
                    println!("  口径：render 排队 + 引擎周期 + loopback 排队，不含任何模型假设；");
                    println!(
                        "        标记铺在输出帧起始 {} ms 处，检出的「能量重心」≈ 其中点（{} ms）——",
                        MARKER_FRAMES / 48,
                        MARKER_FRAMES / 2 / 48
                    );
                    println!(
                        "        因此与「写后排队」同口径的模型值是下面这一行，两者应吻合（差 ≤ 2 ms）："
                    );
                    let expected =
                        percentile_us(&self.worker.marker_expected_us, 50.0).unwrap_or(0);
                    println!(
                        "        同口径模型 P50 = {:.2} ms（写后排队 + 标记中点）vs 实测 P50 = {:.2} ms",
                        expected as f64 / 1000.0,
                        row.p50_us as f64 / 1000.0
                    );
                    println!(
                        "        注：全帧口径的「播放」段仍是「排队 + 半个帧长」，与标记列**不应**相等。"
                    );
                }
                None if self.markers_enabled => {
                    println!(
                        "⚠ 没有检出任何标记脉冲（检查端点是否在播放、环境噪声是否淹没了阈值）"
                    );
                }
                None => println!("（标记往返未启用：需要 wasapi 双端 + 同一端点）"),
            }

            println!("\n================ 计数与健康度 ================");
            let capture = self.capture.stats;
            let playout = self.worker.sink_stats;
            println!(
                "采集：包 {} / 帧 {} / 静音包 {} / 不连续 {} / 读超时 {} / 空唤醒 {} / 节奏间隙 {}",
                capture.packets,
                capture.frames,
                capture.silent_packets,
                capture.discontinuities,
                capture.read_timeouts,
                capture.empty_wakeups,
                capture.gaps
            );
            println!(
                "环节：环丢弃样本 {} / 组帧丢弃帧 {} / 编码帧 {}（{} B）/ PLC {}",
                self.capture.ring_dropped_samples,
                self.worker.chunker_dropped_frames,
                self.worker.frames,
                self.worker.encoded_bytes,
                self.worker.plc_count
            );
            println!(
                "播放：写入 {} 次 / {} 帧 / 欠载 {} / 写失败 {}",
                playout.writes, playout.frames_written, playout.underruns, playout.write_errors
            );
            let to_dbfs = |milli: u32| {
                let value = milli as f64 / 1_000.0;
                if value <= 0.0 {
                    "-∞".to_string()
                } else {
                    format!("{:.1}", 20.0 * value.log10())
                }
            };
            println!(
                "采集电平：峰值 {} dBFS / RMS {} dBFS / 环境 RMS {} dBFS；标记音（19 kHz）实测幅度 {} dBFS，门限 {} dBFS",
                to_dbfs(self.capture.peak_milli),
                to_dbfs(self.capture.rms_milli),
                to_dbfs(self.capture.ambient_rms_milli),
                to_dbfs(self.capture.marker_magnitude_milli),
                to_dbfs((MARKER_MAGNITUDE_THRESHOLD * 1_000.0) as u32),
            );
            if self.markers_enabled && self.capture.marker_magnitude_milli < 100 {
                println!(
                    "⚠ 标记音在采集侧几乎测不到 —— 这次往返测量不可信（端点可能对 19 kHz 做了滤波，或音量过低）"
                );
            }

            println!("\n================ 判定 ================");
            match &latency.e2e_estimated {
                Some(summary) => {
                    let p50_ms = summary.p50 as f64 / 1000.0;
                    let p95_ms = summary.p95 as f64 / 1000.0;
                    println!(
                        "自环（不含网络/Android）P50 = {:.1} ms {} / P95 = {:.1} ms {}",
                        p50_ms,
                        if p50_ms <= 110.0 {
                            "✅ ≤ 110"
                        } else {
                            "❌ > 110"
                        },
                        p95_ms,
                        if p95_ms <= 150.0 {
                            "✅ ≤ 150"
                        } else {
                            "❌ > 150"
                        }
                    );
                    println!(
                        "  注：M1 验收口径是「PC → 手机」端到端；自环只覆盖采集/编码/解码/播放，"
                    );
                    println!(
                        "      网络与 Android 播放（预计 10–30 ms）未计入 —— 自环必须明显低于阈值才留有余量。"
                    );
                }
                None => println!("帧数不足，无法出分位数（时长太短或端点安静）"),
            }
            if self.options.capture_backend == CaptureBackend::Synth {
                println!(
                    "  注：采集后端是 synth（合成源），以上数字只验证链路与记账，不代表真实设备。"
                );
            }
        }

        fn to_json(&self) -> Result<String> {
            let latency = self.worker.ledger.report();
            let summary_json = |summary: &Option<Summary>| {
                summary.map(|value| {
                    serde_json::json!({
                        "count": value.count, "min": value.min, "p50": value.p50,
                        "p95": value.p95, "p99": value.p99, "max": value.max, "mean": value.mean
                    })
                })
            };
            let device_json = |device: &Option<RenderDeviceInfo>| {
                device.as_ref().map(|info| {
                    serde_json::json!({
                        "name": info.name,
                        "id": info.id,
                        "is_default": info.is_default,
                        "format": info.format_text,
                        "sample_rate": info.format.sample_rate,
                        "channels": info.format.channels,
                        "default_period_ms": info.default_period_ms(),
                        "min_period_ms": info.min_period_ms(),
                        "looks_virtual": info.looks_virtual()
                    })
                })
            };
            let generated_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_millis())
                .unwrap_or(0);
            let capture = self.capture.stats;
            let playout = self.worker.sink_stats;
            let report = serde_json::json!({
                "tool": "self-loop",
                "version": env!("CARGO_PKG_VERSION"),
                "generated_at_unix_ms": generated_at,
                "options": {
                    "device": self.options.device,
                    "sink_device": self.options.sink_device,
                    "seconds": self.options.seconds,
                    "frame_ms": self.options.frame_ms,
                    "buffer_ms": self.options.buffer_ms,
                    "gain": self.options.gain,
                    "marker_ms": self.options.marker_ms
                },
                "capture": {
                    "backend": self.capture.backend,
                    "device": device_json(&self.capture.device),
                    "requested_buffer_ms": self.capture.requested_buffer_ms,
                    "effective_buffer_ms": self.capture.effective_buffer_ms,
                    "device_period_ms": {"default": self.capture.period_ms.0, "min": self.capture.period_ms.1},
                    "level_milli": {
                        "peak": self.capture.peak_milli,
                        "rms": self.capture.rms_milli,
                        "ambient_rms": self.capture.ambient_rms_milli,
                        "marker_magnitude": self.capture.marker_magnitude_milli
                    },
                    "stats": {
                        "packets": capture.packets,
                        "frames": capture.frames,
                        "silent_packets": capture.silent_packets,
                        "discontinuities": capture.discontinuities,
                        "read_timeouts": capture.read_timeouts,
                        "empty_wakeups": capture.empty_wakeups,
                        "gaps": capture.gaps,
                        "ring_dropped_samples": self.capture.ring_dropped_samples
                    }
                },
                "playout": {
                    "backend": self.worker.backend,
                    "device": device_json(&self.worker.sink_device),
                    "requested_buffer_ms": self.worker.sink_requested_buffer_ms,
                    "effective_buffer_ms": self.worker.sink_effective_buffer_ms,
                    "stats": {
                        "writes": playout.writes,
                        "frames_written": playout.frames_written,
                        "underruns": playout.underruns,
                        "write_errors": playout.write_errors
                    }
                },
                "codec": {
                    "encoder_frames": self.worker.frames,
                    "encoded_bytes": self.worker.encoded_bytes,
                    "measured_bitrate_bps": self.bitrate_bps(),
                    "plc_count": self.worker.plc_count,
                    "dropped_frames": self.worker.chunker_dropped_frames
                },
                "latency_us": {
                    "capture": summary_json(&latency.capture),
                    "assemble": summary_json(&latency.assemble),
                    "encode": summary_json(&latency.encode),
                    "decode": summary_json(&latency.decode),
                    "playout": summary_json(&latency.playout),
                    "e2e_estimated": summary_json(&latency.e2e_estimated)
                },
                "roundtrip_measured_us": self.measured.as_ref().map(|row| {
                    serde_json::json!({
                        "count": row.count, "min": row.min_us, "p50": row.p50_us,
                        "p95": row.p95_us, "max": row.max_us,
                        "sent": self.worker.markers_sent
                    })
                }),
                "verdict": {
                    "p50_ok": latency.e2e_estimated.as_ref().map(|s| s.p50 <= 110_000).unwrap_or(false),
                    "p95_ok": latency.e2e_estimated.as_ref().map(|s| s.p95 <= 150_000).unwrap_or(false),
                    "scope": "loopback-self-test（不含网络与 Android 播放）"
                }
            });
            Ok(serde_json::to_string_pretty(&report)?)
        }
    }

    fn describe_device(device: &Option<RenderDeviceInfo>) -> String {
        match device {
            Some(info) => format!("（{}，{}）", info.name, info.format_text),
            None => String::new(),
        }
    }

    fn print_row(label: &str, summary: Option<&Summary>) {
        match summary {
            Some(value) => println!(
                "{:<12} {:>8} {:>8} {:>8} {:>8} {:>8}",
                label, value.p50, value.p95, value.p99, value.max, value.count
            ),
            None => println!("{label:<12} {:>8}", "—"),
        }
    }

    // ---------------------------------------------------------------------
    // 参数解析
    // ---------------------------------------------------------------------
    pub(super) fn parse_run_options(args: &[String]) -> Result<RunOptions> {
        let mut options = RunOptions::default();
        let mut index = 0usize;
        while index < args.len() {
            let argument = args[index].as_str();
            match argument {
                "--device" => {
                    options.device = take_value(args, index, "--device")?;
                    index += 2;
                }
                "--sink-device" => {
                    options.sink_device = Some(take_value(args, index, "--sink-device")?);
                    index += 2;
                }
                "--seconds" => {
                    options.seconds =
                        parse_f64(take_value(args, index, "--seconds")?, 0.5, 3_600.0)?;
                    index += 2;
                }
                "--frame-ms" => {
                    let value = parse_u32(take_value(args, index, "--frame-ms")?, 10, 20)?;
                    if !matches!(value, 10 | 20) {
                        return Err(anyhow!(
                            "--frame-ms 只支持 10 或 20（RESTRICTED_LOWDELAY 是 CELT-only 模式），收到 {value}"
                        ));
                    }
                    options.frame_ms = value;
                    index += 2;
                }
                "--buffer-ms" => {
                    options.buffer_ms = parse_u32(take_value(args, index, "--buffer-ms")?, 1, 500)?;
                    index += 2;
                }
                "--capture" => {
                    options.capture_backend = match take_value(args, index, "--capture")?.as_str() {
                        "wasapi" => CaptureBackend::Wasapi,
                        "synth" => CaptureBackend::Synth,
                        other => {
                            return Err(anyhow!("--capture 只支持 wasapi|synth（收到 {other}）"));
                        }
                    };
                    index += 2;
                }
                "--sink" => {
                    options.sink_backend = match take_value(args, index, "--sink")?.as_str() {
                        "wasapi" => SinkBackend::Wasapi,
                        "null" => SinkBackend::Null,
                        other => return Err(anyhow!("--sink 只支持 wasapi|null（收到 {other}）")),
                    };
                    index += 2;
                }
                "--gain" => {
                    options.gain = parse_f64(take_value(args, index, "--gain")?, 0.0, 1.0)? as f32;
                    index += 2;
                }
                "--marker-ms" => {
                    options.marker_ms = u64::from(parse_u32(
                        take_value(args, index, "--marker-ms")?,
                        0,
                        60_000,
                    )?);
                    index += 2;
                }
                "--no-render" => {
                    options.sink_backend = SinkBackend::Null;
                    options.marker_ms = 0;
                    index += 1;
                }
                "--json" => {
                    options.json = Some(PathBuf::from(take_value(args, index, "--json")?));
                    index += 2;
                }
                "--quiet" => {
                    options.quiet = true;
                    index += 1;
                }
                "-h" | "--help" => {
                    print!("{}", super::HELP);
                    std::process::exit(0);
                }
                other => return Err(anyhow!("未知参数 {other}（--help 看用法）")),
            }
        }
        Ok(options)
    }

    fn take_value(args: &[String], index: usize, name: &str) -> Result<String> {
        args.get(index + 1)
            .cloned()
            .ok_or_else(|| anyhow!("{name} 需要一个参数"))
    }

    fn parse_u32(text: String, min: u32, max: u32) -> Result<u32> {
        let value: u32 = text
            .parse()
            .with_context(|| format!("解析整数 {text:?} 失败"))?;
        if value < min || value > max {
            return Err(anyhow!("{value} 超出范围 {min}..={max}"));
        }
        Ok(value)
    }

    fn parse_f64(text: String, min: f64, max: f64) -> Result<f64> {
        let value: f64 = text
            .parse()
            .with_context(|| format!("解析数值 {text:?} 失败"))?;
        if !value.is_finite() || value < min || value > max {
            return Err(anyhow!("{value} 超出范围 {min}..={max}"));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 默认参数符合_m1_基线() {
        let options = RunOptions::default();
        assert_eq!(options.frame_ms, 20);
        assert_eq!(options.buffer_ms, 20);
        assert_eq!(options.capture_backend, CaptureBackend::Wasapi);
        assert_eq!(options.sink_backend, SinkBackend::Wasapi);
        assert!(
            (options.gain - 0.02).abs() < f32::EPSILON,
            "自环增益必须很小"
        );
        assert_eq!(options.marker_ms, 500);
    }

    #[cfg(windows)]
    #[test]
    fn 解析参数与范围校验() {
        let options = imp::parse_run_options(&[
            "--device".to_string(),
            "id:{c85b743e".to_string(),
            "--seconds".to_string(),
            "5".to_string(),
            "--frame-ms".to_string(),
            "10".to_string(),
            "--gain".to_string(),
            "0.05".to_string(),
            "--no-render".to_string(),
        ])
        .unwrap();
        assert_eq!(options.device, "id:{c85b743e");
        assert_eq!(options.seconds, 5.0);
        assert_eq!(options.frame_ms, 10);
        assert!((options.gain - 0.05).abs() < 1e-6);
        assert_eq!(options.sink_backend, SinkBackend::Null);
        assert_eq!(options.marker_ms, 0);

        assert!(imp::parse_run_options(&["--frame-ms".to_string(), "15".to_string()]).is_err());
        assert!(imp::parse_run_options(&["--nope".to_string()]).is_err());
        assert!(imp::parse_run_options(&["--gain".to_string(), "2".to_string()]).is_err());
        assert!(imp::parse_run_options(&["--capture".to_string(), "alsa".to_string()]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn 标记音可被检出且不拖尾() {
        let mut samples = vec![0.0f32; 1_920];
        imp::overlay_marker(&mut samples);

        // 幅度与位置：19 kHz 标记的峰值应接近设定幅度
        let peak = samples
            .iter()
            .cloned()
            .fold(0.0f32, |acc, value| acc.max(value.abs()));
        assert!(peak > 0.4 && peak <= 0.46, "标记音幅度异常：{peak}");

        // 19 kHz 音的采样点相邻样本近似反相（48 kHz 下每周期 2.526 个样本）
        let first = samples.get(100).copied().unwrap_or(0.0);
        let second = samples.get(102).copied().unwrap_or(0.0);
        assert!(
            first * second < 0.0,
            "19 kHz 相邻样本应反相：{first} / {second}"
        );

        // 480 帧之后完全静音（线性淡出到 0）
        let tail_peak = samples[962..]
            .iter()
            .cloned()
            .fold(0.0f32, |acc, value| acc.max(value.abs()));
        assert!(tail_peak < 1e-6, "标记音拖尾过长：{tail_peak}");

        // Goertzel 检出：整段应远高于门限，且明显高于 1 kHz（用同一把尺子量无关频率）
        let marker = imp::goertzel_magnitude(&samples, 19_000.0, 48_000.0, 2);
        let other = imp::goertzel_magnitude(&samples, 1_000.0, 48_000.0, 2);
        assert!(
            marker > 0.2,
            "19 kHz 检出幅度过低：{marker}（门限 {}）",
            imp::MARKER_MAGNITUDE_THRESHOLD
        );
        assert!(
            marker > other * 10.0,
            "频带选择性不足：19k={marker} 1k={other}"
        );
    }
}
