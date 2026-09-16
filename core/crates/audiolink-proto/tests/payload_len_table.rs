//! §3 载荷表护栏：定长载荷的 `LEN` 必须等于**字段宽度之和**。
//!
//! 本文件里的字段宽度表是 `docs/03-protocol.md` §3 的**独立副本**：实现侧的常量、
//! `Ptype::payload_len_rule()` 与实际编码/解码行为三处都要与它对齐。
//! 既有的 `rejects.rs` 长度扫掠拿 `X::LEN` 当期望值 —— 那是**自指**，常量自己算错它抓不到；
//! 本文件专门补这个洞（对应看板条目：`[护栏] 「定长载荷自洽」测试：LEN == 字段宽度之和`）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use audiolink_proto::{AudioDatagram, AudioDatagramHeader, ClockProbe, ClockReply, NackList};
use audiolink_types::{
    AudioLinkError, CLOCK_PROBE_PAYLOAD_LEN, CLOCK_PROBE_PROBE_SEQ_OFFSET, CLOCK_PROBE_T1_OFFSET,
    CLOCK_REPLY_PAYLOAD_LEN, CLOCK_REPLY_PROBE_SEQ_OFFSET, CLOCK_REPLY_T1_OFFSET,
    CLOCK_REPLY_T2_OFFSET, CLOCK_REPLY_T3_OFFSET, CONTROL_HEADER_LEN, DATAGRAM_HEADER_LEN,
    DATAGRAM_MAX_LEN, DATAGRAM_MAX_PAYLOAD, ErrorCode, Flags, KEEPALIVE_PAYLOAD_LEN, NACK_ITEM_LEN,
    NACK_MAX_ITEMS, NACK_MIN_ITEMS, PROTO_MAJOR, PayloadLenRule, Ptype,
};

/// §3 数据报帧头字段宽度表（逐字抄自 `docs/03-protocol.md` §3）。
const DATAGRAM_HEADER_FIELDS: &[(&str, usize)] = &[
    ("ver", 1),
    ("ptype", 1),
    ("flags", 2),
    ("stream_id", 4),
    ("seq", 4),
    ("sample_index", 4),
    ("epoch_id", 8),
];

/// §3 `CLOCK_PROBE` 载荷字段宽度表。
const CLOCK_PROBE_FIELDS: &[(&str, usize)] = &[("probe_seq", 4), ("t1", 8)];

/// §3 `CLOCK_REPLY` 载荷字段宽度表。
const CLOCK_REPLY_FIELDS: &[(&str, usize)] = &[("probe_seq", 4), ("t1", 8), ("t2", 8), ("t3", 8)];

/// §4 控制帧固定帧头字段宽度表（`request_id` 不在其中，另见 `MIN_FRAME_LEN`）。
const CONTROL_HEADER_FIELDS: &[(&str, usize)] =
    &[("ver", 1), ("type", 1), ("flags", 2), ("payload_len", 4)];

/// §3 表值：数据报帧头 24 B。
const DATAGRAM_HEADER_SPEC_LEN: usize = 24;
/// §3 表值：`CLOCK_PROBE` 载荷 12 B。
const CLOCK_PROBE_SPEC_LEN: usize = 12;
/// §3 表值：`CLOCK_REPLY` 载荷 28 B。
const CLOCK_REPLY_SPEC_LEN: usize = 28;
/// §4 表值：控制帧固定帧头 8 B。
const CONTROL_HEADER_SPEC_LEN: usize = 8;

/// 字段宽度之和。
fn width_sum(fields: &[(&str, usize)]) -> usize {
    fields.iter().map(|(_, width)| *width).sum()
}

/// 按表累加得到的字段偏移（不在表里 → 表被写错了，直接失败）。
fn offset_of(fields: &[(&str, usize)], name: &str) -> usize {
    let mut offset = 0;
    for (field, width) in fields {
        if *field == name {
            return offset;
        }
        offset += width;
    }
    panic!("字段 {name} 不在 §3 表里");
}

/// 断言一次解码被 L1 层拒绝，且错误码恰为 `1008 BAD_REQUEST`。
#[track_caller]
fn assert_bad_request<T: core::fmt::Debug>(result: Result<T, AudioLinkError>, case: &str) {
    match result {
        Ok(value) => panic!("{case}: 期望被拒绝，实际解码成功：{value:?}"),
        Err(err) => {
            assert_eq!(err.code(), ErrorCode::BadRequest, "{case}: 必须是 1008");
            assert_eq!(err.code().as_u16(), 1008, "{case}: 数值必须是 1008");
            assert!(!err.context().is_empty(), "{case}: 必须携带失败上下文");
        }
    }
}

/// 造一个「24 B 帧头 + 指定长度载荷」的辅助包（版本与 ptype 按参数写入，其余字段 0）。
fn auxiliary_bytes(ptype: u8, payload_len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; DATAGRAM_HEADER_LEN + payload_len];
    buf[0] = PROTO_MAJOR;
    buf[1] = ptype;
    buf
}

#[test]
fn datagram_header_len_equals_sum_of_field_widths() {
    assert_eq!(
        width_sum(DATAGRAM_HEADER_FIELDS),
        DATAGRAM_HEADER_SPEC_LEN,
        "§3 帧头宽度表自身应为 24 B"
    );
    assert_eq!(
        DATAGRAM_HEADER_LEN,
        width_sum(DATAGRAM_HEADER_FIELDS),
        "DATAGRAM_HEADER_LEN 必须等于 §3 字段宽度之和"
    );
    assert_eq!(
        AudioDatagramHeader::LEN,
        DATAGRAM_HEADER_SPEC_LEN,
        "帧头 LEN 必须与 §3 表值一致"
    );

    let keepalive = AudioDatagramHeader::auxiliary(Ptype::Keepalive);
    assert_eq!(
        keepalive.encode_to_vec().unwrap().len(),
        DATAGRAM_HEADER_SPEC_LEN,
        "实际编码长度必须等于 §3 表值"
    );

    // 哨兵值 → 按 §3 表累加偏移手工解析：字段宽度或偏移错位都会在这里断掉。
    let sentinel = AudioDatagramHeader {
        version: PROTO_MAJOR,
        ptype: Ptype::ClockProbe,
        flags: Flags::FEC_REDUNDANT,
        stream_id: 0x1122_3344,
        seq: 0x5566_7788,
        sample_index: 0x99AA_BBCC,
        epoch_id: 0x0102_0304_0506_0708,
    };
    let wire = sentinel.encode_to_vec().unwrap();
    assert_eq!(wire.len(), width_sum(DATAGRAM_HEADER_FIELDS));
    let at = |name: &str| offset_of(DATAGRAM_HEADER_FIELDS, name);
    assert_eq!(wire[at("ver")], PROTO_MAJOR);
    assert_eq!(wire[at("ptype")], Ptype::ClockProbe.as_u8());
    assert_eq!(
        u16::from_le_bytes([wire[at("flags")], wire[at("flags") + 1]]),
        Flags::FEC_REDUNDANT.bits(),
        "flags 必须是 2 B 小端，且落在这张表算出来的偏移上"
    );
    assert_eq!(
        u32::from_le_bytes([
            wire[at("stream_id")],
            wire[at("stream_id") + 1],
            wire[at("stream_id") + 2],
            wire[at("stream_id") + 3],
        ]),
        0x1122_3344
    );
    assert_eq!(
        u32::from_le_bytes([
            wire[at("seq")],
            wire[at("seq") + 1],
            wire[at("seq") + 2],
            wire[at("seq") + 3],
        ]),
        0x5566_7788
    );
    assert_eq!(
        u32::from_le_bytes([
            wire[at("sample_index")],
            wire[at("sample_index") + 1],
            wire[at("sample_index") + 2],
            wire[at("sample_index") + 3],
        ]),
        0x99AA_BBCC
    );
    assert_eq!(
        u64::from_le_bytes([
            wire[at("epoch_id")],
            wire[at("epoch_id") + 1],
            wire[at("epoch_id") + 2],
            wire[at("epoch_id") + 3],
            wire[at("epoch_id") + 4],
            wire[at("epoch_id") + 5],
            wire[at("epoch_id") + 6],
            wire[at("epoch_id") + 7],
        ]),
        0x0102_0304_0506_0708
    );
    assert_eq!(AudioDatagramHeader::decode(&wire).unwrap(), sentinel);
}

#[test]
fn clock_probe_len_equals_sum_of_field_widths() {
    assert_eq!(width_sum(CLOCK_PROBE_FIELDS), CLOCK_PROBE_SPEC_LEN);
    assert_eq!(CLOCK_PROBE_PAYLOAD_LEN, width_sum(CLOCK_PROBE_FIELDS));
    assert_eq!(ClockProbe::LEN, CLOCK_PROBE_SPEC_LEN);
    assert_eq!(
        CLOCK_PROBE_PROBE_SEQ_OFFSET,
        offset_of(CLOCK_PROBE_FIELDS, "probe_seq")
    );
    assert_eq!(CLOCK_PROBE_T1_OFFSET, offset_of(CLOCK_PROBE_FIELDS, "t1"));

    let probe = ClockProbe {
        probe_seq: 7,
        t1: 0x1122_3344_5566_7788,
    };
    let payload = probe.encode_to_vec().unwrap();
    assert_eq!(
        payload.len(),
        CLOCK_PROBE_SPEC_LEN,
        "编码长度必须等于 §3 表值 12 B"
    );
    assert_eq!(ClockProbe::decode(&payload).unwrap(), probe);
    let t1_at = CLOCK_PROBE_T1_OFFSET;
    assert_eq!(
        i64::from_le_bytes([
            payload[t1_at],
            payload[t1_at + 1],
            payload[t1_at + 2],
            payload[t1_at + 3],
            payload[t1_at + 4],
            payload[t1_at + 5],
            payload[t1_at + 6],
            payload[t1_at + 7],
        ]),
        probe.t1,
        "t1 必须落在这张表算出来的偏移上"
    );

    let probe_wire = probe.to_datagram_bytes().unwrap();
    let datagram = AudioDatagram::decode(&probe_wire).unwrap();
    assert_eq!(datagram.payload.len(), CLOCK_PROBE_SPEC_LEN);
}

#[test]
fn clock_reply_len_equals_sum_of_field_widths() {
    assert_eq!(width_sum(CLOCK_REPLY_FIELDS), CLOCK_REPLY_SPEC_LEN);
    assert_eq!(CLOCK_REPLY_PAYLOAD_LEN, width_sum(CLOCK_REPLY_FIELDS));
    assert_eq!(ClockReply::LEN, CLOCK_REPLY_SPEC_LEN);
    assert_eq!(
        CLOCK_REPLY_PROBE_SEQ_OFFSET,
        offset_of(CLOCK_REPLY_FIELDS, "probe_seq")
    );
    assert_eq!(CLOCK_REPLY_T1_OFFSET, offset_of(CLOCK_REPLY_FIELDS, "t1"));
    assert_eq!(CLOCK_REPLY_T2_OFFSET, offset_of(CLOCK_REPLY_FIELDS, "t2"));
    assert_eq!(CLOCK_REPLY_T3_OFFSET, offset_of(CLOCK_REPLY_FIELDS, "t3"));

    let reply = ClockReply {
        probe_seq: 9,
        t1: 1_000_000,
        t2: 1_000_120,
        t3: 1_000_121,
    };
    let payload = reply.encode_to_vec().unwrap();
    assert_eq!(
        payload.len(),
        CLOCK_REPLY_SPEC_LEN,
        "编码长度必须等于 §3 表值 28 B"
    );
    assert_eq!(ClockReply::decode(&payload).unwrap(), reply);

    // 三个时间戳各自落在表算出来的偏移上（错位会被这一组断言抓住）
    for (name, expected) in [("t1", reply.t1), ("t2", reply.t2), ("t3", reply.t3)] {
        let at = offset_of(CLOCK_REPLY_FIELDS, name);
        let mut raw = [0u8; 8];
        raw.copy_from_slice(&payload[at..at + 8]);
        assert_eq!(i64::from_le_bytes(raw), expected, "{name} 偏移与 §3 表不符");
    }

    let reply_wire = reply.to_datagram_bytes().unwrap();
    let datagram = AudioDatagram::decode(&reply_wire).unwrap();
    assert_eq!(datagram.payload.len(), CLOCK_REPLY_SPEC_LEN);
}

#[test]
fn control_header_len_equals_sum_of_field_widths() {
    assert_eq!(width_sum(CONTROL_HEADER_FIELDS), CONTROL_HEADER_SPEC_LEN);
    assert_eq!(
        CONTROL_HEADER_LEN,
        width_sum(CONTROL_HEADER_FIELDS),
        "§4 控制帧固定帧头 = ver + type + flags + payload_len"
    );
}

#[test]
fn payload_rules_match_the_section_3_table() {
    assert_eq!(
        Ptype::ClockProbe.payload_len_rule(),
        PayloadLenRule::Exact {
            len: CLOCK_PROBE_SPEC_LEN
        },
        "§3：CLOCK_PROBE 载荷恰 12 B"
    );
    assert_eq!(
        Ptype::ClockReply.payload_len_rule(),
        PayloadLenRule::Exact {
            len: CLOCK_REPLY_SPEC_LEN
        },
        "§3：CLOCK_REPLY 载荷恰 28 B"
    );
    assert_eq!(
        Ptype::Keepalive.payload_len_rule(),
        PayloadLenRule::Exact { len: 0 },
        "§3：KEEPALIVE 无载荷"
    );
    assert_eq!(KEEPALIVE_PAYLOAD_LEN, 0);
    assert_eq!(
        Ptype::Nack.payload_len_rule(),
        PayloadLenRule::U32List {
            min_items: NACK_MIN_ITEMS,
            max_items: NACK_MAX_ITEMS,
        },
        "§3：NACK 是 u32 列表，1 ≤ n ≤ 16"
    );
    assert_eq!(
        (NACK_MIN_ITEMS, NACK_MAX_ITEMS, NACK_ITEM_LEN),
        (1, 16, 4),
        "§3 表值：u32 列表 1..=16 项、每项 4 B"
    );
    for ptype in [Ptype::Audio, Ptype::Fec] {
        assert_eq!(
            ptype.payload_len_rule(),
            PayloadLenRule::Opaque {
                max: DATAGRAM_MAX_PAYLOAD
            },
            "§3：AUDIO / FEC 是不透明载荷，上限吃到 MTU"
        );
    }
}

#[test]
fn fixed_payloads_accept_only_their_declared_length() {
    let cases: [(Ptype, usize); 3] = [
        (Ptype::ClockProbe, CLOCK_PROBE_SPEC_LEN),
        (Ptype::ClockReply, CLOCK_REPLY_SPEC_LEN),
        (Ptype::Keepalive, 0),
    ];
    for (ptype, declared) in cases {
        for len in 0..=64usize {
            let bytes = auxiliary_bytes(ptype.as_u8(), len);
            let result = AudioDatagram::decode(&bytes);
            if len == declared {
                assert_eq!(
                    result.unwrap().payload.len(),
                    declared,
                    "{}: {declared} B 载荷必须被接受",
                    ptype.name()
                );
            } else {
                assert_bad_request(
                    result,
                    &format!("{}: 载荷 {len} B（应恰为 {declared} B）", ptype.name()),
                );
            }
        }
    }
}

#[test]
fn every_ptype_payload_rule_agrees_with_the_wire_decoder() {
    for ptype in Ptype::ALL {
        match ptype.payload_len_rule() {
            PayloadLenRule::Opaque { max } => {
                assert_eq!(
                    max + DATAGRAM_HEADER_LEN,
                    DATAGRAM_MAX_LEN,
                    "{}: 载荷上限必须正好吃到 MTU 上限",
                    ptype.name()
                );
                assert!(
                    AudioDatagram::decode(&auxiliary_bytes(ptype.as_u8(), max)).is_ok(),
                    "{}: 载荷上限必须被接受",
                    ptype.name()
                );
                assert_bad_request(
                    AudioDatagram::decode(&auxiliary_bytes(ptype.as_u8(), max + 1)),
                    &format!("{}: 超 MTU 1 B", ptype.name()),
                );
            }
            PayloadLenRule::Exact { len } => {
                let bytes = auxiliary_bytes(ptype.as_u8(), len);
                let accepted = AudioDatagram::decode(&bytes);
                assert!(accepted.is_ok(), "{}: {len} B 载荷必须被接受", ptype.name());
                assert_eq!(accepted.unwrap().payload.len(), len);
                if len > 0 {
                    assert_bad_request(
                        AudioDatagram::decode(&auxiliary_bytes(ptype.as_u8(), len - 1)),
                        &format!("{}: 少 1 B", ptype.name()),
                    );
                }
                assert_bad_request(
                    AudioDatagram::decode(&auxiliary_bytes(ptype.as_u8(), len + 1)),
                    &format!("{}: 多 1 B", ptype.name()),
                );
            }
            PayloadLenRule::U32List {
                min_items,
                max_items,
            } => {
                for items in [min_items, max_items] {
                    assert!(
                        AudioDatagram::decode(&auxiliary_bytes(
                            ptype.as_u8(),
                            items * NACK_ITEM_LEN
                        ))
                        .is_ok(),
                        "{}: {items} 项必须被接受",
                        ptype.name()
                    );
                }
                // 0 项与 max + 1 项都越界；非 4 倍数同样是结构性错误
                assert_bad_request(
                    AudioDatagram::decode(&auxiliary_bytes(ptype.as_u8(), 0)),
                    &format!("{}: 0 项（< min_items）", ptype.name()),
                );
                assert_bad_request(
                    AudioDatagram::decode(&auxiliary_bytes(
                        ptype.as_u8(),
                        (max_items + 1) * NACK_ITEM_LEN,
                    )),
                    &format!("{}: {} 项（> max_items）", ptype.name(), max_items + 1),
                );
                assert_bad_request(
                    AudioDatagram::decode(&auxiliary_bytes(
                        ptype.as_u8(),
                        min_items * NACK_ITEM_LEN + 1,
                    )),
                    &format!("{}: 非 u32 对齐的载荷长度", ptype.name()),
                );
            }
        }
    }
}

#[test]
fn nack_list_ledger_is_self_consistent() {
    assert_eq!(NackList::MAX_ITEMS, NACK_MAX_ITEMS);
    assert_eq!(NackList::MAX_ITEMS, 16, "§3 表值：NACK 最多 16 项");

    let full = NackList::from_slice(&[7u32; 16]).unwrap();
    assert_eq!(full.seqs().len(), 16);
    assert_eq!(full.payload_len(), 16 * NACK_ITEM_LEN);
    assert_eq!(full.payload_len(), 64, "§3：16 × u32 = 64 B");
    assert_eq!(full.encode_to_vec().unwrap().len(), full.payload_len());

    let one = NackList::from_slice(&[42]).unwrap();
    assert_eq!(one.payload_len(), NACK_ITEM_LEN);
    assert_eq!(one.encode_to_vec().unwrap().len(), 4);

    assert!(NackList::from_slice(&[]).is_err(), "0 项必须被拒绝");
    let too_many = vec![0u32; 17];
    assert!(NackList::from_slice(&too_many).is_err(), "17 项必须被拒绝");

    let wire = full.to_datagram_bytes().unwrap();
    assert_eq!(wire.len(), DATAGRAM_HEADER_LEN + full.payload_len());
    let datagram = AudioDatagram::decode(&wire).unwrap();
    assert_eq!(NackList::decode(datagram.payload).unwrap(), full);
}

#[test]
fn ptype_table_covers_every_declared_value() {
    assert_eq!(Ptype::ALL.len(), 6, "§3 ptype 表目前 6 项");
    for ptype in Ptype::ALL {
        assert_eq!(
            Ptype::from_u8(ptype.as_u8()),
            Some(ptype),
            "{}: from_u8/as_u8 必须互为逆运算",
            ptype.name()
        );
    }
    for raw in [0x00u8, 0x07, 0x7F, 0xFF] {
        assert_eq!(Ptype::from_u8(raw), None, "未知 ptype 必须返回 None");
    }
}
