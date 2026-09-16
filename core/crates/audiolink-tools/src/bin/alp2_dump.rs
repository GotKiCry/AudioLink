//! `alp2-dump` —— 把 ALP/2 抓包（hex 文本 / pcap / pcapng）翻译成人类可读字段。
//!
//! 规格：`docs/03-protocol.md` §3（音频数据报）/ §4（控制帧）/ §9（发现报文）。
//! 解码走内核的 **L1 严格解码层**，失败时打印它给出的拒绝上下文 —— 因此这个工具同时是「拒绝用例」的现场取证工具。
//!
//! ```text
//! alp2-dump [FILE|-] [--single] [--quiet] [--pcap] [--port N] [--all-ports]
//! ```
//!
//! - 默认**每行一个报文**（可直接粘 `docs/03-protocol.md` §12 的向量行）；
//! - **输入按内容自动分派**：pcap / pcapng（按 4 字节 magic）走抓包解析，其余按 hex 文本；`--pcap` 可强制抓包分支；
//! - `--single`：整个输入当作**一个**报文（多行拼接；仅 hex 文本）；
//! - 抓包分支只解 `--port`（默认 §1 的 58290）上的 UDP 载荷；`--all-ports` 关掉该过滤；
//! - 解码顺序：数据报 → 控制帧 → 发现报文；
//! - 退出码：0 = 全部解码成功；1 = 有报文解码失败；2 = 用法 / 输入错误。

use std::fs;
use std::io::Read;
use std::process::ExitCode;

use audiolink_proto::control::MIN_FRAME_LEN;
use audiolink_proto::discovery::MAGIC as DISCOVERY_MAGIC;
use audiolink_proto::{
    AudioDatagram, ClockProbe, ClockReply, ControlFrame, ControlFrameHeader, DiscoveryBeacon,
    NackList,
};
use audiolink_tools::pcap::{self, Capture};
use audiolink_tools::{hex_preview, parse_hex_bytes};
use audiolink_types::DATAGRAM_HEADER_LEN;
use audiolink_types::DEFAULT_QUIC_PORT;
use audiolink_types::Ptype;

const USAGE: &str = "\
alp2-dump —— ALP/2 报文解码（docs/03-protocol.md §3/§4/§9）

用法:
    alp2-dump [FILE|-] [--single] [--quiet] [--pcap] [--port N] [--all-ports]

参数:
    FILE        输入文件；`-` 或省略 → 读 stdin
    --single    整个输入当作一个报文（默认：每行一个报文；仅 hex 文本）
    --quiet     只输出解码失败的报文
    --pcap      强制把输入当作 pcap / pcapng 抓包（按 magic 也会自动识别）
    --port N    抓包分支只解该 UDP 端口（默认 58290，§1）
    --all-ports 抓包分支不过滤端口

输入格式:
    1) hex 文本；容忍 `|` 分隔、`0x` 前缀、`//` 与 `#` 注释、`<...>` 占位
       （例：`02 01 00 00 | 01 00 00 00 | 2A 00 00 00 | C0 03 00 00 | 88 77 66 55 44 33 22 11 | DE AD BE EF`）
    2) pcap / pcapng 抓包（Wireshark / tcpdump / dumpcap 直接产物）：剥出 UDP 载荷后再解 ALP/2，
       以太网 VLAN 标签、IPv4 / IPv6、Linux SLL 都能认；IP 分片与 IPv6 扩展头不跟进（按非目标包跳过）
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("alp2-dump: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let mut single = false;
    let mut quiet = false;
    let mut force_capture = false;
    let mut all_ports = false;
    let mut port = DEFAULT_QUIC_PORT;
    let mut source: Option<String> = None;

    let mut index = 0_usize;
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "--single" => single = true,
            "--quiet" => quiet = true,
            "--pcap" => force_capture = true,
            "--all-ports" => all_ports = true,
            "--port" => {
                index += 1;
                let value = args.get(index).ok_or("--port 后面要跟一个端口号")?;
                port = parse_port(value)?;
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            other if other.starts_with("--port=") => {
                let value = other.strip_prefix("--port=").unwrap_or_default();
                port = parse_port(value)?;
            }
            other if other.starts_with("--") => {
                return Err(format!("未知参数 {other}（--help 看用法）"));
            }
            other => {
                if source.is_some() {
                    return Err("只能指定一个输入文件".to_string());
                }
                source = Some(other.to_string());
            }
        }
        index += 1;
    }

    let bytes = read_input(source.as_deref())?;

    // 输入按内容分派：抓包走 pcap 分支（先剥 UDP 载荷），其余仍按 hex 文本。
    if force_capture || pcap::looks_like_capture(&bytes) {
        if single {
            return Err("--single 只对 hex 文本输入有意义，不能与抓包输入同时使用".to_string());
        }
        let capture =
            pcap::parse_capture(&bytes).map_err(|error| format!("抓包解析失败：{error}"))?;
        let filter = if all_ports { None } else { Some(port) };
        return run_capture(&capture, filter, quiet);
    }

    let text = String::from_utf8(bytes)
        .map_err(|error| format!("输入既不是 pcap/pcapng，也不是 UTF-8 文本：{error}"))?;

    if single {
        let bytes = parse_hex_bytes(&text).map_err(|error| format!("hex 解析失败：{error}"))?;
        let ok = report_one(1, &bytes, quiet);
        return Ok(exit_code(ok));
    }

    let mut total = 0_usize;
    let mut failed = 0_usize;
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        total += 1;
        let bytes = match parse_hex_bytes(line) {
            Ok(bytes) => bytes,
            Err(error) => {
                failed += 1;
                println!("[{index}]  hex 解析失败：{error}");
                continue;
            }
        };
        if !report_one(index, &bytes, quiet) {
            failed += 1;
        }
    }

    if total == 0 {
        return Err("输入里没有可解析的行（每行一个 hex 报文；--help 看用法）".to_string());
    }
    println!(
        "\n共 {total} 个报文，解码成功 {}，失败 {failed}",
        total - failed
    );
    Ok(exit_code(failed == 0))
}

/// 解析端口号。
fn parse_port(text: &str) -> Result<u16, String> {
    text.parse::<u16>()
        .map_err(|error| format!("端口号 {text:?} 非法：{error}"))
}

/// 读输入（文件或 stdin）。**按字节读** —— 抓包是二进制，不能先当文本解。
fn read_input(source: Option<&str>) -> Result<Vec<u8>, String> {
    match source {
        None | Some("-") => {
            let mut buffer = Vec::new();
            std::io::stdin()
                .read_to_end(&mut buffer)
                .map_err(|error| format!("读 stdin 失败：{error}"))?;
            Ok(buffer)
        }
        Some(path) => fs::read(path).map_err(|error| format!("读 {path} 失败：{error}")),
    }
}

/// 抓包分支：逐包剥 UDP 载荷 → 交给 L1 严格解码层。
///
/// 「剥不出 UDP 载荷」与「ALP/2 解码失败」是两件事：前者是抓包噪声（超时 / HTTP / 别的 UDP 服务），
/// 计入 `skipped` 而不是 `failed` —— 否则一次普通抓包会被噪声淹没成一片红。
fn run_capture(capture: &Capture, filter: Option<u16>, quiet: bool) -> Result<ExitCode, String> {
    let mut total = 0_usize;
    let mut failed = 0_usize;
    let mut skipped = 0_usize;

    for (index, packet) in capture.packets.iter().enumerate() {
        let Some(udp) = pcap::udp_payload(capture.source.link_type, &packet.frame) else {
            skipped += 1;
            continue;
        };
        if let Some(port) = filter
            && udp.src_port != port
            && udp.dst_port != port
        {
            skipped += 1;
            continue;
        }
        total += 1;
        if !report_udp(index, &udp, quiet) {
            failed += 1;
        }
    }

    if total == 0 {
        return Err(format!(
            "抓包里没有目标 UDP 报文（{} / 链路类型 {}：共 {} 个包，跳过 {skipped} 个非目标包）—— 用 --all-ports 或 --port N 改过滤条件",
            capture.source.format.name(),
            capture.source.link_type,
            capture.packets.len(),
        ));
    }
    println!(
        "\n共 {total} 个 UDP 报文（{} / 链路类型 {}，跳过 {skipped} 个非目标包），解码成功 {}，失败 {failed}",
        capture.source.format.name(),
        capture.source.link_type,
        total - failed,
    );
    Ok(exit_code(failed == 0))
}

/// 解码并打印抓包里的一个报文；返回是否成功。
fn report_udp(index: usize, udp: &pcap::UdpPayload<'_>, quiet: bool) -> bool {
    let route = format!("udp {}→{}", udp.src_port, udp.dst_port);
    match describe(udp.payload) {
        Ok(text) => {
            if !quiet {
                println!("[{index}] {} B  {route}  → {text}", udp.payload.len());
            }
            true
        }
        Err(reason) => {
            println!("[{index}] {} B  {route}  → ✗ {reason}", udp.payload.len());
            false
        }
    }
}

fn exit_code(all_ok: bool) -> ExitCode {
    if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// 解码并打印一个报文；返回是否成功。
fn report_one(index: usize, bytes: &[u8], quiet: bool) -> bool {
    match describe(bytes) {
        Ok(text) => {
            if !quiet {
                println!("[{index}] {} B  → {text}", bytes.len());
            }
            true
        }
        Err(reason) => {
            println!("[{index}] {} B  → ✗ {reason}", bytes.len());
            false
        }
    }
}

/// 依次尝试三种载体，返回人类可读描述；全部失败 → 返回各层的拒绝原因。
///
/// 只尝试**结构上可能的**载体（长度 / 魔数预筛），否则「数据报帧头截断」这类噪声会淹没真正的原因。
fn describe(bytes: &[u8]) -> Result<String, String> {
    let mut errors = Vec::new();

    if bytes.len() >= DATAGRAM_HEADER_LEN {
        match describe_datagram(bytes) {
            Ok(text) => return Ok(text),
            Err(error) => errors.push(format!("数据报：{error}")),
        }
    }
    if bytes.len() >= MIN_FRAME_LEN {
        match describe_control(bytes) {
            Ok(text) => return Ok(text),
            Err(error) => errors.push(format!("控制帧：{error}")),
        }
    }
    if bytes.starts_with(DISCOVERY_MAGIC) {
        match describe_discovery(bytes) {
            Ok(text) => return Ok(text),
            Err(error) => errors.push(format!("发现报文：{error}")),
        }
    }

    if errors.is_empty() {
        return Err(format!(
            "{} B：不足以构成任何已知报文（数据报 ≥ {DATAGRAM_HEADER_LEN} B、控制帧 ≥ {MIN_FRAME_LEN} B、发现报文需魔法 {})",
            bytes.len(),
            String::from_utf8_lossy(DISCOVERY_MAGIC)
        ));
    }
    Err(errors.join(" ｜ "))
}

/// 音频数据报（§3）。
fn describe_datagram(bytes: &[u8]) -> Result<String, String> {
    let datagram = AudioDatagram::decode(bytes).map_err(|error| error.to_string())?;
    let header = &datagram.header;
    let mut out = format!(
        "数据报 {}(0x{:02X})  ver=0x{:02X} flags=0x{:04X} stream_id={} seq={} sample_index={} epoch_id=0x{:016X}",
        header.ptype.name(),
        header.ptype.as_u8(),
        header.version,
        header.flags.bits(),
        header.stream_id,
        header.seq,
        header.sample_index,
        header.epoch_id,
    );

    match header.ptype {
        Ptype::ClockProbe => {
            let probe = ClockProbe::decode(datagram.payload).map_err(|error| error.to_string())?;
            out.push_str(&format!(
                "\n        CLOCK_PROBE probe_seq={} t1={} µs",
                probe.probe_seq, probe.t1
            ));
        }
        Ptype::ClockReply => {
            let reply = ClockReply::decode(datagram.payload).map_err(|error| error.to_string())?;
            out.push_str(&format!(
                "\n        CLOCK_REPLY probe_seq={} t1={} µs t2={} µs t3={} µs",
                reply.probe_seq, reply.t1, reply.t2, reply.t3
            ));
        }
        Ptype::Nack => {
            let nack = NackList::decode(datagram.payload).map_err(|error| error.to_string())?;
            out.push_str(&format!(
                "\n        NACK {} 项: {:?}",
                nack.seqs().len(),
                nack.seqs()
            ));
        }
        Ptype::Keepalive => out.push_str("\n        KEEPALIVE 无载荷"),
        Ptype::Audio | audiolink_types::Ptype::Fec => out.push_str(&format!(
            "\n        载荷 {} B: {}",
            datagram.payload.len(),
            hex_preview(datagram.payload, 32)
        )),
    }
    Ok(out)
}

/// 控制帧（§4）；结构合法但被语义拒绝时（版本 / 命令码 / flags），把宽容信封一并报出来便于定位。
fn describe_control(bytes: &[u8]) -> Result<String, String> {
    match ControlFrame::decode(bytes) {
        Ok(frame) => Ok(format!(
            "控制帧 {}(0x{:02X})  ver=0x02 flags=0x0000 payload_len={} request_id={}\n        载荷 {} B: {}",
            frame.op.name(),
            frame.op.as_u8(),
            frame.payload.len(),
            frame.request_id,
            frame.payload.len(),
            hex_preview(frame.payload, 32)
        )),
        Err(error) => match ControlFrameHeader::decode(bytes) {
            Ok(header) => Err(format!(
                "结构合法但被拒（{}）：ver=0x{:02X} op=0x{:02X} flags=0x{:04X} payload_len={} request_id={}",
                error.context(),
                header.ver,
                header.raw_op,
                header.flags,
                header.payload_len,
                header.request_id
            )),
            Err(_) => Err(error.to_string()),
        },
    }
}

/// 发现报文（§9.2）。
fn describe_discovery(bytes: &[u8]) -> Result<String, String> {
    let beacon = DiscoveryBeacon::decode(bytes).map_err(|error| error.to_string())?;
    Ok(format!(
        "发现报文  ver={} proto=0x{:04X} id={} name={:?} platform={} caps={} port={}",
        beacon.ver,
        beacon.proto,
        beacon.id,
        beacon.name,
        beacon.platform.as_str(),
        beacon.caps.to_hex(),
        beacon.port
    ))
}
