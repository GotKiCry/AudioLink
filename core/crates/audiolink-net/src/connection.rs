//! 一条 QUIC 连接上的音频数据报与可靠控制流（`docs/11-m1-contract.md` §3）。
//!
//! 规格依据：`docs/03-protocol.md` §2（传输映射）、§3（数据报 ≤ 1200 B，**实测修正**见下）、§4（控制帧）。

use std::net::SocketAddr;

use audiolink_types::{DATAGRAM_HEADER_LEN, DATAGRAM_MAX_LEN, NodeId};
use bytes::Bytes;
use rustls::pki_types::CertificateDer;
use sha2::{Digest, Sha256};

use crate::control::ControlChannel;
use crate::error::NetError;

/// AudioLink 的一对节点之间只有一条 QUIC 连接（控制流 #0 + 音频数据报共用）。
#[derive(Debug, Clone)]
pub struct Connection {
    /// quinn 的连接句柄（`Clone` 只是引用计数，可自由克隆给多个任务）。
    inner: quinn::Connection,
}

impl Connection {
    /// crate 内构造（端点 accept / connect 的产物）。
    pub(crate) fn new(inner: quinn::Connection) -> Self {
        Self { inner }
    }

    /// 对端 UDP 地址。
    pub fn remote_addr(&self) -> SocketAddr {
        self.inner.remote_address()
    }

    /// 对端证书指纹 = `SHA-256(对端证书 DER)` → [`NodeId`]。**信任判定的唯一依据**（§2）。
    pub fn peer_id(&self) -> Result<NodeId, NetError> {
        Ok(node_id_of(&self.peer_cert_der()?))
    }

    /// 对端 TLS 证书链的**叶子证书** DER。
    ///
    /// 为什么指纹不够：§5 的 `AUTH_RESPONSE` 验签需要**对端公钥**，而公钥只存在于证书里，
    /// `SHA-256` 摘要还原不回去。engine 拿这份 DER 调
    /// `audiolink-identity` 的 `verify_challenge`。
    ///
    /// 取不到时返回 `Err` 而**不是**空 `Vec`：空证书会让验签静默失败，
    /// 把「没有材料」伪装成「认证不通过」，排障时会指向完全错误的方向。
    ///
    /// 实现上走 quinn 的 `peer_identity()` 而不是在自定义校验器里缓存证书：
    /// [`peer_id`](Self::peer_id) 与本法必须永远看到**同一份字节**（否则两个方法会悄悄漂移），
    /// 共用同一个来源是唯一能保证这点的做法；校验器只保留「接受 / 拒绝」的语义，不额外持状态。
    pub fn peer_cert_der(&self) -> Result<Vec<u8>, NetError> {
        let identity = self.inner.peer_identity().ok_or_else(|| {
            NetError::peer_identity("对端未提供证书：握手没有协商客户端证书（§2 要求双向认证）")
        })?;
        let chain = identity
            .downcast::<Vec<CertificateDer<'static>>>()
            .map_err(|_| {
                NetError::peer_identity("对端身份不是 rustls 证书链（加密后端被换过？）")
            })?;
        let leaf = chain
            .first()
            .ok_or_else(|| NetError::peer_identity("对端证书链为空"))?;
        let leaf: &[u8] = leaf.as_ref();
        Ok(leaf.to_vec())
    }

    /// QUIC 给出的数据报上限（本机实测 1162 B，§3「实测修正」）。
    ///
    /// 这是 QUIC 的**原始**值，**没有**与 §3 的 1200 B 协议预算取 min —— 那个 min
    /// 由 [`max_audio_payload`](Self::max_audio_payload) 与 `send_datagram` 的长度检查承担。
    pub fn max_datagram_size(&self) -> Option<usize> {
        self.inner.max_datagram_size()
    }

    /// 音频数据报的**可用载荷上限** = `min(DATAGRAM_MAX_LEN, max_datagram_size()) - DATAGRAM_HEADER_LEN`。
    ///
    /// §3 的实测修正：1200 B 是**协议预算**，不是实际可用值 —— QUIC 的短头 + AEAD 开销会吃掉约 38 B
    /// （默认初始 MTU 下实测 1162 B），且 DPLPMTUD 探测到更大 MTU 后该值还会上升。所以必须逐连接读。
    ///
    /// 数据报不可用（对端未协商 RFC 9221）时返回 0：调用方据此**不发**音频，
    /// 而不是按 1200 硬发一堆注定失败的包。
    pub fn max_audio_payload(&self) -> usize {
        self.datagram_budget()
            .map_or(0, |budget| budget.saturating_sub(DATAGRAM_HEADER_LEN))
    }

    /// 单条数据报的**总长**上限（= `min(DATAGRAM_MAX_LEN, max_datagram_size())`）；不可用 → `None`。
    fn datagram_budget(&self) -> Option<usize> {
        self.inner
            .max_datagram_size()
            .map(|limit| limit.min(DATAGRAM_MAX_LEN))
    }

    /// 发一条数据报（不可靠、可重排、可丢）。
    ///
    /// 长度检查在**调用 quinn 之前**做：quinn 只会给一个 `TooLarge`，拿不到
    /// 「上限多少 / 实长多少」这两个分片必需的数（§3 要求发送侧按 `min(1200, max)` 分片）。
    pub async fn send_datagram(&self, bytes: &[u8]) -> Result<(), NetError> {
        match self.datagram_budget() {
            None => {
                return Err(NetError::datagram_unsupported(
                    "对端未协商 QUIC 数据报（RFC 9221），或本端未启用数据报缓冲",
                ));
            }
            Some(budget) if bytes.len() > budget => {
                return Err(NetError::datagram_too_large_owned(format!(
                    "数据报 {} B 超过上限 {} B（= min(DATAGRAM_MAX_LEN={DATAGRAM_MAX_LEN}, \
                     max_datagram_size={:?})）",
                    bytes.len(),
                    budget,
                    self.inner.max_datagram_size()
                )));
            }
            Some(_) => {}
        }

        self.inner
            .send_datagram(Bytes::copy_from_slice(bytes))
            .map_err(|error| match error {
                quinn::SendDatagramError::TooLarge => NetError::datagram_too_large_owned(format!(
                    "quinn 拒绝超长数据报（{} B；上限 {:?}）",
                    bytes.len(),
                    self.inner.max_datagram_size()
                )),
                quinn::SendDatagramError::UnsupportedByPeer => {
                    NetError::datagram_unsupported("对端未协商 QUIC 数据报（RFC 9221）")
                }
                quinn::SendDatagramError::Disabled => {
                    NetError::datagram_unsupported("本端数据报接收缓冲未启用")
                }
                quinn::SendDatagramError::ConnectionLost(reason) => {
                    NetError::transport_owned(format!("连接已断开，数据报未发出：{reason}"))
                }
            })
    }

    /// 读一个数据报**到调用方缓冲**（实时路径零分配），返回写入长度。
    ///
    /// 缓冲不足时返回 `Err` 而**不是**静默截断：§3 的载荷是「不透明定长 / 变长字节串」，
    /// 长度本身就是语义的一部分，半个 Opus 帧被当成完整帧解码只会产出噪声，
    /// 比明确丢一个包糟糕得多。该数据报被丢弃，调用方计数后继续。
    ///
    /// 关于「零分配」的准确边界：调用方缓冲全程复用，本函数不分配；
    /// 但 quinn 0.11 的读接口只有 `read_datagram() -> Bytes`（没有 `_into` 变体），
    /// 所以每个数据报在 quinn 内部仍会产生一个 `Bytes` 句柄（底层缓冲由其池化复用）。
    /// 这是 quinn 的接口能力上限，不是本实现的选择。
    pub async fn read_datagram_into(&self, buf: &mut [u8]) -> Result<usize, NetError> {
        let datagram: Bytes = self
            .inner
            .read_datagram()
            .await
            .map_err(|error| NetError::transport_owned(format!("读数据报失败：{error}")))?;

        let length = datagram.len();
        // 先把缓冲长度取出来：`get_mut` 的可变借用活到 `ok_or_else` 的闭包调用期间，
        // 闭包里再读 `buf.len()` 会被判成「同时可变 + 不可变借用」。
        let capacity = buf.len();
        let target = buf.get_mut(..length).ok_or_else(|| {
            NetError::datagram_buffer_too_small_owned(format!(
                "数据报 {length} B 装不进 {capacity} B 的调用方缓冲；该数据报已丢弃（§3：不得静默截断）"
            ))
        })?;
        target.copy_from_slice(&datagram);
        Ok(length)
    }

    /// 打开 / 接管控制流 #0（§2：双向可靠流 #0）。
    ///
    /// 「谁 open、谁 accept」由**连接角色**决定，不是由调用顺序决定：
    /// - 客户端：`open_bi()`，QUIC 里 0 号双向流只能由客户端发起，拿到的必然是 #0；
    /// - 服务端：`accept_bi()`，等对端把 #0 送上来。
    ///
    /// ⚠️ QUIC 的 `open_bi()` 是**惰性**的：本端 open 不会产生任何报文，对端要等
    /// **第一个字节**到达才会从 `accept_bi()` 返回。所以双方同时调用本方法时，
    /// 「先写」的那一端不会阻塞，另一端会挂到第一次写入为止 —— 这是协议行为。
    pub async fn open_control(&self) -> Result<ControlChannel, NetError> {
        let (send, recv) = match self.inner.side() {
            quinn::Side::Client => self.inner.open_bi().await.map_err(|error| {
                NetError::transport_owned(format!("打开控制流 #0 失败：{error}"))
            })?,
            quinn::Side::Server => {
                let (send, recv) = self.inner.accept_bi().await.map_err(|error| {
                    NetError::transport_owned(format!("接受控制流 #0 失败：{error}"))
                })?;
                let index = send.id().index();
                if index != 0 {
                    // §2：v1 只有 #0 是双向流（#2+ 是单向流）。出现别的双向流说明对端跑的不是本协议；
                    // 与其把它当控制流读出一堆垃圾，不如明确报错。
                    return Err(NetError::control_malformed_owned(format!(
                        "对端打开了双向流 #{index}，但 v1 只把 #0 定义为控制流（§2）"
                    )));
                }
                (send, recv)
            }
        };
        Ok(ControlChannel::new(send, recv))
    }

    /// QUIC 平滑 RTT（µs）。
    pub fn rtt_us(&self) -> u64 {
        u64::try_from(self.inner.rtt().as_micros()).unwrap_or(u64::MAX)
    }

    /// 关闭连接（`code` 为应用错误码，原样送达对端）。
    pub fn close(&self, code: u32, reason: &str) {
        // quinn 0.11 的 `VarInt::from_u32` 是不可失败的（u32 恒 < 2^62）
        self.inner
            .close(quinn::VarInt::from_u32(code), reason.as_bytes());
    }

    /// 测试专用：直接开一条双向流并交出**原始**读写半边。
    ///
    /// 为什么需要：`ControlChannel::send` 只接受已知 `OpCode`，而「未知命令码必须以
    /// `op == None` 正常送达 L2」是 §1.1 的行为契约 —— 要覆盖它，测试必须能往流里写一个
    /// `type = 0x7F` 的**合法帧**。仅测试期可见，不进公开面。
    #[cfg(test)]
    pub(crate) async fn open_raw_bi(
        &self,
    ) -> Result<(quinn::SendStream, quinn::RecvStream), NetError> {
        self.inner
            .open_bi()
            .await
            .map_err(|error| NetError::transport_owned(format!("打开原始双向流失败：{error}")))
    }
}

/// `NodeId` = `SHA-256(证书 DER)`（§2 的节点身份定义）。
fn node_id_of(cert_der: &[u8]) -> NodeId {
    let mut hasher = Sha256::new();
    hasher.update(cert_der);
    NodeId::from_bytes(hasher.finalize().into())
}
