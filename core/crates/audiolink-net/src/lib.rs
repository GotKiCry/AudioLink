//! audiolink-net —— AudioLink 的 QUIC 传输层（quinn）：端点、音频数据报、控制流 #0、时钟估计
//!
//! 规格：`docs/02-architecture.md` §5（传输层设计）、`docs/03-protocol.md` §2 / §3 / §4 / §6。
//! 冻结接口：`docs/11-m1-contract.md` §3。
//!
//! # 分层纪律（契约 §2）
//!
//! 1. **net 不依赖 `audiolink-identity`**：证书 / 私钥以原始 DER 字节进出，
//!    net 不生成、不解析、不落盘（`EndpointConfig::cert_der` / `key_der_pkcs8`）。
//! 2. **net 只做传输，不懂协议语义**：控制帧载荷是不透明的 postcard 字节
//!    （[`ControlMessage::payload`]），net 不解释它的 schema；帧的“未知命令码”也只做透传，
//!    由 engine 按 §1.1 忽略并计数。
//! 3. **信任判定不在 net**：TLS 层只证明「对端持有该证书的私钥」，
//!    指纹（[`Connection::peer_id`]）值不值得信任由 engine 查 `audiolink-identity` 的信任库决定。
//!
//! # 模块
//!
//! - [`endpoint`]：绑定 / 接受 / 发起（[`AudioLinkEndpoint`]）
//! - [`connection`]：一条连接上的数据报与控制流（[`Connection`]）
//! - [`control`]：控制流 #0 的帧收发（[`ControlChannel`] / [`ControlMessage`]）
//! - [`clock`]：§6 的四时间戳偏移 / 漂移估计（[`ClockEstimator`]，**纯逻辑**）
//! - [`error`]：唯一错误类型 [`NetError`]
//! - [`local_addr`]：本机可达地址探测（[`local_endpoints`] / [`reachable_endpoints`]）
//!
//! # 实测提醒（§3「实测修正」）
//!
//! `DATAGRAM_MAX_LEN = 1200` 是**协议预算**，不是实际可用值：默认初始 MTU 下 QUIC 只给
//! **1162 B**（短头 + AEAD 开销约 38 B），DPLPMTUD 探测到更大 MTU 后还会变。
//! 因此发送侧必须用 [`Connection::max_audio_payload`] 逐连接取上限再分片，
//! [`Connection::send_datagram`] 也会按 `min(1200, max_datagram_size())` 再兜一层底线。

#![deny(unsafe_code)] // 必须使用 unsafe 的 crate（如 FFI 绑定）在文件顶部显式 #[allow] 并注明理由
#![deny(clippy::unwrap_used, clippy::expect_used)] // 实时路径禁止 panic；确需处用 #[allow] 并注明理由
#![deny(clippy::panic)]

pub mod clock;
pub mod connection;
pub mod control;
pub mod endpoint;
pub mod error;
pub mod local_addr;
mod tls;

pub use clock::{BEST_RTT_SAMPLES, ClockEstimate, ClockEstimator, ClockSample, WINDOW_SAMPLES};
pub use connection::Connection;
pub use control::{ControlChannel, ControlMessage};
pub use endpoint::{
    AudioLinkEndpoint, DEFAULT_IDLE_TIMEOUT_MS, DEFAULT_KEEP_ALIVE_MS, EndpointConfig,
};
pub use error::NetError;
pub use local_addr::{local_endpoints, reachable_endpoints};

/// 回环 / 测试用：`server_name` 的固定取值（自签证书下 SNI 不参与信任判定，§2）。
pub const TLS_SERVER_NAME: &str = "audiolink";

#[cfg(test)]
pub(crate) mod testing;
