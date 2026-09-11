//! 控制帧编解码（`docs/03-protocol.md` §4）。
//!
//! 帧布局：8 B 帧头（`ver` + `type` + `flags` + `payload_len`）+ `request_id`(4) + postcard 载荷。
//! 「宽容信封」仅用于握手阶段读取对端版本（§1.1），数据面一律用严格解码。

use audiolink_types::{
    AudioLinkError, CONTROL_HEADER_LEN, CONTROL_MAX_PAYLOAD, OpCode, PROTO_MAJOR,
};

use crate::{ERR_OUT_OF_SPACE, fixed_bytes, put};

/// 控制帧最小长度：8 B 帧头 + 4 B `request_id`（§4）。
pub const MIN_FRAME_LEN: usize = CONTROL_HEADER_LEN + 4;

/// 控制帧**宽容信封**：只做结构校验，**不校验** `ver` / `type` / `flags`。
///
/// 用途：握手阶段先读对端 `ver`，据此决定是否回 `1001 VERSION_MISMATCH`（§1.1）。
/// 数据面必须使用严格解码 [`ControlFrame::decode`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlFrameHeader {
    /// 对端声明的版本字节（**未校验**）。
    pub ver: u8,
    /// 原始命令码（**未校验**，可能是本版本未知的 `type`）。
    pub raw_op: u8,
    /// 原始 `flags`（**未校验**；v1 要求为 0）。
    pub flags: u16,
    /// 载荷长度（已校验：等于剩余字节数且 ≤ 64 KiB）。
    pub payload_len: u32,
    /// 请求 ID（0 表示单向通知）。
    pub request_id: u32,
}

impl ControlFrameHeader {
    /// 帧头长度（8 B）。
    pub const LEN: usize = CONTROL_HEADER_LEN;

    /// 解析信封；结构非法（截断 / `payload_len` 越界或与剩余字节不符）→ `BadRequest`。
    pub fn decode(buf: &[u8]) -> Result<Self, AudioLinkError> {
        const TRUNCATED: &str = "control frame truncated: need at least 12 B";
        let [ver] = fixed_bytes::<1>(buf, 0, TRUNCATED)?;
        let [raw_op] = fixed_bytes::<1>(buf, 1, TRUNCATED)?;
        let flags = u16::from_le_bytes(fixed_bytes::<2>(buf, 2, TRUNCATED)?);
        let payload_len = u32::from_le_bytes(fixed_bytes::<4>(buf, 4, TRUNCATED)?);
        let request_id = u32::from_le_bytes(fixed_bytes::<4>(buf, 8, TRUNCATED)?);

        let declared = usize::try_from(payload_len)
            .map_err(|_| AudioLinkError::bad_request("control payload_len overflow"))?;
        if declared > CONTROL_MAX_PAYLOAD {
            return Err(AudioLinkError::bad_request("control payload_len > 64 KiB"));
        }
        let expected_total = MIN_FRAME_LEN
            .checked_add(declared)
            .ok_or(AudioLinkError::bad_request("control frame length overflow"))?;
        if buf.len() != expected_total {
            return Err(AudioLinkError::bad_request(
                "control payload_len must equal remaining bytes",
            ));
        }
        Ok(Self {
            ver,
            raw_op,
            flags,
            payload_len,
            request_id,
        })
    }
}

/// 控制帧（严格解码，§4）：`ver == 0x02`、`type` 已知、`flags == 0`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlFrame<'a> {
    /// 命令码（§4.1）。
    pub op: OpCode,
    /// 请求 ID（0 表示单向通知，无需响应）。
    pub request_id: u32,
    /// postcard 载荷（借用输入缓冲；schema 由 M1+ 逐命令冻结）。
    pub payload: &'a [u8],
}

impl<'a> ControlFrame<'a> {
    /// 解析并严格校验（§1.1 的 L1 规则）。
    pub fn decode(buf: &'a [u8]) -> Result<Self, AudioLinkError> {
        let header = ControlFrameHeader::decode(buf)?;
        if header.ver != PROTO_MAJOR {
            return Err(AudioLinkError::bad_request(
                "control frame ver != 0x02 (use ControlFrameHeader for handshake)",
            ));
        }
        let op = OpCode::from_u8(header.raw_op)
            .ok_or(AudioLinkError::bad_request("unknown control op"))?;
        if header.flags != 0 {
            return Err(AudioLinkError::bad_request(
                "control frame flags must be 0 in v1",
            ));
        }
        let payload = buf
            .get(MIN_FRAME_LEN..)
            .ok_or(AudioLinkError::bad_request("control frame truncated"))?;
        Ok(Self {
            op,
            request_id: header.request_id,
            payload,
        })
    }

    /// 总长度（含帧头与 `request_id`）。
    pub fn total_len(&self) -> Result<usize, AudioLinkError> {
        frame_len(self.payload.len())
    }

    /// 编码进调用方缓冲，返回写入字节数（`flags` 在 v1 恒为 0）。
    pub fn encode_into(&self, out: &mut [u8]) -> Result<usize, AudioLinkError> {
        encode_parts(self.op, self.request_id, self.payload, out)
    }

    /// 编码为 `Vec`（控制面非热路径，允许分配）。
    pub fn encode_to_vec(&self) -> Result<Vec<u8>, AudioLinkError> {
        let total = self.total_len()?;
        let mut out = vec![0u8; total];
        self.encode_into(&mut out)?;
        Ok(out)
    }

    /// 便捷构造：把命令体按 postcard 序列化后组帧（§4.1 的典型用法）。
    pub fn encode_with_payload<T: serde::Serialize>(
        op: OpCode,
        request_id: u32,
        value: &T,
    ) -> Result<Vec<u8>, AudioLinkError> {
        let payload = payload_encode(value)?;
        let mut out = vec![0u8; frame_len(payload.len())?];
        encode_parts(op, request_id, &payload, &mut out)?;
        Ok(out)
    }
}

/// 组帧长度：8 B 帧头 + 4 B `request_id` + 载荷（§4）。
fn frame_len(payload_len: usize) -> Result<usize, AudioLinkError> {
    if payload_len > CONTROL_MAX_PAYLOAD {
        return Err(AudioLinkError::bad_request("control payload > 64 KiB"));
    }
    MIN_FRAME_LEN
        .checked_add(payload_len)
        .ok_or(AudioLinkError::bad_request("control frame length overflow"))
}

/// 按 §4 布局写出控制帧（`ver` = `0x02`、`flags` = 0）。
fn encode_parts(
    op: OpCode,
    request_id: u32,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, AudioLinkError> {
    let total = frame_len(payload.len())?;
    let payload_len = u32::try_from(payload.len())
        .map_err(|_| AudioLinkError::bad_request("control payload > 64 KiB"))?;
    let dst = out
        .get_mut(..total)
        .ok_or(AudioLinkError::bad_request(ERR_OUT_OF_SPACE))?;
    put(dst, 0, &[PROTO_MAJOR], ERR_OUT_OF_SPACE)?;
    put(dst, 1, &[op.as_u8()], ERR_OUT_OF_SPACE)?;
    put(dst, 2, &0u16.to_le_bytes(), ERR_OUT_OF_SPACE)?;
    put(dst, 4, &payload_len.to_le_bytes(), ERR_OUT_OF_SPACE)?;
    put(dst, 8, &request_id.to_le_bytes(), ERR_OUT_OF_SPACE)?;
    put(dst, MIN_FRAME_LEN, payload, ERR_OUT_OF_SPACE)?;
    Ok(total)
}

/// postcard 编码命令体载荷（§4）。
pub fn payload_encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, AudioLinkError> {
    postcard::to_allocvec(value).map_err(|_| AudioLinkError::bad_request("postcard encode failed"))
}

/// postcard 解码命令体载荷：必须**恰好**消费全部字节，尾随数据视为非法（§4）。
pub fn payload_decode<'a, T: serde::Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, AudioLinkError> {
    let (value, rest) = postcard::take_from_bytes::<T>(bytes)
        .map_err(|_| AudioLinkError::bad_request("postcard decode failed"))?;
    if !rest.is_empty() {
        return Err(AudioLinkError::bad_request(
            "postcard payload has trailing bytes",
        ));
    }
    Ok(value)
}
