//! `docs/03-protocol.md` §12 golden vectors —— **契约测试**。
//!
//! 规约：
//! - 下列向量字节**不得修改**（要改必须先改文档并说明理由，见交接文档纪律）；
//! - 每个向量都必须「编码 → 解码 → 再编码」字节完全一致；
//! - 向量 3 是**信封级**向量（10 B 载荷不透明），见 §12 示例 3 的附注。
//!
//! 测试代码不受实时路径的 `unwrap` / `expect` / `panic` 禁令约束（那三条针对运行时音频路径），
//! 见 `docs/02-architecture.md` §4 实时铁律 1。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use audiolink_proto::{AudioDatagram, ClockProbe, ControlFrame, ControlFrameHeader};
use audiolink_types::{Flags, OpCode, PROTO_MAJOR, PROTO_MINOR, PROTO_VERSION, Ptype};

/// §12 示例 1：立体声 Opus 音频包（28 B）。
const VECTOR_AUDIO: &[u8] = &[
    0x02, 0x01, 0x00, 0x00, // version = 0x02, ptype = 0x01 (AUDIO), flags = 0x0000
    0x01, 0x00, 0x00, 0x00, // stream_id = 1
    0x2A, 0x00, 0x00, 0x00, // seq = 42
    0xC0, 0x03, 0x00, 0x00, // sample_index = 960
    0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, // epoch_id = 0x1122334455667788
    0xDE, 0xAD, 0xBE, 0xEF, // payload
];

/// §12 示例 2：时钟探测数据报（24 B 帧头 + 12 B 载荷 = 36 B）。
const VECTOR_CLOCK_PROBE: &[u8] = &[
    0x02, 0x03, 0x00, 0x00, // version = 0x02, ptype = 0x03 (CLOCK_PROBE), flags = 0
    0x00, 0x00, 0x00, 0x00, // stream_id = 0（探测不使用流）
    0x00, 0x00, 0x00, 0x00, // seq = 0
    0x00, 0x00, 0x00, 0x00, // sample_index = 0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // epoch_id = 0
    0x07, 0x00, 0x00, 0x00, // probe_seq = 7
    0x40, 0x42, 0x0F, 0x00, 0x00, 0x00, 0x00, 0x00, // t1 = 1_000_000 µs
];

/// §12 示例 3：HELLO 控制帧的 12 B 信封（8 B 帧头 + `request_id`）。
const VECTOR_CONTROL_HEAD: &[u8] = &[
    0x02, 0x01, 0x00, 0x00, // ver = 0x02, type = 0x01 (HELLO), flags = 0x0000
    0x0A, 0x00, 0x00, 0x00, // payload_len = 10
    0x01, 0x00, 0x00, 0x00, // request_id = 1
];

/// 向量 3 的 10 B 载荷：文档未定义其内容 → 按不透明字节处理（§12 示例 3 附注）。
const VECTOR_CONTROL_PAYLOAD: &[u8; 10] =
    &[0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09];

#[test]
fn vector_1_audio_datagram_round_trip() {
    let datagram = AudioDatagram::decode(VECTOR_AUDIO).unwrap();

    assert_eq!(datagram.header.version, PROTO_MAJOR);
    assert_eq!(datagram.header.ptype, Ptype::Audio);
    assert_eq!(datagram.header.flags, Flags::NONE);
    assert_eq!(datagram.header.stream_id, 1);
    assert_eq!(datagram.header.seq, 42);
    assert_eq!(datagram.header.sample_index, 960);
    assert_eq!(datagram.header.epoch_id, 0x1122_3344_5566_7788);
    assert_eq!(datagram.payload, &[0xDE, 0xAD, 0xBE, 0xEF]);

    // 编码 → 解码 → 再编码，字节完全一致（§12 一致性测试要求）
    assert_eq!(datagram.encode_to_vec().unwrap(), VECTOR_AUDIO);

    // 热路径写法：复用缓冲
    let mut buf = [0u8; 64];
    let written = datagram.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..written], VECTOR_AUDIO);
}

#[test]
fn vector_1_sample_index_is_20ms_at_48khz() {
    // §12 示例 1 的注释：960 = 20 ms × 48000
    assert_eq!(960, 20 * 48_000 / 1000);
    let datagram = AudioDatagram::decode(VECTOR_AUDIO).unwrap();
    assert_eq!(datagram.header.sample_index, 20 * 48_000 / 1000);
}

#[test]
fn vector_2_clock_probe_round_trip() {
    assert_eq!(VECTOR_CLOCK_PROBE.len(), 24 + 12);

    let datagram = AudioDatagram::decode(VECTOR_CLOCK_PROBE).unwrap();
    assert_eq!(datagram.header.ptype, Ptype::ClockProbe);
    assert_eq!(datagram.header.flags, Flags::NONE);
    assert_eq!(datagram.header.stream_id, 0);
    assert_eq!(datagram.header.seq, 0);
    assert_eq!(datagram.header.sample_index, 0);
    assert_eq!(datagram.header.epoch_id, 0);

    let probe = ClockProbe::from_datagram(&datagram).unwrap();
    assert_eq!(probe.probe_seq, 7);
    assert_eq!(probe.t1, 1_000_000); // 1 s，单位 µs
    assert_eq!(
        probe.encode_to_vec().unwrap().as_slice(),
        &VECTOR_CLOCK_PROBE[24..]
    );
    assert_eq!(probe.to_datagram_bytes().unwrap(), VECTOR_CLOCK_PROBE);
}

#[test]
fn vector_3_control_frame_envelope_round_trip() {
    let mut wire = VECTOR_CONTROL_HEAD.to_vec();
    wire.extend_from_slice(VECTOR_CONTROL_PAYLOAD);

    // 信封级断言（§12 示例 3）
    let header = ControlFrameHeader::decode(&wire).unwrap();
    assert_eq!(header.ver, PROTO_MAJOR);
    assert_eq!(header.raw_op, OpCode::Hello.as_u8());
    assert_eq!(header.flags, 0);
    assert_eq!(header.payload_len, 10);
    assert_eq!(header.request_id, 1);

    // 严格解码 + 载荷 round-trip
    let frame = ControlFrame::decode(&wire).unwrap();
    assert_eq!(frame.op, OpCode::Hello);
    assert_eq!(frame.request_id, 1);
    assert_eq!(frame.payload, VECTOR_CONTROL_PAYLOAD);
    assert_eq!(frame.encode_to_vec().unwrap(), wire);
}

#[test]
fn protocol_version_constants_are_consistent() {
    assert_eq!(PROTO_VERSION, 0x0201);
    assert_eq!(
        u16::from(PROTO_MAJOR) << 8 | u16::from(PROTO_MINOR),
        PROTO_VERSION,
        "PROTO_VERSION 必须等于「主版本 << 8 | 小版本」（§13）"
    );
    assert_eq!(
        u16::from_be_bytes([PROTO_MAJOR, PROTO_MINOR]),
        PROTO_VERSION
    );
}
