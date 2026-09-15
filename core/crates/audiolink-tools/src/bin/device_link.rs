//! device-link —— PC 侧真机（Android）验收工具：连接 → PIN 配对 → 推流 → 跨机延迟账本
//!
//! 规格：`docs/03-protocol.md`（§5 握手与 PIN、§6 时钟同步、§10 遥测）、
//! `docs/11-m1-contract.md`（§5 引擎冻结 API）、`docs/10-handoff.md` §4.2（跨机测量）。
//!
//! # 它解决什么问题
//!
//! M1 的真机验收需要一个**无头、可复跑、可审计**的路径：桌面上只有 Tauri GUI（无头环境点不动），
//! 而 `link-loop` 量的是 PC↔PC（两端共享同一个进程级单调时钟）。本工具把同一件事搬到真机上：
//! 真实 `Engine` + 真实 QUIC（mTLS）+ 真实 §5 握手 + 真实 PIN 配对 + 真实推流，
//! 产出一份**逐项标注来源**的跨机延迟账本 + JSON。
//!
//! # 报告口径（这份工具的灵魂：不许把模型值混进实测值，也不许把「没测到」写成 0）
//!
//! | 标注 | 含义 |
//! |---|---|
//! | `[实测]` | 本次运行真的量到的数（附样本数 n） |
//! | `[模型]` | 结构性常量，或由实测值按公式换算（公式写在行内） |
//! | `[未测]` | 没量到 —— **绝不填 0**，写清原因并进「未测项清单」 |
//!
//! 跨机延迟的分解（PC 推流 → 手机出声）：
//!
//! ```text
//!   PC: 采集半周期 ──► 组帧 ──► 编码 ──► 数据报发出
//!        [模型]        [模型]   [实测]     [未测/自检]
//!                                    │
//!                          网络单向（对称路径假设）[实测]
//!                                    │
//!       手机: 到达 ──► 解码 ──► 播放环 ──► AudioTrack 输出
//!                     [未测]    [实测]      [未测（设备侧补）]
//! ```
//!
//! **为什么 §6 是跨机测量的前提**：`Instant` 不可跨机相减（`docs/10-handoff.md` §4.2）。
//! 只有拿到 `offset = 对端 − 本机`，才能把「对端时间戳」换算到本机基准。
//! 逐探针的四时间戳（`t1/t2/t3/t4`）来自引擎会话内的 §6 环；未收敛时（样本 < 8）
//! 估计为 `None`，报告里呈现为「未收敛」，**不是 0**。
//!
//! # 身份与信任库（复跑的前提）
//!
//! 默认落在 `target/device-link/`（`cert.pem` / `key.pem` / `trust.json`）。
//! 真机验收要复跑很多次，配对结果跨次留存 —— 第一次 `--pin` 之后，后续运行直接走
//! `AUTH_CHALLENGE/RESPONSE` 分支，不再需要看手机屏幕。
//!
//! # PIN 必须在**同一条连接内**提交（工具为什么需要 `--pin-file`）
//!
//! 内核是**每连接新建一个 `PinGate`**：会话一断/重连，那条连接上的 PIN 立刻作废
//! （实测三次连接拿到三个不同 PIN）。所以「先连上 → 手机显示 PIN → 人工读到 → 再提交」这一步
//! **必须发生在同一条会话里**，不能断开重连后拿刚才那个 PIN 去试。
//!
//! 无人值守（后台作业）没法在正确时刻往 stdin 打字，于是三条来源按「确定性」排序：
//! `--pin <NNNNNN>`（已知，复跑用）→ `--pin-file <path>`（连接期间随时把手机屏幕上的 6 位数字写进文件，
//! 工具每 300 ms 轮询、上限 60 s）→ 交互 stdin。
//!
//! 死线口径（task-10 之后）：进入配对等待时，会话死线会顺延到「PIN 有效期 + 15 s 余量」（PIN 60 s → 75 s），
//! 所以「人工读屏再敲字」的正常节奏不会再被 10 s 握手死线掐断（本机实测：把人工延迟模拟成 20 s 仍配对成功）。
//! 工具这边的 60 s 上限是 **PIN 自身的有效期** —— 再等下去手机上的那个 PIN 已经过期，等下去没有意义。配对成功后写入信任库，
//! **之后每次运行都直接走 `AUTH_CHALLENGE/AUTH_RESPONSE`，再也不需要看手机屏幕**（这正是要复跑很多次的真机验收的前提）。
//!
//! # 用法
//!
//! ```text
//! device-link run --peer 172.16.2.54 --seconds 30 --pin 123456 --capture wasapi
//! device-link --self-test            # 同进程起两个真 Engine（不依赖真机，验证工具自身流程）
//! ```
//!
//! 退出码：`0` 成功 · `2` 参数/连接/配对失败 · `3` 对端无遥测或无出声证据 ·
//! `4` §6 时钟未收敛（offset 一直为 0）· `5` PC 侧采集疑似静音（默认端点是虚拟声卡？）

use std::io::IsTerminal;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use audiolink_audio::{
    CaptureSource, CodecConfig, NullPlayout, OpusEncoder, PlayoutSink, SampleStats, Summary,
    SyntheticCapture,
};
use audiolink_engine::session::SessionState;
use audiolink_engine::{
    CaptureFactory, ClockProbeStats, Engine, EngineConfig, EngineEvent, MeasurementTap,
    PlayoutFactory,
};
use audiolink_proto::{AudioDatagram, ClockProbe, ClockReply};
use audiolink_tools::{ClockSamples, Sample};
use audiolink_types::{DEFAULT_QUIC_PORT, ErrorCode, NodeId, StreamStats};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// 退出码（失败模式必须显式，不许静默返回漂亮数字）
// ---------------------------------------------------------------------------

/// 成功：观测完成，且所有「必须有」的实测项都拿到。
const EXIT_OK: u8 = 0;
/// 参数错误 / 连不上 / 握手失败 / PIN 错。
const EXIT_USAGE: u8 = 2;
/// 会话建立但**对端没有出声证据**（没收到 1 Hz `STREAM_STATS`，或全程 buffer/underrun/e2e 为 0）。
const EXIT_NO_PEER: u8 = 3;
/// §6 时钟同步未收敛：`offset` 一直为 0 / 样本 < 8 —— 跨机账本不成立。
const EXIT_NO_CLOCK: u8 = 4;
/// PC 侧采集疑似静音（可能是虚拟端点，见 `docs/10-handoff.md` 坑 12）。
const EXIT_SILENT_CAPTURE: u8 = 5;

/// 1 Hz `STREAM_STATS` 下，码率低于此值判为「PC 侧采到的基本是静音」。
///
/// 依据：`--capture synth` 的 440 Hz 正弦在 160 kbps 档实测落到 120+ kbps，
/// 而静音帧（CELT-only、VBR、DTX 关）只有几 kbps 量级。这是**启发式**，报告里会打印原始数字。
const SILENT_CAPTURE_BITRATE_BPS: u32 = 20_000;

/// 默认身份/信任库目录（跨次留存，真机验收要复跑很多次）。
const DEFAULT_DIR: &str = "target/device-link";

/// 证据目录（JSON 报告默认落这里）。
const EVIDENCE_DIR: &str = "target/evidence/device-tool";

const USAGE: &str = "\
device-link —— PC 侧真机（Android）验收：连接 / PIN 配对 / 推流 / 跨机延迟账本

用法：
  device-link run --peer <IP[:PORT]> [选项]      # 对真机（或任意 peer）跑一次验收
  device-link --self-test [选项]                 # 同进程起两个真 Engine，验证工具自身流程（不依赖真机）

选项：
  --peer <IP[:PORT]>    对端地址（省略端口 = 默认 QUIC 端口 58290）
  --seconds <n>         观测时长（秒），默认 30
  --pin <NNNNNN>        已知 PIN 时直接提交（复跑用：配对成功会落盘信任库，第二次就不再需要 PIN）
  --pin-file <path>     无头/无人值守：连接后收到 PinNeeded 起轮询该文件（每 300 ms 一次，上限 60 s），
                        读到 6 位数字就提交 —— 你可以随时把手机屏幕上的 PIN 写进这个文件
  --frame-ms <n>        帧长（10 / 20），默认 20
  --capture <wasapi|synth>
                        采集后端，默认 wasapi（真实 WASAPI loopback = 系统正在播放的声音）
                        synth = 合成正弦，无声卡 / 无播放内容时的对照路径
  --device <sel>        WASAPI 渲染端点：default | id:<子串> | name:<友好名>（默认 default）
  --buffer-ms <n>       采集源请求缓冲（ms），默认 20（共享模式实测下限 22 ms）
  --cross-probe <n>     额外起一条**独立测量连接**跑 §6 探针（mTLS，需本机身份目录），n = 探针数，默认 0（关闭）
  --dir <path>          身份/信任库目录，默认 target/device-link（--self-test 默认用临时目录）
  --listen-port <n>     --self-test 时 node-b 的监听端口（默认 0 = 随机）。给固定端口便于用
                        latency-probe 等外部工具对准同一条链路做交叉验证
  --json <path>         额外导出 JSON 报告（默认总是写 target/evidence/device-tool/）
  --quiet               只打印结论账本，不打进度

退出码：0 成功 · 2 参数/连接/配对失败 · 3 对端无遥测或无出声证据 · 4 时钟未收敛 · 5 PC 采集疑似静音

说明：
  · 身份与信任库落在 --dir（默认 target/device-link/），配对结果跨次留存：第一次用 --pin 配对后，
    后续运行直接走 AUTH_CHALLENGE/RESPONSE，不再需要手机屏幕。
  · --self-test 默认用临时目录（每次都实跑一遍 PIN 配对流程），给它 --dir 才会留存。
  · PIN 是**每连接新建**的（内核每连接一个 PinGate，重连即作废）：必须在**同一条连接内**提交。
    无头环境用 --pin-file —— 连接期间随时把手机屏幕上的 6 位数字写进该文件即可，工具会轮询读到。
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("device-link: 参数错误：{error:#}");
            eprintln!();
            print!("{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("device-link: 创建 tokio 运行时失败：{error}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    match runtime.block_on(run(&options)) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("device-link: 失败：{error:#}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

/// 采集后端。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureBackend {
    /// 真实 WASAPI loopback（系统正在播放的声音）。
    Wasapi,
    /// 合成正弦（对照路径）。
    Synth,
}

impl CaptureBackend {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Wasapi => "wasapi-loopback",
            Self::Synth => "synth",
        }
    }

    /// 本后端的信号来源（报告里必须写清，两种来源的码率不许混为一条数字）。
    const fn source_note(self) -> &'static str {
        match self {
            Self::Wasapi => "真实系统输出（本机渲染端点 loopback）",
            Self::Synth => "合成正弦 440 Hz（无真实声源的对照路径）",
        }
    }
}

/// 运行模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 对真机（或任意 peer）推流。
    Device,
    /// 同进程双 Engine 自检。
    SelfTest,
}

/// 运行参数。
#[derive(Debug, Clone, PartialEq)]
struct Options {
    mode: Mode,
    peer: Option<String>,
    seconds: u64,
    pin: Option<String>,
    pin_file: Option<PathBuf>,
    listen_port: u16,
    frame_ms: u32,
    capture: CaptureBackend,
    device: String,
    buffer_ms: u32,
    need_cross_probe: bool,
    cross_probe: u32,
    dir: Option<PathBuf>,
    json: Option<PathBuf>,
    quiet: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Device,
            peer: None,
            seconds: 30,
            pin: None,
            pin_file: None,
            listen_port: 0,
            frame_ms: 20,
            capture: CaptureBackend::Wasapi,
            device: "default".to_string(),
            buffer_ms: 20,
            need_cross_probe: false,
            cross_probe: 0,
            dir: None,
            json: None,
            quiet: false,
        }
    }
}

/// 解析命令行；`Ok(None)` = 打印用法后正常退出。
fn parse_args(args: &[String]) -> Result<Option<Options>> {
    let mut options = Options::default();
    let mut index = 0;

    // `run` 子命令可省略（`--self-test` 直接作为首参数）
    if args.first().map(String::as_str) == Some("run") {
        index = 1;
    } else if let Some(first) = args.first()
        && matches!(first.as_str(), "-h" | "--help" | "help")
    {
        return Ok(None);
    }

    while index < args.len() {
        let key = args[index].as_str();
        let take = |what: &str| -> Result<String> {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| anyhow!("{what} 缺少取值"))
        };
        match key {
            "--self-test" => {
                options.mode = Mode::SelfTest;
                index += 1;
            }
            "--peer" => {
                options.peer = Some(take("--peer")?);
                index += 2;
            }
            "--seconds" => {
                options.seconds = parse_u64(&take("--seconds")?, "--seconds", 1, 86_400)?;
                index += 2;
            }
            "--pin" => {
                let pin = take("--pin")?;
                if pin.len() != 6 || !pin.chars().all(|c| c.is_ascii_digit()) {
                    bail!("--pin 必须是 6 位数字（收到 {pin:?}）");
                }
                options.pin = Some(pin);
                index += 2;
            }
            "--pin-file" => {
                options.pin_file = Some(PathBuf::from(take("--pin-file")?));
                index += 2;
            }
            "--listen-port" => {
                let port = parse_u64(&take("--listen-port")?, "--listen-port", 1, 65_535)?;
                options.listen_port = port as u16;
                index += 2;
            }
            "--frame-ms" => {
                let value = parse_u64(&take("--frame-ms")?, "--frame-ms", 10, 60)?;
                if !audiolink_audio::format::is_supported_frame_ms(value as u32) {
                    bail!("--frame-ms 只支持 10 / 20 / 40 / 60（收到 {value}）");
                }
                options.frame_ms = value as u32;
                index += 2;
            }
            "--capture" => {
                options.capture = match take("--capture")?.as_str() {
                    "wasapi" => CaptureBackend::Wasapi,
                    "synth" => CaptureBackend::Synth,
                    other => bail!("--capture 只支持 wasapi|synth（收到 {other}）"),
                };
                index += 2;
            }
            "--device" => {
                options.device = take("--device")?;
                index += 2;
            }
            "--buffer-ms" => {
                options.buffer_ms = parse_u64(&take("--buffer-ms")?, "--buffer-ms", 1, 200)? as u32;
                index += 2;
            }
            "--cross-probe" => {
                let count = parse_u64(&take("--cross-probe")?, "--cross-probe", 0, 10_000)? as u32;
                options.cross_probe = count;
                options.need_cross_probe = count > 0;
                index += 2;
            }
            "--dir" => {
                options.dir = Some(PathBuf::from(take("--dir")?));
                index += 2;
            }
            "--json" => {
                options.json = Some(PathBuf::from(take("--json")?));
                index += 2;
            }
            "--quiet" => {
                options.quiet = true;
                index += 1;
            }
            "-h" | "--help" => return Ok(None),
            other => bail!("未知参数 {other}（--help 看用法）"),
        }
    }

    if options.mode == Mode::Device && options.peer.is_none() {
        bail!("run 模式必须给 --peer <IP[:PORT]>（或改用 --self-test）");
    }
    Ok(Some(options))
}

/// 带范围检查的整数解析。
fn parse_u64(raw: &str, flag: &str, min: u64, max: u64) -> Result<u64> {
    let value: u64 = raw
        .parse()
        .with_context(|| format!("{flag} 的取值 {raw:?} 不是合法整数"))?;
    if value < min || value > max {
        bail!("{flag} 必须在 {min}..={max} 之间，收到 {value}");
    }
    Ok(value)
}

/// 解析 `--peer`：`IP` / `IP:PORT` / 主机名。
fn resolve_peer(text: &str) -> Result<SocketAddr> {
    if let Ok(addr) = text.parse::<SocketAddr>() {
        return Ok(addr);
    }
    if let Ok(ip) = text.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_QUIC_PORT));
    }
    let (host, port) = match text.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => (
            host.to_string(),
            port.parse::<u16>().context("端口不是合法数字")?,
        ),
        _ => (text.to_string(), DEFAULT_QUIC_PORT),
    };
    let mut resolved = (host.as_str(), port)
        .to_socket_addrs()
        .with_context(|| format!("解析主机名 {host:?} 失败"))?;
    resolved
        .next()
        .ok_or_else(|| anyhow!("主机名 {host:?} 没有解析出任何地址"))
}

// ---------------------------------------------------------------------------
// 采集端点事实（Lead 口径：启动时必须打印，虚拟端点必须显式警告）
// ---------------------------------------------------------------------------

/// 采集侧事实快照（报告与 JSON 共用）。
#[derive(Debug, Clone, Default)]
struct CaptureFacts {
    backend: String,
    source_note: String,
    device_name: Option<String>,
    device_short_id: Option<String>,
    is_default: Option<bool>,
    looks_virtual: Option<bool>,
    format_text: Option<String>,
    format_unified: Option<bool>,
    period_default_ms: Option<f64>,
    period_min_ms: Option<f64>,
    buffer_frames: Option<u32>,
    requested_buffer_ms: u32,
    effective_buffer_ms: Option<f64>,
    /// 需要用户在报告里看见的警告（虚拟端点 / 非 48 kHz / 对照路径等）。
    warnings: Vec<String>,
}

/// 采集源工厂（引擎在**采集线程内**调用；`CaptureSource` 是 `!Send`，谁用谁建）。
fn capture_factory(options: &Options) -> Option<CaptureFactory> {
    match options.capture {
        CaptureBackend::Synth => {
            let buffer_ms = options.buffer_ms;
            Some(Arc::new(move || {
                SyntheticCapture::new(buffer_ms, 440.0)
                    .map(|capture| Box::new(capture) as Box<dyn CaptureSource>)
            }))
        }
        CaptureBackend::Wasapi => {
            #[cfg(windows)]
            {
                let selector = audiolink_audio::wasapi::DeviceSelector::parse(&options.device);
                let buffer_ms = options.buffer_ms;
                Some(Arc::new(move || {
                    audiolink_audio::wasapi::LoopbackCapture::open(&selector, buffer_ms)
                        .map(|capture| Box::new(capture) as Box<dyn CaptureSource>)
                }))
            }
            #[cfg(not(windows))]
            {
                let _ = options;
                None
            }
        }
    }
}

/// 空播放工厂（自检模式的「对端」用它：不出声，只接住 sink 写出时刻）。
fn null_playout_factory(buffer_ms: u32) -> PlayoutFactory {
    Arc::new(move || Ok(Box::new(NullPlayout::new(buffer_ms)) as Box<dyn PlayoutSink>))
}

/// 探测并打印采集端点事实（**真打开一次**，拿设备实际生效的缓冲与周期）。
fn probe_capture(options: &Options) -> Result<CaptureFacts> {
    let mut facts = CaptureFacts {
        backend: options.capture.as_str().to_string(),
        source_note: options.capture.source_note().to_string(),
        requested_buffer_ms: options.buffer_ms,
        ..CaptureFacts::default()
    };

    match options.capture {
        CaptureBackend::Synth => {
            let capture = SyntheticCapture::new(options.buffer_ms, 440.0)
                .map_err(|error| anyhow!("构造合成采集源失败：{error}"))?;
            let format = capture.device_format();
            facts.format_text = Some(format!(
                "{} Hz / {} ch / {:?}（合成）",
                format.sample_rate, format.channels, format.sample_format
            ));
            facts.format_unified = Some(format.is_unified());
            facts.effective_buffer_ms = Some(f64::from(capture.effective_buffer_ms()));
            facts.device_name = Some("(合成，无设备)".to_string());
            facts.warnings.push(
                "合成路径没有真实声源：它验证的是链路与工具流程，**不是**真实系统输出".to_string(),
            );
            if !format.is_unified() {
                facts
                    .warnings
                    .push("合成设备格式不是统一格式（48 kHz / 2ch / f32）".to_string());
            }
            Ok(facts)
        }
        CaptureBackend::Wasapi => {
            #[cfg(windows)]
            {
                use audiolink_audio::wasapi::{
                    DeviceSelector, LoopbackCapture, list_render_devices,
                };

                let devices =
                    list_render_devices().map_err(|error| anyhow!("枚举渲染端点失败：{error}"))?;
                if !options.quiet {
                    println!("    活动渲染端点 {} 个：", devices.len());
                    for info in &devices {
                        println!("      - {}", info.summary());
                        if info.looks_virtual() {
                            println!("          ⚠ 疑似虚拟声卡：loopback 采它可能只有静音");
                        }
                    }
                }
                let selector = DeviceSelector::parse(&options.device);
                let capture = LoopbackCapture::open(&selector, options.buffer_ms)
                    .map_err(|error| anyhow!("打开 loopback 采集失败：{error}"))?;
                let info = capture.device_info().clone();
                let (period_default, period_min) = capture.device_period_ms();
                let format = <LoopbackCapture as CaptureSource>::device_format(&capture);

                facts.device_name = Some(info.name.clone());
                facts.device_short_id = Some(info.short_id());
                facts.is_default = Some(info.is_default);
                facts.looks_virtual = Some(info.looks_virtual());
                facts.format_text = Some(info.format_text.clone());
                facts.format_unified = Some(format.is_unified());
                facts.period_default_ms = Some(period_default);
                facts.period_min_ms = Some(period_min);
                facts.buffer_frames = Some(capture.buffer_frames());
                facts.effective_buffer_ms = Some(capture.effective_buffer_ms().into());
                facts
                    .warnings
                    .extend(wasapi_warnings(&info, period_default, &format));

                if !options.quiet {
                    println!("    使用端点：{}", info.summary());
                }
                // 只是探测事实；真正的采集对象由引擎的采集线程自己建（COM 是线程绑定的）
                drop(capture);
                Ok(facts)
            }
            #[cfg(not(windows))]
            {
                bail!(
                    "--capture wasapi 需要 Windows（WASAPI loopback 是 Windows 专有）；本平台请用 --capture synth"
                );
            }
        }
    }
}

#[cfg(windows)]
fn wasapi_warnings(
    info: &audiolink_audio::wasapi::RenderDeviceInfo,
    period_default_ms: f64,
    format: &audiolink_audio::DeviceFormat,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if info.looks_virtual() {
        warnings.push(format!(
            "所选端点是**疑似虚拟声卡**（{}）：虚拟端点上空闲时 loopback 零数据（docs/10-handoff.md 坑 12），除非确实有程序在往它播放，否则本机测到的会是静音。先播一段测试音，或显式选真实端点：--device id:{}",
            info.name,
            info.short_id()
        ));
    }
    if !info.is_default {
        warnings.push(format!(
            "当前不是默认渲染端点（{}）：系统播放的声音可能走的是**另一个**端点 —— 本条只是口径提醒",
            info.name
        ));
    }
    if format.sample_rate != audiolink_audio::format::SAMPLE_RATE_HZ {
        warnings.push(format!(
            "端点混音格式不是 48 kHz（{} Hz）：本项目全链路 48 kHz 且禁止静默重采样，请到「声音设置 → 设备属性 → 高级」改成 48000 Hz",
            format.sample_rate
        ));
    }
    if period_default_ms <= 0.0 {
        warnings.push("读不到设备周期（get_device_period 返回 0）".to_string());
    }
    warnings
}

// ---------------------------------------------------------------------------
// 主流程
// ---------------------------------------------------------------------------

/// 顶层流程：探测 → 起引擎 → 连接配对 → 推流观测 → 报告。
async fn run(options: &Options) -> Result<u8> {
    println!("=== device-link —— PC 侧真机验收（真实 Engine · 真实 QUIC · 真实 §5 配对）===");
    println!(
        "模式：{} · 观测 {} s · 帧长 {} ms · 采集 {}（{}）",
        match options.mode {
            Mode::Device => "对真机推流",
            Mode::SelfTest => "同进程自检（两个真 Engine）",
        },
        options.seconds,
        options.frame_ms,
        options.capture.as_str(),
        options.capture.source_note(),
    );
    if options.mode == Mode::Device {
        println!(
            "提示：真机验收前请在 PC 上**播放一段测试音**（loopback 采的就是它），并确认手机端服务已启动、屏幕能显示 PIN"
        );
    }

    // ---- [1] 采集端点事实 ----
    println!("\n[1] 采集端点事实（本机默认渲染端点）");
    let capture = probe_capture(options)?;
    report_capture_facts(&capture);
    let silent_by_config =
        capture.looks_virtual.unwrap_or(false) && options.capture == CaptureBackend::Wasapi;

    // ---- [2] 身份与信任库 ----
    let (dir, _temp_guard) = resolve_dir(options)?;
    match options.mode {
        Mode::Device => println!("\n[2] 身份与信任库：{}（配对结果跨次留存）", dir.display()),
        Mode::SelfTest => println!("\n[2] 自检目录：{}", dir.display()),
    }

    let session = match options.mode {
        Mode::Device => run_device_session(options, &dir).await?,
        Mode::SelfTest => run_self_test_session(options, &dir).await?,
    };
    let engine = session.engine;
    let peer = session.peer;

    // ---- [3] 推流 + 观测 ----
    println!(
        "\n[3] 开流并观测 {} s（本机 {} → 对端 {}）",
        options.seconds,
        engine.info().id.short(),
        peer.short()
    );
    engine
        .start_send(peer)
        .await
        .map_err(|error| anyhow!("开流失败：{}", link_error(&error)))?;
    tokio::time::sleep(Duration::from_millis(600)).await;

    let mut events = session.events;
    let snapshots = observe(&engine, peer, session.tap.as_ref(), options, &mut events).await?;
    let tap = tap_view(session.tap.as_ref());

    // ---- [4] 停流 ----
    let _ = engine.stop_send(peer).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // ---- [5] 独立测量连接（可选，交叉验证）----
    let cross = if options.need_cross_probe {
        println!(
            "\n[4] 独立测量连接（--cross-probe {}）：与本机身份同源的 mTLS 连接，跑 §6 四时间戳探针 → {}",
            options.cross_probe, session.peer_addr
        );
        match cross_probe(
            session.peer_addr,
            options.cross_probe,
            &dir,
            &local_node_name(),
            Duration::from_secs(10),
        )
        .await
        {
            Ok(probe) => {
                report_cross_probe(&probe);
                Some(probe)
            }
            Err(error) => {
                println!("    ⚠ 独立测量连接不可用：{error:#}（该项按 [未测] 呈现）");
                None
            }
        }
    } else {
        None
    };

    // ---- [6] 报告 ----
    let report = build_report(
        options,
        &capture,
        &session.auth,
        &session.remote,
        &snapshots,
        &tap,
        cross.as_ref(),
        silent_by_config,
    );
    render_report(&report, options);

    engine.shutdown().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    write_reports(&report, options)?;
    Ok(report.verdict.code)
}

/// 一次会话的句柄集合。
struct SessionHandles {
    engine: Arc<Engine>,
    peer: NodeId,
    /// 对端套接字地址（独立测量连接要连同一个地址）。
    peer_addr: SocketAddr,
    auth: AuthPath,
    events: tokio::sync::broadcast::Receiver<EngineEvent>,
    tap: Option<Arc<MeasurementTap>>,
    remote: RemoteView,
}

/// 身份/信任库目录：`--dir` 优先；`--self-test` 无 `--dir` 时用临时目录（每次都实跑 PIN 配对）。
fn resolve_dir(options: &Options) -> Result<(PathBuf, Option<tempfile::TempDir>)> {
    if let Some(dir) = options.dir.as_ref() {
        std::fs::create_dir_all(dir).with_context(|| format!("创建目录 {} 失败", dir.display()))?;
        return Ok((dir.clone(), None));
    }
    if options.mode == Mode::SelfTest {
        let temp = tempfile::TempDir::new().context("创建临时目录失败")?;
        return Ok((temp.path().to_path_buf(), Some(temp)));
    }
    let dir = PathBuf::from(DEFAULT_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("创建目录 {} 失败", dir.display()))?;
    Ok((dir, None))
}

/// 真机模式：起一个引擎（发送端），连到对端，必要时走 PIN 配对。
async fn run_device_session(options: &Options, dir: &Path) -> Result<SessionHandles> {
    let peer_text = options.peer.clone().ok_or_else(|| anyhow!("缺少 --peer"))?;
    let peer_addr = resolve_peer(&peer_text)?;

    let tap = Arc::new(MeasurementTap::new(4096));
    let mut config = EngineConfig::new(local_node_name(), dir);
    config.listen = SocketAddr::from(([0, 0, 0, 0], 0)); // 主动连接端：临时端口即可
    config.codec.frame_ms = options.frame_ms;
    config.capture = capture_factory(options);
    config.measurement = Some(Arc::clone(&tap));

    let engine = Engine::start(config)
        .await
        .map_err(|error| anyhow!("启动引擎失败：{}", link_error(&error)))?;
    let _accept = engine.spawn_accept_loop();

    println!(
        "    本机 {}  {}  监听 {}",
        engine.info().id.short(),
        engine.info().name,
        engine.local_addr()
    );
    println!("    目标 {}（{}）", peer_addr, peer_text);

    // ---- 连接 + §5 握手 + （必要时）PIN 配对 ----
    println!("\n[3] §5 握手与 PIN 配对");
    let mut events = engine.subscribe();
    let outcome = tokio::time::timeout(Duration::from_secs(25), engine.connect(peer_addr))
        .await
        .map_err(|_| {
            anyhow!(
                "连接 {peer_addr} 超时（25 s）：对端没在监听？端口被防火墙挡了？先确认手机端服务在跑、PC 与手机同一局域网（手机 IP 会变，重连前用 adb shell ip addr 复核）"
            )
        })?;

    let (auth, remote) = match outcome {
        Ok(peer) => {
            println!("    已配对（信任库命中）→ 走 AUTH_CHALLENGE/AUTH_RESPONSE 分支");
            let remote = remote_view(&engine, peer);
            (AuthPath::Trusted, remote)
        }
        Err(error) if error.code() == ErrorCode::NotPaired => {
            println!("    对端不认识本机 → 进入 PIN 配对分支（PIN 显示在手机屏幕上）");
            let peer = first_peer(&engine)
                .ok_or_else(|| anyhow!("对端要求 PIN，但本机侧没有建立任何会话（会话表为空）"))?;

            // PIN 的三条来源，按「确定性」排序：--pin（已知）→ --pin-file（无头异步投喂）→ stdin（有人守着终端）。
            // 无头环境下 --pin-file 是**唯一可行**的那条：PIN 由手机屏幕显示、PC 端输入，
            // 而工具必须先在线上手机才显示 PIN —— 后台作业没法在正确时刻往 stdin 打字。
            let pin = if let Some(pin) = options.pin.clone() {
                println!("    使用 --pin 提供的 PIN");
                pin
            } else if let Some(path) = options.pin_file.clone() {
                wait_for_pin_file(&path, Duration::from_secs(60)).await?
            } else if std::io::stdin().is_terminal() {
                print!("    请输入手机屏幕上的 6 位 PIN：");
                let mut line = String::new();
                std::io::stdin()
                    .read_line(&mut line)
                    .context("读取 PIN 失败")?;
                let pin = line.trim().to_string();
                if pin.len() != 6 || !pin.chars().all(|c| c.is_ascii_digit()) {
                    bail!("PIN 必须是 6 位数字（收到 {pin:?}）");
                }
                pin
            } else {
                bail!(
                    "对端要求 PIN 配对，但既没给 --pin 也没给 --pin-file，且当前不是交互终端：无头环境请用 --pin-file <path>（连接期间随时把手机屏幕上的 6 位 PIN 写进该文件）"
                )
            };

            drain_info_events(&mut events);
            engine
                .submit_pin(peer, &pin)
                .await
                .map_err(|error| anyhow!("提交 PIN 失败：{}", link_error(&error)))?;

            // 等「进入 Streaming」或「被拒」——**先等到哪个算哪个**。
            // 为什么不能只等 Streaming：PIN 错时对端会立刻回 PAIR_RESULT(ok=false)，
            // 而会话状态永远停在 Handshaking，如果只等状态机就会在 10 s 后报「超时」，
            // 白白把「PIN 错了，还剩 4 次」这个真正有用的原因丢掉。
            match wait_pair_outcome(&engine, peer, &mut events, Duration::from_secs(10)).await {
                PairOutcome::Streaming => println!("    PIN 校验通过，双方已写入信任库"),
                PairOutcome::Rejected(reason) => bail!(
                    "PIN 配对失败：{reason}（PIN 错会消耗尝试次数，5 次后锁定 5 分钟；请重新看手机屏幕上的 PIN —— 注意 PIN 是**每连接新建**的，重连后会换一个新的）"
                ),
                PairOutcome::Timeout => bail!(
                    "提交 PIN 后 10 s 内既没进入 Streaming 也没收到配对结果：对端可能已经走了，或握手卡住"
                ),
            }
            let remote = remote_view(&engine, peer);
            (AuthPath::Pin { used: true }, remote)
        }
        Err(error) => return Err(anyhow!("连接 {} 失败：{}", peer_addr, link_error(&error))),
    };

    Ok(SessionHandles {
        engine,
        peer: remote.id,
        peer_addr,
        auth,
        events,
        tap: Some(tap),
        remote,
    })
}

/// 自检模式：同进程两个真 Engine，PIN 自动回填（不需要人看屏幕）。
async fn run_self_test_session(options: &Options, dir: &Path) -> Result<SessionHandles> {
    let dir_a = dir.join("node-a");
    let dir_b = dir.join("node-b");
    std::fs::create_dir_all(&dir_a).context("创建 node-a 目录失败")?;
    std::fs::create_dir_all(&dir_b).context("创建 node-b 目录失败")?;

    // 同一个探针给两端：自检里「发送侧封口」与「接收侧写出」共享同一个进程级时钟，
    // 因此这一对样本是**真实测**（跨机时它不成立，见文件头）。
    let tap = Arc::new(MeasurementTap::new(16_384));

    let mut config_a = EngineConfig::new("device-link-a", &dir_a);
    config_a.listen = SocketAddr::from(([127, 0, 0, 1], 0));
    config_a.codec.frame_ms = options.frame_ms;
    config_a.capture = capture_factory(options);
    config_a.measurement = Some(Arc::clone(&tap));

    let mut config_b = EngineConfig::new("device-link-b", &dir_b);
    config_b.listen = SocketAddr::from(([127, 0, 0, 1], options.listen_port));
    config_b.codec.frame_ms = options.frame_ms;
    config_b.playout = Some(null_playout_factory(options.buffer_ms.max(20) * 3));
    config_b.measurement = Some(Arc::clone(&tap));

    let engine_a = Engine::start(config_a)
        .await
        .map_err(|error| anyhow!("启动 node-a 失败：{}", link_error(&error)))?;
    let engine_b = Engine::start(config_b)
        .await
        .map_err(|error| anyhow!("启动 node-b 失败：{}", link_error(&error)))?;
    let _accept_a = engine_a.spawn_accept_loop();
    let _accept_b = engine_b.spawn_accept_loop();

    let addr_b = engine_b.local_addr();
    println!(
        "    node-a {}（发送）  node-b {}（接收 / NullPlayout，不出声）  目标 {addr_b}",
        engine_a.info().id.short(),
        engine_b.info().id.short()
    );

    println!("\n[3] §5 握手与 PIN 配对（自动回填）");
    let mut events_a = engine_a.subscribe();
    let mut events_b = engine_b.subscribe();

    let auth = match engine_a.connect(addr_b).await {
        Ok(_) => {
            println!("    两端已在同一信任库中 → 走 AUTH_CHALLENGE/AUTH_RESPONSE 分支");
            AuthPath::Trusted
        }
        Err(error) if error.code() == ErrorCode::NotPaired => {
            println!("    node-b 不认识 node-a → 实跑 PIN 配对流程");
            let pin = wait_for_pin(&mut events_b, Duration::from_secs(5)).await?;
            println!("    node-b 展示 PIN：{pin}（自检自动回填）");
            let peer = first_peer(&engine_a)
                .ok_or_else(|| anyhow!("node-a 侧没有建立会话（会话表为空）"))?;
            engine_a
                .submit_pin(peer, &pin)
                .await
                .map_err(|error| anyhow!("提交 PIN 失败：{}", link_error(&error)))?;
            wait_for_streaming(&engine_a, peer, Duration::from_secs(10)).await?;
            println!("    PIN 校验通过，双方已写入信任库");
            AuthPath::Pin { used: true }
        }
        Err(error) => return Err(anyhow!("node-a 连接 node-b 失败：{}", link_error(&error))),
    };

    let peer_on_a = first_peer(&engine_a).ok_or_else(|| anyhow!("node-a 侧没有会话"))?;
    let _ = drain_pair_events(&mut events_a);
    let remote = remote_view(&engine_a, peer_on_a);
    Ok(SessionHandles {
        engine: engine_a,
        peer: peer_on_a,
        peer_addr: addr_b,
        auth,
        events: events_a,
        tap: Some(tap),
        remote,
    })
}

/// 认证路径（报告里要能一眼看出这次是「已配对直连」还是「现配对」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthPath {
    /// 信任库命中：`AUTH_CHALLENGE/AUTH_RESPONSE`。
    Trusted,
    /// 本次运行实跑了 PIN 配对。
    Pin {
        /// 是否真的提交过 PIN。
        used: bool,
    },
}

impl AuthPath {
    const fn name(self) -> &'static str {
        match self {
            Self::Trusted => "trusted (AUTH_CHALLENGE/RESPONSE)",
            Self::Pin { .. } => "pin (PAIR_REQUIRED/PAIR_SUBMIT)",
        }
    }
}

/// 对端视图。
#[derive(Debug, Clone)]
struct RemoteView {
    id: NodeId,
    name: String,
    addr: String,
    trusted: bool,
}

fn remote_view(engine: &Arc<Engine>, peer: NodeId) -> RemoteView {
    match engine.peers().into_iter().find(|status| status.id == peer) {
        Some(status) => RemoteView {
            id: peer,
            name: status.name,
            addr: status.addr.to_string(),
            trusted: status.trusted,
        },
        None => RemoteView {
            id: peer,
            name: "unknown".to_string(),
            addr: "-".to_string(),
            trusted: false,
        },
    }
}

fn local_node_name() -> String {
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "pc".to_string());
    format!("pc-{host}")
}

fn first_peer(engine: &Arc<Engine>) -> Option<NodeId> {
    engine.peers().into_iter().next().map(|peer| peer.id)
}

/// 从文本里取 PIN：第一个**恰好 6 位**的连续数字串。
///
/// 为什么不用严格的「全文等于 6 位」：投喂方往往写成 `123456` 换行、`PIN: 123456` 甚至带时间戳注释；
/// 只要出现一个恰好 6 位的数字串就取它（更短或更长的数字串不会被误取，例如 8 位时间戳）。
fn extract_pin(text: &str) -> Option<String> {
    let mut run = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            run.push(ch);
        } else {
            if run.len() == 6 {
                return Some(run);
            }
            run.clear();
        }
    }
    if run.len() == 6 { Some(run) } else { None }
}

/// 轮询 PIN 文件直到读到 6 位数字（无头/无人值守的主路径，见 `--pin-file` 的说明）。
///
/// 上限 60 s 与 §5 的 PIN 有效期一致：再等下去手机上的那个 PIN 也过期了，继续等没有意义。
async fn wait_for_pin_file(path: &Path, timeout: Duration) -> Result<String> {
    println!(
        "    等待 PIN 文件 {}：每 300 ms 轮询一次，上限 {} s（PIN 本身只有 60 s 有效期）",
        path.display(),
        timeout.as_secs()
    );
    let started = Instant::now();
    let mut last_note = 0_u64;
    loop {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                if let Some(pin) = extract_pin(&text) {
                    println!("    从 PIN 文件读到 PIN：{pin}");
                    return Ok(pin);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => println!("    ⚠ 读 PIN 文件失败：{error}（继续轮询）"),
        }

        let waited = started.elapsed();
        if waited >= timeout {
            bail!(
                "等待 PIN 文件 {} 超时（{} s）：手机上的 PIN 已过期，请重新在手机上触发配对后用 --pin-file 重投",
                path.display(),
                timeout.as_secs()
            );
        }
        if waited.as_secs() >= last_note + 5 {
            last_note = waited.as_secs();
            println!(
                "    仍在等待 PIN 文件…（已等 {} s / 上限 {} s）",
                waited.as_secs(),
                timeout.as_secs()
            );
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// 等对端展示 PIN（响应方视角）。
async fn wait_for_pin(
    events: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
    timeout: Duration,
) -> Result<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!("等待对端展示 PIN 超时（§5 配对没走到 PAIR_REQUIRED？）");
        }
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(EngineEvent::DisplayPin { pin, .. })) => return Ok(pin),
            Ok(Ok(_)) => continue,
            Ok(Err(error)) => bail!("事件订阅中断：{error}"),
            Err(_) => bail!("等待对端展示 PIN 超时"),
        }
    }
}

/// 等会话进入 `Streaming`（= 握手完成、会话可用）。
async fn wait_for_streaming(engine: &Arc<Engine>, peer: NodeId, timeout: Duration) -> Result<()> {
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
            bail!(
                "等待会话进入 Streaming 超时（PIN 不对 / 对端握手失败？用 --pin 复核手机屏幕上的 6 位数字）"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// 提交 PIN 之后的两种结局（决定报什么错）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum PairOutcome {
    /// 会话进入 `Streaming`：握手完成、配对成功。
    Streaming,
    /// 对端明确回绝（`PAIR_RESULT{ok:false}`）：reason 是对端给的原文（含剩余尝试次数）。
    Rejected(String),
    /// 两边都没等到：超时。
    Timeout,
}

/// 等「配对成功」或「被回绝」，先到哪个算哪个（见调用点的注释：只等状态机会丢掉真正的原因）。
async fn wait_pair_outcome(
    engine: &Arc<Engine>,
    peer: NodeId,
    events: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
    timeout: Duration,
) -> PairOutcome {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if engine
            .peers()
            .into_iter()
            .any(|status| status.id == peer && status.state == SessionState::Streaming)
        {
            return PairOutcome::Streaming;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return PairOutcome::Timeout;
        }
        let slice = remaining.min(Duration::from_millis(200));
        match tokio::time::timeout(slice, events.recv()).await {
            // 成功事件不提前返回：还要等状态机真的进 Streaming（否则会把「已回 PAIR_RESULT 但尚未迁移」当成完成）
            Ok(Ok(EngineEvent::PairCompleted {
                ok: false, reason, ..
            })) => {
                return PairOutcome::Rejected(reason);
            }
            Ok(Ok(_)) | Err(_) => {}
            Ok(Err(_)) => return PairOutcome::Timeout,
        }
    }
}

/// 排干并打印信息类事件（`PinNeeded` / `DisplayPin` / `Error`），不消费 `PairCompleted`。
///
/// 为什么单独一个函数：`submit_pin` 之前先把已排队的 `PinNeeded` 打印掉，日志顺序才与真实时序一致
/// （否则「提示需要 PIN」会晚于「PIN 校验通过」出现，读日志的人会以为流程错了）。
fn drain_info_events(events: &mut tokio::sync::broadcast::Receiver<EngineEvent>) {
    let mut seen = 0;
    while let Ok(event) = events.try_recv() {
        match event {
            EngineEvent::DisplayPin { pin, .. } => {
                println!("    展示过 PIN：{pin}");
                seen += 1;
            }
            EngineEvent::PinNeeded { name, .. } => {
                println!("    提示需要 PIN（对端 {name}）");
                seen += 1;
            }
            EngineEvent::Error { code, context } => {
                println!("    会话错误 {code}：{context}");
                seen += 1;
            }
            _ => {}
        }
        if seen >= 16 {
            break;
        }
    }
}

/// 排干配对相关事件，返回 `PairCompleted` 的 `(ok, reason)`。
fn drain_pair_events(
    events: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
) -> Option<(bool, String)> {
    let mut outcome = None;
    let mut seen = 0;
    while let Ok(event) = events.try_recv() {
        match event {
            EngineEvent::PairCompleted { ok, reason, .. } => {
                outcome = Some((ok, reason));
                seen += 1;
            }
            EngineEvent::DisplayPin { pin, .. } => {
                println!("    展示过 PIN：{pin}");
                seen += 1;
            }
            EngineEvent::PinNeeded { name, .. } => {
                println!("    提示需要 PIN（对端 {name}）");
                seen += 1;
            }
            EngineEvent::Error { code, context } => {
                println!("    会话错误 {code}：{context}");
                seen += 1;
            }
            _ => {}
        }
        if seen >= 16 {
            break;
        }
    }
    outcome
}

// ---------------------------------------------------------------------------
// 1 Hz 观测
// ---------------------------------------------------------------------------

/// 单次 1 Hz 采样。
#[derive(Debug, Clone, Default)]
struct Snapshot {
    at_s: f64,
    /// 本机聚合视图（1 Hz）。
    local: Option<StreamStats>,
    /// 对端发来的 1 Hz `STREAM_STATS`（None = 从来没收到）。
    peer: Option<StreamStats>,
    /// §6 时钟估计（None = 未收敛 / 样本 < 8）。
    clock: Option<ClockView>,
    /// §6 探针计数。
    probes: Option<ProbeView>,
    /// 对端会话状态。
    state: Option<String>,
    /// 本机探针：已封口但未配对的帧数（跨机时没有本地接收侧，因此它就是「已封口帧数」）。
    sealed_orphans: usize,
    /// 本机探针：已配对的样本数（仅自检模式会增长）。
    matched: usize,
}

/// §6 时钟估计视图（`ClockEstimator` 的当前估计）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct ClockView {
    offset_us: i64,
    drift_ppm: i32,
    rtt_us: u64,
    quality: &'static str,
    samples: usize,
}

/// §6 探针计数视图（引擎会话内的探测环）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct ProbeView {
    sent: u64,
    received: u64,
    replies: u64,
    unmatched: u64,
    rtt_p50_us: Option<u32>,
    rtt_p95_us: Option<u32>,
    rtt_samples: u32,
}

/// 读一次引擎侧的冻结 API（**T1 触点**：所有对 `clock_estimate` / `peer_stats` /
/// `clock_probe_stats` 的依赖都收在这三个函数里，形状一变只改这里）。
fn clock_view(engine: &Arc<Engine>, peer: NodeId) -> Option<ClockView> {
    let estimate = engine.clock_estimate(peer)?;
    Some(ClockView {
        offset_us: estimate.offset_us,
        drift_ppm: estimate.drift_ppm,
        rtt_us: estimate.rtt_us,
        quality: estimate.quality.name(),
        samples: estimate.samples,
    })
}

fn probe_view(engine: &Arc<Engine>, peer: NodeId) -> Option<ProbeView> {
    let stats: ClockProbeStats = engine.clock_probe_stats(peer)?;
    Some(ProbeView {
        sent: stats.sent,
        received: stats.received,
        replies: stats.replies,
        unmatched: stats.unmatched,
        rtt_p50_us: stats.rtt_p50_us,
        rtt_p95_us: stats.rtt_p95_us,
        rtt_samples: stats.rtt_samples,
    })
}

fn peer_view(engine: &Arc<Engine>, peer: NodeId) -> Option<StreamStats> {
    engine.peer_stats(peer)
}

/// 1 Hz 采样循环；会话掉线立即返回错误（静默停止是禁止的）。
async fn observe(
    engine: &Arc<Engine>,
    peer: NodeId,
    tap: Option<&Arc<MeasurementTap>>,
    options: &Options,
    events: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
) -> Result<Vec<Snapshot>> {
    let started = Instant::now();
    let total = Duration::from_secs(options.seconds);
    let mut snapshots = Vec::with_capacity(options.seconds as usize + 1);
    let mut next_tick = Duration::from_secs(1);
    let mut last_progress = 0_u64;

    loop {
        let elapsed = started.elapsed();
        if elapsed >= total {
            break;
        }
        let wait = next_tick
            .saturating_sub(elapsed)
            .max(Duration::from_millis(10))
            .min(total.saturating_sub(elapsed));
        tokio::time::sleep(wait).await;

        // 事件先排干：致命事件必须立刻可见（不许静默返回漂亮数字）
        while let Ok(event) = events.try_recv() {
            match event {
                EngineEvent::PeerDisconnected { reason, .. } => {
                    bail!("对端在观测期间断开：{reason}（会话已中断，本次观测不成立）");
                }
                EngineEvent::Error { code, context } => {
                    println!("    ⚠ 会话错误 {code}：{context}");
                }
                _ => {}
            }
        }

        let snapshot = Snapshot {
            at_s: started.elapsed().as_secs_f64(),
            local: engine.telemetry(peer),
            peer: peer_view(engine, peer),
            clock: clock_view(engine, peer),
            probes: probe_view(engine, peer),
            state: engine
                .peers()
                .into_iter()
                .find(|status| status.id == peer)
                .map(|status| status.state.name().to_string()),
            sealed_orphans: tap.map_or(0, |tap| tap.orphan_counts().0),
            matched: tap.map_or(0, |tap| tap.matched()),
        };

        if !options.quiet {
            let second = snapshot.at_s as u64;
            if second > last_progress {
                last_progress = second;
                let clock = snapshot.clock.map_or("未收敛".to_string(), |clock| {
                    format!("offset {} µs / n={}", clock.offset_us, clock.samples)
                });
                let peer_note = snapshot.peer.map_or("无".to_string(), |stats| {
                    format!(
                        "水位 {:.1} ms / 欠载 {} / 迟到 {} / 丢包 {:.2}%",
                        f64::from(stats.buffer_level_us) / 1000.0,
                        stats.underruns,
                        stats.late_drops,
                        f64::from(stats.loss_pct_x100) / 100.0
                    )
                });
                let probe_note = match options.mode {
                    Mode::SelfTest => format!(
                        "  同进程配对 {} 样本 / 发送侧孤儿 {}",
                        snapshot.matched, snapshot.sealed_orphans
                    ),
                    Mode::Device => format!("  已封口 {} 帧", snapshot.sealed_orphans),
                };
                println!(
                    "    t={second:>3}s  对端 STREAM_STATS: {peer_note}   §6: {clock}{probe_note}   {}",
                    snapshot.state.clone().unwrap_or_else(|| "?".to_string())
                );
            }
        }

        snapshots.push(snapshot);
        next_tick += Duration::from_secs(1);
    }

    Ok(snapshots)
}

// ---------------------------------------------------------------------------
// 报告（实测 / 模型 / 未测 三态）
// ---------------------------------------------------------------------------

/// 一行账本（文本与 JSON 共用，保证两边永远一致）。
#[derive(Debug, Clone)]
struct Row {
    section: &'static str,
    item: String,
    label: &'static str,
    value: String,
    note: String,
}

impl Row {
    fn measured(
        section: &'static str,
        item: impl Into<String>,
        value: impl Into<String>,
        note: impl Into<String>,
    ) -> Self {
        Self {
            section,
            item: item.into(),
            label: "实测",
            value: value.into(),
            note: note.into(),
        }
    }

    fn modeled(
        section: &'static str,
        item: impl Into<String>,
        value: impl Into<String>,
        note: impl Into<String>,
    ) -> Self {
        Self {
            section,
            item: item.into(),
            label: "模型",
            value: value.into(),
            note: note.into(),
        }
    }

    fn unmeasured(section: &'static str, item: impl Into<String>, note: impl Into<String>) -> Self {
        Self {
            section,
            item: item.into(),
            label: "未测",
            value: "—".to_string(),
            note: note.into(),
        }
    }
}

/// 本机探针视图（自检模式下才可能配对上样本；跨机时只有发送侧记录）。
#[derive(Debug, Clone, Default)]
struct TapView {
    matched: usize,
    sealed_orphans: usize,
    played_orphans: usize,
    /// 「封口 → sink 写出」的分位摘要（仅同进程自检配得上对；跨机必为 None）。
    summary: Option<Summary>,
}

/// 账本 + 判定。
#[derive(Debug, Clone, Default)]
struct Report {
    mode: &'static str,
    started_unix: u64,
    rows: Vec<Row>,
    verdict: Verdict,
    /// 结构化报告（JSON）。
    data: Value,
}

/// 判定结果。
#[derive(Debug, Clone, Default)]
struct Verdict {
    code: u8,
    ok: bool,
    reasons: Vec<String>,
    warnings: Vec<String>,
}

/// 汇总一个 µs 样本集。
fn summary_ms(summary: Option<Summary>) -> String {
    match summary {
        Some(summary) => format!(
            "P50 {:.2} ms / P95 {:.2} ms / P99 {:.2} ms / max {:.2} ms（n={}）",
            f64::from(summary.p50) / 1000.0,
            f64::from(summary.p95) / 1000.0,
            f64::from(summary.p99) / 1000.0,
            f64::from(summary.max) / 1000.0,
            summary.count
        ),
        None => "无样本".to_string(),
    }
}

/// 最近秩百分位（与 `audiolink_tools::ClockSamples` 的口径一致，用于单程样本）。
fn percentile_i64(values: &[i64], pct: u8) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = (usize::from(pct.min(100)) * sorted.len())
        .div_ceil(100)
        .max(1);
    sorted.get(rank - 1).copied()
}

/// f64 的最近秩百分位（水位等）。
fn percentile_f64(values: &[f64], pct: u8) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = (usize::from(pct.min(100)) * sorted.len())
        .div_ceil(100)
        .max(1);
    sorted.get(rank - 1).copied()
}

fn us_to_ms(value: u64) -> f64 {
    value as f64 / 1000.0
}

/// 构建账本。
#[allow(clippy::too_many_arguments)]
fn build_report(
    options: &Options,
    capture: &CaptureFacts,
    auth: &AuthPath,
    remote: &RemoteView,
    snapshots: &[Snapshot],
    tap: &TapView,
    cross: Option<&CrossProbe>,
    silent_by_config: bool,
) -> Report {
    let mut rows = Vec::new();
    let mut verdict = Verdict::default();
    let mode = match options.mode {
        Mode::Device => "device",
        Mode::SelfTest => "self-test",
    };

    // ---- 采集端点事实 ----
    rows.push(Row::measured(
        "PC 采集端点",
        "端点",
        capture
            .device_name
            .clone()
            .unwrap_or_else(|| "-".to_string()),
        format!(
            "{} · {} · 默认端点={} · 疑似虚拟={}",
            capture.backend,
            capture.source_note,
            capture
                .is_default
                .map_or("?".to_string(), |value| value.to_string()),
            capture
                .looks_virtual
                .map_or("?".to_string(), |value| value.to_string()),
        ),
    ));
    rows.push(Row::measured(
        "PC 采集端点",
        "设备格式与缓冲",
        capture
            .format_text
            .clone()
            .unwrap_or_else(|| "-".to_string()),
        format!(
            "统一格式（48 kHz/2ch/f32）={} · 缓冲请求 {} ms → 实际 {} ms（{} 帧）",
            capture
                .format_unified
                .map_or("?".to_string(), |value| value.to_string()),
            capture.requested_buffer_ms,
            capture
                .effective_buffer_ms
                .map_or("-".to_string(), |value| format!("{value:.1}")),
            capture
                .buffer_frames
                .map_or("-".to_string(), |value| value.to_string()),
        ),
    ));
    if let (Some(default), Some(min)) = (capture.period_default_ms, capture.period_min_ms) {
        rows.push(Row::measured(
            "PC 采集端点",
            "设备周期",
            format!("默认 {default:.1} ms / 最小 {min:.1} ms"),
            "事件驱动下读出粒度 = 周期（不是缓冲大小）；共享模式缓冲下限实测 22 ms".to_string(),
        ));
    }
    for warning in &capture.warnings {
        verdict.warnings.push(warning.clone());
    }

    // ---- PC 侧：编码耗时（工具内独立探针）----
    match measure_encode_probe(options) {
        Ok(summary) => rows.push(Row::measured(
            "PC 本机",
            "编码耗时（Opus encode_into）",
            summary_ms(summary),
            format!(
                "工具内独立 OpusEncoder（同 crate / 同参数：{} ms / 160 kbps / complexity 7 / VBR / in-band FEC 关）；\
                 **不是**引擎采集线程里的同一次调用 —— 引擎当前不导出逐帧编码耗时",
                options.frame_ms
            ),
        )),
        Err(error) => rows.push(Row::unmeasured(
            "PC 本机",
            "编码耗时（Opus encode_into）",
            format!("构造编码探针失败：{error:#}"),
        )),
    }

    // ---- PC 侧：帧封口 ----
    // 口径：探针把「封口」与「写出」按序号配对，配上的进 summary、配不上的留成孤儿。
    // 因此**已封口总数 = 已配对 + 发送侧孤儿** —— 只读孤儿会在自检模式里给出一个假数字（看起来像 2 帧）。
    let sealed_total = tap.matched + tap.sealed_orphans;
    rows.push(Row::measured(
        "PC 本机",
        "已封口帧数（MeasurementTap）",
        format!(
            "{sealed_total} 帧（已配对 {} + 发送侧孤儿 {} + 接收侧孤儿 {}）",
            tap.matched, tap.sealed_orphans, tap.played_orphans
        ),
        "= 已配对样本 + 未配对孤儿。跨机时没有本地接收侧，因此全部落在发送侧孤儿里；         接收侧孤儿长期非 0 说明序号配对有问题（数字看着正常但其实是错的）"
            .to_string(),
    ));
    rows.push(Row::unmeasured(
        "PC 本机",
        "封口 → 数据报发出",
        "引擎不导出「发出时刻」，跨机又不能拿本机 Instant 相减；\
         同口径替代证据见 --self-test 的「封口 → sink 写出」（同一段软件栈）"
            .to_string(),
    ));

    // ---- 模型段 ----
    rows.push(Row::modeled(
        "PC 模型段",
        "采集半周期",
        format!("{:.1} ms", f64::from(options.frame_ms) / 2.0),
        format!("= frame_ms / 2（frame_ms = {}）", options.frame_ms),
    ));
    rows.push(Row::modeled(
        "PC 模型段",
        "组帧",
        format!("{:.1} ms", f64::from(options.frame_ms)),
        "= frame_ms（攒满一帧才封口）".to_string(),
    ));

    // ---- §6 时钟同步（会话内，T1 冻结 API）----
    let clock_last = snapshots.iter().rev().find_map(|snapshot| snapshot.clock);
    let first_clock_at = snapshots
        .iter()
        .find(|snapshot| snapshot.clock.is_some())
        .map(|snapshot| snapshot.at_s);
    match clock_last {
        Some(clock) => {
            rows.push(Row::measured(
                "跨机 · §6 时钟",
                "offset（对端 − 本机）",
                format!(
                    "{} µs（{:.3} ms）",
                    clock.offset_us,
                    us_to_ms(clock.offset_us.unsigned_abs())
                ),
                format!(
                    "§6 best-8 offset 中位数 · 收敛于第 {} s。**绝对值只反映两端单调时钟的原点之差**（进程/开机时刻不同它就会不同，真机上是「手机已开机多久 − PC 已开机多久」量级）；有意义的是它的**稳定性**（§6 要求极差 ≤ 2 ms）与后续 e2e 换算的一致性",
                    first_clock_at.map_or("-".to_string(), |value| format!("{value:.0}"))
                ),
            ));
            rows.push(Row::measured(
                "跨机 · §6 时钟",
                "漂移 / 质量 / 样本",
                format!(
                    "{} ppm / {} / {} 样本",
                    clock.drift_ppm, clock.quality, clock.samples
                ),
                if clock.samples < 200 {
 "**drift_ppm 此刻不可信**：§6 的漂移 = 200 样本窗口内的线性回归斜率，窗口没满时 µs 级抖动会被 ppm 放大成噪声（本机回环实测 8 样本窗口给过 -57 ppm）。稳态跑够再读这一列。 质量分级 §6.5：Good = 样本 ≥ 8 且代表 RTT ≤ 5 ms；Fair ≤ 20 ms；其余 Poor"
                        .to_string()
                } else {
 "漂移 = 窗口内 (t_mid, offset) 的线性回归斜率（§6 第 4 条）； 质量分级 §6.5：Good = 样本 ≥ 8 且代表 RTT ≤ 5 ms；Fair ≤ 20 ms；其余 Poor"
                        .to_string()
                },
            ));
            rows.push(Row::measured(
                "跨机 · §6 时钟",
                "§6 代表 RTT",
                format!("{:.2} ms", us_to_ms(clock.rtt_us)),
                "= RTT 最小的 8 个样本里最大的那个 RTT（§6 口径；不是单次探针 RTT）".to_string(),
            ));
        }
        None => {
            rows.push(Row::unmeasured(
                "跨机 · §6 时钟",
                "offset / drift / quality / samples",
                "全程未收敛（clock_estimate 一直是 None = 样本 < 8）：offset 一直为 0 只说明**没测到**，\
                 不能当「已同步」。可能是对端没在会话内跑 §6 探针（APK 未含时钟同步？），或探针全丢"
                    .to_string(),
            ));
            verdict
                .reasons
                .push("§6 时钟未收敛：offset 全程为 None（未测），跨机延迟账本不成立".to_string());
            verdict.code = EXIT_NO_CLOCK;
        }
    }

    // ---- §6 探针计数 + 分布（A：会话内）----
    let probe_last = snapshots.iter().rev().find_map(|snapshot| snapshot.probes);
    match probe_last {
        Some(probes) => {
            let pending = probes.sent.saturating_sub(probes.replies);
            rows.push(Row::measured(
                "跨机 · §6 探针",
                "计数 sent / received / replies / unmatched",
                format!(
                    "{} / {} / {} / {}（未决 = sent − replies = {}）",
                    probes.sent, probes.received, probes.replies, probes.unmatched, pending
                ),
                "sent = 本机发出的 CLOCK_PROBE；received = 本机作为响应方收到的探针；\
                 replies = 配上的 REPLY（每 1 个 = 1 个四时间戳样本）；unmatched = **结构合法但配不上**的 REPLY。\
                 注意 rtt_samples 只数有效样本（负 RTT 的异常样本计入 replies 但不入 RTT 环），因此它可能 < replies；\
                 解不出来的 CLOCK_REPLY 走数据报级「忽略并计数、不断流」，不计进 unmatched"
                    .to_string(),
            ));
            match (probes.rtt_p50_us, probes.rtt_p95_us) {
                (Some(p50), Some(p95)) => rows.push(Row::measured(
                    "跨机 · §6 探针",
                    "逐探针 RTT P50 / P95",
                    format!(
                        "{:.2} ms / {:.2} ms（n={}）",
                        us_to_ms(u64::from(p50)),
                        us_to_ms(u64::from(p95)),
                        probes.rtt_samples
                    ),
                    "会话内探针环（与音频走同一条 QUIC 连接 / 同一条路径）—— RTT 分位的主来源"
                        .to_string(),
                )),
                _ => rows.push(Row::unmeasured(
                    "跨机 · §6 探针",
                    "逐探针 RTT P50 / P95",
                    format!(
                        "引擎未给出分位（rtt_samples = {}；样本 < 8 时为 None）—— 不填 0",
                        probes.rtt_samples
                    ),
                )),
            }
        }
        None => rows.push(Row::unmeasured(
            "跨机 · §6 探针",
            "计数与 RTT 分位",
            "引擎未给出探针计数（对端不存在或会话未建立）".to_string(),
        )),
    }

    // ---- 独立测量连接（B：交叉验证）----
    match cross {
        Some(probe) => {
            rows.push(Row::measured(
                "跨机 · 独立测量连接",
                "逐探针 RTT P50 / P95",
                if probe.replies == 0 {
                    "—".to_string()
                } else {
                    format!(
                        "{} / {}（n={}）",
                        probe
                            .rtt_p50_us
                            .map_or("?".to_string(), |value| format!("{:.2} ms", us_to_ms(value))),
                        probe
                            .rtt_p95_us
                            .map_or("?".to_string(), |value| format!("{:.2} ms", us_to_ms(value))),
                        probe.samples
                    )
                },
                format!(
                    "**独立测量连接**（与音频会话同路径、不同 QUIC 连接；mTLS，本机身份）· sent {} / replies {} / 超时 {} / 异常 {} / 首探针预热重发 {} · {}",
                    probe.sent, probe.replies, probe.timeouts, probe.anomalies, probe.warmup_retries, probe.note
                ),
            ));
            match (probe.one_way_p50_us, probe.one_way_p95_us) {
                (Some(p50), Some(p95)) => rows.push(Row::measured(
                    "跨机 · 独立测量连接",
                    "网络单向 = t2 − t1 − offset",
                    format!(
                        "P50 {} / P95 {}（min {} / max {}）",
                        signed_ms(p50),
                        signed_ms(p95),
                        probe.one_way_min_us.map_or("-".to_string(), signed_ms),
                        probe.one_way_max_us.map_or("-".to_string(), signed_ms)
                    ),
                    "offset 取自**同一条连接**的 §6 best-8 中位数；单向值含**对称路径假设**（该假设不成立时，\
                     单向误差 = 非对称量的一半）"
                        .to_string(),
                )),
                _ => rows.push(Row::unmeasured(
                    "跨机 · 独立测量连接",
                    "网络单向 = t2 − t1 − offset",
                    format!("没有可用的四时间戳样本（replies = {}）", probe.replies),
                )),
            }
        }
        None => {
            rows.push(Row::unmeasured(
                "跨机 · 独立测量连接",
                "逐探针 RTT 与单向（交叉验证）",
                "未启用（--cross-probe 0）或连接不可用".to_string(),
            ));
        }
    }

    // ---- 本机 1 Hz 视图（RTT / 抖动）----
    let mut local_rtt = SampleStats::new(4096);
    let mut local_jitter = SampleStats::new(4096);
    for snapshot in snapshots {
        if let Some(stats) = snapshot.local {
            local_rtt.push(stats.rtt_us);
            local_jitter.push(stats.jitter_us);
        }
    }
    rows.push(Row::measured(
        "跨机 · 本机视图",
        "RTT（1 Hz 采样）",
        summary_ms(local_rtt.summary()),
        "口径：QUIC 平滑 RTT（StreamStats.rtt_us）。§6 的数字只走 clock_estimate / ClockProbeStats，两者不要混读；逐探针分位见上面「§6 探针」一节"
            .to_string(),
    ));
    rows.push(Row::measured(
        "跨机 · 本机视图",
        "抖动 P50（1 Hz 采样）",
        summary_ms(local_jitter.summary()),
        "到达间隔抖动 |实际 − 标称帧长|；发送侧不接音频，因此这一列通常是 0（真正的抖动在接收侧）"
            .to_string(),
    ));

    // ---- 对端 1 Hz STREAM_STATS ----
    let peer_samples: Vec<StreamStats> = snapshots
        .iter()
        .filter_map(|snapshot| snapshot.peer)
        .collect();
    let mut peer_stats_data = Value::Null;
    if peer_samples.is_empty() {
        rows.push(Row::unmeasured(
            "对端 · 1 Hz STREAM_STATS",
            "buffer_level_us / underruns / late_drops / bitrate / loss",
            "全程**一份都没收到**：对端没有发出 STREAM_STATS（对端服务没跑？会话没进 Streaming？）"
                .to_string(),
        ));
        verdict
            .reasons
            .push("对端 1 Hz STREAM_STATS 全程未收到 —— 无法证明对端在收流/出声".to_string());
        if verdict.code != EXIT_NO_CLOCK {
            verdict.code = EXIT_NO_PEER;
        }
    } else {
        let first = peer_samples.first().copied().unwrap_or_default();
        let last = peer_samples.last().copied().unwrap_or_default();
        let mut buffer = SampleStats::new(4096);
        let mut bitrate = SampleStats::new(4096);
        let mut loss = SampleStats::new(4096);
        for stats in &peer_samples {
            buffer.push(stats.buffer_level_us);
            bitrate.push(stats.bitrate_bps);
            loss.push(u32::from(stats.loss_pct_x100));
        }
        rows.push(Row::measured(
            "对端 · 1 Hz STREAM_STATS",
            "播放环水位 buffer_level_us",
            summary_ms(buffer.summary()),
            "对端按「待播队列深度 × 帧长」换算；这是接收侧才量得到的量".to_string(),
        ));
        rows.push(Row::measured(
            "对端 · 1 Hz STREAM_STATS",
            "欠载 / 迟到丢弃 / PLC",
            format!(
                "欠载 {}（首 {}）/ 迟到 {}（首 {}）/ PLC {}",
                last.underruns, first.underruns, last.late_drops, first.late_drops, last.plc_count
            ),
            "累计计数，取末值；首值一并给出便于判断观测期内是否新增".to_string(),
        ));
        rows.push(Row::measured(
            "对端 · 1 Hz STREAM_STATS",
            "码率 bitrate_bps",
            format!(
                "末值 {:.1} kbps · 均值 {:.1} kbps（n={}）",
                f64::from(last.bitrate_bps) / 1000.0,
                bitrate.summary().map_or(0.0, |summary| summary.mean) / 1000.0,
                peer_samples.len()
            ),
            "对端按「1 Hz 窗口内收到的载荷字节 × 8」算 —— 这就是**真实在链路上跑的码率**"
                .to_string(),
        ));
        rows.push(Row::measured(
            "对端 · 1 Hz STREAM_STATS",
            "丢包率 loss_pct",
            format!(
                "末值 {:.2}% · 最大 {:.2}%（n={}）",
                f64::from(last.loss_pct_x100) / 100.0,
                loss.summary().map_or(0.0, |summary| f64::from(summary.max)) / 100.0,
                peer_samples.len()
            ),
            "接收侧口径：分母随发送端包率走（丢包不会让分母塌陷）".to_string(),
        ));
        rows.push(Row::measured(
            "对端 · 1 Hz STREAM_STATS",
            "对端自报 e2e_latency_us",
            format!("{} µs", last.e2e_latency_us),
            "M1 无预约播放 / 无 epoch，引擎不产生 e2e 样本 → 对端恒为 0：**这是「未测」而不是「0 ms」**"
                .to_string(),
        ));
        peer_stats_data = json!({
            "samples": peer_samples.len(),
            "first": stats_json(&first),
            "last": stats_json(&last),
        });
    }

    // ---- 对端设备侧（由设备侧补充）----
    rows.push(Row::unmeasured(
        "对端 · 设备侧（由 Lead 补）",
        "对端 AudioTrack 输出缓冲",
        "未测（由设备侧补充）：Lead 会在真机侧用 getPerformanceMode() / dumpsys media.audio_flinger 填这一项"
            .to_string(),
    ));
    rows.push(Row::unmeasured(
        "对端 · 设备侧（由 Lead 补）",
        "对端底层输出是否低延迟",
        "未测（由设备侧补充）：AudioTrack.getPerformanceMode() == PERFORMANCE_MODE_LOW_LATENCY 只能在设备侧读"
            .to_string(),
    ));

    // ---- 自检专属：同进程「封口 → sink 写出」----
    if options.mode == Mode::SelfTest {
        rows.push(Row::measured(
            "PC · 自检同进程",
            "封口 → sink 写出（探针配对）",
            match tap.summary {
                Some(_) => summary_ms(tap.summary),
                None => format!("未配对到样本（已配对 {}）", tap.matched),
            },
 "两个 Engine 共享同一个进程级时钟，因此这一对样本是真实测（口径与 link-loop 一致）； **跨机不成立** —— 跨机必须靠 §6 offset"
                .to_string(),
        ));
    }

    // ---- 合计 e2e（下限 + 未测项）----
    let half_frame_us = f64::from(options.frame_ms) * 1000.0 / 2.0;
    let assemble_us = f64::from(options.frame_ms) * 1000.0;
    let one_way_p50_us = probe_last
        .and_then(|probes| probes.rtt_p50_us)
        .map(u64::from)
        .map(|rtt| rtt / 2)
        .or_else(|| {
            cross
                .and_then(|probe| probe.one_way_p50_us)
                .and_then(|value| u64::try_from(value).ok())
        })
        .or_else(|| cross.and_then(|probe| probe.rtt_p50_us).map(|rtt| rtt / 2))
        .or_else(|| clock_last.map(|clock| clock.rtt_us / 2));
    let buffer_values: Vec<f64> = peer_samples
        .iter()
        .map(|stats| f64::from(stats.buffer_level_us))
        .collect();
    let buffer_p50 = percentile_f64(&buffer_values, 50);
    let base_p50 = half_frame_us
        + assemble_us
        + one_way_p50_us.map_or(0.0, |value| value as f64)
        + buffer_p50.unwrap_or(0.0);
    let e2e_lower = (base_p50 / 1000.0) as u64;
    rows.push(Row {
        section: "合计",
        item: "e2e 下限（仅含已实测/已建模段）".to_string(),
        label: "模型",
        value: format!("≥ {e2e_lower} ms（P50 口径）"),
        note: format!(
            "= 采集半周期 {:.1} + 组帧 {:.1} + 网络单向 {} + 对端播放环水位 {}；\
             未测项（PC 封口→发出 / 对端解码 / 对端 AudioTrack 输出）**没有上界**，因此给不出上限",
            half_frame_us / 1000.0,
            assemble_us / 1000.0,
            one_way_p50_us.map_or("未测".to_string(), |value| format!(
                "{:.2} ms",
                us_to_ms(value)
            )),
            buffer_p50.map_or("未测".to_string(), |value| format!(
                "{:.1} ms",
                value / 1000.0
            )),
        ),
    });
    rows.push(Row {
        section: "合计",
        item: "网络单向（对称路径假设）".to_string(),
        label: if one_way_p50_us.is_some() { "模型" } else { "未测" },
        value: one_way_p50_us.map_or("—".to_string(), |value| format!("{:.2} ms", us_to_ms(value))),
        note: "优先取 §6 探针 RTT P50 / 2，其次取独立测量连接的逐探针单向 P50；两者都含**对称路径假设**。\
               逐探针 t2 − t1 − offset 的原始值须走 --cross-probe（独立连接）"
            .to_string(),
    });

    // ---- 未测项清单 ----
    let unmeasured: Vec<String> = rows
        .iter()
        .filter(|row| row.label == "未测")
        .map(|row| format!("[{}] {}：{}", row.section, row.item, row.note))
        .collect();

    // ---- 判定：对端出声证据 ----
    if options.mode == Mode::Device
        && let Some(_last) = peer_samples.last()
        && !peer_samples
            .iter()
            .any(|stats| stats.buffer_level_us > 0 || stats.underruns > 0)
    {
        verdict.reasons.push(
            "对端全程没有任何出声证据（buffer_level_us / underruns 全为 0）—— 手机端很可能没有在收流/播放"
                .to_string(),
        );
        if verdict.code != EXIT_NO_CLOCK {
            verdict.code = EXIT_NO_PEER;
        }
    }

    // ---- 判定：采集端根本没数据（虚拟端点 / 空闲端点 / 选错端点）----
    // loopback 只在端点**有播放**时交付数据（`docs/10-handoff.md` 坑 12：空闲端点零数据不是错误）。
    // 全程 0 帧封口意味着这次观测**什么都没量到**，必须非零退出，不能报「观测成立」。
    if options.capture == CaptureBackend::Wasapi && sealed_total == 0 {
        verdict.reasons.push(format!(
 "本机采集端全程 0 帧封口：端点「{}」一个字节都没交付。loopback 只在端点有播放时交付数据 —— 先在 PC 上播一段测试音，或换真实端点 --device id:<子串>，或用 --capture synth 走对照路径",
            capture.device_name.clone().unwrap_or_else(|| "-".to_string())
        ));
        if verdict.code == EXIT_OK {
            verdict.code = EXIT_SILENT_CAPTURE;
        }
    }

    // ---- 判定：PC 侧采集疑似静音（采到数据但基本是静音）----
    if options.mode == Mode::Device && options.capture == CaptureBackend::Wasapi {
        let bitrate_bps = peer_samples.last().map_or(0, |stats| stats.bitrate_bps);
        let virtual_hint = silent_by_config || capture.looks_virtual.unwrap_or(false);
        if !peer_samples.is_empty() && bitrate_bps < SILENT_CAPTURE_BITRATE_BPS {
            verdict.reasons.push(format!(
                "PC 侧采集疑似静音：对端实测码率只有 {} kbps（< {} kbps 阈值）{}",
                bitrate_bps / 1000,
                SILENT_CAPTURE_BITRATE_BPS / 1000,
                if virtual_hint {
                    "，而所选端点是疑似虚拟声卡 —— 先播一段测试音，或换 --device id:<真实端点> / --capture synth 复核"
                } else {
                    " —— 确认 PC 上真的在播放声音（loopback 采的就是系统输出）"
                }
            ));
            if verdict.code == EXIT_OK {
                verdict.code = EXIT_SILENT_CAPTURE;
            }
        }
    }

    verdict.ok = verdict.code == EXIT_OK;
    let started_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);

    let data = json!({
        "tool": "device-link",
        "mode": mode,
        "started_unix": started_unix,
        "options": {
            "peer": options.peer,
            "seconds": options.seconds,
            "frame_ms": options.frame_ms,
            "capture": options.capture.as_str(),
            "capture_source": options.capture.source_note(),
            "device": options.device,
            "buffer_ms": options.buffer_ms,
            "cross_probe": options.cross_probe,
            "pin_provided": options.pin.is_some(),
        },
        "auth_path": auth.name(),
        "peer": {
            "id": remote.id.short(),
            "name": remote.name,
            "addr": remote.addr,
            "trusted": remote.trusted,
        },
        "capture": {
            "backend": capture.backend,
            "source_note": capture.source_note,
            "device_name": capture.device_name,
            "device_short_id": capture.device_short_id,
            "is_default": capture.is_default,
            "looks_virtual": capture.looks_virtual,
            "format": capture.format_text,
            "format_unified": capture.format_unified,
            "period_default_ms": capture.period_default_ms,
            "period_min_ms": capture.period_min_ms,
            "buffer_frames": capture.buffer_frames,
            "requested_buffer_ms": capture.requested_buffer_ms,
            "effective_buffer_ms": capture.effective_buffer_ms,
            "warnings": capture.warnings,
        },
        "clock": clock_last.map(|clock| json!({
            "offset_us": clock.offset_us,
            "drift_ppm": clock.drift_ppm,
            "drift_reliable": clock.samples >= 200,
            "rtt_us": clock.rtt_us,
            "quality": clock.quality,
            "samples": clock.samples,
            "first_converged_at_s": first_clock_at,
        })),
        "probes": probe_last.map(|probes| json!({
            "sent": probes.sent,
            "received": probes.received,
            "replies": probes.replies,
            "unmatched": probes.unmatched,
            "pending": probes.sent.saturating_sub(probes.replies),
            "rtt_p50_us": probes.rtt_p50_us,
            "rtt_p95_us": probes.rtt_p95_us,
            "rtt_samples": probes.rtt_samples,
        })),
        "cross_probe": cross.map(cross_json),
        "peer_stream_stats": peer_stats_data,
        "ledger": rows.iter().map(|row| json!({
            "section": row.section,
            "item": row.item,
            "label": row.label,
            "value": row.value,
            "note": row.note,
        })).collect::<Vec<Value>>(),
        "unmeasured": unmeasured,
        "e2e": {
            "lower_bound_p50_ms": e2e_lower,
            "half_frame_ms": half_frame_us / 1000.0,
            "assemble_ms": assemble_us / 1000.0,
            "one_way_model_ms": one_way_p50_us.map(us_to_ms),
            "peer_buffer_p50_ms": buffer_p50.map(|value| value / 1000.0),
            "note": "未测项没有上界，因此只有下限；见 unmeasured",
        },
        "verdict": {
            "exit_code": verdict.code,
            "ok": verdict.ok,
            "reasons": verdict.reasons,
            "warnings": verdict.warnings,
        },
    });

    Report {
        mode,
        started_unix,
        rows,
        verdict,
        data,
    }
}

/// 从探针读出「已配对 / 孤儿 / 分位摘要」。
fn tap_view(tap: Option<&Arc<MeasurementTap>>) -> TapView {
    match tap {
        Some(tap) => {
            let (sealed_orphans, played_orphans) = tap.orphan_counts();
            TapView {
                matched: tap.matched(),
                sealed_orphans,
                played_orphans,
                summary: tap.summary(),
            }
        }
        None => TapView::default(),
    }
}

/// 有符号毫秒（单向值可能为负 —— 那说明对称假设不成立或样本太少，必须如实显示）。
fn signed_ms(value: i64) -> String {
    format!("{:.2} ms", value as f64 / 1000.0)
}

/// `StreamStats` → JSON。
fn stats_json(stats: &StreamStats) -> Value {
    json!({
        "rtt_us": stats.rtt_us,
        "jitter_us": stats.jitter_us,
        "jitter_p95_us": stats.jitter_p95_us,
        "loss_pct_x100": stats.loss_pct_x100,
        "bitrate_bps": stats.bitrate_bps,
        "clock_offset_us": stats.clock_offset_us,
        "drift_ppm": stats.drift_ppm,
        "buffer_level_us": stats.buffer_level_us,
        "underruns": stats.underruns,
        "plc_count": stats.plc_count,
        "nack_count": stats.nack_count,
        "e2e_latency_us": stats.e2e_latency_us,
        "late_drops": stats.late_drops,
    })
}

/// 独立测量连接 → JSON。
fn cross_json(probe: &CrossProbe) -> Value {
    json!({
        "target": probe.target,
        "sent": probe.sent,
        "replies": probe.replies,
        "timeouts": probe.timeouts,
        "anomalies": probe.anomalies,
        "warmup_retries": probe.warmup_retries,
        "rtt_p50_us": probe.rtt_p50_us,
        "rtt_p95_us": probe.rtt_p95_us,
        "rtt_min_us": probe.rtt_min_us,
        "rtt_max_us": probe.rtt_max_us,
        "offset_us": probe.offset_us,
        "offset_spread_us": probe.offset_spread_us,
        "quality": probe.quality,
        "samples": probe.samples,
        "one_way_p50_us": probe.one_way_p50_us,
        "one_way_p95_us": probe.one_way_p95_us,
        "one_way_min_us": probe.one_way_min_us,
        "one_way_max_us": probe.one_way_max_us,
        "note": probe.note,
    })
}

/// PC 侧编码耗时探针（工具内独立 OpusEncoder，与引擎同 crate / 同参数）。
fn measure_encode_probe(options: &Options) -> Result<Option<Summary>> {
    let mut codec = CodecConfig::m1_default();
    codec.frame_ms = options.frame_ms;
    codec
        .validate()
        .map_err(|error| anyhow!("编码参数校验失败：{error}"))?;
    let mut encoder =
        OpusEncoder::new(codec).map_err(|error| anyhow!("创建 Opus 编码器失败：{error}"))?;
    let frame_len = codec.interleaved_frame();
    let mut pcm = vec![0.0_f32; frame_len];
    let mut phase = 0.0_f64;
    for sample in pcm.iter_mut() {
        *sample = (phase * std::f64::consts::TAU).sin() as f32 * 0.25;
        phase += 440.0 / 48_000.0;
        if phase >= 1.0 {
            phase -= 1.0;
        }
    }
    let mut out = vec![0_u8; 4096];
    let mut stats = SampleStats::new(512);
    for _ in 0..200 {
        let started = Instant::now();
        let written = encoder
            .encode_into(&pcm, &mut out)
            .map_err(|error| anyhow!("编码探针失败：{error}"))?;
        let micros = u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX);
        if written == 0 {
            continue;
        }
        stats.push(micros);
    }
    Ok(stats.summary())
}

/// 打印采集端点事实。
fn report_capture_facts(facts: &CaptureFacts) {
    println!("    后端 {}（{}）", facts.backend, facts.source_note);
    println!(
        "    端点 {}（默认={} 疑似虚拟={}）",
        facts.device_name.clone().unwrap_or_else(|| "-".to_string()),
        facts
            .is_default
            .map_or("?".to_string(), |value| value.to_string()),
        facts
            .looks_virtual
            .map_or("?".to_string(), |value| value.to_string()),
    );
    println!(
        "    格式 {}（统一格式 = {}）",
        facts.format_text.clone().unwrap_or_else(|| "-".to_string()),
        facts
            .format_unified
            .map_or("?".to_string(), |value| value.to_string()),
    );
    if let (Some(default), Some(min)) = (facts.period_default_ms, facts.period_min_ms) {
        println!("    周期 默认 {default:.1} ms / 最小 {min:.1} ms（实测）");
    }
    println!(
        "    缓冲 请求 {} ms → 实际 {}（{} 帧）",
        facts.requested_buffer_ms,
        facts
            .effective_buffer_ms
            .map_or("-".to_string(), |value| format!("{value:.1} ms")),
        facts
            .buffer_frames
            .map_or("-".to_string(), |value| value.to_string()),
    );
    for warning in &facts.warnings {
        println!("    ⚠ {warning}");
    }
}

/// 渲染账本（文本）。
fn render_report(report: &Report, options: &Options) {
    println!("\n=== device-link 延迟账本（{}）===", report.mode);
    println!(
        "口径：[实测] = 本次运行真的量到；[模型] = 常量或按公式换算；[未测] = 没量到（不填 0）"
    );

    let mut section = "";
    for row in &report.rows {
        if row.section != section {
            section = row.section;
            println!("\n---- {section} ----");
        }
        println!("    [{}] {:<40} {}", row.label, row.item, row.value);
        if !options.quiet && !row.note.is_empty() {
            for line in wrap_note(&row.note, 96) {
                println!("          {line}");
            }
        }
    }

    println!("\n---- 未测项清单（补齐前不许给承诺区间）----");
    let unmeasured: Vec<&Row> = report
        .rows
        .iter()
        .filter(|row| row.label == "未测")
        .collect();
    if unmeasured.is_empty() {
        println!("    （无）");
    } else {
        for (index, row) in unmeasured.iter().enumerate() {
            println!("    {}. [{}] {}", index + 1, row.section, row.item);
        }
    }

    println!("\n---- 判定 ----");
    if report.verdict.ok {
        println!("    ✅ 本次观测成立（退出码 0）");
    } else {
        println!(
            "    ❌ 本次观测**不成立**（退出码 {}）",
            report.verdict.code
        );
    }
    for reason in &report.verdict.reasons {
        println!("    · {reason}");
    }
    for warning in &report.verdict.warnings {
        println!("    ⚠ {warning}");
    }
    println!(
        "    退出码约定：0 成功 · 2 参数/连接/配对失败 · 3 对端无遥测/无出声证据 · 4 时钟未收敛 · 5 PC 采集疑似静音"
    );
}

/// 简易折行（注释文本长，终端里要能读）。
fn wrap_note(note: &str, width: usize) -> Vec<String> {
    // 先把空白折叠掉：注释文本在源码里用  续行，续行的缩进会原样进入字符串，
    // 不折叠的话终端上会出现「一堆空格」这种看着像排版坏了的输出（报告要能直接贴进证据里）。
    let normalized = note.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut lines = Vec::new();
    let mut current = String::new();
    for chunk in normalized.split_inclusive(char::is_whitespace) {
        if current.chars().count() + chunk.chars().count() > width && !current.is_empty() {
            lines.push(current.trim_end().to_string());
            current = String::new();
        }
        current.push_str(chunk);
    }
    if !current.trim().is_empty() {
        lines.push(current.trim_end().to_string());
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// 落盘：默认写 `target/evidence/device-tool/`，`--json <path>` 再写一份到指定路径。
fn write_reports(report: &Report, options: &Options) -> Result<()> {
    let stamp = report.started_unix;
    let default_path = PathBuf::from(EVIDENCE_DIR).join(format!("{}-{stamp}.json", report.mode));
    write_json(&default_path, &report.data)?;
    println!("\n证据（JSON）：{}", default_path.display());
    if let Some(path) = options.json.as_ref() {
        write_json(path, &report.data)?;
        println!("证据（JSON，--json）：{}", path.display());
    }
    Ok(())
}

fn write_json(path: &Path, data: &Value) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建目录 {} 失败", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(data).context("序列化 JSON 报告失败")?;
    std::fs::write(path, text).with_context(|| format!("写入 {} 失败", path.display()))?;
    Ok(())
}

/// `AudioLinkError` → 可读文本（保留错误码，报告里要能定位）。
fn link_error(error: &audiolink_types::AudioLinkError) -> String {
    format!(
        "{} {}（{}）",
        error.code().as_u16(),
        error.code().name(),
        error.context()
    )
}

// ---------------------------------------------------------------------------
// 独立测量连接（--cross-probe）：逐探针四时间戳
// ---------------------------------------------------------------------------

/// 独立测量连接的逐探针结果。
#[derive(Debug, Clone, Default)]
struct CrossProbe {
    target: String,
    sent: u32,
    replies: u32,
    timeouts: u32,
    anomalies: u32,
    /// 首探针预热重发次数（连接刚建立时的时序窗口，不是丢包）。
    warmup_retries: u32,
    rtt_p50_us: Option<u64>,
    rtt_p95_us: Option<u64>,
    rtt_min_us: Option<u64>,
    rtt_max_us: Option<u64>,
    offset_us: Option<i64>,
    offset_spread_us: Option<i64>,
    quality: Option<&'static str>,
    samples: usize,
    one_way_p50_us: Option<i64>,
    one_way_p95_us: Option<i64>,
    one_way_min_us: Option<i64>,
    one_way_max_us: Option<i64>,
    note: String,
}

/// 用**本机身份**（mTLS，`audiolink_net::AudioLinkEndpoint`）起一条独立测量连接跑 §6 探针。
///
/// 口径：这条连接与音频会话**同路径、不同 QUIC 连接**；它的样本不能与引擎会话内的 §6 估计混为一谈。
/// 生产端点是强制 mTLS（`audiolink-net::tls`），所以这条连接必须带本机客户端证书 —— 不能用
/// `latency-probe` 的 `SkipServerVerification` 那条路（会被服务端以 `peer sent no certificates` 拒绝）。
/// 对端响应方若只在会话内处理 `CLOCK_PROBE`，则会零回复 —— 此时按 **[未测]** 呈现，不编数字。
async fn cross_probe(
    target: SocketAddr,
    count: u32,
    identity_dir: &Path,
    node_name: &str,
    timeout: Duration,
) -> Result<CrossProbe> {
    let mut result = CrossProbe {
        target: target.to_string(),
        ..CrossProbe::default()
    };

    let identity = audiolink_identity::NodeIdentity::load_or_create(identity_dir, node_name)
        .map_err(|error| anyhow!("加载本机身份失败：{error}"))?;
    let endpoint = audiolink_net::AudioLinkEndpoint::bind(audiolink_net::EndpointConfig {
        bind: SocketAddr::from(([0, 0, 0, 0], 0)),
        cert_der: identity.cert_der().to_vec(),
        key_der_pkcs8: identity.key_der_pkcs8().to_vec(),
        idle_timeout_ms: 10_000,
        keep_alive_ms: 3_000,
    })
    .await
    .map_err(|error| anyhow!("独立测量连接绑定失败：{}", error.context()))?;

    let connection =
        match tokio::time::timeout(timeout, endpoint.connect(target, "audiolink")).await {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => {
                result.note = format!("独立测量连接建立失败（{}）", error.context());
                return Ok(result);
            }
            Err(_) => {
                result.note = format!("独立测量连接建立超时（{timeout:?}）");
                return Ok(result);
            }
        };
    result.note = format!(
        "已建立独立测量连接（本机身份 mTLS；max_audio_payload = {} B）",
        connection.max_audio_payload()
    );

    let started = Instant::now();
    let mut samples = ClockSamples::new();
    let mut one_way: Vec<i64> = Vec::new();
    for seq in 0..count {
        let t1 = i64::try_from(started.elapsed().as_micros()).unwrap_or(i64::MAX);
        let probe = ClockProbe { probe_seq: seq, t1 };
        let wire = match probe.to_datagram_bytes() {
            Ok(wire) => wire,
            Err(error) => {
                result.note = format!("编码 CLOCK_PROBE 失败：{error}");
                break;
            }
        };
        if let Err(error) = connection.send_datagram(&wire).await {
            result.note = format!("发送 CLOCK_PROBE 失败：{}", error.context());
            break;
        }
        result.sent += 1;

        // 首探针预热 + 重试：QUIC 的**客户端**握手完成 ≠ 服务端已经 accept 这条连接，
        // 而服务端把连接挂进「读数据报」循环之前到达的数据报会被静默丢弃。
        // 这是已知的时序窗口（不是「对端不回」）：首探针多等一会儿、必要时重发几次即可跨过去。
        let mut retries = 0_u32;
        let outcome = loop {
            let wait = if seq == 0 {
                Duration::from_millis(1_000)
            } else {
                Duration::from_millis(300)
            };
            match wait_reply(&connection, seq, wait).await {
                ProbeReply::Timeout if seq == 0 && retries < 3 => {
                    retries += 1;
                    result.warmup_retries = result.warmup_retries.saturating_add(1);
                    if let Err(error) = connection.send_datagram(&wire).await {
                        break ProbeReply::Failed(error.context().to_string());
                    }
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
                other => break other,
            }
        };

        match outcome {
            ProbeReply::Reply(reply) => {
                let t4 = i64::try_from(started.elapsed().as_micros()).unwrap_or(i64::MAX);
                let offset_us = ((reply.t2 - reply.t1) + (reply.t3 - t4)) / 2;
                let rtt_us = (t4 - reply.t1) - (reply.t3 - reply.t2);
                if rtt_us < 0 {
                    result.anomalies += 1;
                } else {
                    samples.push(Sample {
                        rtt_us: u64::try_from(rtt_us).unwrap_or(0),
                        offset_us,
                    });
                    one_way.push(reply.t2 - reply.t1 - offset_us);
                    result.replies += 1;
                }
            }
            ProbeReply::Timeout => result.timeouts += 1,
            ProbeReply::Failed(reason) => {
                result.note = format!("独立测量连接中断：{reason}");
                break;
            }
        }
        if seq + 1 < count {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    result.rtt_p50_us = samples.rtt_percentile_us(50);
    result.rtt_p95_us = samples.rtt_percentile_us(95);
    result.rtt_min_us = samples.rtt_percentile_us(0);
    result.rtt_max_us = samples.rtt_percentile_us(100);
    result.offset_us = samples.best_offset_us();
    result.offset_spread_us = samples.best_offset_spread_us();
    result.samples = samples.len();
    result.quality = Some(samples.quality().as_str());
    result.one_way_p50_us = percentile_i64(&one_way, 50);
    result.one_way_p95_us = percentile_i64(&one_way, 95);
    result.one_way_min_us = percentile_i64(&one_way, 0);
    result.one_way_max_us = percentile_i64(&one_way, 100);

    connection.close(0, "cross-probe done");
    endpoint.close(0, "cross-probe done");
    Ok(result)
}

/// 打印独立测量连接的结果。
fn report_cross_probe(probe: &CrossProbe) {
    println!(
        "    sent {} / replies {} / 超时 {} / 异常 {} · {}",
        probe.sent, probe.replies, probe.timeouts, probe.anomalies, probe.note
    );
    if probe.replies == 0 {
        println!(
            "    ⚠ 零回复：对端响应方目前只在 §5 握手完成后处理 CLOCK_PROBE —— 该项按 [未测] 呈现（不填 0）"
        );
        return;
    }
    println!(
        "    逐探针 RTT：P50 {} / P95 {}（min {} / max {}，n={}）· 质量 {}",
        probe.rtt_p50_us.map_or("-".to_string(), |value| format!(
            "{:.2} ms",
            us_to_ms(value)
        )),
        probe.rtt_p95_us.map_or("-".to_string(), |value| format!(
            "{:.2} ms",
            us_to_ms(value)
        )),
        probe.rtt_min_us.map_or("-".to_string(), |value| format!(
            "{:.2} ms",
            us_to_ms(value)
        )),
        probe.rtt_max_us.map_or("-".to_string(), |value| format!(
            "{:.2} ms",
            us_to_ms(value)
        )),
        probe.samples,
        probe.quality.unwrap_or("-")
    );
    println!(
        "    网络单向 = t2 − t1 − offset：P50 {} / P95 {}（min {} / max {}）—— offset 取自同一条连接的 best-8 中位数，含对称路径假设",
        probe.one_way_p50_us.map_or("-".to_string(), signed_ms),
        probe.one_way_p95_us.map_or("-".to_string(), signed_ms),
        probe.one_way_min_us.map_or("-".to_string(), signed_ms),
        probe.one_way_max_us.map_or("-".to_string(), signed_ms),
    );
}

/// 等一条匹配的 `CLOCK_REPLY`。
async fn wait_reply(
    connection: &audiolink_net::Connection,
    probe_seq: u32,
    timeout: Duration,
) -> ProbeReply {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut buffer = vec![0_u8; 2048];
    loop {
        let read =
            tokio::time::timeout_at(deadline, connection.read_datagram_into(&mut buffer)).await;
        let length = match read {
            Err(_) => return ProbeReply::Timeout,
            Ok(Err(error)) => return ProbeReply::Failed(error.context().to_string()),
            Ok(Ok(length)) => length,
        };
        let Some(bytes) = buffer.get(..length) else {
            continue;
        };
        let Ok(datagram) = AudioDatagram::decode(bytes) else {
            continue; // §1.1：解不出来的数据报忽略（不计进 unmatched：那只是「结构合法但配不上」）
        };
        let Ok(reply) = ClockReply::from_datagram(&datagram) else {
            continue;
        };
        if reply.probe_seq == probe_seq {
            return ProbeReply::Reply(reply);
        }
    }
}

/// 一次探针的等待结果。
enum ProbeReply {
    Reply(ClockReply),
    Timeout,
    Failed(String),
}

// ---------------------------------------------------------------------------
// 单元测试（纯逻辑：参数解析 / 百分位 / 三态标注）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parses_run_command_with_defaults() {
        let options = parse_args(&args(&["run", "--peer", "172.16.2.54"]))
            .unwrap()
            .unwrap();
        assert_eq!(options.mode, Mode::Device);
        assert_eq!(options.peer.as_deref(), Some("172.16.2.54"));
        assert_eq!(options.seconds, 30);
        assert_eq!(options.frame_ms, 20);
        assert_eq!(options.capture, CaptureBackend::Wasapi);
        assert!(!options.quiet);
    }

    #[test]
    fn parses_full_option_set() {
        let options = parse_args(&args(&[
            "run",
            "--peer",
            "172.16.2.54:58290",
            "--seconds",
            "10",
            "--pin",
            "012345",
            "--frame-ms",
            "10",
            "--capture",
            "synth",
            "--json",
            "out.json",
            "--quiet",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(options.seconds, 10);
        assert_eq!(options.pin.as_deref(), Some("012345"));
        assert_eq!(options.frame_ms, 10);
        assert_eq!(options.capture, CaptureBackend::Synth);
        assert_eq!(options.json, Some(PathBuf::from("out.json")));
        assert!(options.quiet);
    }

    #[test]
    fn self_test_does_not_require_peer() {
        let options = parse_args(&args(&["--self-test"])).unwrap().unwrap();
        assert_eq!(options.mode, Mode::SelfTest);
        assert!(options.peer.is_none());
    }

    #[test]
    fn run_without_peer_is_an_error() {
        assert!(parse_args(&args(&["run"])).is_err());
    }

    #[test]
    fn rejects_bad_pin_and_unknown_flags() {
        assert!(parse_args(&args(&["run", "--peer", "1.2.3.4", "--pin", "12345"])).is_err());
        assert!(parse_args(&args(&["run", "--peer", "1.2.3.4", "--pin", "abcdef"])).is_err());
        assert!(parse_args(&args(&["run", "--peer", "1.2.3.4", "--nope"])).is_err());
        assert!(parse_args(&args(&["run", "--peer", "1.2.3.4", "--capture", "alsa"])).is_err());
    }

    #[test]
    fn parses_pin_file_and_listen_port() {
        let options = parse_args(&args(&[
            "run",
            "--peer",
            "172.16.2.54",
            "--pin-file",
            "pin.txt",
            "--listen-port",
            "58800",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(options.pin_file, Some(PathBuf::from("pin.txt")));
        assert_eq!(options.listen_port, 58800);
        assert!(parse_args(&args(&["run", "--peer", "1.2.3.4", "--listen-port", "0"])).is_err());
    }

    #[test]
    fn extract_pin_takes_first_six_digit_run() {
        assert_eq!(extract_pin("123456"), Some("123456".to_string()));
        assert_eq!(extract_pin("  123456  "), Some("123456".to_string()));
        assert_eq!(
            extract_pin("PIN: 123456 # 20260916"),
            Some("123456".to_string())
        );
        assert_eq!(extract_pin("12345"), None);
        assert_eq!(extract_pin("1234567"), None);
        assert_eq!(extract_pin("20260916"), None);
        assert_eq!(extract_pin(""), None);
    }

    /// **真链路**验证 `--pin-file` 的完整机制：两个真 Engine + 真 QUIC + 真 §5 握手，
    /// PIN 由「响应方展示 → 写进文件 → 发起方轮询读到 → 在**同一条连接内**提交」这条无头路径传递。
    ///
    /// 这里用进程内的两个 Engine 而不是命令行，是因为「手机屏幕上的 PIN」只有**响应方**知道：
    /// 测试里由响应方事件流充当那个人，把 PIN 写进文件；命令行版本对真机跑时，那个人就是 Lead。
    #[tokio::test]
    async fn pin_file_pairs_over_real_quic() {
        let dir = tempfile::TempDir::new().unwrap();
        let dir_a = dir.path().join("a");
        let dir_b = dir.path().join("b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();

        let options = Options {
            capture: CaptureBackend::Synth,
            ..Options::default()
        };

        let mut config_a = EngineConfig::new("pin-file-a", &dir_a);
        config_a.listen = "127.0.0.1:0".parse().unwrap();
        config_a.capture = capture_factory(&options);

        let mut config_b = EngineConfig::new("pin-file-b", &dir_b);
        config_b.listen = "127.0.0.1:0".parse().unwrap();
        config_b.playout = Some(null_playout_factory(60));

        let engine_a = Engine::start(config_a).await.unwrap();
        let engine_b = Engine::start(config_b).await.unwrap();
        let _accept_a = engine_a.spawn_accept_loop();
        let _accept_b = engine_b.spawn_accept_loop();

        // 「人」：把响应方展示的 PIN 写进文件（延迟 200 ms，模拟读屏幕）
        let pin_path = dir.path().join("pin.txt");
        let writer_path = pin_path.clone();
        let mut events_b = engine_b.subscribe();
        tokio::spawn(async move {
            while let Ok(event) = events_b.recv().await {
                if let EngineEvent::DisplayPin { pin, .. } = event {
                    // 200 ms ≈ 人工读屏+敲字的快版；task-10 之后配对死线顺延到 PIN 有效期 + 15 s 余量，
                    // 「慢版」（20 s 级人工节奏）也验证过：见 target/evidence/device-tool/e8-pinfile-delayed-20s.log
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    let _ = std::fs::write(&writer_path, format!("{pin}\n"));
                    return;
                }
            }
        });

        let outcome = engine_a.connect(engine_b.local_addr()).await;
        assert!(
            matches!(outcome, Err(ref error) if error.code() == ErrorCode::NotPaired),
            "未配对的连接必须返回 1002 NOT_PAIRED，实际 {outcome:?}"
        );
        let peer = first_peer(&engine_a).expect("会话表里应已有对端");

        let pin = wait_for_pin_file(&pin_path, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(pin.len(), 6);

        // 关键：PIN 必须在**同一条连接**里提交（内核每连接新建 PinGate，重连后那个 PIN 就作废了）
        engine_a.submit_pin(peer, &pin).await.unwrap();
        wait_for_streaming(&engine_a, peer, Duration::from_secs(10))
            .await
            .unwrap();

        // 响应方是「先回 PAIR_RESULT、再置 trusted 位」，所以这里要轮询等它落定（不是断言失败）。
        // 注意：在 B 的会话表里，对端是 **A 自己的 NodeId** —— 别拿 A 眼里的 peer（那是 B）去查。
        let local_id = engine_a.info().id;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut trusted = false;
        while tokio::time::Instant::now() < deadline {
            trusted = engine_b
                .peers()
                .into_iter()
                .find(|status| status.id == local_id)
                .map(|status| status.trusted)
                .unwrap_or(false);
            if trusted {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(trusted, "配对成功后对端应把本机写进信任库");
        // 「跨次留存」的落盘证据：信任库文件必须真的在
        assert!(
            dir_b.join("trust.json").exists(),
            "配对成功后信任库必须已落盘（否则复跑还要再看一次手机屏幕）"
        );

        engine_a.shutdown().await;
        engine_b.shutdown().await;
    }

    #[tokio::test]
    async fn pin_file_polling_reads_late_pin() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("pin.txt");
        let writer = path.clone();
        // 模拟真实时序：工具已经在线上，投喂方稍后才把文件写出来
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            std::fs::write(&writer, "PIN: 246810\n").unwrap();
        });
        let pin = wait_for_pin_file(&path, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(pin, "246810");
    }

    #[tokio::test]
    async fn pin_file_wait_times_out_with_clear_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("missing.txt");
        let error = wait_for_pin_file(&path, Duration::from_millis(400))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("超时"), "{error:#}");
    }

    #[test]
    fn help_returns_none() {
        assert!(parse_args(&args(&["--help"])).unwrap().is_none());
        assert!(parse_args(&args(&["run", "-h"])).unwrap().is_none());
    }

    #[test]
    fn resolves_peer_with_and_without_port() {
        assert_eq!(
            resolve_peer("172.16.2.54").unwrap(),
            "172.16.2.54:58290".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            resolve_peer("172.16.2.54:1234").unwrap(),
            "172.16.2.54:1234".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(resolve_peer("127.0.0.1").unwrap().port(), DEFAULT_QUIC_PORT);
    }

    #[test]
    fn percentile_is_nearest_rank() {
        assert_eq!(percentile_i64(&[10, 20, 30, 40], 0), Some(10));
        assert_eq!(percentile_i64(&[10, 20, 30, 40], 50), Some(20));
        assert_eq!(percentile_i64(&[10, 20, 30, 40], 95), Some(40));
        assert_eq!(percentile_i64(&[], 50), None);
        assert_eq!(percentile_f64(&[1.0, 2.0, 3.0], 100), Some(3.0));
        assert_eq!(percentile_f64(&[], 50), None);
    }

    #[test]
    fn encode_probe_returns_samples() {
        let options = Options::default();
        let summary = measure_encode_probe(&options).unwrap().unwrap();
        assert!(summary.count >= 100, "样本数 {}", summary.count);
        assert!(
            summary.p50 < 20_000,
            "P50 编码耗时 {} µs 不像真实值",
            summary.p50
        );
    }

    #[test]
    fn wrap_note_splits_long_text() {
        let wrapped = wrap_note("a b c d e f g h", 4);
        assert!(wrapped.len() > 1);
        assert!(wrapped.iter().all(|line| line.chars().count() <= 8));
    }

    #[test]
    fn row_labels_are_three_state() {
        let measured = Row::measured("s", "i", "v", "n");
        let modeled = Row::modeled("s", "i", "v", "n");
        let unmeasured = Row::unmeasured("s", "i", "n");
        assert_eq!(measured.label, "实测");
        assert_eq!(modeled.label, "模型");
        assert_eq!(unmeasured.label, "未测");
        assert_eq!(unmeasured.value, "—");
    }
}
