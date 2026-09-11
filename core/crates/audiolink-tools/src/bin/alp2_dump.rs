//! `alp2-dump` —— 把 ALP/2 抓包（hex 文本）翻译成人类可读字段。
//!
//! 规格：`docs/03-protocol.md` §3（音频数据报）/ §4（控制帧）/ §9（发现报文）。
//! 解码走内核的 **L1 严格解码层**，失败时打印它给出的拒绝上下文 —— 因此这个工具同时是「拒绝用例」的现场取证工具。
//!
//! ```text
//! alp2-dump [FILE|-] [--single] [--quiet]
//! ```
//!
//! - 默认**每行一个报文**（可直接粘 `docs/03-protocol.md` §12 的向量行）；
//! - `--single`：整个输入当作**一个**报文（多行拼接）；
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
use audiolink_tools::{hex_preview, parse_hex_bytes};
use audiolink_types::DATAGRAM_HEADER_LEN;
use audiolink_types::Ptype;

const USAGE: &str = "\
alp2-dump —— ALP/2 报文解码（docs/03-protocol.md §3/§4/§9）

用法:
    alp2-dump [FILE|-] [--single] [--quiet]

参数:
    FILE        输入文件；`-` 或省略 → 读 stdin
    --single    整个输入当作一个报文（默认：每行一个报文）
    --quiet     只输出解码失败的报文

输入格式:
    hex 文本；容忍 `|` 分隔、`0x` 前缀、`//` 与 `#` 注释、`<...>` 占位
    （例：`02 01 00 00 | 01 00 00 00 | 2A 00 00 00 | C0 03 00 00 | 88 77 66 55 44 33 22 11 | DE AD BE EF`）
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
    let mut source: Option<String> = None;

    for arg in args {
        match arg.as_str() {
            "--single" => single = true,
            "--quiet" => quiet = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(ExitCode::SUCCESS);
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
    }

    let text = match source.as_deref() {
        None | Some("-") => {
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .map_err(|error| format!("读 stdin 失败：{error}"))?;
            buffer
        }
        Some(path) => {
            fs::read_to_string(path).map_err(|error| format!("读 {path} 失败：{error}"))?
        }
    };

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
