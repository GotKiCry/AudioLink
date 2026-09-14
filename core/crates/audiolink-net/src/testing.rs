//! 测试脚手架：自签证书 + 一对完成真实 QUIC 握手的端点。
//!
//! 放在 crate 内（`#[cfg(test)]`）而不是 `tests/`，因为有几条契约要用**真实握手**验证，
//! 且需要 crate 内的能力：
//! - `Connection::peer_cert_der` 必须对着真实证书链验证；
//! - 「未知命令码必须送达 L2」需要往控制流里写一个 `type = 0x7F` 的合法帧，
//!   而公开的 `ControlChannel::send` 只接受已知 `OpCode`（用 `Connection::open_raw_bi`）。

// 测试代码不受实时路径的 unwrap / expect / panic 禁令约束（那三条针对运行时音频路径）
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rustls::pki_types::CertificateDer;
use sha2::{Digest, Sha256};

use audiolink_types::NodeId;

use crate::connection::Connection;
use crate::endpoint::{AudioLinkEndpoint, EndpointConfig};

/// 测试用节点身份（**不落盘**：生产路径的证书来自 `audiolink-identity`）。
pub(crate) struct TestIdentity {
    /// 自签证书 DER。
    pub(crate) cert_der: Vec<u8>,
    /// 私钥 PKCS#8 DER。
    pub(crate) key_der_pkcs8: Vec<u8>,
}

/// 生成一份自签证书（配置方式与 `latency-probe` 一致：rcgen 默认 ring 后端，不需要 CMake）。
pub(crate) fn identity() -> TestIdentity {
    let certified =
        rcgen::generate_simple_self_signed(vec!["audiolink".to_owned()]).expect("生成自签证书失败");
    let key_der_pkcs8 = certified.signing_key.serialize_der();
    let cert: CertificateDer<'static> = certified.cert.into();
    let leaf: &[u8] = cert.as_ref();
    TestIdentity {
        cert_der: leaf.to_vec(),
        key_der_pkcs8,
    }
}

/// 独立计算指纹（**不调被测代码**：契约 §4.2 的同款要求，避免「用被测实现验证被测实现」）。
pub(crate) fn sha256_node_id(bytes: &[u8]) -> NodeId {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    NodeId::from_bytes(hasher.finalize().into())
}

/// 两端点 + 双向连接。
///
/// 端点必须一起返回：`quinn::Endpoint` 一 drop，它的连接也就没了。
pub(crate) struct Pair {
    pub(crate) server_endpoint: AudioLinkEndpoint,
    pub(crate) client_endpoint: AudioLinkEndpoint,
    pub(crate) server: Connection,
    pub(crate) client: Connection,
    pub(crate) server_identity: TestIdentity,
    pub(crate) client_identity: TestIdentity,
}

/// 在 127.0.0.1 上起两个端点并完成一次真实 QUIC 握手（双向认证）。
pub(crate) async fn pair() -> Pair {
    let server_identity = identity();
    let client_identity = identity();

    let server_endpoint = AudioLinkEndpoint::bind(EndpointConfig {
        bind: "127.0.0.1:0".parse().expect("回环地址"),
        cert_der: server_identity.cert_der.clone(),
        key_der_pkcs8: server_identity.key_der_pkcs8.clone(),
        ..EndpointConfig::default()
    })
    .await
    .expect("绑定服务端端点");

    let client_endpoint = AudioLinkEndpoint::bind(EndpointConfig {
        bind: "127.0.0.1:0".parse().expect("回环地址"),
        cert_der: client_identity.cert_der.clone(),
        key_der_pkcs8: client_identity.key_der_pkcs8.clone(),
        ..EndpointConfig::default()
    })
    .await
    .expect("绑定客户端端点");

    let addr = server_endpoint.local_addr().expect("读取服务端地址");
    let (server, client) = tokio::join!(
        async { server_endpoint.accept().await.expect("接受入站连接") },
        async {
            client_endpoint
                .connect(addr, crate::TLS_SERVER_NAME)
                .await
                .expect("主动连接")
        },
    );

    Pair {
        server_endpoint,
        client_endpoint,
        server,
        client,
        server_identity,
        client_identity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 脚手架自检：没有它，上面任何一条失败都可能是「脚手架坏了」而不是被测代码有问题。
    #[tokio::test]
    async fn 脚手架能建起双向认证连接() {
        let pair = pair().await;

        assert!(pair.client.max_datagram_size().is_some());
        assert!(pair.server.max_datagram_size().is_some());

        // 客户端看到的对端地址 = 服务端端点地址；客户端自己的端点在回环上
        assert_eq!(
            pair.client.remote_addr(),
            pair.server_endpoint.local_addr().expect("服务端地址")
        );
        assert!(
            pair.client_endpoint
                .local_addr()
                .expect("客户端地址")
                .ip()
                .is_loopback()
        );

        // 双向认证真的是双向的：两边都拿到了对方的证书，
        // 且 `peer_id()` 恒等于 SHA-256(`peer_cert_der()`)（两份材料不得各自漂移）
        assert_eq!(
            pair.client.peer_cert_der().expect("客户端读对端证书"),
            pair.server_identity.cert_der
        );
        assert_eq!(
            pair.server.peer_cert_der().expect("服务端读对端证书"),
            pair.client_identity.cert_der
        );
        assert_eq!(
            pair.client.peer_id().expect("客户端指纹"),
            sha256_node_id(&pair.server_identity.cert_der)
        );
        assert_eq!(
            pair.server.peer_id().expect("服务端指纹"),
            sha256_node_id(&pair.client_identity.cert_der)
        );
    }
}
