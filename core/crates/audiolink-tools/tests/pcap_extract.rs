//! `alp2-dump --pcap` 的输入层回归：pcap / pcapng → 逐包 UDP 载荷 → ALP/2。
//!
//! 夹具全部**现场合成**（不引外部二进制夹具）：以太网 / VLAN / IPv4 / IPv6 / Linux SLL 的帧头按
//! RFC 逐字节拼出来，pcap 与 pcapng 容器同理 —— 这样断言的期望值直接来自规范，而不是来自实现自己。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use audiolink_proto::AudioDatagram;
use audiolink_tools::pcap::{
    CaptureFormat, PcapError, looks_like_capture, parse_capture, udp_payload,
};
use audiolink_types::Ptype;

/// `docs/03-protocol.md` §12 示例 2：CLOCK_PROBE 报文（36 B = 24 B 帧头 + 12 B 载荷）。
const VECTOR_CLOCK_PROBE: &[u8] = &[
    0x02, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x40, 0x42, 0x0F, 0x00,
    0x00, 0x00, 0x00, 0x00,
];

const ALP_PORT: u16 = 58290;

/// 以太网 + IPv4 + UDP 帧。
fn ethernet_ipv4_udp_frame(payload: &[u8], src_port: u16, dst_port: u16) -> Vec<u8> {
    let mut frame = vec![0xAA; 6];
    frame.extend_from_slice(&[0xBB; 6]);
    frame.extend_from_slice(&0x0800u16.to_be_bytes());
    frame.extend_from_slice(&ipv4_udp_body(payload, src_port, dst_port));
    frame
}

/// IPv4 头（20 B，无选项）+ UDP 头 + 载荷。
fn ipv4_udp_body(payload: &[u8], src_port: u16, dst_port: u16) -> Vec<u8> {
    let udp_len = 8 + payload.len();
    let total_len = 20 + udp_len;
    let mut out = vec![0x45, 0x00];
    out.extend_from_slice(&(total_len as u16).to_be_bytes());
    out.extend_from_slice(&[0x00, 0x00]); // id
    out.extend_from_slice(&[0x00, 0x00]); // flags + fragment offset
    out.push(64); // ttl
    out.push(17); // protocol = UDP
    out.extend_from_slice(&[0x00, 0x00]); // checksum
    out.extend_from_slice(&[192, 168, 3, 200]);
    out.extend_from_slice(&[172, 16, 2, 54]);
    out.extend_from_slice(&udp_header(payload, src_port, dst_port));
    out
}

/// IPv6 头（40 B）+ UDP 头 + 载荷。
fn ipv6_udp_body(payload: &[u8], src_port: u16, dst_port: u16) -> Vec<u8> {
    let mut out = vec![0x60, 0x00, 0x00, 0x00];
    out.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    out.push(17); // next header = UDP
    out.push(64); // hop limit
    out.extend_from_slice(&[0x20, 0x01, 0x0D, 0xB8]);
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&[0xFE, 0x80]);
    out.extend_from_slice(&[0u8; 14]);
    out.extend_from_slice(&udp_header(payload, src_port, dst_port));
    out
}

fn udp_header(payload: &[u8], src_port: u16, dst_port: u16) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&src_port.to_be_bytes());
    out.extend_from_slice(&dst_port.to_be_bytes());
    out.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    out.extend_from_slice(&[0x00, 0x00]); // checksum（工具不校验，抓包常为 0）
    out.extend_from_slice(payload);
    out
}

/// pcap 文件（小端）。`nanoseconds = true` 时用纳秒 magic。
fn pcap_file(link_type: u32, packets: &[(u64, Vec<u8>)], nanoseconds: bool) -> Vec<u8> {
    let mut out = if nanoseconds {
        vec![0x4D, 0x3C, 0xB2, 0xA1]
    } else {
        vec![0xD4, 0xC3, 0xB2, 0xA1]
    };
    out.extend_from_slice(&2u16.to_le_bytes()); // version major
    out.extend_from_slice(&4u16.to_le_bytes()); // version minor
    out.extend_from_slice(&0i32.to_le_bytes()); // thiszone
    out.extend_from_slice(&0u32.to_le_bytes()); // sigfigs
    out.extend_from_slice(&65535u32.to_le_bytes()); // snaplen
    out.extend_from_slice(&link_type.to_le_bytes());
    for (ts_us, frame) in packets {
        let seconds = ts_us / 1_000_000;
        let micros = ts_us % 1_000_000;
        out.extend_from_slice(&(seconds as u32).to_le_bytes());
        let fraction = if nanoseconds { micros * 1_000 } else { micros };
        out.extend_from_slice(&(fraction as u32).to_le_bytes());
        out.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        out.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        out.extend_from_slice(frame);
    }
    out
}

/// pcapng 块（小端，自动 4 B 对齐并回填块尾长度）。
fn pcapng_block(block_type: u32, body: &[u8]) -> Vec<u8> {
    let mut padded = body.to_vec();
    while !padded.len().is_multiple_of(4) {
        padded.push(0);
    }
    let total = (12 + padded.len()) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(&block_type.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&padded);
    out.extend_from_slice(&total.to_le_bytes());
    out
}

/// pcapng 文件：SHB + IDB + 若干 EPB。
fn pcapng_file(link_type: u16, packets: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut shb = Vec::new();
    shb.extend_from_slice(&0x1A2B_3C4Du32.to_le_bytes()); // byte-order magic（小端写出）
    shb.extend_from_slice(&1u16.to_le_bytes());
    shb.extend_from_slice(&0u16.to_le_bytes());
    shb.extend_from_slice(&(-1i64).to_le_bytes());
    out.extend_from_slice(&pcapng_block(0x0A0D_0D0A, &shb));

    let mut idb = Vec::new();
    idb.extend_from_slice(&link_type.to_le_bytes());
    idb.extend_from_slice(&0u16.to_le_bytes()); // reserved
    idb.extend_from_slice(&65535u32.to_le_bytes()); // snaplen
    out.extend_from_slice(&pcapng_block(0x0000_0001, &idb));

    for (ts_us, frame) in packets {
        let mut epb = Vec::new();
        epb.extend_from_slice(&0u32.to_le_bytes()); // interface id
        epb.extend_from_slice(&((ts_us >> 32) as u32).to_le_bytes());
        epb.extend_from_slice(&((ts_us & 0xFFFF_FFFF) as u32).to_le_bytes());
        epb.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        epb.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        epb.extend_from_slice(frame);
        out.extend_from_slice(&pcapng_block(0x0000_0006, &epb));
    }
    out
}

#[test]
fn pcap_ethernet_ipv4_udp_is_unwrapped_and_decoded() {
    let frame = ethernet_ipv4_udp_frame(VECTOR_CLOCK_PROBE, ALP_PORT, 48210);
    let file = pcap_file(1, &[(1_700_000_000_000_000, frame.clone())], false);
    assert!(looks_like_capture(&file), "pcap magic 必须被识别");

    let capture = parse_capture(&file).unwrap();
    assert_eq!(capture.source.format, CaptureFormat::Pcap);
    assert_eq!(capture.source.link_type, 1);
    assert_eq!(capture.packets.len(), 1);
    assert_eq!(capture.packets[0].ts_us, 1_700_000_000_000_000);
    assert_eq!(capture.packets[0].frame, frame);

    let udp = udp_payload(capture.source.link_type, &capture.packets[0].frame).unwrap();
    assert_eq!(udp.src_port, ALP_PORT);
    assert_eq!(udp.dst_port, 48210);
    assert_eq!(
        udp.payload, VECTOR_CLOCK_PROBE,
        "剥出来的必须是原始 ALP/2 字节"
    );

    // 端到端：抓包 → L1 严格解码
    let datagram = AudioDatagram::decode(udp.payload).unwrap();
    assert_eq!(datagram.header.ptype, Ptype::ClockProbe);
}

#[test]
fn pcapng_enhanced_packet_block_is_unwrapped() {
    let frame = ethernet_ipv4_udp_frame(VECTOR_CLOCK_PROBE, ALP_PORT, 48210);
    let file = pcapng_file(1, &[(1_700_000_000_123_456, frame)]);
    assert!(looks_like_capture(&file), "pcapng magic 必须被识别");

    let capture = parse_capture(&file).unwrap();
    assert_eq!(capture.source.format, CaptureFormat::Pcapng);
    assert_eq!(capture.source.link_type, 1, "链路类型取自 IDB");
    assert_eq!(capture.packets.len(), 1);
    assert_eq!(capture.packets[0].ts_us, 1_700_000_000_123_456);
    let udp = udp_payload(capture.source.link_type, &capture.packets[0].frame).unwrap();
    assert_eq!(udp.payload, VECTOR_CLOCK_PROBE);
}

#[test]
fn nanosecond_pcap_timestamps_are_converted_to_micros() {
    let frame = ethernet_ipv4_udp_frame(VECTOR_CLOCK_PROBE, ALP_PORT, 1);
    let file = pcap_file(1, &[(1_700_000_000_000_000, frame)], true);
    let capture = parse_capture(&file).unwrap();
    assert_eq!(capture.packets[0].ts_us, 1_700_000_000_000_000);
}

#[test]
fn vlan_tags_are_stripped_before_ip() {
    let mut frame = vec![0xAA; 6];
    frame.extend_from_slice(&[0xBB; 6]);
    frame.extend_from_slice(&0x8100u16.to_be_bytes()); // 802.1Q
    frame.extend_from_slice(&0x0064u16.to_be_bytes()); // VLAN id 100
    frame.extend_from_slice(&0x8100u16.to_be_bytes()); // 再来一层（QinQ）
    frame.extend_from_slice(&0x00C8u16.to_be_bytes());
    frame.extend_from_slice(&0x0800u16.to_be_bytes());
    frame.extend_from_slice(&ipv4_udp_body(VECTOR_CLOCK_PROBE, ALP_PORT, 2));

    let udp = udp_payload(1, &frame).unwrap();
    assert_eq!(udp.payload, VECTOR_CLOCK_PROBE);
}

#[test]
fn ipv6_and_linux_sll_are_supported() {
    let ipv6 = ipv6_udp_body(VECTOR_CLOCK_PROBE, ALP_PORT, 3);
    let udp = udp_payload(229, &ipv6).unwrap();
    assert_eq!(udp.payload, VECTOR_CLOCK_PROBE);

    // Linux SLL v1：16 B 头，最后 2 B 是协议
    let mut sll = vec![0u8; 14];
    sll.extend_from_slice(&0x0800u16.to_be_bytes());
    sll.extend_from_slice(&ipv4_udp_body(VECTOR_CLOCK_PROBE, ALP_PORT, 4));
    let udp = udp_payload(113, &sll).unwrap();
    assert_eq!(udp.payload, VECTOR_CLOCK_PROBE);
}

#[test]
fn non_udp_or_unknown_link_types_are_skipped_not_misparsed() {
    // ICMP（protocol 1）：必须跳过，而不是当成 ALP/2 硬解
    let mut icmp = vec![0xAA; 6];
    icmp.extend_from_slice(&[0xBB; 6]);
    icmp.extend_from_slice(&0x0800u16.to_be_bytes());
    let mut ip = vec![0x45, 0x00];
    ip.extend_from_slice(&28u16.to_be_bytes());
    ip.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    ip.push(64);
    ip.push(1); // ICMP
    ip.extend_from_slice(&[0x00, 0x00]);
    ip.extend_from_slice(&[192, 168, 3, 200]);
    ip.extend_from_slice(&[172, 16, 2, 54]);
    ip.extend_from_slice(&[0x08, 0x00, 0x00, 0x00]);
    icmp.extend_from_slice(&ip);
    assert!(udp_payload(1, &icmp).is_none());

    // 未知链路类型：跳过
    let frame = ethernet_ipv4_udp_frame(VECTOR_CLOCK_PROBE, ALP_PORT, 5);
    assert!(udp_payload(999, &frame).is_none());

    // IPv6 扩展头：不跟进（返回 None，而不是猜）
    let mut ipv6 = ipv6_udp_body(VECTOR_CLOCK_PROBE, ALP_PORT, 6);
    ipv6[6] = 0x00; // hop-by-hop 扩展头
    assert!(udp_payload(229, &ipv6).is_none());
}

#[test]
fn udp_length_zero_falls_back_to_remaining_bytes() {
    // 某些抓包工具对 GSO 大包会写 udp_len = 0：工具按「剩下的全是载荷」处理
    let mut frame = ethernet_ipv4_udp_frame(VECTOR_CLOCK_PROBE, ALP_PORT, 7);
    let udp_offset = 14 + 20;
    frame[udp_offset + 4] = 0;
    frame[udp_offset + 5] = 0;
    let udp = udp_payload(1, &frame).unwrap();
    assert_eq!(udp.payload, VECTOR_CLOCK_PROBE);
}

#[test]
fn broken_inputs_are_errors_not_panics() {
    assert!(parse_capture(&[]).is_err());
    assert!(parse_capture(&[0x00, 0x01, 0x02]).is_err());
    match parse_capture(&[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00, 0x00, 0x00]) {
        Err(PcapError::UnsupportedFormat { magic }) => {
            assert_eq!(magic, [0xDE, 0xAD, 0xBE, 0xEF]);
        }
        other => panic!("坏 magic 必须报 UnsupportedFormat，实际 {other:?}"),
    }

    // 全局头都不够 24 B
    assert!(matches!(
        parse_capture(&[0xD4, 0xC3, 0xB2, 0xA1, 0x02, 0x00]),
        Err(PcapError::Truncated { .. })
    ));

    // 记录声明 1000 B，文件里只剩 4 B
    let mut file = pcap_file(1, &[(0, vec![0u8; 4])], false);
    let record_header = 24;
    file[record_header + 8..record_header + 12].copy_from_slice(&1000u32.to_le_bytes());
    match parse_capture(&file) {
        Err(PcapError::BadRecordLength { declared, .. }) => assert_eq!(declared, 1000),
        other => panic!("记录超长必须报 BadRecordLength，实际 {other:?}"),
    }

    // pcapng 块长度不是 4 的倍数（改块头的 total_length，偏移 4..8）
    let mut pcapng = pcapng_file(1, &[(0, vec![0u8; 4])]);
    pcapng[4..8].copy_from_slice(&13u32.to_le_bytes());
    assert!(matches!(
        parse_capture(&pcapng),
        Err(PcapError::BadBlockLength { .. })
    ));

    // pcapng 块尾长度与块头不一致（防把垃圾当块）
    let mut pcapng = pcapng_file(1, &[(0, vec![0u8; 4])]);
    let first_block_len = u32::from_le_bytes([pcapng[4], pcapng[5], pcapng[6], pcapng[7]]) as usize;
    let trailer = first_block_len - 4;
    pcapng[trailer..trailer + 4].copy_from_slice(&777u32.to_le_bytes());
    assert!(matches!(
        parse_capture(&pcapng),
        Err(PcapError::BadBlockLength { .. })
    ));
}

#[test]
fn looks_like_capture_rejects_hex_text() {
    let hex = b"02 01 00 00 01 00 00 00";
    assert!(!looks_like_capture(hex));
    assert!(looks_like_capture(&pcap_file(1, &[], false)));
    assert!(looks_like_capture(&pcapng_file(1, &[])));
}
