//! QUIC 端点：绑定、接受入站连接、主动连接（`docs/11-m1-contract.md` §3「端点」）。
//!
//! # 为什么不用 `quinn::Endpoint::client()` / `server()`
//!
//! 那两个是**单角色**便捷构造。`docs/03-protocol.md` §1 规定「QUIC 所有节点共用端口，
//! 监听 + 发起同一端口」，也就是每个节点**既要 accept 也要 connect**，而 `quinn::Endpoint` 的
//! 默认客户端配置只能在构造后、共享之前设置（`set_default_client_config(&mut self)`）。
//! 因此这里走 `Endpoint::new(...)`：自己 bind UDP socket，同时挂上服务端配置，
//! 再用 `&mut` 挂默认客户端配置。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use audiolink_types::DEFAULT_QUIC_PORT;

use crate::connection::Connection;
use crate::error::NetError;
use crate::tls;

/// 数据报收发缓冲默认值（1 MiB）。
///
/// quinn 的 `TransportConfig` 默认是 `datagram_receive_buffer_size = Some(0)`、
/// `datagram_send_buffer_size = 0` —— 即「协商了数据报但缓冲为 0」，一发就 `Disabled`。
/// 音频是突发流量（20 ms 一包），所以必须显式给一个能吃下几秒抖动的缓冲。
const DATAGRAM_BUFFER_BYTES: usize = 1 << 20;

/// 空闲超时默认值（ms）：§3 的存活判定以 1 s 心跳为基准，10 s 留足了丢包余量。
pub const DEFAULT_IDLE_TIMEOUT_MS: u32 = 10_000;

/// keep-alive 间隔默认值（ms）：§3 规定 1 s 发 `KEEPALIVE`；QUIC 自身的 keep-alive
/// 用来保证 NAT / 状态防火墙不回收映射（比应用层心跳更早生效）。
pub const DEFAULT_KEEP_ALIVE_MS: u32 = 3_000;

/// 端点配置（契约 §3 冻结形状）。
#[derive(Debug, Clone)]
pub struct EndpointConfig {
    /// 本机监听地址（§1：默认端口 58290，`0.0.0.0` 表示全部网卡）。
    pub bind: SocketAddr,
    /// 本机自签证书 DER（来自 `audiolink-identity`，**net 不解析**、不落盘）。
    pub cert_der: Vec<u8>,
    /// 本机私钥 PKCS#8 DER。
    pub key_der_pkcs8: Vec<u8>,
    /// 空闲超时（ms）；默认 [`DEFAULT_IDLE_TIMEOUT_MS`]；`0` = 禁用空闲超时。
    pub idle_timeout_ms: u32,
    /// keep-alive 间隔（ms）；默认 [`DEFAULT_KEEP_ALIVE_MS`]；`0` = 关闭。
    pub keep_alive_ms: u32,
}

impl Default for EndpointConfig {
    /// 除证书/私钥外全部给默认值（证书必须由 `audiolink-identity` 提供，net 不生成）。
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([0, 0, 0, 0], DEFAULT_QUIC_PORT)),
            cert_der: Vec::new(),
            key_der_pkcs8: Vec::new(),
            idle_timeout_ms: DEFAULT_IDLE_TIMEOUT_MS,
            keep_alive_ms: DEFAULT_KEEP_ALIVE_MS,
        }
    }
}

/// QUIC 端点：一个 UDP socket 上的全部连接与入站握手。
#[derive(Debug)]
pub struct AudioLinkEndpoint {
    /// quinn 端点（`send_datagram` / `accept` / 连接集合都挂在它上面）。
    inner: quinn::Endpoint,
}

impl AudioLinkEndpoint {
    /// 绑定并准备好收发（证书来自调用方，net 只做装载）。
    pub async fn bind(cfg: EndpointConfig) -> Result<Self, NetError> {
        if cfg.cert_der.is_empty() || cfg.key_der_pkcs8.is_empty() {
            return Err(NetError::config(
                "cert_der / key_der_pkcs8 为空：证书必须由 audiolink-identity 提供（net 不生成、不落盘）",
            ));
        }

        let transport = transport_config(&cfg)?;
        // 同一份传输参数挂到两个方向：服务端配置（含索取客户端证书的校验器）与默认客户端配置
        let server = tls::server_config(&cfg.cert_der, &cfg.key_der_pkcs8, transport.clone())?;
        let client = tls::client_config(&cfg.cert_der, &cfg.key_der_pkcs8, transport)?;

        let socket = std::net::UdpSocket::bind(cfg.bind).map_err(|error| {
            NetError::bind_owned(format!("UDP 绑定 {} 失败：{error}", cfg.bind))
        })?;

        // quinn 需要一个异步运行时的适配器；`default_runtime()` 在 tokio 上下文里返回 `Some`
        let runtime = quinn::default_runtime().ok_or_else(|| {
            NetError::config(
                "找不到异步运行时：bind() 必须在 tokio 运行时内调用（quinn 的 runtime-tokio）",
            )
        })?;

        let mut endpoint = quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            Some(server),
            socket,
            runtime,
        )
        .map_err(|error| NetError::bind_owned(format!("创建 QUIC 端点失败：{error}")))?;
        // 必须在 `&mut` 阶段挂上：共享之后 quinn 不再提供设置入口
        endpoint.set_default_client_config(client);

        Ok(Self { inner: endpoint })
    }

    /// 实际监听的地址（`bind` 传 `:0` 时用它取系统分配的真实端口）。
    pub fn local_addr(&self) -> Result<SocketAddr, NetError> {
        self.inner
            .local_addr()
            .map_err(|error| NetError::bind_owned(format!("读取本地地址失败：{error}")))
    }

    /// 接受下一个入站连接（循环调用）。
    pub async fn accept(&self) -> Result<Connection, NetError> {
        let incoming = self
            .inner
            .accept()
            .await
            .ok_or_else(|| NetError::transport("端点已关闭，不会再接受新连接"))?;
        let connection = incoming
            .await
            .map_err(|error| NetError::handshake_owned(format!("入站 QUIC 握手失败：{error}")))?;
        Ok(Connection::new(connection))
    }

    /// 主动连接。`server_name` 用固定值 `"audiolink"`：自签证书下 SNI 不参与信任判定
    /// （信任判定只认 [`Connection::peer_id`] 的指纹，见 `tls` 模块文档）。
    pub async fn connect(
        &self,
        addr: SocketAddr,
        server_name: &str,
    ) -> Result<Connection, NetError> {
        let connecting = self
            .inner
            .connect(addr, server_name)
            .map_err(|error| NetError::handshake_owned(format!("发起连接 {addr} 失败：{error}")))?;
        let connection = connecting
            .await
            .map_err(|error| NetError::handshake_owned(format!("与 {addr} 握手失败：{error}")))?;
        Ok(Connection::new(connection))
    }

    /// 关闭端点：现有连接与后续入站全部终止。
    pub fn close(&self, code: u32, reason: &str) {
        // quinn 0.11 的 `VarInt::from_u32` 是不可失败的（u32 恒 < 2^62）
        self.inner
            .close(quinn::VarInt::from_u32(code), reason.as_bytes());
    }
}

/// 组装 quinn 传输参数。
///
/// 三处必须显式设置（默认值都不能用）：
/// 1. 数据报收发缓冲：默认 0 → 数据报直接 `Disabled`；
/// 2. 空闲超时：默认 30 s，与 §3 的「10 s → Reconnecting」不符；
/// 3. keep-alive：默认关闭，链路空闲时中间设备会回收 UDP 映射。
fn transport_config(cfg: &EndpointConfig) -> Result<Arc<quinn::TransportConfig>, NetError> {
    let mut transport = quinn::TransportConfig::default();
    transport
        .datagram_receive_buffer_size(Some(DATAGRAM_BUFFER_BYTES))
        .datagram_send_buffer_size(DATAGRAM_BUFFER_BYTES);

    if cfg.idle_timeout_ms > 0 {
        let duration = Duration::from_millis(u64::from(cfg.idle_timeout_ms));
        let timeout = quinn::IdleTimeout::try_from(duration).map_err(|_| {
            NetError::config_owned(format!(
                "idle_timeout_ms={} 超出 QUIC VarInt 可表示范围",
                cfg.idle_timeout_ms
            ))
        })?;
        transport.max_idle_timeout(Some(timeout));
    } else {
        // 0 = 禁用空闲超时（长会话调试用；生产不应关闭，否则半开连接会永久占着）
        transport.max_idle_timeout(None);
    }

    if cfg.keep_alive_ms > 0 {
        transport.keep_alive_interval(Some(Duration::from_millis(u64::from(cfg.keep_alive_ms))));
    } else {
        transport.keep_alive_interval(None);
    }

    Ok(Arc::new(transport))
}
