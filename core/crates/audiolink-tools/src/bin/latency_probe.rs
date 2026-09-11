//! `latency-probe` —— 两台机器之间跑**真实 QUIC 数据报**，量 RTT / 抖动，并复现 §6 的四时间戳时钟偏移估计。
//!
//! 规格：`docs/03-protocol.md` §3（`CLOCK_PROBE` / `CLOCK_REPLY` 载荷）、§6（四时间戳、最小 RTT 过滤 + 中位数、质量分级）。
//!
//! ```text
//! 服务端：latency-probe listen [--bind 0.0.0.0:58290]
//! 客户端：latency-probe probe <HOST|IP>[:PORT] [--count 200] [--interval-ms 50] [--timeout-ms 500] [--quiet]
//! ```
//!
//! 用途（M0 交付物 6）：在写音频代码之前先量出「QUIC 数据报在这张网上的真实 RTT 分布」，
//! 它同时是 ADR-004（QUIC vs 裸 UDP 的回退判据）与 M1 抖动缓冲深度（§7）的输入。
//!
//! 注意：
//! - 客户端**跳过证书校验**（`SkipServerVerification`）—— 本工具只做测量，不建立信任；生产的身份校验见 §2 / `audiolink-identity`；
//! - 收发的就是生产协议的 `CLOCK_PROBE` / `CLOCK_REPLY` 报文，因此同时验证 alp2 编解码在真实网络上的可用性；
//! - `offset` 是**两台进程单调时钟原点之差**（`Instant` 不可跨进程/跨机比较），所以它的**绝对值没有意义**；
//!   有意义的两个指标是 RTT 分布与「选中样本 offset 极差」（收敛性，§6 要求 ≤ 2 ms）；
//! - 启动时会打印 QUIC 的 `max_datagram_size()` —— 它是 §3「单包 ≤ 1200 B」预算的**真实上限**（默认初始 MTU 下实测 1162 B）。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use audiolink_proto::{AudioDatagram, ClockProbe, ClockReply};
use audiolink_tools::{ClockSamples, Sample};
use audiolink_types::DEFAULT_QUIC_PORT;
use quinn::Endpoint;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime};

const USAGE: &str = "\
latency-probe —— QUIC 数据报 RTT/抖动 + 四时间戳时钟偏移（docs/03-protocol.md §3、§6）

用法:
    latency-probe listen [--bind 0.0.0.0:58290]
    latency-probe probe <HOST|IP>[:PORT] [--count 200] [--interval-ms 50] [--timeout-ms 500] [--quiet]

子命令:
    listen      等待 CLOCK_PROBE 并用 CLOCK_REPLY 立即回填（服务端）
    probe       按 interval 发探针，统计 RTT 分布与 §6.3 的偏移估计（客户端）

常用组合（对齐 §6 的采样节奏）:
    首连快速同步:  --count 50  --interval-ms 100
    稳态观察:      --count 60  --interval-ms 1000

退出码: 0 = 成功；1 = 编译期不会出现（保留）；2 = 参数/连接/无样本错误
";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args.first().map(String::as_str) {
        Some("listen") => run_listen(&args[1..]).await,
        Some("probe") => run_probe(&args[1..]).await,
        Some("-h" | "--help") | None => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(other) => Err(anyhow!("未知子命令 {other}（--help 看用法）")),
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("latency-probe: {error:#}");
            ExitCode::from(2)
        }
    }
}

// ---------------------------------------------------------------------------
// 服务端
// ---------------------------------------------------------------------------

async fn run_listen(args: &[String]) -> Result<()> {
    let mut bind = SocketAddr::from((Ipv4Addr::UNSPECIFIED, DEFAULT_QUIC_PORT));
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--bind" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| anyhow!("--bind 需要一个地址参数"))?;
                bind = value
                    .parse()
                    .with_context(|| format!("解析 --bind {value:?} 失败"))?;
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => return Err(anyhow!("未知参数 {other}（--help 看用法）")),
        }
        index += 1;
    }

    let endpoint = Endpoint::server(server_config()?, bind).context("绑定 QUIC 端口失败")?;
    println!(
        "latency-probe listen: 监听 {}（QUIC 数据报，等待 CLOCK_PROBE；Ctrl+C 退出）",
        endpoint.local_addr()?
    );

    while let Some(incoming) = endpoint.accept().await {
        match incoming.await {
            Ok(connection) => {
                println!(
                    "[server] 已连接 {}  max_datagram_size={:?}",
                    connection.remote_address(),
                    connection.max_datagram_size()
                );
                tokio::spawn(serve_connection(connection));
            }
            Err(error) => eprintln!("[server] 握手失败：{error}"),
        }
    }
    Ok(())
}

/// 每个连接一个任务：收到 `CLOCK_PROBE` 立即用 `CLOCK_REPLY` 回填（§6 的 `t2` / `t3`）。
async fn serve_connection(connection: quinn::Connection) {
    let start = Instant::now();
    loop {
        let bytes = match connection.read_datagram().await {
            Ok(bytes) => bytes,
            Err(error) => {
                println!("[server] 连接结束：{error}");
                return;
            }
        };

        let datagram = match AudioDatagram::decode(&bytes) {
            Ok(datagram) => datagram,
            Err(error) => {
                eprintln!("[server] 非法数据报（L1 拒绝）：{error}");
                continue;
            }
        };
        let probe = match ClockProbe::from_datagram(&datagram) {
            Ok(probe) => probe,
            Err(error) => {
                eprintln!("[server] 非 CLOCK_PROBE：{error}");
                continue;
            }
        };

        let now = now_us(start);
        let reply = ClockReply {
            probe_seq: probe.probe_seq,
            t1: probe.t1,
            t2: now,
            t3: now, // §6：t3 = t2（紧随其后，不引入额外处理延迟）
        };

        match reply.to_datagram_bytes() {
            Ok(wire) => {
                if let Err(error) = connection.send_datagram(wire.into()) {
                    eprintln!("[server] 发送失败：{error}");
                }
            }
            Err(error) => eprintln!("[server] 编码 CLOCK_REPLY 失败：{error}"),
        }
    }
}

// ---------------------------------------------------------------------------
// 客户端
// ---------------------------------------------------------------------------

async fn run_probe(args: &[String]) -> Result<()> {
    let mut target_text: Option<String> = None;
    let mut count: u32 = 200;
    let mut interval_ms: u64 = 50;
    let mut timeout_ms: u64 = 500;
    let mut quiet = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--count" => {
                index += 1;
                count = parse_u32(args.get(index), "--count", 1, 100_000)?;
            }
            "--interval-ms" => {
                index += 1;
                interval_ms = u64::from(parse_u32(args.get(index), "--interval-ms", 0, 60_000)?);
            }
            "--timeout-ms" => {
                index += 1;
                timeout_ms = u64::from(parse_u32(args.get(index), "--timeout-ms", 1, 60_000)?);
            }
            "--quiet" => quiet = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if other.starts_with("--") => {
                return Err(anyhow!("未知参数 {other}（--help 看用法）"));
            }
            other => {
                if target_text.is_some() {
                    return Err(anyhow!("只能指定一个目标地址"));
                }
                target_text = Some(other.to_string());
            }
        }
        index += 1;
    }

    let target_text = target_text.ok_or_else(|| anyhow!("缺少目标地址（--help 看用法）"))?;
    let target = resolve(&target_text, DEFAULT_QUIC_PORT).await?;

    let mut endpoint = Endpoint::client(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
        .context("创建 QUIC 客户端端点失败")?;
    endpoint.set_default_client_config(client_config()?);

    println!(
        "latency-probe probe: 目标 {target}（count={count} interval={interval_ms} ms timeout={timeout_ms} ms）"
    );
    println!(
        "提示：§6.0 的首连快速同步 = `--count 50 --interval-ms 100`；稳态观察 = `--count 60 --interval-ms 1000`"
    );

    let connection = endpoint
        .connect(target, "localhost")
        .with_context(|| format!("发起 QUIC 连接 {target} 失败"))?
        .await
        .context("QUIC 握手失败（对端没在 listen？端口被占用？）")?;
    let max_datagram = connection.max_datagram_size();
    println!(
        "已连接 {}（证书校验已跳过，仅测量用途）  max_datagram_size={max_datagram:?} B",
        connection.remote_address()
    );
    if let Some(limit) = max_datagram {
        println!(
            "提示：§3 的「单包 ≤ 1200 B」必须与它取 min —— 默认初始 MTU 下 QUIC 只给出 ~1162 B，发送侧需按此分片/截断"
        );
        if limit < 1200 {
            println!("      ↑ 当前上限 {limit} B < 1200 B：M1 发送侧不得假设 1200 B 可用");
        }
    }

    let start = Instant::now();
    let mut samples = ClockSamples::new();
    let mut timeouts = 0_u32;
    let mut anomalies = 0_u32;

    for seq in 0..count {
        let t1 = now_us(start);
        let probe = ClockProbe { probe_seq: seq, t1 };
        let wire = probe.to_datagram_bytes().context("编码 CLOCK_PROBE 失败")?;
        connection
            .send_datagram(wire.into())
            .context("发送数据报失败")?;

        match wait_reply(&connection, seq, Duration::from_millis(timeout_ms)).await {
            ReplyOutcome::Reply(reply) => {
                let t4 = now_us(start);
                // §6：offset = ((t2 - t1) + (t3 - t4)) / 2，rtt = (t4 - t1) - (t3 - t2)
                let offset_us = ((reply.t2 - reply.t1) + (reply.t3 - t4)) / 2;
                let rtt_us = (t4 - reply.t1) - (reply.t3 - reply.t2);
                if rtt_us < 0 {
                    anomalies += 1;
                    if !quiet {
                        println!(
                            "  #{seq:>4}  异常：rtt={rtt_us} µs（时钟回退或重复应答），已丢弃"
                        );
                    }
                } else {
                    samples.push(Sample {
                        rtt_us: u64::try_from(rtt_us).unwrap_or(0),
                        offset_us,
                    });
                    if !quiet {
                        println!("  #{seq:>4}  rtt={rtt_us:>7} µs  offset={offset_us:>8} µs");
                    }
                }
            }
            ReplyOutcome::Timeout => {
                timeouts += 1;
                if !quiet {
                    println!("  #{seq:>4}  超时（>{timeout_ms} ms）");
                }
            }
            ReplyOutcome::Failed(reason) => {
                return Err(anyhow!("连接中断：{reason}"));
            }
        }

        if seq + 1 < count && interval_ms > 0 {
            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        }
    }

    let elapsed = start.elapsed();
    connection.close(0_u32.into(), b"done");
    endpoint.wait_idle().await;
    report(&samples, count, timeouts, anomalies, elapsed)
}

/// 打印统计摘要；无有效样本 → 报错（退出码 2）。
fn report(
    samples: &ClockSamples,
    count: u32,
    timeouts: u32,
    anomalies: u32,
    elapsed: Duration,
) -> Result<()> {
    println!("\n=== 结果 ===");
    println!(
        "样本 {}/{}   超时 {}   异常 {}   耗时 {:.1} s",
        samples.len(),
        count,
        timeouts,
        anomalies,
        elapsed.as_secs_f64()
    );

    if samples.is_empty() {
        return Err(anyhow!(
            "没有任何有效样本（全超时？对端是 `listen` 吗？先跑一次回环：两个终端各起一条命令）"
        ));
    }

    if let (Some(min), Some(p50), Some(p95), Some(p99), Some(max)) = (
        samples.rtt_percentile_us(0),
        samples.rtt_percentile_us(50),
        samples.rtt_percentile_us(95),
        samples.rtt_percentile_us(99),
        samples.rtt_percentile_us(100),
    ) {
        println!("RTT µs: min={min}  P50={p50}  P95={p95}  P99={p99}  max={max}");
    }
    if let (Some(p50), Some(p95)) = (
        samples.rtt_delta_percentile_us(50),
        samples.rtt_delta_percentile_us(95),
    ) {
        println!("相邻 RTT 波动 µs: P50={p50}  P95={p95}（抖动代理指标，§7 抖动缓冲深度的输入）");
    }
    if let Some(offset) = samples.best_offset_us() {
        println!("时钟偏移（best8 中位数，对端 − 本地）= {offset} µs");
    }
    if let Some(spread) = samples.best_offset_spread_us() {
        println!("选中样本 offset 极差 = {spread} µs（§6 收敛要求：稳定后 ≤ 2000 µs）");
    }
    println!(
        "质量分级 = {}（§6.5：Good = 样本 ≥ 8 且代表 RTT ≤ 5 ms）",
        samples.quality().as_str()
    );

    if let Some(p95) = samples.rtt_percentile_us(95) {
        if p95 > 20_000 {
            println!(
                "提示：RTT P95 > 20 ms —— 按 §7 建议从缓冲上限（60 ms）起步，并复核 ADR-004 的回退路径（裸 UDP）"
            );
        } else if p95 > 5_000 {
            println!(
                "提示：RTT P95 > 5 ms —— 局域网一般应更低，检查 Wi-Fi 信道占用 / 省电模式 / 是否有其他大流量"
            );
        }
    }
    Ok(())
}

/// 等待与 `probe_seq` 匹配的应答（忽略其它数据报），超时或连接断开则返回对应结果。
async fn wait_reply(
    connection: &quinn::Connection,
    probe_seq: u32,
    timeout: Duration,
) -> ReplyOutcome {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let received = tokio::time::timeout_at(deadline, connection.read_datagram()).await;
        let bytes = match received {
            Err(_) => return ReplyOutcome::Timeout,
            Ok(Err(error)) => return ReplyOutcome::Failed(error.to_string()),
            Ok(Ok(bytes)) => bytes,
        };
        let Ok(datagram) = AudioDatagram::decode(&bytes) else {
            continue;
        };
        let Ok(reply) = ClockReply::from_datagram(&datagram) else {
            continue;
        };
        if reply.probe_seq == probe_seq {
            return ReplyOutcome::Reply(reply);
        }
    }
}

/// 一次探针的等待结果。
enum ReplyOutcome {
    /// 收到匹配的应答。
    Reply(ClockReply),
    /// 超时。
    Timeout,
    /// 连接中断 / 读失败。
    Failed(String),
}

// ---------------------------------------------------------------------------
// 工具函数与 QUIC 配置
// ---------------------------------------------------------------------------

/// 解析目标地址：支持 `IP:PORT`、裸 `IP`（用协议默认端口）与主机名。
async fn resolve(text: &str, default_port: u16) -> Result<SocketAddr> {
    if let Ok(addr) = text.parse::<SocketAddr>() {
        return Ok(addr);
    }
    if let Ok(ip) = text.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, default_port));
    }
    // 主机名：交给系统解析（例如 `客厅-PC.local`、`localhost`）
    let (host, port) = match text.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() => (
            host.to_string(),
            port.parse::<u16>().context("端口不是合法数字")?,
        ),
        _ => (text.to_string(), default_port),
    };
    let mut resolved = tokio::net::lookup_host((host.as_str(), port))
        .await
        .with_context(|| format!("解析主机名 {host:?} 失败"))?;
    resolved
        .next()
        .ok_or_else(|| anyhow!("主机名 {host:?} 没有解析出任何地址"))
}

/// 解析一个带范围检查的整数参数。
fn parse_u32(value: Option<&String>, flag: &str, min: u32, max: u32) -> Result<u32> {
    let raw = value.ok_or_else(|| anyhow!("{flag} 需要一个数字参数"))?;
    let parsed: u32 = raw
        .parse()
        .with_context(|| format!("{flag} 的取值 {raw:?} 不是合法整数"))?;
    if parsed < min || parsed > max {
        return Err(anyhow!("{flag} 必须在 {min}..={max} 之间，收到 {parsed}"));
    }
    Ok(parsed)
}

/// 进程内单调时钟（§1：不使用挂钟做时间运算）。
fn now_us(start: Instant) -> i64 {
    i64::try_from(start.elapsed().as_micros()).unwrap_or(i64::MAX)
}

/// rustls 加密后端：**强制 ring**（aws-lc-rs 需要 CMake/NASM，见 `docs/06-dev-environment.md` §4）。
fn crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// QUIC 传输参数：显式打开数据报收发缓冲（§3 音频走数据报；这里沿用同一通道做探测）。
fn transport_config() -> Arc<quinn::TransportConfig> {
    let mut transport = quinn::TransportConfig::default();
    transport
        .datagram_receive_buffer_size(Some(1 << 20))
        .datagram_send_buffer_size(1 << 20);
    Arc::new(transport)
}

/// 服务端配置：运行时生成自签证书（工具用，不落盘）。
fn server_config() -> Result<quinn::ServerConfig> {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .context("生成自签证书失败")?;
    let key = PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
    let cert: CertificateDer<'static> = certified.cert.into();

    let crypto = rustls::ServerConfig::builder_with_provider(crypto_provider())
        .with_safe_default_protocol_versions()
        .context("rustls 协议版本配置失败")?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key.into())
        .context("装载自签证书失败")?;

    let quic_crypto = quinn::crypto::rustls::QuicServerConfig::try_from(crypto)
        .map_err(|error| anyhow!("QUIC 加密配置失败：{error}"))?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));
    config.transport_config(transport_config());
    Ok(config)
}

/// 客户端配置：跳过证书校验（仅测量用途，见文件头说明）。
fn client_config() -> Result<quinn::ClientConfig> {
    let crypto = rustls::ClientConfig::builder_with_provider(crypto_provider())
        .with_safe_default_protocol_versions()
        .context("rustls 协议版本配置失败")?
        .dangerous()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_no_client_auth();

    let quic_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
        .map_err(|error| anyhow!("QUIC 加密配置失败：{error}"))?;
    let mut config = quinn::ClientConfig::new(Arc::new(quic_crypto));
    config.transport_config(transport_config());
    Ok(config)
}

/// 把任何证书都当合法的校验器 —— **仅用于本测量工具**，绝不可用于生产路径。
#[derive(Debug)]
struct SkipServerVerification(Arc<rustls::crypto::CryptoProvider>);

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self(Arc::new(rustls::crypto::ring::default_provider())))
    }
}

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
