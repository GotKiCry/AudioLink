//! 拒绝用例（`docs/03-protocol.md` §1.1、§12 拒绝用例矩阵）。
//!
//! L1 契约：截断 / 超长 / 未知 ptype / 保留位非 0 / 长度越界 → 一律 `1008 BAD_REQUEST`，**绝不 panic**。
//! 测试代码不受实时路径的 `unwrap` / `expect` / `panic` 禁令约束（那三条针对运行时音频路径）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use audiolink_proto::discovery::MAGIC;
use audiolink_proto::{
    AudioDatagram, AudioDatagramHeader, ClockProbe, ClockReply, ControlFrame, ControlFrameHeader,
    DiscoveryBeacon, NackList,
};
use audiolink_types::{AudioLinkError, Caps, ErrorCode, Platform};

/// 断言一次解码被 L1 层拒绝，且错误码恰为 `1008 BAD_REQUEST`。
#[track_caller]
fn assert_bad_request<T: core::fmt::Debug>(result: Result<T, AudioLinkError>, case: &str) {
    match result {
        Ok(value) => panic!("{case}: 期望 BadRequest，实际解码成功：{value:?}"),
        Err(err) => {
            assert_eq!(
                err.code(),
                ErrorCode::BadRequest,
                "{case}: 错误码必须是 BAD_REQUEST"
            );
            assert_eq!(err.code().as_u16(), 1008, "{case}: 数值必须是 1008");
            assert!(!err.context().is_empty(), "{case}: 必须携带失败上下文");
        }
    }
}

/// §12 示例 1 的向量字节（用于构造非法变体）。
const VECTOR_AUDIO: &[u8] = &[
    0x02, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x2A, 0x00, 0x00, 0x00, 0xC0, 0x03, 0x00, 0x00,
    0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0xDE, 0xAD, 0xBE, 0xEF,
];

/// §12 示例 2 的向量字节（定长载荷，适合做截断扫掠）。
const VECTOR_CLOCK_PROBE: &[u8] = &[
    0x02, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x40, 0x42, 0x0F, 0x00,
    0x00, 0x00, 0x00, 0x00,
];

const CONTROL_PAYLOAD: &[u8; 10] = &[0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09];

// ---------------------------------------------------------------------------
// 构造辅助
// ---------------------------------------------------------------------------

/// 造一个「24 B 帧头 + 指定长度载荷」的辅助包（`ver = 0x02`、`flags = 0`、其余字段 0）。
fn auxiliary_bytes(ptype: u8, payload_len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; 24 + payload_len];
    buf[0] = 0x02;
    buf[1] = ptype;
    buf
}

/// 造一个控制帧（`request_id = 1`）。
fn control_frame_bytes(op: u8, ver: u8, flags: u16, payload: &[u8]) -> Vec<u8> {
    let mut buf = vec![0u8; 12 + payload.len()];
    buf[0] = ver;
    buf[1] = op;
    buf[2..4].copy_from_slice(&flags.to_le_bytes());
    let len = u32::try_from(payload.len()).unwrap();
    buf[4..8].copy_from_slice(&len.to_le_bytes());
    buf[8..12].copy_from_slice(&1u32.to_le_bytes());
    buf[12..].copy_from_slice(payload);
    buf
}

/// 造一条发现广播报文（合法 JSON，字段可覆盖）。
fn beacon_bytes(
    v: &str,
    proto: &str,
    id: &str,
    name: &str,
    platform: &str,
    caps: &str,
    port: &str,
) -> Vec<u8> {
    let json = format!(
        r#"{{"v":{v},"proto":"{proto}","id":"{id}","name":"{name}","platform":"{platform}","caps":"{caps}","port":{port}}}"#
    );
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(1);
    out.extend_from_slice(&u16::try_from(json.len()).unwrap().to_le_bytes());
    out.extend_from_slice(json.as_bytes());
    out
}

/// 合法的广播报文。
fn valid_beacon_bytes() -> Vec<u8> {
    beacon_bytes(
        "1",
        "0x0201",
        "3f9a1c0b8e77d2a4",
        "客厅 PC",
        "win",
        "3",
        "58290",
    )
}

// ---------------------------------------------------------------------------
// 音频数据报（§3、§12）
// ---------------------------------------------------------------------------

#[test]
fn datagram_header_truncation_is_rejected() {
    for len in 0..24 {
        assert_bad_request(
            AudioDatagram::decode(&VECTOR_AUDIO[..len]),
            &format!("数据报截断到 {len} B"),
        );
        assert_bad_request(
            AudioDatagramHeader::decode(&VECTOR_AUDIO[..len]),
            &format!("帧头截断到 {len} B"),
        );
    }
    // 24 B = 帧头 + 空载荷：AUDIO 未置 DTX 时不允许空载荷（§3 载荷表）
    assert_bad_request(
        AudioDatagram::decode(&VECTOR_AUDIO[..24]),
        "AUDIO 空载荷且未置 DTX",
    );
    // 正对照：AUDIO + DTX + 空载荷必须被接受
    let mut dtx = VECTOR_AUDIO[..24].to_vec();
    dtx[2..4].copy_from_slice(&(1u16 << 1).to_le_bytes());
    assert!(AudioDatagram::decode(&dtx).is_ok(), "DTX 空载荷应合法");
}

#[test]
fn datagram_opaque_payload_is_length_agnostic_past_header() {
    // AUDIO / FEC 是不透明载荷：25 B 以上没有可检测的「结构性截断」边界
    // （唯一的帧头截断边界已在上一个用例覆盖）。
    for len in 25..=VECTOR_AUDIO.len() {
        let datagram = AudioDatagram::decode(&VECTOR_AUDIO[..len]).unwrap();
        assert_eq!(datagram.payload.len(), len - 24);
    }
}

#[test]
fn datagram_above_mtu_is_rejected() {
    for total in [1201usize, 2000, 4096] {
        let mut buf = vec![0u8; total];
        buf[0] = 0x02;
        buf[1] = 0x01;
        assert_bad_request(AudioDatagram::decode(&buf), &format!("超 MTU：{total} B"));
    }
    // 正对照：恰 1200 B（AUDIO 不透明载荷）必须被接受
    assert!(AudioDatagram::decode(&auxiliary_bytes(0x01, 1200 - 24)).is_ok());
}

#[test]
fn datagram_unknown_ptype_is_rejected() {
    for raw in [0x00u8, 0x07, 0x08, 0x7F, 0x80, 0xFF] {
        let mut buf = VECTOR_AUDIO.to_vec();
        buf[1] = raw;
        assert_bad_request(
            AudioDatagram::decode(&buf),
            &format!("未知 ptype {raw:#04x}"),
        );
    }
}

#[test]
fn datagram_reserved_flag_bits_are_rejected() {
    for bit in 5..=15 {
        let mut buf = VECTOR_AUDIO.to_vec();
        buf[2..4].copy_from_slice(&(1u16 << bit).to_le_bytes());
        assert_bad_request(
            AudioDatagram::decode(&buf),
            &format!("保留位 bit{bit} 非 0"),
        );
    }
    // 正对照：已知位（bit 0–4）必须被接受
    let mut known = VECTOR_AUDIO.to_vec();
    known[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
    assert!(
        AudioDatagram::decode(&known).is_ok(),
        "bit 0–4 是已知位，必须接受"
    );
}

#[test]
fn datagram_version_mismatch_is_rejected() {
    for ver in [0x00u8, 0x01, 0x03, 0xFF] {
        let mut buf = VECTOR_AUDIO.to_vec();
        buf[0] = ver;
        assert_bad_request(AudioDatagram::decode(&buf), &format!("版本 {ver:#04x}"));
    }
}

#[test]
fn datagram_fixed_payload_lengths_are_enforced() {
    // CLOCK_PROBE：恰 12 B —— 对真实向量逐长度扫掠（0..36 全部非法）
    for len in 0..VECTOR_CLOCK_PROBE.len() {
        assert_bad_request(
            AudioDatagram::decode(&VECTOR_CLOCK_PROBE[..len]),
            &format!("CLOCK_PROBE 总长 {len} B"),
        );
    }
    // CLOCK_REPLY：恰 24 B
    for payload_len in [0usize, 8, 12, 23, 25, 48] {
        assert_bad_request(
            AudioDatagram::decode(&auxiliary_bytes(0x04, payload_len)),
            &format!("CLOCK_REPLY 载荷 {payload_len} B"),
        );
    }
    assert!(AudioDatagram::decode(&auxiliary_bytes(0x04, 24)).is_ok());
    // KEEPALIVE：恰 0 B
    assert_bad_request(
        AudioDatagram::decode(&auxiliary_bytes(0x05, 1)),
        "KEEPALIVE 带载荷",
    );
    assert!(AudioDatagram::decode(&auxiliary_bytes(0x05, 0)).is_ok());
    // NACK：4×n，1 ≤ n ≤ 16
    assert_bad_request(
        AudioDatagram::decode(&auxiliary_bytes(0x06, 0)),
        "NACK 0 项",
    );
    assert_bad_request(
        AudioDatagram::decode(&auxiliary_bytes(0x06, 3)),
        "NACK 长度非 4 的倍数",
    );
    assert_bad_request(
        AudioDatagram::decode(&auxiliary_bytes(0x06, 68)),
        "NACK 17 项",
    );
    assert!(AudioDatagram::decode(&auxiliary_bytes(0x06, 4)).is_ok());
    assert!(AudioDatagram::decode(&auxiliary_bytes(0x06, 64)).is_ok());
}

#[test]
fn clock_payload_decoders_reject_wrong_lengths() {
    for bad in [vec![0u8; 0], vec![0u8; 11], vec![0u8; 13], vec![0u8; 24]] {
        assert_bad_request(
            ClockProbe::decode(&bad),
            &format!("CLOCK_PROBE 载荷 {} B", bad.len()),
        );
    }
    for bad in [vec![0u8; 0], vec![0u8; 12], vec![0u8; 23], vec![0u8; 25]] {
        assert_bad_request(
            ClockReply::decode(&bad),
            &format!("CLOCK_REPLY 载荷 {} B", bad.len()),
        );
    }
    assert_bad_request(NackList::decode(&[]), "NACK 空载荷");
    assert_bad_request(NackList::decode(&[0u8; 3]), "NACK 3 B");
    assert_bad_request(NackList::decode(&[0u8; 68]), "NACK 17 项");
    // ptype 不符
    let probe = AudioDatagram::decode(VECTOR_CLOCK_PROBE).unwrap();
    assert_bad_request(
        ClockReply::from_datagram(&probe),
        "把 CLOCK_PROBE 当 CLOCK_REPLY",
    );
}

// ---------------------------------------------------------------------------
// 控制帧（§4、§12）
// ---------------------------------------------------------------------------

#[test]
fn control_frame_truncation_is_rejected() {
    let full = control_frame_bytes(0x01, 0x02, 0, CONTROL_PAYLOAD);
    for len in 0..full.len() {
        assert_bad_request(
            ControlFrame::decode(&full[..len]),
            &format!("控制帧截断到 {len} B"),
        );
        assert_bad_request(
            ControlFrameHeader::decode(&full[..len]),
            &format!("控制帧信封截断到 {len} B"),
        );
    }
}

#[test]
fn control_frame_length_field_must_match_remaining() {
    let mut short = control_frame_bytes(0x01, 0x02, 0, CONTROL_PAYLOAD);
    short[4..8].copy_from_slice(&9u32.to_le_bytes());
    assert_bad_request(ControlFrame::decode(&short), "payload_len 比实际少 1");

    let mut long = control_frame_bytes(0x01, 0x02, 0, CONTROL_PAYLOAD);
    long[4..8].copy_from_slice(&11u32.to_le_bytes());
    assert_bad_request(ControlFrame::decode(&long), "payload_len 比实际多 1");

    // 声明 > 64 KiB
    let mut huge = vec![0u8; 12 + 65_537];
    huge[0] = 0x02;
    huge[1] = 0x01;
    huge[4..8].copy_from_slice(&65_537u32.to_le_bytes());
    assert_bad_request(ControlFrame::decode(&huge), "payload_len 65537 > 64 KiB");
}

#[test]
fn control_frame_version_op_and_flags_are_enforced() {
    for ver in [0x01u8, 0x03, 0xFF] {
        assert_bad_request(
            ControlFrame::decode(&control_frame_bytes(0x01, ver, 0, &[])),
            &format!("控制帧版本 {ver:#04x}"),
        );
    }
    for op in [0x00u8, 0x08, 0x14, 0x31, 0x99, 0xFF] {
        assert_bad_request(
            ControlFrame::decode(&control_frame_bytes(op, 0x02, 0, &[])),
            &format!("未知命令 {op:#04x}"),
        );
    }
    for bit in [0u16, 1, 5, 15] {
        assert_bad_request(
            ControlFrame::decode(&control_frame_bytes(0x01, 0x02, 1 << bit, &[])),
            &format!("控制帧 flags bit{bit} 非 0"),
        );
    }
    // 宽容信封只做结构校验：版本 / 命令未知也必须能读出信封（§1.1 握手用途）
    let header = ControlFrameHeader::decode(&control_frame_bytes(0x99, 0x03, 1, &[])).unwrap();
    assert_eq!(header.ver, 0x03);
    assert_eq!(header.raw_op, 0x99);
    assert_eq!(header.flags, 1);
    assert_eq!(header.payload_len, 0);
}

// ---------------------------------------------------------------------------
// 发现报文（§9.2）
// ---------------------------------------------------------------------------

#[test]
fn discovery_magic_version_length_are_enforced() {
    let good = valid_beacon_bytes();
    assert!(DiscoveryBeacon::decode(&good).is_ok());

    // magic 不符
    let mut bad_magic = good.clone();
    bad_magic[0] = b'B';
    assert_bad_request(DiscoveryBeacon::decode(&bad_magic), "magic 不符");

    // 逐长度截断扫掠
    for len in 0..good.len() {
        assert_bad_request(
            DiscoveryBeacon::decode(&good[..len]),
            &format!("广播报文截断到 {len} B"),
        );
    }

    // ver 不符
    let mut bad_ver = good.clone();
    bad_ver[9] = 2;
    assert_bad_request(DiscoveryBeacon::decode(&bad_ver), "ver = 2");

    // len 与剩余字节不符
    let mut bad_len = good.clone();
    bad_len[10] = bad_len[10].wrapping_add(1);
    assert_bad_request(DiscoveryBeacon::decode(&bad_len), "len 与剩余不符");

    // len > 1024
    let mut oversized = vec![0u8; 12 + 1025];
    oversized[..9].copy_from_slice(MAGIC);
    oversized[9] = 1;
    oversized[10..12].copy_from_slice(&1025u16.to_le_bytes());
    assert_bad_request(DiscoveryBeacon::decode(&oversized), "len 1025 > 1024");

    // JSON 非法（把首字节 `{` 换成 `[` → 结构不匹配）
    let mut bad_json = good.clone();
    bad_json[12] = b'[';
    assert_bad_request(DiscoveryBeacon::decode(&bad_json), "JSON 非法");
}

#[test]
fn discovery_field_formats_are_enforced() {
    // 合法基线
    assert!(DiscoveryBeacon::decode(&valid_beacon_bytes()).is_ok());

    let cases: [(Vec<u8>, &str); 11] = [
        (
            beacon_bytes("2", "0x0201", "3f9a1c0b8e77d2a4", "n", "win", "3", "1"),
            "v = 2",
        ),
        (
            beacon_bytes("1", "0201", "3f9a1c0b8e77d2a4", "n", "win", "3", "1"),
            "proto 缺少 0x 前缀",
        ),
        (
            beacon_bytes("1", "0x02", "3f9a1c0b8e77d2a4", "n", "win", "3", "1"),
            "proto 位数不足",
        ),
        (
            beacon_bytes("1", "0xGGGG", "3f9a1c0b8e77d2a4", "n", "win", "3", "1"),
            "proto 非 hex",
        ),
        (
            beacon_bytes("1", "0x0201", "3f9a", "n", "win", "3", "1"),
            "id 过短",
        ),
        (
            beacon_bytes("1", "0x0201", "zz9a1c0b8e77d2a4", "n", "win", "3", "1"),
            "id 非 hex",
        ),
        (
            beacon_bytes("1", "0x0201", "3f9a1c0b8e77d2a4", "n", "mac", "3", "1"),
            "platform 未知",
        ),
        (
            beacon_bytes("1", "0x0201", "3f9a1c0b8e77d2a4", "n", "win", "0x3", "1"),
            "caps 带前缀",
        ),
        (
            beacon_bytes("1", "0x0201", "3f9a1c0b8e77d2a4", "n", "win", "", "1"),
            "caps 空",
        ),
        (
            beacon_bytes("1", "0x0201", "3f9a1c0b8e77d2a4", "n", "win", "abcde", "1"),
            "caps 超 4 位",
        ),
        (
            beacon_bytes("1", "0x0201", "3f9a1c0b8e77d2a4", "n", "win", "3", "99999"),
            "port 越界",
        ),
    ];
    for (bytes, case) in cases {
        assert_bad_request(DiscoveryBeacon::decode(&bytes), case);
    }

    // 缺 port（serde 必填字段）
    let missing_port = r#"{"v":1,"proto":"0x0201","id":"3f9a1c0b8e77d2a4","name":"n","platform":"win","caps":"3"}"#;
    assert_bad_request(
        DiscoveryBeacon::from_json_payload(missing_port.as_bytes()),
        "缺 port",
    );

    // 未知字段必须忽略（向前兼容）
    let unknown_field = r#"{"v":1,"proto":"0x0201","id":"3f9a1c0b8e77d2a4","name":"n","platform":"win","caps":"3","port":1,"future":"x"}"#;
    let beacon = DiscoveryBeacon::from_json_payload(unknown_field.as_bytes()).unwrap();
    assert_eq!(beacon.id, "3f9a1c0b8e77d2a4");
    assert_eq!(beacon.caps, Caps::from_bits(0x3));
    assert_eq!(beacon.platform, Platform::Windows);
}

// ---------------------------------------------------------------------------
// 健壮性：任何输入都不得 panic（§12）
// ---------------------------------------------------------------------------

/// 定种子 xorshift64，避免引入 dev-dependency。
struct XorShift64(u64);

impl XorShift64 {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// 把所有解码入口跑一遍（panic 会让测试失败，即「绝不 panic」的断言）。
fn exercise_all_decoders(bytes: &[u8]) {
    let _ = AudioDatagram::decode(bytes);
    let _ = AudioDatagramHeader::decode(bytes);
    let _ = ClockProbe::decode(bytes);
    let _ = ClockReply::decode(bytes);
    let _ = NackList::decode(bytes);
    let _ = ControlFrame::decode(bytes);
    let _ = ControlFrameHeader::decode(bytes);
    let _ = DiscoveryBeacon::decode(bytes);
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = XorShift64(0x0123_4567_89AB_CDEF);
    for _ in 0..5_000 {
        let len = (rng.next_u64() % 1400) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| (rng.next_u64() & 0xFF) as u8).collect();
        exercise_all_decoders(&bytes);
    }
}

#[test]
fn mutated_valid_packets_never_panic() {
    let control = control_frame_bytes(0x01, 0x02, 0, CONTROL_PAYLOAD);
    let beacon = valid_beacon_bytes();
    let seeds: [&[u8]; 4] = [VECTOR_AUDIO, VECTOR_CLOCK_PROBE, &control, &beacon];
    let mut rng = XorShift64(0xDEAD_BEEF_1234_5678);
    for _ in 0..5_000 {
        let seed = seeds[(rng.next_u64() % seeds.len() as u64) as usize];
        let mut bytes = seed.to_vec();
        let flips = 1 + (rng.next_u64() % 4) as usize;
        for _ in 0..flips {
            let index = (rng.next_u64() as usize) % bytes.len().max(1);
            if let Some(byte) = bytes.get_mut(index) {
                *byte ^= 1u8 << (rng.next_u64() % 8);
            }
        }
        exercise_all_decoders(&bytes);
    }
}
