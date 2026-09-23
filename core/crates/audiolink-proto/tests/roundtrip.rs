//! 往返（round-trip）、线格式与常量自洽性测试。
//!
//! 测试代码不受实时路径的 `unwrap` / `expect` / `panic` 禁令约束（那三条针对运行时音频路径）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use audiolink_proto::{
    AudioDatagram, AudioDatagramHeader, ClockReply, ControlFrame, DiscoveryBeacon, DiscoveryTxt,
    NackList, payload_decode, payload_encode,
};
use audiolink_types::{
    Caps, ErrorCode, Flags, OpCode, PROTO_MAJOR, PROTO_VERSION, PayloadLenRule, Platform, Ptype,
    StreamStats,
};
use serde::{Deserialize, Serialize};

#[test]
fn datagram_round_trip_for_every_ptype() {
    for ptype in Ptype::ALL {
        let payload = payload_for(ptype);
        let header = match ptype {
            Ptype::Audio => AudioDatagramHeader {
                version: PROTO_MAJOR,
                ptype,
                flags: Flags::FEC_REDUNDANT,
                stream_id: 7,
                seq: 1234,
                sample_index: 960,
                epoch_id: 0xAABB_CCDD_EEFF_0011,
            },
            _ => AudioDatagramHeader::auxiliary(ptype),
        };
        let datagram = AudioDatagram {
            header,
            payload: &payload,
        };

        let encoded = datagram.encode_to_vec().unwrap();
        let decoded = AudioDatagram::decode(&encoded).unwrap();
        assert_eq!(decoded.header, header, "{ptype:?}: 帧头必须无损");
        assert_eq!(
            decoded.payload,
            payload.as_slice(),
            "{ptype:?}: 载荷必须无损"
        );
        assert_eq!(
            decoded.encode_to_vec().unwrap(),
            encoded,
            "{ptype:?}: 编 → 解 → 再编必须字节一致"
        );

        // 热路径：复用缓冲的编码路径必须与 Vec 路径一致
        let mut buf = vec![0u8; encoded.len()];
        assert_eq!(datagram.encode_into(&mut buf).unwrap(), encoded.len());
        assert_eq!(buf, encoded);

        // 缓冲不足 → BadRequest（绝不 panic）
        if !encoded.is_empty() {
            let err = datagram
                .encode_into(&mut buf[..encoded.len() - 1])
                .unwrap_err();
            assert_eq!(err.code(), ErrorCode::BadRequest);
        }
    }
}

#[test]
fn nack_list_round_trip() {
    for count in [1usize, 2, 8, 16] {
        let seqs: Vec<u32> = (0..count).map(|i| 100 + i as u32).collect();
        let nack = NackList::from_slice(&seqs).unwrap();
        assert_eq!(nack.seqs(), seqs.as_slice());
        assert_eq!(nack.payload_len(), count * 4);

        let payload = nack.encode_to_vec().unwrap();
        let decoded = NackList::decode(&payload).unwrap();
        assert_eq!(decoded.seqs(), seqs.as_slice());

        // 整包往返
        let wire = nack.to_datagram_bytes().unwrap();
        let datagram = AudioDatagram::decode(&wire).unwrap();
        assert_eq!(
            NackList::from_datagram(&datagram).unwrap().seqs(),
            seqs.as_slice()
        );
    }
    assert!(NackList::from_slice(&[]).is_err());
    assert!(NackList::from_slice(&[0u32; 17]).is_err());
}

#[test]
fn control_frame_round_trip_with_limits() {
    for payload_len in [0usize, 1, 10, 1024, 65_536] {
        let payload = vec![0x5Au8; payload_len];
        let frame = ControlFrame {
            op: OpCode::StreamStats,
            request_id: 0xDEAD_BEEF,
            payload: &payload,
        };
        let encoded = frame.encode_to_vec().unwrap();
        assert_eq!(encoded.len(), 12 + payload_len);
        let decoded = ControlFrame::decode(&encoded).unwrap();
        assert_eq!(decoded.op, OpCode::StreamStats);
        assert_eq!(decoded.request_id, 0xDEAD_BEEF);
        assert_eq!(decoded.payload, payload.as_slice());
        assert_eq!(decoded.encode_to_vec().unwrap(), encoded);
    }

    // 超过 64 KiB 上限：编码与解码两个方向都必须拒绝（§4）
    let too_big = vec![0u8; 65_537];
    let frame = ControlFrame {
        op: OpCode::Bye,
        request_id: 0,
        payload: &too_big,
    };
    assert!(frame.total_len().is_err());
    assert!(frame.encode_to_vec().is_err());
    let mut out = vec![0u8; 12 + too_big.len()];
    assert!(frame.encode_into(&mut out).is_err());
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct DemoPayload {
    proto_version: u16,
    name: String,
    accepted: bool,
}

#[test]
fn postcard_payload_helpers_round_trip() {
    let value = DemoPayload {
        proto_version: PROTO_VERSION,
        name: "客厅 PC".to_string(),
        accepted: true,
    };
    let bytes = payload_encode(&value).unwrap();
    assert_eq!(payload_decode::<DemoPayload>(&bytes).unwrap(), value);

    // 尾随字节必须视为非法（§4：载荷长度与 schema 必须完全吻合）
    let mut trailing = bytes.clone();
    trailing.push(0x00);
    assert!(payload_decode::<DemoPayload>(&trailing).is_err());
    // 截断必须视为非法
    assert!(payload_decode::<DemoPayload>(&bytes[..bytes.len() - 1]).is_err());

    // 组帧 → 解帧 → 解载荷
    let wire = ControlFrame::encode_with_payload(OpCode::Hello, 1, &value).unwrap();
    let frame = ControlFrame::decode(&wire).unwrap();
    assert_eq!(frame.op, OpCode::Hello);
    assert_eq!(frame.request_id, 1);
    assert_eq!(payload_decode::<DemoPayload>(frame.payload).unwrap(), value);
}

#[test]
fn stream_stats_is_postcard_serializable() {
    // 证明 types 的可选 serde feature 已由 proto 正确开启（§10 遥测结构上控制帧）
    let stats = StreamStats {
        stream_id: 3,
        rtt_us: 2_400,
        jitter_us: 1_100,
        jitter_p95_us: 4_800,
        loss_pct_x100: 25,
        bitrate_bps: 160_000,
        codec: audiolink_types::CodecStats {
            frame_ms: 20,
            channels: 2,
            complexity: 7,
        },
        clock_offset_us: -3_200,
        drift_ppm: 18,
        buffer_level_us: 40_000,
        underruns: 1,
        plc_count: 4,
        nack_count: 2,
        e2e_latency_us: 92_000,
        late_drops: 0,
    };
    let bytes = payload_encode(&stats).unwrap();
    assert_eq!(payload_decode::<StreamStats>(&bytes).unwrap(), stats);
}

#[test]
fn discovery_txt_round_trip() {
    let txt = DiscoveryTxt::new(
        "3F9A1C0B8E77D2A4".to_string(),
        "客厅 PC".to_string(),
        Platform::Android,
        Caps::from_bits(0x3),
    );
    let pairs = txt.to_pairs();
    assert_eq!(pairs.len(), 6);
    assert_eq!(pairs[0].1, "1");
    assert_eq!(pairs[1].1, "0x0201");
    assert_eq!(pairs[2].1, "3f9a1c0b8e77d2a4"); // 规范化小写（§9.2）
    assert_eq!(pairs[3].1, "客厅 PC");
    assert_eq!(pairs[4].1, "android");
    assert_eq!(pairs[5].1, "3");

    let expected = DiscoveryTxt::new(
        "3f9a1c0b8e77d2a4".to_string(),
        "客厅 PC".to_string(),
        Platform::Android,
        Caps::from_bits(0x3),
    );
    assert_eq!(DiscoveryTxt::from_pairs(&pairs).unwrap(), expected);

    // 未知 Key 忽略（§9.2 向前兼容）：旧版 TXT 里的 `paired` 如今也只是一条未知 Key。
    let mut relaxed: Vec<(String, String)> = pairs.clone();
    relaxed.push(("paired".to_string(), "1".to_string()));
    relaxed.push(("vendor_x".to_string(), "whatever".to_string()));
    let parsed = DiscoveryTxt::from_pairs(&relaxed).unwrap();
    assert_eq!(parsed.id, "3f9a1c0b8e77d2a4");

    // 缺任一必需 Key → BadRequest
    for missing in ["v", "proto", "id", "name", "platform", "caps"] {
        let partial: Vec<(String, String)> = pairs
            .iter()
            .filter(|(key, _)| key != missing)
            .cloned()
            .collect();
        let err = DiscoveryTxt::from_pairs(&partial).unwrap_err();
        assert_eq!(err.code(), ErrorCode::BadRequest, "缺 {missing} 必须拒绝");
    }

    // §9.2：`v` 只接受 ASCII 数字（`+1` / `-1` / 空串必须拒绝）
    for value in ["+1", "-1", "", "1.0"] {
        let mut bad_v: Vec<(String, String)> = pairs
            .iter()
            .filter(|(key, _)| key != "v")
            .cloned()
            .collect();
        bad_v.push(("v".to_string(), value.to_string()));
        assert!(
            DiscoveryTxt::from_pairs(&bad_v).is_err(),
            "v = {value:?} 必须被拒绝"
        );
    }
}

#[test]
fn discovery_beacon_wire_format() {
    let beacon = DiscoveryBeacon::new(
        "3f9a1c0b8e77d2a4".to_string(),
        "客厅 PC".to_string(),
        Platform::Windows,
        Caps::from_bits(0x3),
        58_290,
    );

    // JSON 字段名与格式（§9.2）
    let json = beacon.json_payload().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(value["v"].as_u64(), Some(1));
    assert_eq!(value["proto"].as_str(), Some("0x0201"));
    assert_eq!(value["id"].as_str(), Some("3f9a1c0b8e77d2a4"));
    assert_eq!(value["name"].as_str(), Some("客厅 PC"));
    assert_eq!(value["platform"].as_str(), Some("win"));
    assert_eq!(value["caps"].as_str(), Some("3"));
    assert_eq!(value["port"].as_u64(), Some(58_290));

    // 报文布局：magic(9) + ver(1) + len(u16 LE) + JSON
    let wire = beacon.encode_to_vec().unwrap();
    assert_eq!(&wire[..9], b"AUDIOLINK");
    assert_eq!(wire[9], 1);
    assert_eq!(
        u16::from_le_bytes([wire[10], wire[11]]) as usize,
        json.len()
    );
    assert_eq!(&wire[12..], json.as_slice());
    assert_eq!(DiscoveryBeacon::decode(&wire).unwrap(), beacon);

    // 大小写规范化：id 大写 / caps 大写 hex 均可解析（§9.2）
    let upper = r#"{"v":1,"proto":"0x0201","id":"3F9A1C0B8E77D2A4","name":"n","platform":"win","caps":"A","port":1}"#;
    let parsed = DiscoveryBeacon::from_json_payload(upper.as_bytes()).unwrap();
    assert_eq!(parsed.id, "3f9a1c0b8e77d2a4");
    assert_eq!(parsed.caps, Caps::from_bits(0xA));
    // 再编码 → 再次解析，必须稳定
    assert_eq!(
        DiscoveryBeacon::decode(&parsed.encode_to_vec().unwrap()).unwrap(),
        parsed
    );
}

#[test]
fn enum_tables_are_consistent() {
    // ptype：已知值自洽，未知值必须返回 None（L1 据此拒绝）
    let pt_known: Vec<u8> = Ptype::ALL.iter().map(|p| p.as_u8()).collect();
    assert_eq!(pt_known.len(), 6);
    for raw in 0..=u8::MAX {
        match Ptype::from_u8(raw) {
            Some(ptype) => assert!(pt_known.contains(&raw) && ptype.as_u8() == raw),
            None => assert!(!pt_known.contains(&raw)),
        }
    }

    // 命令码：同上
    let op_known: Vec<u8> = OpCode::ALL.iter().map(|op| op.as_u8()).collect();
    // M4 新增 RECEIVER_EPOCH（0x44）、再删掉配对用的 0x03–0x07 五条之后是 20 项。
    // 改这个数字必须是有意的 —— 它存在的意义就是让「协议表变了」这件事无法悄悄溜过去。
    assert_eq!(op_known.len(), 20);
    for raw in 0..=u8::MAX {
        match OpCode::from_u8(raw) {
            Some(op) => assert!(op_known.contains(&raw) && op.as_u8() == raw),
            None => assert!(!op_known.contains(&raw)),
        }
    }

    // 错误码：§11 全表自洽 + 未分配值必须为 None
    for code in [
        1001u16, 1002, 1005, 1006, 1007, 1008, 1009, 2001, 2002, 2003, 3001,
    ] {
        let parsed = ErrorCode::from_u16(code).unwrap();
        assert_eq!(parsed.as_u16(), code);
        assert!(!parsed.name().is_empty());
    }
    // 1003 / 1004（旧 PAIR_REJECTED / AUTH_FAILED）随配对机制一起删除 → 现在是未分配值。
    for unknown in [
        0u16,
        1000,
        1003,
        1004,
        1010,
        2000,
        2004,
        3000,
        3002,
        u16::MAX,
    ] {
        assert!(
            ErrorCode::from_u16(unknown).is_none(),
            "{unknown} 应为未分配"
        );
    }
}

/// `CLOCK_REPLY` 正路径往返（§3 载荷表：`probe_seq(u32)` + `t1/t2/t3(i64)` = **28 B**）。
#[test]
fn clock_reply_round_trip() {
    let reply = ClockReply {
        probe_seq: 7,
        t1: 1_000_000,
        t2: 1_000_120,
        t3: 1_000_121,
    };
    let payload = reply.encode_to_vec().unwrap();
    assert_eq!(payload.len(), ClockReply::LEN);
    assert_eq!(payload.len(), 4 + 8 + 8 + 8, "字段布局决定载荷必须 28 B");
    assert_eq!(ClockReply::decode(&payload).unwrap(), reply);

    let wire = reply.to_datagram_bytes().unwrap();
    assert_eq!(wire.len(), 24 + ClockReply::LEN);
    let datagram = AudioDatagram::decode(&wire).unwrap();
    assert_eq!(datagram.header.ptype, Ptype::ClockReply);
    assert_eq!(ClockReply::from_datagram(&datagram).unwrap(), reply);
    assert_eq!(
        ClockReply::from_datagram(&datagram)
            .unwrap()
            .to_datagram_bytes()
            .unwrap(),
        wire
    );
}

/// 为指定 ptype 造一个长度合法的载荷。
fn payload_for(ptype: Ptype) -> Vec<u8> {
    match ptype.payload_len_rule() {
        PayloadLenRule::Opaque { .. } => vec![0xAA; 400],
        PayloadLenRule::Exact { len } => vec![0xBB; len],
        PayloadLenRule::U32List { min_items, .. } => vec![0xCC; min_items * 4],
    }
}
