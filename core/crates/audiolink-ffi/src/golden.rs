//! 跨端一致性夹具：`docs/03-protocol.md` §12 的三组 golden vectors（契约 §8 的 `protocolSelfTest`）
//!
//! # 这个模块在防什么
//!
//! 看板条目 `[护栏] 跨端一致性夹具：同一组 golden vectors 在 Rust 与 FFI 双跑`。
//! 真正的风险不是「协议编解码写错了」（`audiolink-proto` 的契约测试已经在盯），
//! 而是**同一组向量在两处各存一份、然后悄悄漂移** —— 那是"看起来绿、实际两端不一致"的经典形态。
//!
//! 所以这里做两件事：
//!
//! 1. 用**同一组字节**在 FFI 侧再跑一遍「解码 → 字段 → 再编码逐字节相同」，
//!    结果以字符串返回给 Kotlin（`protocolSelfTest()`）；
//! 2. 把 `audiolink-proto/tests/golden_vectors.rs` 的**原文**用 `include_str!` 在编译期嵌进来，
//!    运行期把下面的四份字节字面量与它逐字节比对（见 [`check_frozen_source`]）。
//!    这样「两处是不是同一组向量」不再靠人眼，而是每次 `protocolSelfTest()` 都验一次。
//!
//! # 为什么不是直接 `include!` 那个测试文件
//!
//! 那个文件里的四个向量常量都是**私有**的（`const` 无 `pub`），`#[path]` 引入后父模块取不到；
//! 而在那文件里加 `pub` 属于改别人的写域（`docs/11-m1-contract.md` §1：proto 属 lead）。
//! `include_str!` 只读不写，并且同样能把漂移变成红灯。

use audiolink_proto::{AudioDatagram, ClockProbe, ControlFrame, ControlFrameHeader};
use audiolink_types::{Flags, OpCode, PROTO_MAJOR, PROTO_MINOR, PROTO_VERSION, Ptype};

/// §12 示例 1：立体声 Opus 音频包（28 B）。
///
/// 与 `audiolink-proto/tests/golden_vectors.rs` 的 `VECTOR_AUDIO` 逐字节相同（由 [`check_frozen_source`] 断言）。
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

/// §12 示例 3 的 8 B 帧头 + `request_id`（信封级向量）。
const VECTOR_CONTROL_HEAD: &[u8] = &[
    0x02, 0x01, 0x00, 0x00, // ver = 0x02, type = 0x01 (HELLO), flags = 0x0000
    0x0A, 0x00, 0x00, 0x00, // payload_len = 10
    0x01, 0x00, 0x00, 0x00, // request_id = 1
];

/// 向量 3 的 10 B 载荷：文档未定义其内容 → 按不透明字节处理（§12 示例 3 附注）。
const VECTOR_CONTROL_PAYLOAD: &[u8] = &[0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09];

/// 冻结来源的原文（编译期嵌入）。**只读**：本 crate 不写 `audiolink-proto`（契约 §1 写域）。
const FROZEN_SOURCE: &str = include_str!("../../audiolink-proto/tests/golden_vectors.rs");

/// 冻结来源的相对路径（报告里写清楚「跟谁比」）。
const FROZEN_SOURCE_PATH: &str = "core/crates/audiolink-proto/tests/golden_vectors.rs";

/// 单个检查项的结果。
#[derive(Debug, Clone)]
pub struct Check {
    /// 检查项名。
    pub name: &'static str,
    /// 通过与否。
    pub ok: bool,
    /// 细节（通过时也给：报告要能自证跑了什么）。
    pub detail: String,
}

/// 整轮自检的结果。
#[derive(Debug, Clone)]
pub struct SelfTest {
    /// 逐项结果。
    pub checks: Vec<Check>,
}

impl SelfTest {
    /// 全部通过？
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|check| check.ok)
    }

    /// 通过项数。
    pub fn passed_count(&self) -> usize {
        self.checks.iter().filter(|check| check.ok).count()
    }

    /// 人类可读摘要（Kotlin 侧直接展示 / 断言 `RESULT: PASS`）。
    pub fn summary(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "audiolink-proto self-test @ audiolink-ffi {}\n",
            env!("CARGO_PKG_VERSION")
        ));
        out.push_str(&format!(
            "proto_version=0x{PROTO_VERSION:04x} (major={PROTO_MAJOR}, minor={PROTO_MINOR})\n"
        ));
        for check in &self.checks {
            let mark = if check.ok { "ok" } else { "FAIL" };
            out.push_str(&format!("[{mark}] {}: {}\n", check.name, check.detail));
        }
        out.push_str(&format!(
            "RESULT: {} ({}/{})",
            if self.passed() { "PASS" } else { "FAIL" },
            self.passed_count(),
            self.checks.len()
        ));
        out
    }
}

/// 跑一遍全部检查（**不 panic**：失败就是 `ok: false` + 细节）。
pub fn run() -> SelfTest {
    SelfTest {
        checks: vec![
            check_audio_vector(),
            check_clock_probe_vector(),
            check_control_vector(),
            check_proto_version(),
            check_frozen_source(),
        ],
    }
}

/// `protocolSelfTest()` 的实现体。
///
/// 返回摘要；`RESULT: PASS` 表示三组向量在 FFI 侧与 Rust 侧表现一致。
pub fn summary() -> String {
    run().summary()
}

/// 契约 §8 的 `protocolSelfTest() -> String`。
#[uniffi::export]
pub fn protocol_self_test() -> String {
    summary()
}

/// 向量 1：音频数据报「解码 → 字段 → 再编码逐字节相同」。
fn check_audio_vector() -> Check {
    let name = "vector1.audio";
    let datagram = match AudioDatagram::decode(VECTOR_AUDIO) {
        Ok(datagram) => datagram,
        Err(error) => return fail(name, format!("decode failed: {error}")),
    };

    let expected_epoch = 0x1122_3344_5566_7788u64;
    let mismatches: Vec<&str> = [
        (datagram.header.version == PROTO_MAJOR, "version"),
        (datagram.header.ptype == Ptype::Audio, "ptype"),
        (datagram.header.flags == Flags::NONE, "flags"),
        (datagram.header.stream_id == 1, "stream_id"),
        (datagram.header.seq == 42, "seq"),
        (datagram.header.sample_index == 960, "sample_index"),
        (datagram.header.epoch_id == expected_epoch, "epoch_id"),
        (datagram.payload == [0xDE, 0xAD, 0xBE, 0xEF], "payload"),
    ]
    .into_iter()
    .filter(|(ok, _)| !ok)
    .map(|(_, field)| field)
    .collect();
    if !mismatches.is_empty() {
        return fail(name, format!("field mismatch: {}", mismatches.join(", ")));
    }

    match datagram.encode_to_vec().as_deref() {
        Ok(bytes) if bytes == VECTOR_AUDIO => {}
        Ok(bytes) => {
            return fail(
                name,
                format!(
                    "re-encode differs: {} B vs {} B",
                    bytes.len(),
                    VECTOR_AUDIO.len()
                ),
            );
        }
        Err(error) => return fail(name, format!("re-encode failed: {error}")),
    }

    // 热路径写法（复用缓冲）也必须逐字节相同 —— 它与 `encode_to_vec` 是两条实现路径。
    let mut buffer = [0u8; 64];
    match datagram.encode_into(&mut buffer) {
        Ok(written) if buffer.get(..written) == Some(VECTOR_AUDIO) => {}
        Ok(written) => {
            return fail(name, format!("encode_into differs (wrote {written} B)"));
        }
        Err(error) => return fail(name, format!("encode_into failed: {error}")),
    }

    ok(
        name,
        format!(
            "{} B decode+re-encode+encode_into 一致；seq=42 sample_index=960 epoch=0x{:016x} payload=4 B",
            VECTOR_AUDIO.len(),
            datagram.header.epoch_id
        ),
    )
}

/// 向量 2：时钟探测数据报。
fn check_clock_probe_vector() -> Check {
    let name = "vector2.clock_probe";
    if VECTOR_CLOCK_PROBE.len() != 24 + 12 {
        return fail(name, format!("length {} != 36", VECTOR_CLOCK_PROBE.len()));
    }

    let datagram = match AudioDatagram::decode(VECTOR_CLOCK_PROBE) {
        Ok(datagram) => datagram,
        Err(error) => return fail(name, format!("decode failed: {error}")),
    };
    if datagram.header.ptype != Ptype::ClockProbe {
        return fail(name, "ptype != ClockProbe".to_string());
    }

    let probe = match ClockProbe::from_datagram(&datagram) {
        Ok(probe) => probe,
        Err(error) => return fail(name, format!("from_datagram failed: {error}")),
    };
    if probe.probe_seq != 7 {
        return fail(name, format!("probe_seq = {} != 7", probe.probe_seq));
    }
    if probe.t1 != 1_000_000 {
        return fail(name, format!("t1 = {} != 1_000_000", probe.t1));
    }

    match probe.encode_to_vec().as_deref() {
        Ok(bytes) if bytes == &VECTOR_CLOCK_PROBE[24..] => {}
        Ok(bytes) => {
            return fail(
                name,
                format!("payload re-encode is {} B, expected 12 B", bytes.len()),
            );
        }
        Err(error) => return fail(name, format!("payload re-encode failed: {error}")),
    }
    match probe.to_datagram_bytes().as_deref() {
        Ok(bytes) if bytes == VECTOR_CLOCK_PROBE => {}
        Ok(_) => {
            return fail(
                name,
                "to_datagram_bytes differs from the golden vector".to_string(),
            );
        }
        Err(error) => return fail(name, format!("to_datagram_bytes failed: {error}")),
    }

    ok(
        name,
        format!(
            "{} B（24 B 帧头 + 12 B 载荷）round-trip 一致；probe_seq=7 t1={} µs",
            VECTOR_CLOCK_PROBE.len(),
            probe.t1
        ),
    )
}

/// 向量 3：控制帧信封（10 B 载荷按不透明字节处理）。
fn check_control_vector() -> Check {
    let name = "vector3.control";
    let mut wire = VECTOR_CONTROL_HEAD.to_vec();
    wire.extend_from_slice(VECTOR_CONTROL_PAYLOAD);

    let header = match ControlFrameHeader::decode(&wire) {
        Ok(header) => header,
        Err(error) => return fail(name, format!("header decode failed: {error}")),
    };
    if header.ver != PROTO_MAJOR
        || header.raw_op != OpCode::Hello.as_u8()
        || header.flags != 0
        || header.payload_len != 10
        || header.request_id != 1
    {
        return fail(
            name,
            format!(
                "envelope mismatch: ver={} op={} flags={} len={} request_id={}",
                header.ver, header.raw_op, header.flags, header.payload_len, header.request_id
            ),
        );
    }

    let frame = match ControlFrame::decode(&wire) {
        Ok(frame) => frame,
        Err(error) => return fail(name, format!("frame decode failed: {error}")),
    };
    if frame.op != OpCode::Hello || frame.request_id != 1 {
        return fail(name, "frame op / request_id mismatch".to_string());
    }
    if frame.payload != VECTOR_CONTROL_PAYLOAD {
        return fail(name, format!("payload = {:?}", frame.payload));
    }
    match frame.encode_to_vec().as_deref() {
        Ok(bytes) if bytes == wire.as_slice() => {}
        Ok(bytes) => {
            return fail(
                name,
                format!("re-encode is {} B, expected {}", bytes.len(), wire.len()),
            );
        }
        Err(error) => return fail(name, format!("re-encode failed: {error}")),
    }

    ok(
        name,
        format!(
            "{} B envelope + {} B opaque payload round-trip 一致；op=HELLO request_id=1",
            VECTOR_CONTROL_HEAD.len(),
            VECTOR_CONTROL_PAYLOAD.len()
        ),
    )
}

/// §13：版本常量必须自洽（`PROTO_VERSION == major << 8 | minor`）。
fn check_proto_version() -> Check {
    let name = "proto_version.constants";
    let composed = u16::from(PROTO_MAJOR) << 8 | u16::from(PROTO_MINOR);
    if composed != PROTO_VERSION {
        return fail(
            name,
            format!("major<<8|minor = 0x{composed:04x} != PROTO_VERSION 0x{PROTO_VERSION:04x}"),
        );
    }
    if u16::from_be_bytes([PROTO_MAJOR, PROTO_MINOR]) != PROTO_VERSION {
        return fail(
            name,
            "big-endian bytes do not compose PROTO_VERSION".to_string(),
        );
    }
    ok(name, format!("PROTO_VERSION = 0x{PROTO_VERSION:04x} 自洽"))
}

/// **漂移守卫**：本模块的四份字节字面量与 `audiolink-proto/tests/golden_vectors.rs` 逐字节相同。
fn check_frozen_source() -> Check {
    let name = "frozen_vectors_source";
    let pairs: [(&str, &[u8]); 4] = [
        ("VECTOR_AUDIO", VECTOR_AUDIO),
        ("VECTOR_CLOCK_PROBE", VECTOR_CLOCK_PROBE),
        ("VECTOR_CONTROL_HEAD", VECTOR_CONTROL_HEAD),
        ("VECTOR_CONTROL_PAYLOAD", VECTOR_CONTROL_PAYLOAD),
    ];

    let mut compared = 0usize;
    for (const_name, local) in pairs {
        let Some(frozen) = literal_bytes(FROZEN_SOURCE, const_name) else {
            // 解析失败也要红灯：不能因为「读不懂来源」就放行。
            return fail(
                name,
                format!("cannot parse `{const_name}` out of {FROZEN_SOURCE_PATH}"),
            );
        };
        if frozen != local {
            return fail(
                name,
                format!(
                    "`{const_name}` drifted: frozen {} B vs local {} B",
                    frozen.len(),
                    local.len()
                ),
            );
        }
        compared += 1;
    }

    ok(
        name,
        format!("{compared}/4 consts 与 {FROZEN_SOURCE_PATH} 逐字节相同"),
    )
}

fn ok(name: &'static str, detail: String) -> Check {
    Check {
        name,
        ok: true,
        detail,
    }
}

fn fail(name: &'static str, detail: String) -> Check {
    Check {
        name,
        ok: false,
        detail,
    }
}

/// 从源码原文里取 `const <name>: ... = &[0x.., ...];` 的字节。
///
/// 刻意不引 regex：这段解析要跟着上游源码格式走，越简单越好。两个必须处理的写法：
/// 1. 类型里也有 `&[`（如 `&[u8; 10]`）→ 只认「后面紧跟 `0x`」的那个 `&[`；
/// 2. 声明可以折行（`= &[…]` 的 `&[` 在下一行）→ 不能锚在 `= &[` 上。
fn literal_bytes(source: &str, const_name: &str) -> Option<Vec<u8>> {
    let head = format!("const {const_name}:");
    let start = source.find(&head)?;
    let rest = &source[start..];

    let mut search_from = 0usize;
    loop {
        let open = rest.get(search_from..)?.find("&[")? + search_from;
        let body_start = open + 2;
        let body = rest.get(body_start..)?;
        let trimmed = body.trim_start();
        if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
            let close = body.find(']')?;
            return parse_hex_literals(&body[..close]);
        }
        search_from = body_start;
    }
}

/// 扫出正文里的 `0xNN` 字面量。
///
/// **必须先砍掉行尾注释**：上游源码的注释里写着同样的十六进制（如
/// `0x02, 0x01, ... // version = 0x02, ptype = 0x01`），不砍就会把注释也当成数据 ——
/// 这不是假想：本模块第一版就是这么比的，`frozen_vectors_source` 直接报了
/// `frozen 32 B vs local 28 B`，多出来的 4 B 全来自注释。
fn parse_hex_literals(body: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for line in body.lines() {
        let code = match line.find("//") {
            Some(index) => &line[..index],
            None => line,
        };
        out.extend(scan_line(code)?);
    }
    if out.is_empty() { None } else { Some(out) }
}

/// 单行里的 `0xNN` 字面量；遇到非法十六进制返回 `None`（宁可红灯，不要静默漏字节）。
fn scan_line(line: &str) -> Option<Vec<u8>> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut index = 0usize;
    while index + 3 < bytes.len() {
        if bytes[index] == b'0' && matches!(bytes[index + 1], b'x' | b'X') {
            let high = hex_nibble(bytes[index + 2])?;
            let low = hex_nibble(bytes[index + 3])?;
            out.push((high << 4) | low);
            index += 4;
        } else {
            index += 1;
        }
    }
    Some(out)
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// 看板 `[护栏] 跨端一致性夹具` 的 **Rust 侧断言**。
    #[test]
    fn 自检全绿且摘要自证跑了什么() {
        let report = run();
        let summary = report.summary();
        assert!(report.passed(), "自检未全绿:\n{summary}");
        assert_eq!(report.checks.len(), 5);
        assert_eq!(report.passed_count(), 5);
        assert!(summary.contains("RESULT: PASS"), "{summary}");
        assert!(summary.contains("vector1.audio"), "{summary}");
        assert!(summary.contains("vector2.clock_probe"), "{summary}");
        assert!(summary.contains("vector3.control"), "{summary}");
        // `cargo test -p audiolink-ffi -- --nocapture` 直接打印夹具摘要：报告要附**原始输出**，
        // 而不是转述。
        println!("{summary}");
    }

    /// 导出函数与内部实现必须是同一个结果（Kotlin 看到的就是这里断言的东西）。
    #[test]
    fn 导出函数与摘要一致() {
        let exported = protocol_self_test();
        assert_eq!(exported, summary());
        assert!(exported.contains("RESULT: PASS"), "{exported}");
    }

    /// 漂移守卫必须真的比到了四份常量（而不是解析失败被当成空通过）。
    #[test]
    fn 冻结来源解析出四份向量且长度正确() {
        assert_eq!(
            literal_bytes(FROZEN_SOURCE, "VECTOR_AUDIO").unwrap(),
            VECTOR_AUDIO
        );
        assert_eq!(
            literal_bytes(FROZEN_SOURCE, "VECTOR_CLOCK_PROBE").unwrap(),
            VECTOR_CLOCK_PROBE
        );
        assert_eq!(
            literal_bytes(FROZEN_SOURCE, "VECTOR_CONTROL_HEAD").unwrap(),
            VECTOR_CONTROL_HEAD
        );
        assert_eq!(
            literal_bytes(FROZEN_SOURCE, "VECTOR_CONTROL_PAYLOAD").unwrap(),
            VECTOR_CONTROL_PAYLOAD
        );
        assert_eq!(
            VECTOR_CONTROL_PAYLOAD.len(),
            10,
            "类型里的 `&[u8; 10]` 不能被当成数组体"
        );
    }

    /// 解析器对「人为篡改」必须敏感 —— 否则守卫是装饰。
    #[test]
    fn 人为改一个字节会被守卫抓住() {
        let tampered =
            FROZEN_SOURCE.replacen("0xDE, 0xAD, 0xBE, 0xEF", "0xDE, 0xAD, 0xBE, 0xEE", 1);
        assert_ne!(tampered, FROZEN_SOURCE, "用例前提：原文里存在那四个字节");
        let frozen = literal_bytes(&tampered, "VECTOR_AUDIO").unwrap();
        assert_ne!(frozen, VECTOR_AUDIO);
    }

    /// 上游改名 / 删常量时必须红灯，不能静默跳过。
    #[test]
    fn 来源里找不到常量时判失败() {
        assert!(literal_bytes(FROZEN_SOURCE, "VECTOR_DOES_NOT_EXIST").is_none());
    }

    #[test]
    fn 十六进制字面量解析正确() {
        assert_eq!(
            parse_hex_literals("0x00, 0x0A, 0xff").unwrap(),
            vec![0x00, 0x0A, 0xFF]
        );
        assert!(parse_hex_literals("no literals here").is_none());
        assert!(parse_hex_literals("0xZZ").is_none());
    }

    /// 注释里的 `0xNN` 不能算进数据 —— 这正是第一版守卫误报 `32 B vs 28 B` 的原因。
    #[test]
    fn 行尾注释里的十六进制被忽略() {
        let source = "\
            0x02, 0x01, 0x00, 0x00, // version = 0x02, ptype = 0x01 (AUDIO), flags = 0x0000\n\
            0x2A, 0x00, 0x00, 0x00, // seq = 42\n\
        ";
        assert_eq!(
            parse_hex_literals(source).unwrap(),
            vec![0x02, 0x01, 0x00, 0x00, 0x2A, 0x00, 0x00, 0x00]
        );
    }
}
