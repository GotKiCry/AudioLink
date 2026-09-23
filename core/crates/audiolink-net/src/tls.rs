//! rustls 配置与「只收证书、不做信任裁决」的身份证明逻辑。
//!
//! **强制 ring 后端**：rustls 默认的 `aws-lc-rs` 需要 CMake/NASM，本机与 CI 都不装
//! （`docs/06-dev-environment.md` §4、`docs/11-m1-contract.md` §10.3）。
//! 配置方式照抄仓库内已验证可运行的 `core/crates/audiolink-tools/src/bin/latency_probe.rs`。
//!
//! # 为什么要做**双向**认证
//!
//! `docs/03-protocol.md` §2：节点身份 = `SHA-256(证书 DER)`，握手不依赖 CA（自签证书，无信任锚可查）。
//! 双方都要能回答「对端是谁」，所以服务端也必须索取客户端证书：
//! 否则接收侧的 `peer_identity()` 恒为 `None`，`peer_id()` 就永远拿不到「你是谁」。
//! 出示证书只证明「对端持有该证书的私钥」，**没有**任何「这个指纹值不值得信任」的裁决
//! （`docs/71-remove-pairing.md`）。
//!
//! # 这里的 `verify_*_cert` 返回「接受」为什么不等于「跳过校验」
//!
//! - TLS 签名校验照做（[`AcceptAnyServerVerifier::verify_tls13_signature`] 等用 ring 的算法表实现），
//!   因此「对端确实持有该证书的私钥」仍被密码学证明；
//! - 剩下的唯一问题是「这张证书属于谁」—— 那由 [`crate::Connection::peer_id`] 的指纹回答。
//!   自签场景没有 CA 可查，AudioLink 也**不再**在指纹之上做任何信任裁决
//!   （`docs/71-remove-pairing.md`），所以校验器只需要把证书**收下来**。
//!
//! 换句话说：CA 链校验被**换掉**了 —— 不是被遗忘，也不是悄悄「跳过」。

use std::sync::Arc;

use rustls::DistinguishedName;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};

use crate::error::NetError;

/// rustls 加密后端：**强制 ring**（与 `latency-probe` 同一套配置方式）。
pub(crate) fn crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// 证书 DER → rustls 的证书链（自签证书：链里只有叶子证书）。
fn cert_chain(cert_der: &[u8]) -> Vec<CertificateDer<'static>> {
    vec![CertificateDer::from(cert_der.to_vec())]
}

/// PKCS#8 DER 私钥（`rcgen` 的 `signing_key.serialize_der()` 与 `identity` 落盘的就是这个格式）。
fn private_key(key_der_pkcs8: &[u8]) -> PrivateKeyDer<'static> {
    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der_pkcs8.to_vec()))
}

/// 服务端配置：**索取客户端证书**（双向认证，见模块文档）。
pub(crate) fn server_config(
    cert_der: &[u8],
    key_der_pkcs8: &[u8],
    transport: Arc<quinn::TransportConfig>,
) -> Result<quinn::ServerConfig, NetError> {
    let crypto = rustls::ServerConfig::builder_with_provider(crypto_provider())
        .with_safe_default_protocol_versions()
        .map_err(|error| NetError::config_owned(format!("rustls 协议版本配置失败：{error}")))?
        .with_client_cert_verifier(Arc::new(AcceptAnyClientVerifier::new()))
        .with_single_cert(cert_chain(cert_der), private_key(key_der_pkcs8))
        .map_err(|error| NetError::config_owned(format!("装载本机证书/私钥失败：{error}")))?;

    let quic_crypto = quinn::crypto::rustls::QuicServerConfig::try_from(crypto)
        .map_err(|error| NetError::config_owned(format!("QUIC 服务端加密配置失败：{error}")))?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));
    config.transport_config(transport);
    Ok(config)
}

/// 客户端配置：出示本机证书（双向认证），并接受任何结构合法的对端自签证书（不做信任裁决）。
pub(crate) fn client_config(
    cert_der: &[u8],
    key_der_pkcs8: &[u8],
    transport: Arc<quinn::TransportConfig>,
) -> Result<quinn::ClientConfig, NetError> {
    let crypto = rustls::ClientConfig::builder_with_provider(crypto_provider())
        .with_safe_default_protocol_versions()
        .map_err(|error| NetError::config_owned(format!("rustls 协议版本配置失败：{error}")))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyServerVerifier::new()))
        // `with_client_auth_cert` 内部用 `SingleCertAndKey`，其 `resolve()` **忽略**服务端的
        // CA 提示、恒返回本机证书 —— 这正是自签场景想要的（服务端不发任何信任锚提示）。
        .with_client_auth_cert(cert_chain(cert_der), private_key(key_der_pkcs8))
        .map_err(|error| NetError::config_owned(format!("装载本机证书/私钥失败：{error}")))?;

    let quic_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
        .map_err(|error| NetError::config_owned(format!("QUIC 客户端加密配置失败：{error}")))?;
    let mut config = quinn::ClientConfig::new(Arc::new(quic_crypto));
    config.transport_config(transport);
    Ok(config)
}

/// 客户端侧：接受任何**结构合法**的对端自签证书（不做信任裁决，见模块文档）。
#[derive(Debug)]
pub(crate) struct AcceptAnyServerVerifier {
    /// 签名校验算法表来源（ring 后端）。
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl AcceptAnyServerVerifier {
    fn new() -> Self {
        Self {
            provider: crypto_provider(),
        }
    }
}

impl rustls::client::danger::ServerCertVerifier for AcceptAnyServerVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        // 只做「把证书拿到手」：指纹只用于标识对端身份，本层不做任何信任裁决。
        // `server_name`（SNI）刻意不参与判定 —— 自签证书的 CN/SAN 没有可信来源。
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// 服务端侧：接受任何**结构合法**的客户端自签证书（不做信任裁决，见模块文档）。
#[derive(Debug)]
pub(crate) struct AcceptAnyClientVerifier {
    /// 签名校验算法表来源（ring 后端）。
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl AcceptAnyClientVerifier {
    fn new() -> Self {
        Self {
            provider: crypto_provider(),
        }
    }
}

impl rustls::server::danger::ClientCertVerifier for AcceptAnyClientVerifier {
    /// 空提示列表的语义是「客户端只要持有证书就必须出示」（RFC 8446 §4.3.2 / rustls 文档）。
    ///
    /// 我们没有 CA 可提示（自签证书，无信任锚），所以列表必须为空；
    /// 一旦填了内容，反而会让客户端去「匹配」一个不存在的信任锚。
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<rustls::server::danger::ClientCertVerified, rustls::Error> {
        // 同上：这里只把证书收下来，指纹怎么用由上层决定。
        Ok(rustls::server::danger::ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
