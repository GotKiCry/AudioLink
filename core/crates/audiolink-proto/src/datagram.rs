//! 音频数据报编解码（`docs/03-protocol.md` §3）。
//!
//! 帧头 24 B 定长小端 + ptype 专用载荷（定长，**不使用 postcard**）；解码零拷贝（载荷借用原缓冲）。

use audiolink_types::{
    AudioLinkError, DATAGRAM_HEADER_LEN, DATAGRAM_MAX_LEN, Flags, PROTO_MAJOR, PayloadLenRule,
    Ptype,
};

use crate::{ERR_OUT_OF_SPACE, fixed_bytes, put};

/// NACK 载荷最多携带的 seq 个数（§3：1 ≤ n ≤ 16）。
pub const NACK_MAX_ITEMS: usize = 16;

/// 音频数据报帧头（§3，24 B 定长小端）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioDatagramHeader {
    /// ALP 主版本字节，必须为 `0x02`。
    pub version: u8,
    /// 报文类型（§3 ptype 表）。
    pub ptype: Ptype,
    /// 标志位；保留位（bit 5–15）必须为 0。
    pub flags: Flags,
    /// 会话内唯一的流 ID（辅助包置 0）。
    pub stream_id: u32,
    /// 每流单调 +1 的序号（辅助包置 0）。
    pub seq: u32,
    /// 负载首样本在 epoch 基准下的样本序号（48 kHz 计）。
    pub sample_index: u32,
    /// 会话 / 同步组基准标识（会话重建后换新值）。
    pub epoch_id: u64,
}

impl AudioDatagramHeader {
    /// 帧头长度（24 B）。
    pub const LEN: usize = DATAGRAM_HEADER_LEN;

    /// 构造**辅助包**帧头（`CLOCK_*` / `KEEPALIVE` / `NACK`）：版本 `0x02`、`flags` 为 0、其余字段置 0（§3、§12 示例 2）。
    pub const fn auxiliary(ptype: Ptype) -> Self {
        Self {
            version: PROTO_MAJOR,
            ptype,
            flags: Flags::NONE,
            stream_id: 0,
            seq: 0,
            sample_index: 0,
            epoch_id: 0,
        }
    }

    /// 解析并校验帧头（§1.1 的 L1 规则）。
    pub fn decode(buf: &[u8]) -> Result<Self, AudioLinkError> {
        let [version] = fixed_bytes::<1>(buf, 0, "datagram header truncated")?;
        let [raw_ptype] = fixed_bytes::<1>(buf, 1, "datagram header truncated")?;
        let raw_flags = u16::from_le_bytes(fixed_bytes::<2>(buf, 2, "datagram header truncated")?);
        let stream_id = u32::from_le_bytes(fixed_bytes::<4>(buf, 4, "datagram header truncated")?);
        let seq = u32::from_le_bytes(fixed_bytes::<4>(buf, 8, "datagram header truncated")?);
        let sample_index =
            u32::from_le_bytes(fixed_bytes::<4>(buf, 12, "datagram header truncated")?);
        let epoch_id = u64::from_le_bytes(fixed_bytes::<8>(buf, 16, "datagram header truncated")?);

        let ptype =
            Ptype::from_u8(raw_ptype).ok_or(AudioLinkError::bad_request("unknown ptype"))?;
        let header = Self {
            version,
            ptype,
            flags: Flags::from_bits(raw_flags),
            stream_id,
            seq,
            sample_index,
            epoch_id,
        };
        header.validate()?;
        Ok(header)
    }

    /// 写入 24 B 帧头，返回写入长度；字段非法或缓冲不足 → `BadRequest`。
    pub fn encode_into(&self, out: &mut [u8]) -> Result<usize, AudioLinkError> {
        self.validate()?;
        let dst = out
            .get_mut(..Self::LEN)
            .ok_or(AudioLinkError::bad_request(ERR_OUT_OF_SPACE))?;
        put(dst, 0, &[self.version], ERR_OUT_OF_SPACE)?;
        put(dst, 1, &[self.ptype.as_u8()], ERR_OUT_OF_SPACE)?;
        put(dst, 2, &self.flags.bits().to_le_bytes(), ERR_OUT_OF_SPACE)?;
        put(dst, 4, &self.stream_id.to_le_bytes(), ERR_OUT_OF_SPACE)?;
        put(dst, 8, &self.seq.to_le_bytes(), ERR_OUT_OF_SPACE)?;
        put(dst, 12, &self.sample_index.to_le_bytes(), ERR_OUT_OF_SPACE)?;
        put(dst, 16, &self.epoch_id.to_le_bytes(), ERR_OUT_OF_SPACE)?;
        Ok(Self::LEN)
    }

    /// 编码为 24 B 帧头。
    pub fn encode_to_vec(&self) -> Result<Vec<u8>, AudioLinkError> {
        let mut out = [0u8; Self::LEN];
        self.encode_into(&mut out)?;
        Ok(out.to_vec())
    }

    /// 帧头字段校验：版本必须匹配主版本，保留位必须为 0（§1.1）。
    fn validate(&self) -> Result<(), AudioLinkError> {
        if self.version != PROTO_MAJOR {
            return Err(AudioLinkError::bad_request("datagram version != 0x02"));
        }
        if self.flags.has_reserved_bits() {
            return Err(AudioLinkError::bad_request(
                "reserved flag bits (5-15) must be 0",
            ));
        }
        Ok(())
    }
}

/// 音频数据报：帧头 + 载荷（载荷借用输入缓冲 → 解码零拷贝）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioDatagram<'a> {
    /// 帧头。
    pub header: AudioDatagramHeader,
    /// 载荷（`AUDIO` / `FEC` 为不透明字节串；其余 ptype 为定长结构）。
    pub payload: &'a [u8],
}

impl<'a> AudioDatagram<'a> {
    /// 解析并**严格校验**整包：总长 ≤ 1200 B、帧头合法、ptype 载荷长度符合 §3 载荷表。
    pub fn decode(buf: &'a [u8]) -> Result<Self, AudioLinkError> {
        if buf.len() > DATAGRAM_MAX_LEN {
            return Err(AudioLinkError::bad_request(
                "datagram exceeds 1200 B MTU limit",
            ));
        }
        let header = AudioDatagramHeader::decode(buf)?;
        let payload = buf
            .get(AudioDatagramHeader::LEN..)
            .ok_or(AudioLinkError::bad_request("datagram header truncated"))?;
        check_payload(&header, payload.len())?;
        Ok(Self { header, payload })
    }

    /// 整包长度（含帧头）；载荷非法或超 MTU → `BadRequest`。
    pub fn total_len(&self) -> Result<usize, AudioLinkError> {
        check_payload(&self.header, self.payload.len())?;
        let total = DATAGRAM_HEADER_LEN
            .checked_add(self.payload.len())
            .ok_or(AudioLinkError::bad_request("datagram length overflow"))?;
        if total > DATAGRAM_MAX_LEN {
            return Err(AudioLinkError::bad_request(
                "datagram exceeds 1200 B MTU limit",
            ));
        }
        Ok(total)
    }

    /// 编码进调用方缓冲（热路径零分配），返回写入字节数。
    pub fn encode_into(&self, out: &mut [u8]) -> Result<usize, AudioLinkError> {
        let total = self.total_len()?;
        let dst = out
            .get_mut(..total)
            .ok_or(AudioLinkError::bad_request(ERR_OUT_OF_SPACE))?;
        self.header.encode_into(dst)?;
        put(
            dst,
            AudioDatagramHeader::LEN,
            self.payload,
            ERR_OUT_OF_SPACE,
        )?;
        Ok(total)
    }

    /// 编码为 `Vec`（用于抓包 / 测试 / 控制面；音频热路径请用 [`AudioDatagram::encode_into`]）。
    pub fn encode_to_vec(&self) -> Result<Vec<u8>, AudioLinkError> {
        let total = self.total_len()?;
        let mut out = vec![0u8; total];
        self.encode_into(&mut out)?;
        Ok(out)
    }
}

/// 校验 ptype 专用载荷长度是否符合 §3 载荷表。
fn check_payload(header: &AudioDatagramHeader, payload_len: usize) -> Result<(), AudioLinkError> {
    match header.ptype.payload_len_rule() {
        PayloadLenRule::Opaque { max } => {
            if payload_len > max {
                return Err(AudioLinkError::bad_request_owned(format!(
                    "{} payload must be <= {max} B, got {payload_len} B",
                    header.ptype.name()
                )));
            }
            if payload_len == 0
                && header.ptype == Ptype::Audio
                && !header.flags.contains(Flags::DTX)
            {
                return Err(AudioLinkError::bad_request(
                    "AUDIO payload may be empty only with the DTX flag",
                ));
            }
            Ok(())
        }
        PayloadLenRule::Exact { len } => {
            if payload_len != len {
                return Err(AudioLinkError::bad_request_owned(format!(
                    "{} payload must be exactly {len} B, got {payload_len} B",
                    header.ptype.name()
                )));
            }
            Ok(())
        }
        PayloadLenRule::U32List {
            min_items,
            max_items,
        } => {
            u32_items(payload_len, min_items, max_items)?;
            Ok(())
        }
    }
}

/// 校验 `u32` 列表载荷长度并返回元素个数（§3：4 的倍数，元素数落在 `min..=max`）。
fn u32_items(
    payload_len: usize,
    min_items: usize,
    max_items: usize,
) -> Result<usize, AudioLinkError> {
    if !payload_len.is_multiple_of(4) || payload_len < min_items * 4 || payload_len > max_items * 4
    {
        return Err(AudioLinkError::bad_request_owned(format!(
            "u32 list payload must be {}..={} items, got {} B",
            min_items, max_items, payload_len
        )));
    }
    Ok(payload_len / 4)
}

/// 辅助包（非 `AUDIO` / `FEC`）的载荷借用：ptype 不符 → `BadRequest`。
fn auxiliary_payload<'a>(
    datagram: &AudioDatagram<'a>,
    expected: Ptype,
) -> Result<&'a [u8], AudioLinkError> {
    if datagram.header.ptype != expected {
        return Err(AudioLinkError::bad_request_owned(format!(
            "expected {} datagram, got {}",
            expected.name(),
            datagram.header.ptype.name()
        )));
    }
    Ok(datagram.payload)
}

/// 组装辅助包整包（帧头字段按 §3 置 0）。
fn auxiliary_datagram_bytes(ptype: Ptype, payload: &[u8]) -> Result<Vec<u8>, AudioLinkError> {
    AudioDatagram {
        header: AudioDatagramHeader::auxiliary(ptype),
        payload,
    }
    .encode_to_vec()
}

/// `Ptype` 的人类可读名称（L1 错误上下文用）。
trait PtypeName {
    /// 名称，如 `CLOCK_PROBE`。
    fn name(self) -> &'static str;
}

impl PtypeName for Ptype {
    fn name(self) -> &'static str {
        match self {
            Self::Audio => "AUDIO",
            Self::Fec => "FEC",
            Self::ClockProbe => "CLOCK_PROBE",
            Self::ClockReply => "CLOCK_REPLY",
            Self::Keepalive => "KEEPALIVE",
            Self::Nack => "NACK",
        }
    }
}

/// 时钟探测请求载荷（§3 载荷表：恰 12 B）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockProbe {
    /// 探测序号（应答中回填，用于配对请求/应答）。
    pub probe_seq: u32,
    /// 发送时刻（本地单调时钟，µs）。
    pub t1: i64,
}

impl ClockProbe {
    /// 载荷长度（12 B）。
    pub const LEN: usize = 12;

    /// 解析载荷（长度必须恰为 12 B）。
    pub fn decode(payload: &[u8]) -> Result<Self, AudioLinkError> {
        if payload.len() != Self::LEN {
            return Err(AudioLinkError::bad_request(
                "CLOCK_PROBE payload must be exactly 12 B",
            ));
        }
        let probe_seq = u32::from_le_bytes(fixed_bytes::<4>(
            payload,
            0,
            "CLOCK_PROBE payload must be exactly 12 B",
        )?);
        let t1 = i64::from_le_bytes(fixed_bytes::<8>(
            payload,
            4,
            "CLOCK_PROBE payload must be exactly 12 B",
        )?);
        Ok(Self { probe_seq, t1 })
    }

    /// 编码进缓冲，返回写入长度（12）。
    pub fn encode_into(&self, out: &mut [u8]) -> Result<usize, AudioLinkError> {
        let dst = out
            .get_mut(..Self::LEN)
            .ok_or(AudioLinkError::bad_request(ERR_OUT_OF_SPACE))?;
        put(dst, 0, &self.probe_seq.to_le_bytes(), ERR_OUT_OF_SPACE)?;
        put(dst, 4, &self.t1.to_le_bytes(), ERR_OUT_OF_SPACE)?;
        Ok(Self::LEN)
    }

    /// 编码为 12 B 载荷。
    pub fn encode_to_vec(&self) -> Result<Vec<u8>, AudioLinkError> {
        let mut payload = [0u8; Self::LEN];
        self.encode_into(&mut payload)?;
        Ok(payload.to_vec())
    }

    /// 组装完整数据报（§12 示例 2 的形态）。
    pub fn to_datagram_bytes(&self) -> Result<Vec<u8>, AudioLinkError> {
        let payload = self.encode_to_vec()?;
        auxiliary_datagram_bytes(Ptype::ClockProbe, &payload)
    }

    /// 从整包解析（ptype 必须为 `CLOCK_PROBE`）。
    pub fn from_datagram(datagram: &AudioDatagram<'_>) -> Result<Self, AudioLinkError> {
        Self::decode(auxiliary_payload(datagram, Ptype::ClockProbe)?)
    }
}

/// 时钟探测应答载荷（§3 载荷表：恰 24 B）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockReply {
    /// 回填的探测序号。
    pub probe_seq: u32,
    /// 发起方发送时刻（µs，原样回填）。
    pub t1: i64,
    /// 应答方接收时刻（µs，应答方单调时钟）。
    pub t2: i64,
    /// 应答方发送时刻（µs，紧随 `t2`）。
    pub t3: i64,
}

impl ClockReply {
    /// 载荷长度（24 B）。
    pub const LEN: usize = 24;

    /// 解析载荷（长度必须恰为 24 B）。
    pub fn decode(payload: &[u8]) -> Result<Self, AudioLinkError> {
        if payload.len() != Self::LEN {
            return Err(AudioLinkError::bad_request(
                "CLOCK_REPLY payload must be exactly 24 B",
            ));
        }
        let probe_seq = u32::from_le_bytes(fixed_bytes::<4>(
            payload,
            0,
            "CLOCK_REPLY payload must be exactly 24 B",
        )?);
        let t1 = i64::from_le_bytes(fixed_bytes::<8>(
            payload,
            4,
            "CLOCK_REPLY payload must be exactly 24 B",
        )?);
        let t2 = i64::from_le_bytes(fixed_bytes::<8>(
            payload,
            12,
            "CLOCK_REPLY payload must be exactly 24 B",
        )?);
        let t3 = i64::from_le_bytes(fixed_bytes::<8>(
            payload,
            20,
            "CLOCK_REPLY payload must be exactly 24 B",
        )?);
        Ok(Self {
            probe_seq,
            t1,
            t2,
            t3,
        })
    }

    /// 编码进缓冲，返回写入长度（24）。
    pub fn encode_into(&self, out: &mut [u8]) -> Result<usize, AudioLinkError> {
        let dst = out
            .get_mut(..Self::LEN)
            .ok_or(AudioLinkError::bad_request(ERR_OUT_OF_SPACE))?;
        put(dst, 0, &self.probe_seq.to_le_bytes(), ERR_OUT_OF_SPACE)?;
        put(dst, 4, &self.t1.to_le_bytes(), ERR_OUT_OF_SPACE)?;
        put(dst, 12, &self.t2.to_le_bytes(), ERR_OUT_OF_SPACE)?;
        put(dst, 20, &self.t3.to_le_bytes(), ERR_OUT_OF_SPACE)?;
        Ok(Self::LEN)
    }

    /// 编码为 24 B 载荷。
    pub fn encode_to_vec(&self) -> Result<Vec<u8>, AudioLinkError> {
        let mut payload = [0u8; Self::LEN];
        self.encode_into(&mut payload)?;
        Ok(payload.to_vec())
    }

    /// 组装完整数据报。
    pub fn to_datagram_bytes(&self) -> Result<Vec<u8>, AudioLinkError> {
        let payload = self.encode_to_vec()?;
        auxiliary_datagram_bytes(Ptype::ClockReply, &payload)
    }

    /// 从整包解析（ptype 必须为 `CLOCK_REPLY`）。
    pub fn from_datagram(datagram: &AudioDatagram<'_>) -> Result<Self, AudioLinkError> {
        Self::decode(auxiliary_payload(datagram, Ptype::ClockReply)?)
    }
}

/// NACK 载荷（§3：`u32` 列表，1 ≤ n ≤ 16）。
///
/// 内部为定长数组 → 解码零分配、无对齐要求（不借用 `&[u32]`，避免未对齐读取）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NackList {
    items: [u32; NACK_MAX_ITEMS],
    len: usize,
}

impl NackList {
    /// 最多可携带的 seq 个数。
    pub const MAX_ITEMS: usize = NACK_MAX_ITEMS;

    /// 由切片构造（元素个数必须落在 `1..=16`）。
    pub fn from_slice(seqs: &[u32]) -> Result<Self, AudioLinkError> {
        if seqs.is_empty() || seqs.len() > NACK_MAX_ITEMS {
            return Err(AudioLinkError::bad_request(
                "NACK must carry 1..=16 seq values",
            ));
        }
        let mut items = [0u32; NACK_MAX_ITEMS];
        for (slot, value) in items.iter_mut().zip(seqs.iter()) {
            *slot = *value;
        }
        Ok(Self {
            items,
            len: seqs.len(),
        })
    }

    /// 解析载荷（长度必须为 4 的倍数且落在 4…64 B）。
    pub fn decode(payload: &[u8]) -> Result<Self, AudioLinkError> {
        let count = u32_items(payload.len(), 1, NACK_MAX_ITEMS)?;
        let mut items = [0u32; NACK_MAX_ITEMS];
        for (index, slot) in items.iter_mut().take(count).enumerate() {
            *slot = u32::from_le_bytes(fixed_bytes::<4>(
                payload,
                index * 4,
                "NACK payload must be 4..=64 B",
            )?);
        }
        Ok(Self { items, len: count })
    }

    /// 待重传的 seq 列表。
    pub fn seqs(&self) -> &[u32] {
        self.items.get(..self.len).unwrap_or(&[])
    }

    /// 载荷长度（字节）。
    pub const fn payload_len(&self) -> usize {
        self.len * 4
    }

    /// 编码进缓冲，返回写入长度。
    pub fn encode_into(&self, out: &mut [u8]) -> Result<usize, AudioLinkError> {
        let total = self.payload_len();
        let dst = out
            .get_mut(..total)
            .ok_or(AudioLinkError::bad_request(ERR_OUT_OF_SPACE))?;
        for (index, value) in self.seqs().iter().enumerate() {
            put(dst, index * 4, &value.to_le_bytes(), ERR_OUT_OF_SPACE)?;
        }
        Ok(total)
    }

    /// 编码为载荷 `Vec`。
    pub fn encode_to_vec(&self) -> Result<Vec<u8>, AudioLinkError> {
        let mut payload = vec![0u8; self.payload_len()];
        self.encode_into(&mut payload)?;
        Ok(payload)
    }

    /// 组装完整数据报。
    pub fn to_datagram_bytes(&self) -> Result<Vec<u8>, AudioLinkError> {
        let payload = self.encode_to_vec()?;
        auxiliary_datagram_bytes(Ptype::Nack, &payload)
    }

    /// 从整包解析（ptype 必须为 `NACK`）。
    pub fn from_datagram(datagram: &AudioDatagram<'_>) -> Result<Self, AudioLinkError> {
        Self::decode(auxiliary_payload(datagram, Ptype::Nack)?)
    }
}
