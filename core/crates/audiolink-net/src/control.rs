//! 控制流 #0（双向可靠流，`docs/03-protocol.md` §2 / §4）。
//!
//! # 本模块的职责边界
//!
//! net **只做定界与搬运**：把字节流切成帧、把帧写成字节流。载荷是**不透明 postcard 字节**，
//! net 不解释、不校验其 schema（那是 engine 的事，契约 §2 第 2 条）。
//!
//! # 校验分工（决定 `recv()` 何时返回 `Err`，何时返回 `Ok(op = None)`）
//!
//! | 情况 | 结果 | 理由 |
//! |---|---|---|
//! | 帧头截断 / `payload_len` 与剩余字节不符 / `payload_len > 64 KiB` | `Err` | 帧边界不可信，**无法重新同步流边界**，只能报错 |
//! | 帧头 `ver != 0x02` | `Err` | 帧头布局由主版本定义；认不出主版本同样无法安全定界 |
//! | 命令码不在本版本内（`type` 未知） | `Ok` 且 `op == None` | §1.1：未知类型「忽略并计数」是 **L2（engine）** 的职责，net 不得把「不认识」升级成断连 |
//! | `flags` 非 0 | `Ok`，原样透出 | 同上：`flags` 是保留位，语义处置属 L2；帧长完好所以可以安全跳过 |
//!
//! 握手期的版本协商不依赖这里放宽 `ver`：它走 `HELLO` / `HELLO_ACK` 载荷里的 `proto_version`
//! 字段（§5、§13），帧头 `ver` 只用于定界。

use std::fmt;

use audiolink_proto::ControlFrameHeader;
use audiolink_proto::control::MIN_FRAME_LEN;
use audiolink_types::{CONTROL_HEADER_LEN, CONTROL_MAX_PAYLOAD, OpCode, PROTO_MAJOR};

use crate::error::NetError;

/// 收到的控制帧（契约修订 #1 的形状：`op` 是 `Option`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlMessage {
    /// 已识别的命令码；`None` = 未知类型（§1.1：交给 L2 忽略并计数，**不得断流**）。
    pub op: Option<OpCode>,
    /// 线上原始 `type` 字节（诊断 / 计数用；`op` 为 `Some(v)` 时 `v.as_u8() == raw_type`）。
    pub raw_type: u8,
    /// 原始 `flags`（v1 要求为 0；非 0 由 L2 计数处置）。
    pub flags: u16,
    /// 请求 ID（0 表示单向通知，无需响应）。
    pub request_id: u32,
    /// postcard 原始字节，net 不解释。
    pub payload: Vec<u8>,
}

/// 控制流 #0 的收发句柄。
pub struct ControlChannel {
    /// 本端写方向（流 #0 的发送半边）。
    send: quinn::SendStream,
    /// 本端读方向（流 #0 的接收半边）。
    recv: quinn::RecvStream,
    /// 帧缓冲：每帧 resize 到**恰好**该帧总长（缩容保留 capacity → 稳态不再分配）。
    buf: Vec<u8>,
    /// `buf` 中已从流读入的字节数。
    filled: usize,
}

impl fmt::Debug for ControlChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlChannel")
            .field("stream_id", &self.send.id())
            .field("buffered", &self.filled)
            .field("buffer_capacity", &self.buf.capacity())
            .finish()
    }
}

impl ControlChannel {
    /// crate 内构造（流由 [`crate::Connection::open_control`] 按连接角色取得）。
    pub(crate) fn new(send: quinn::SendStream, recv: quinn::RecvStream) -> Self {
        Self {
            send,
            recv,
            buf: Vec::new(),
            filled: 0,
        }
    }

    /// 发送一帧（§4 布局：8 B 帧头 + `request_id` + 载荷）。
    ///
    /// 载荷超过 `CONTROL_MAX_PAYLOAD`（64 KiB）时**发送侧**返回 `Err` —— 不让超长帧进到线路上，
    /// 由对端去判非法（那样对端的 L1 会拒绝整帧，问题反而更难定位）。
    pub async fn send(
        &mut self,
        op: OpCode,
        request_id: u32,
        payload: &[u8],
    ) -> Result<(), NetError> {
        if payload.len() > CONTROL_MAX_PAYLOAD {
            return Err(NetError::control_payload_too_large_owned(format!(
                "控制帧载荷 {} B 超过 CONTROL_MAX_PAYLOAD={} B（§4：单帧上限 64 KiB）",
                payload.len(),
                CONTROL_MAX_PAYLOAD
            )));
        }
        let payload_len = u32::try_from(payload.len())
            .map_err(|_| NetError::control_payload_too_large("控制帧载荷长度无法用 u32 表示"))?;

        // 手工组帧，不调 proto 的 `encode_to_vec`：控制面虽非热路径，但两段 write 就够，
        // 没必要为此多分配一个「帧头 + 载荷」的整帧缓冲。
        let mut header = [0u8; MIN_FRAME_LEN];
        header[0] = PROTO_MAJOR;
        header[1] = op.as_u8();
        // header[2..4] 保持 0：v1 的 flags 全部保留，发送端必须置 0（§4）
        header[4..8].copy_from_slice(&payload_len.to_le_bytes());
        header[8..MIN_FRAME_LEN].copy_from_slice(&request_id.to_le_bytes());

        self.send
            .write_all(&header)
            .await
            .map_err(|error| NetError::transport_owned(format!("控制帧头写入失败：{error}")))?;
        if !payload.is_empty() {
            self.send.write_all(payload).await.map_err(|error| {
                NetError::transport_owned(format!("控制帧载荷写入失败：{error}"))
            })?;
        }
        Ok(())
    }

    /// 读一帧。
    ///
    /// 内部缓冲自动增长到当前帧长度；单帧上限 `CONTROL_MAX_PAYLOAD`（超过即 `Err`，
    /// 先于分配检查，避免对端用一个巨大的 `payload_len` 让我们 OOM）。
    pub async fn recv(&mut self) -> Result<ControlMessage, NetError> {
        // 1) 先凑够 8 B 帧头 —— 控制帧没有别的定界手段，`payload_len` 是唯一线索。
        self.fill_at_least(CONTROL_HEADER_LEN).await?;
        let payload_len = self.declared_payload_len()?;
        if payload_len > CONTROL_MAX_PAYLOAD {
            return Err(NetError::control_malformed_owned(format!(
                "payload_len={payload_len} 超过 CONTROL_MAX_PAYLOAD={CONTROL_MAX_PAYLOAD}（§4）"
            )));
        }
        let total = MIN_FRAME_LEN
            .checked_add(payload_len)
            .ok_or_else(|| NetError::control_malformed("控制帧总长溢出 usize"))?;

        // 2) 读满整帧再交给信封解析：`ControlFrameHeader::decode` 要求
        //    「payload_len **恰好**等于剩余字节数」，拿 8 B 前缀去调它必然被判截断。
        self.fill_at_least(total).await?;
        let frame = self
            .buf
            .get(..total)
            .ok_or_else(|| NetError::control_malformed("控制帧缓冲短于声明的帧长"))?;
        let message = decode_frame(frame);

        // 3) 无论成功与否都把游标归零：本帧的字节已被消费（失败时流边界已不可信，调用方会丢弃本通道）。
        self.filled = 0;
        message
    }

    /// 从已读入的帧头里取 `payload_len`（此时 `buf.len() >= CONTROL_HEADER_LEN`）。
    fn declared_payload_len(&self) -> Result<usize, NetError> {
        let raw = self
            .buf
            .get(4..CONTROL_HEADER_LEN)
            .ok_or_else(|| NetError::control_malformed("控制帧头不足 8 B"))?;
        let width: [u8; 4] = raw
            .try_into()
            .map_err(|_| NetError::control_malformed("控制帧头不足 8 B"))?;
        // u32 → usize：32 位平台上 usize == u32，取不到只可能是更窄的平台，那时按 usize::MAX 报上限错。
        Ok(usize::try_from(u32::from_le_bytes(width)).unwrap_or(usize::MAX))
    }

    /// 把缓冲填充到至少 `need` 字节。
    ///
    /// # 不变量
    ///
    /// 调用点上恒有 `filled <= CONTROL_HEADER_LEN <= MIN_FRAME_LEN <= need`，因此
    /// `buf.resize(need, _)` 即使**缩容**也不会丢掉尚未解析的字节（缩容保留 capacity，稳态零分配）。
    async fn fill_at_least(&mut self, need: usize) -> Result<(), NetError> {
        if self.buf.len() != need {
            self.buf.resize(need, 0);
        }

        while self.filled < need {
            // 先算完这一轮读入，再改 `filled` —— 让 `buffer`/`recv` 的借用在本块结束时终止。
            let outcome = {
                let start = self.filled;
                let buffer = self
                    .buf
                    .get_mut(start..)
                    .ok_or_else(|| NetError::control_malformed("控制帧读游标越出缓冲范围"))?;
                self.recv.read(buffer).await
            };

            match outcome {
                // 空 STREAM 帧（保持连接的零长写）：不构成进度，继续读。
                // 唯一的不收敛路径是对端持续灌空帧，由连接带宽与空闲超时兜底。
                Ok(Some(0)) => {}
                Ok(Some(read)) => self.filled += read,
                Ok(None) => {
                    return Err(NetError::transport(
                        "控制流在对端尚未发满一帧时被关闭（FIN）",
                    ));
                }
                Err(error) => {
                    return Err(NetError::transport_owned(format!(
                        "控制流读取失败：{error}"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// 把一帧字节解成 [`ControlMessage`]（**纯函数**，可对着手写字节直接单测）。
///
/// 校验分工见模块文档：只有「帧边界不可信」（截断 / 长度不自洽 / 主版本不认识）才返回 `Err`；
/// 未知命令码与保留位非 0 一律 `Ok` 透出，由 L2 忽略并计数（§1.1）。
fn decode_frame(frame: &[u8]) -> Result<ControlMessage, NetError> {
    // 信封解析（proto 的 L1 helper）：校验 `payload_len <= 64 KiB` 且**恰好**等于剩余字节数。
    // 它刻意不校验 op / ver / flags —— 那正是这里需要的分工。
    let header = ControlFrameHeader::decode(frame)
        .map_err(|error| NetError::control_malformed_owned(format!("控制帧信封非法：{error}")))?;

    if header.ver != PROTO_MAJOR {
        return Err(NetError::control_malformed_owned(format!(
            "控制帧 ver={:#04x} 不是 {:#04x}：帧头布局由主版本定义，无法安全定界",
            header.ver, PROTO_MAJOR
        )));
    }

    let op = OpCode::from_u8(header.raw_op);
    if op.is_none() {
        tracing::debug!(
            raw_type = header.raw_op,
            request_id = header.request_id,
            "收到未知控制命令码：正常返回，由 L2 忽略并计数（docs/03-protocol.md §1.1）"
        );
    }

    let payload = frame
        .get(MIN_FRAME_LEN..)
        .ok_or_else(|| NetError::control_malformed("控制帧短于最小帧长 12 B"))?
        .to_vec();

    Ok(ControlMessage {
        op,
        raw_type: header.raw_op,
        flags: header.flags,
        request_id: header.request_id,
        payload,
    })
}

/// 组装一帧（仅测试用：`ControlChannel::send` 只接受**已知** `OpCode`，
/// 而要覆盖「未知命令码必须送达 L2」就得手写 `type`）。
#[cfg(test)]
fn build_frame(op: u8, flags: u16, request_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(MIN_FRAME_LEN + payload.len());
    out.push(PROTO_MAJOR);
    out.push(op);
    out.extend_from_slice(&flags.to_le_bytes());
    // 测试载荷都是几十字节的字面量，装得进 u32；取不到只可能是构造错了用例，按上限落个明显值
    let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&request_id.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

#[cfg(test)]
mod tests {
    // 测试代码不受实时路径的 unwrap / expect / panic 禁令约束（那三条针对运行时音频路径）
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 未知命令码以op_none正常返回() {
        let bytes = build_frame(0x7F, 0, 9, &[0xDE, 0xAD, 0xBE]);
        let message = decode_frame(&bytes).expect("信封完好的未知命令码必须返回 Ok，而不是 Err");
        assert_eq!(message.op, None);
        assert_eq!(message.raw_type, 0x7F);
        assert_eq!(message.flags, 0);
        assert_eq!(message.request_id, 9);
        assert_eq!(message.payload, vec![0xDE, 0xAD, 0xBE]);
    }

    #[test]
    fn 非零flags原样透出交给l2() {
        // v1 的 flags 全保留（必须为 0），但「非 0 怎么办」是 L2 的计数策略 —— net 不吞掉这个事实
        let bytes = build_frame(OpCode::Ping.as_u8(), 0x0001, 1, &[1, 2, 3]);
        let message = decode_frame(&bytes).expect("flags 非 0 不是帧边界问题");
        assert_eq!(message.flags, 0x0001);
        assert_eq!(message.op, Some(OpCode::Ping));
    }

    #[test]
    fn 帧边界不可信时返回err() {
        // 主版本不是 0x02：帧头布局不认识 → 无法安全定界
        let mut wrong_ver = build_frame(OpCode::Ping.as_u8(), 0, 1, &[0; 4]);
        wrong_ver[0] = 0x03;
        assert!(matches!(
            decode_frame(&wrong_ver),
            Err(NetError::ControlMalformed { .. })
        ));

        // 截断：少一个字节（payload_len 与剩余字节数不符）
        let full = build_frame(OpCode::Ping.as_u8(), 0, 1, &[0; 4]);
        assert!(matches!(
            decode_frame(&full[..full.len() - 1]),
            Err(NetError::ControlMalformed { .. })
        ));

        // payload_len 声明 65537 > 64 KiB（§4 上限）
        let mut huge = build_frame(OpCode::Ping.as_u8(), 0, 1, &[]);
        huge[4..8].copy_from_slice(&65_537_u32.to_le_bytes());
        assert!(matches!(
            decode_frame(&huge),
            Err(NetError::ControlMalformed { .. })
        ));

        // 空缓冲（连 12 B 都不到）
        assert!(decode_frame(&[]).is_err());
    }

    #[test]
    fn 空载荷与request_id零是合法帧() {
        // §4：`request_id = 0` 表示单向通知；`payload_len = 0` 完全合法（载荷 schema 由命令决定）
        let bytes = build_frame(OpCode::Hello.as_u8(), 0, 0, &[]);
        let message = decode_frame(&bytes).expect("空载荷是合法帧");
        assert_eq!(message.request_id, 0);
        assert!(message.payload.is_empty());
        assert_eq!(message.op, Some(OpCode::Hello));
        assert_eq!(message.raw_type, OpCode::Hello.as_u8());
    }

    /// 契约修订 #1 的行为契约，走**真实 QUIC 连接**的 `recv()`：
    /// 手工往流 #0 写一个 `type = 0x7F` 的合法帧，`recv()` 必须 `Ok` 且 `op == None`。
    #[tokio::test]
    async fn recv对未知命令码返回ok而非err() {
        let pair = crate::testing::pair().await;

        // 客户端直接开一条双向流并手写帧 —— 绕开 `send()`（它只收已知 OpCode）
        let mut raw = pair.client.open_raw_bi().await.expect("打开原始双向流");
        let payload = [0x11_u8, 0x22, 0x33, 0x44];
        let frame = build_frame(0x7F, 0, 0xABCD, &payload);
        raw.0.write_all(&frame).await.expect("写入原始帧");

        let mut control = pair.server.open_control().await.expect("接管控制流 #0");
        let message = control.recv().await.expect("未知命令码不得变成 Err");

        assert_eq!(message.op, None, "§1.1：未知类型交给 L2 忽略并计数");
        assert_eq!(message.raw_type, 0x7F);
        assert_eq!(message.request_id, 0xABCD);
        assert_eq!(message.payload, payload, "帧长完好就必须完整取回载荷");
    }

    /// 载荷超上限：**发送侧**返回 `Err`（契约 §3 验收 5），且不发到线路上。
    #[tokio::test]
    async fn 超长控制帧载荷发送侧返回err() {
        let pair = crate::testing::pair().await;
        let mut control = pair.client.open_control().await.expect("打开控制流 #0");

        let oversized = vec![0u8; CONTROL_MAX_PAYLOAD + 1];
        let error = control
            .send(OpCode::Hello, 1, &oversized)
            .await
            .expect_err("超过 CONTROL_MAX_PAYLOAD 必须拒绝");
        assert!(matches!(error, NetError::ControlPayloadTooLarge { .. }));
        assert_eq!(error.code(), audiolink_types::ErrorCode::BadRequest);
        assert!(!error.is_statistical(), "控制面错误一律非统计类");

        // 被拒绝的帧**不得**被写到线路上：连接仍然可用，下一帧正常发出去
        // （§4 的限制是「> 65536」，恰好等于上限必须放行 —— 由集成测试的 64 KiB 用例覆盖）
        control
            .send(OpCode::Ping, 7, b"still alive")
            .await
            .expect("被拒绝的帧不该污染流");
    }

    /// 大帧跨多次 `read` 的定界正确性：一帧 200 KiB 不行（超上限），
    /// 但 40 KiB 的一帧必须能被 `recv()` 分片读入并完整还原。
    #[tokio::test]
    async fn 大帧跨多次读入仍能正确定界() {
        let pair = crate::testing::pair().await;
        let mut client = pair.client.open_control().await.expect("打开控制流 #0");

        let big = vec![0x5A_u8; 40 * 1024];
        let server = tokio::spawn(async move {
            let mut control = pair.server.open_control().await.expect("接管控制流 #0");
            control.recv().await
        });
        client
            .send(OpCode::StreamStats, 42, &big)
            .await
            .expect("40 KiB 载荷在上限内");
        let message = server.await.expect("服务端任务").expect("读回一帧");

        assert_eq!(message.op, Some(OpCode::StreamStats));
        assert_eq!(message.request_id, 42);
        assert_eq!(message.payload.len(), big.len());
        assert_eq!(message.payload, big);
    }
}
