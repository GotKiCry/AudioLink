//! pcap / pcapng 抓包解析（`alp2-dump --pcap` 的输入层）。
//!
//! 只做「拿到 ALP/2 字节」这件事：文件容器（pcap / pcapng）→ 链路层帧 → IP → UDP 载荷。
//! ALP/2 本身的解释仍归 `audiolink-proto` 的 L1 严格解码层（`alp2-dump` 只是它的现场取证前端）。
//!
//! - [`parse_capture`]：字节 → [`Capture`]（来源信息 + 逐包记录），**不 panic、不猜测**：
//!   容器不合法一律返回 [`PcapError`]；
//! - [`udp_payload`]：链路层帧 → [`UdpPayload`]，不认识的链路层 / 非 UDP 返回 `None`（调用方计数跳过）。
//!
//! 支持范围（够用为准）：pcap 全四种 magic（µs/ns × 大小端）；pcapng 的 SHB / IDB / EPB / SPB；
//! 链路层 Ethernet（含 802.1Q/802.1ad VLAN 标签）、Linux SLL v1、raw IP、loopback(`NULL`)。
//! **不做**：IP 分片重组、IPv6 扩展头链、GRE/PPPoE 等隧道 —— 遇到就当作「不是目标包」跳过，绝不猜。

/// pcap 全局头长度（24 B）。
const PCAP_GLOBAL_HEADER_LEN: usize = 24;

/// pcap 单条记录头长度（16 B）。
const PCAP_RECORD_HEADER_LEN: usize = 16;

/// pcapng Section Header Block 的块类型（固定字节，与端序无关）。
const PCAPNG_SHB: [u8; 4] = [0x0A, 0x0D, 0x0D, 0x0A];

// pcapng 块类型（见 pcapng 规范 §3.2）
const PCAPNG_IDB: u32 = 0x0000_0001;
const PCAPNG_SPB: u32 = 0x0000_0003;
const PCAPNG_EPB: u32 = 0x0000_0006;

// libpcap LINKTYPE_*（只列我们真的会遇到的）
/// `LINKTYPE_NULL`：BSD loopback，前 4 B 是地址族。
const LINKTYPE_NULL: u32 = 0;
/// `LINKTYPE_ETHERNET`：以太网。
const LINKTYPE_ETHERNET: u32 = 1;
/// `LINKTYPE_RAW`：裸 IP。
const LINKTYPE_RAW: u32 = 101;
/// `LINKTYPE_LINUX_SLL`：Linux cooked capture v1（16 B 头）。
const LINKTYPE_LINUX_SLL: u32 = 113;
/// `LINKTYPE_IPV4` / `LINKTYPE_IPV6`：明确声明的 IP 载荷。
const LINKTYPE_IPV4: u32 = 228;
/// 见 [`LINKTYPE_IPV4`]。
const LINKTYPE_IPV6: u32 = 229;

/// 抓包解析失败的原因（一律带「要多少 / 只有多少」，便于定位截断）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PcapError {
    /// 文件比声明的长度短。
    Truncated {
        /// 需要的字节数。
        need: usize,
        /// 实际可用的字节数。
        got: usize,
        /// 正在读什么。
        what: &'static str,
    },
    /// 既不是 pcap 也不是 pcapng。
    UnsupportedFormat {
        /// 文件头 4 字节（原样回显，便于对照）。
        magic: [u8; 4],
    },
    /// pcapng 块的 `total_length` 非法（不是 4 的倍数、太小、或与块尾副本不一致）。
    BadBlockLength {
        /// 块类型。
        block_type: u32,
        /// 块声明的总长度。
        total_length: u32,
    },
    /// 包记录声明的长度超过文件剩余字节。
    BadRecordLength {
        /// 记录声明的长度。
        declared: usize,
        /// 文件里真正剩下的字节数。
        remaining: usize,
    },
}

impl std::fmt::Display for PcapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { need, got, what } => {
                write!(
                    f,
                    "抓包文件在读「{what}」时截断：需要 {need} B，实际只有 {got} B"
                )
            }
            Self::UnsupportedFormat { magic } => write!(
                f,
                "不是 pcap / pcapng（文件头 {:02X} {:02X} {:02X} {:02X}）",
                magic[0], magic[1], magic[2], magic[3]
            ),
            Self::BadBlockLength {
                block_type,
                total_length,
            } => write!(
                f,
                "pcapng 块 0x{block_type:08X} 的 total_length={total_length} 非法"
            ),
            Self::BadRecordLength {
                declared,
                remaining,
            } => write!(f, "pcap 记录声明 {declared} B，文件只剩 {remaining} B"),
        }
    }
}

impl std::error::Error for PcapError {}

/// 抓包文件格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureFormat {
    /// 经典 libpcap 格式。
    Pcap,
    /// pcapng（Wireshark 默认）。
    Pcapng,
}

impl CaptureFormat {
    /// 展示名（日志 / 汇总行）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pcap => "pcap",
            Self::Pcapng => "pcapng",
        }
    }
}

/// 抓包来源信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureSource {
    /// 文件格式。
    pub format: CaptureFormat,
    /// libpcap `LINKTYPE_*`（pcap 取自全局头，pcapng 取自 interface description block）。
    pub link_type: u32,
}

/// 一条抓包记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturePacket {
    /// 时间戳（µs；pcap 的纳秒格式已换算，pcapng 按默认 1 µs 分辨率解释）。
    pub ts_us: u64,
    /// 链路层帧（已按 caplen 截断）。
    pub frame: Vec<u8>,
}

/// 解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    /// 来源信息。
    pub source: CaptureSource,
    /// 按文件顺序的包记录。
    pub packets: Vec<CapturePacket>,
}

/// 输入看起来是不是抓包文件（按 4 字节 magic 判断；**只看格式，不看内容**）。
pub fn looks_like_capture(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(0..4),
        Some([0xD4, 0xC3, 0xB2, 0xA1])
            | Some([0x4D, 0x3C, 0xB2, 0xA1])
            | Some([0xA1, 0xB2, 0xC3, 0xD4])
            | Some([0xA1, 0xB2, 0x3C, 0x4D])
            | Some([0x0A, 0x0D, 0x0D, 0x0A])
    )
}

/// 解析 pcap / pcapng 文件。
pub fn parse_capture(bytes: &[u8]) -> Result<Capture, PcapError> {
    let magic = bytes.get(0..4).ok_or(PcapError::Truncated {
        need: 4,
        got: bytes.len(),
        what: "文件头 magic",
    })?;
    match magic {
        [0x0A, 0x0D, 0x0D, 0x0A] => parse_pcapng(bytes),
        [0xD4, 0xC3, 0xB2, 0xA1] => parse_pcap(bytes, true, false),
        [0x4D, 0x3C, 0xB2, 0xA1] => parse_pcap(bytes, true, true),
        [0xA1, 0xB2, 0xC3, 0xD4] => parse_pcap(bytes, false, false),
        [0xA1, 0xB2, 0x3C, 0x4D] => parse_pcap(bytes, false, true),
        _ => Err(PcapError::UnsupportedFormat {
            magic: [magic[0], magic[1], magic[2], magic[3]],
        }),
    }
}

/// 经典 pcap：`24 B 全局头 + N × (16 B 记录头 + caplen 字节)`。
fn parse_pcap(
    bytes: &[u8],
    little_endian: bool,
    ts_nanoseconds: bool,
) -> Result<Capture, PcapError> {
    let header = bytes
        .get(..PCAP_GLOBAL_HEADER_LEN)
        .ok_or(PcapError::Truncated {
            need: PCAP_GLOBAL_HEADER_LEN,
            got: bytes.len(),
            what: "pcap 全局头",
        })?;
    let link_type = read_u32(header, 20, little_endian)?;
    let mut packets = Vec::new();
    let mut offset = PCAP_GLOBAL_HEADER_LEN;
    while bytes.len().saturating_sub(offset) >= PCAP_RECORD_HEADER_LEN {
        let ts_sec = read_u32(bytes, offset, little_endian)? as u64;
        let ts_frac = read_u32(bytes, offset + 4, little_endian)? as u64;
        let incl_len = read_u32(bytes, offset + 8, little_endian)? as usize;
        offset += PCAP_RECORD_HEADER_LEN;
        let remaining = bytes.len() - offset;
        let frame = bytes
            .get(offset..offset + incl_len)
            .ok_or(PcapError::BadRecordLength {
                declared: incl_len,
                remaining,
            })?;
        packets.push(CapturePacket {
            ts_us: ts_sec * 1_000_000
                + if ts_nanoseconds {
                    ts_frac / 1_000
                } else {
                    ts_frac
                },
            frame: frame.to_vec(),
        });
        offset += incl_len;
    }
    Ok(Capture {
        source: CaptureSource {
            format: CaptureFormat::Pcap,
            link_type,
        },
        packets,
    })
}

/// pcapng：一串自描述的块；只提取 IDB（链路类型）与 EPB / SPB（包）。
fn parse_pcapng(bytes: &[u8]) -> Result<Capture, PcapError> {
    let mut packets = Vec::new();
    let mut link_type: u32 = LINKTYPE_ETHERNET;
    let mut little_endian = false;
    let mut offset = 0usize;
    let mut saw_section = false;

    while bytes.len().saturating_sub(offset) >= 12 {
        let is_shb = bytes.get(offset..offset + 4) == Some(&PCAPNG_SHB[..]);
        if is_shb {
            // Section Header Block 的 byte-order magic 在偏移 8；它决定本节其余块的端序。
            let bom = bytes4(bytes, offset + 8, "pcapng byte-order magic")?;
            little_endian = match bom {
                [0x4D, 0x3C, 0x2B, 0x1A] => true,
                [0x1A, 0x2B, 0x3C, 0x4D] => false,
                _ => {
                    return Err(PcapError::UnsupportedFormat { magic: bom });
                }
            };
            saw_section = true;
        } else if !saw_section {
            let magic = bytes4(bytes, offset, "块类型")?;
            return Err(PcapError::UnsupportedFormat { magic });
        }

        let block_type = read_u32(bytes, offset, little_endian)?;
        let total_length = read_u32(bytes, offset + 4, little_endian)?;
        let total = total_length as usize;
        if total < 12 || !total.is_multiple_of(4) {
            return Err(PcapError::BadBlockLength {
                block_type,
                total_length,
            });
        }
        let block_end = offset.checked_add(total).ok_or(PcapError::BadBlockLength {
            block_type,
            total_length,
        })?;
        if block_end > bytes.len() {
            return Err(PcapError::Truncated {
                need: block_end,
                got: bytes.len(),
                what: "pcapng 块体",
            });
        }
        // 块尾必须重复一遍 total_length（自校验，防把垃圾当块头）
        let trailer = read_u32(bytes, block_end - 4, little_endian)?;
        if trailer != total_length {
            return Err(PcapError::BadBlockLength {
                block_type,
                total_length,
            });
        }

        let body = offset + 8;
        let body_len = total - 12;
        match block_type {
            PCAPNG_IDB if body_len >= 8 => {
                link_type = read_u16(bytes, body, little_endian)? as u32;
            }
            PCAPNG_EPB if body_len >= 20 => {
                let ts_high = read_u32(bytes, body + 4, little_endian)? as u64;
                let ts_low = read_u32(bytes, body + 8, little_endian)? as u64;
                let caplen = read_u32(bytes, body + 12, little_endian)? as usize;
                let data_start = body + 20;
                let remaining = bytes.len() - data_start;
                let frame = bytes.get(data_start..data_start + caplen).ok_or(
                    PcapError::BadRecordLength {
                        declared: caplen,
                        remaining,
                    },
                )?;
                packets.push(CapturePacket {
                    ts_us: ts_high << 32 | ts_low,
                    frame: frame.to_vec(),
                });
            }
            PCAPNG_SPB if body_len >= 4 => {
                let caplen = read_u32(bytes, body, little_endian)? as usize;
                let data_start = body + 4;
                let remaining = bytes.len() - data_start;
                let frame = bytes.get(data_start..data_start + caplen).ok_or(
                    PcapError::BadRecordLength {
                        declared: caplen,
                        remaining,
                    },
                )?;
                packets.push(CapturePacket {
                    ts_us: 0,
                    frame: frame.to_vec(),
                });
            }
            _ => {}
        }
        offset = block_end;
    }

    Ok(Capture {
        source: CaptureSource {
            format: CaptureFormat::Pcapng,
            link_type,
        },
        packets,
    })
}

/// UDP 载荷（借用帧缓冲）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UdpPayload<'a> {
    /// 源端口。
    pub src_port: u16,
    /// 目的端口。
    pub dst_port: u16,
    /// UDP 载荷（QUIC 数据报 / ALP/2 报文的所在）。
    pub payload: &'a [u8],
}

/// 从链路层帧里剥出 UDP 载荷。
///
/// 不认识的链路类型、非 IP、非 UDP（含分片）一律返回 `None` —— 调用方按「不是目标包」计数跳过，
/// 而不是把它当成解码失败（那是两件事：一个是抓包噪声，一个是协议缺陷）。
pub fn udp_payload(link_type: u32, frame: &[u8]) -> Option<UdpPayload<'_>> {
    let ip_offset = match link_type {
        LINKTYPE_ETHERNET => ethernet_ip_offset(frame)?,
        LINKTYPE_LINUX_SLL => 16,
        LINKTYPE_RAW | LINKTYPE_IPV4 | LINKTYPE_IPV6 => 0,
        LINKTYPE_NULL => 4,
        _ => return None,
    };
    parse_ip(frame, ip_offset)
}

/// 以太网：跳过 MAC(12) + EtherType(2)，并吃掉 802.1Q / 802.1ad 的 VLAN 标签。
fn ethernet_ip_offset(frame: &[u8]) -> Option<usize> {
    let mut offset = 12usize;
    let mut ethertype = u16::from_be_bytes([*frame.get(offset)?, *frame.get(offset + 1)?]);
    offset += 2;
    while matches!(ethertype, 0x8100 | 0x88A8 | 0x9100) {
        ethertype = u16::from_be_bytes([*frame.get(offset + 2)?, *frame.get(offset + 3)?]);
        offset += 4;
    }
    match ethertype {
        0x0800 | 0x86DD => Some(offset),
        _ => None,
    }
}

/// IPv4 / IPv6 → UDP。IPv6 扩展头不跟进（返回 `None`）。
fn parse_ip(frame: &[u8], ip_offset: usize) -> Option<UdpPayload<'_>> {
    let first = *frame.get(ip_offset)?;
    let (protocol, header_len) = match first >> 4 {
        4 => {
            let ihl = usize::from(first & 0x0F) * 4;
            if ihl < 20 {
                return None;
            }
            (*frame.get(ip_offset + 9)?, ihl)
        }
        6 => (*frame.get(ip_offset + 6)?, 40),
        _ => return None,
    };
    if protocol != 17 {
        return None; // 只看 UDP
    }
    let udp_offset = ip_offset + header_len;
    let src_port = u16::from_be_bytes([*frame.get(udp_offset)?, *frame.get(udp_offset + 1)?]);
    let dst_port = u16::from_be_bytes([*frame.get(udp_offset + 2)?, *frame.get(udp_offset + 3)?]);
    let udp_len = usize::from(u16::from_be_bytes([
        *frame.get(udp_offset + 4)?,
        *frame.get(udp_offset + 5)?,
    ]));
    let available = frame.len().checked_sub(udp_offset + 8)?;
    // `udp_len` 小于 8 说明发送侧用了 GSO / 未填长度：此时按「剩下的全是载荷」处理，
    // 而不是把整包当噪声丢掉。
    let take = if udp_len >= 8 {
        (udp_len - 8).min(available)
    } else {
        available
    };
    let payload = frame.get(udp_offset + 8..udp_offset + 8 + take)?;
    Some(UdpPayload {
        src_port,
        dst_port,
        payload,
    })
}

fn bytes2(bytes: &[u8], offset: usize, what: &'static str) -> Result<[u8; 2], PcapError> {
    let end = offset.checked_add(2).ok_or(PcapError::Truncated {
        need: offset,
        got: bytes.len(),
        what,
    })?;
    let slice = bytes.get(offset..end).ok_or(PcapError::Truncated {
        need: end,
        got: bytes.len(),
        what,
    })?;
    let mut out = [0u8; 2];
    out.copy_from_slice(slice);
    Ok(out)
}

fn bytes4(bytes: &[u8], offset: usize, what: &'static str) -> Result<[u8; 4], PcapError> {
    let end = offset.checked_add(4).ok_or(PcapError::Truncated {
        need: offset,
        got: bytes.len(),
        what,
    })?;
    let slice = bytes.get(offset..end).ok_or(PcapError::Truncated {
        need: end,
        got: bytes.len(),
        what,
    })?;
    let mut out = [0u8; 4];
    out.copy_from_slice(slice);
    Ok(out)
}

fn read_u16(bytes: &[u8], offset: usize, little_endian: bool) -> Result<u16, PcapError> {
    let raw = bytes2(bytes, offset, "u16 字段")?;
    Ok(if little_endian {
        u16::from_le_bytes(raw)
    } else {
        u16::from_be_bytes(raw)
    })
}

fn read_u32(bytes: &[u8], offset: usize, little_endian: bool) -> Result<u32, PcapError> {
    let raw = bytes4(bytes, offset, "u32 字段")?;
    Ok(if little_endian {
        u32::from_le_bytes(raw)
    } else {
        u32::from_be_bytes(raw)
    })
}
