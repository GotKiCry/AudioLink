//! 节点身份：自签证书 + 私钥 + 指纹（`docs/03-protocol.md` §2、`docs/02-architecture.md` §9）
//!
//! 身份 = 证书 DER 的 SHA-256（[`NodeId`]），这是 AudioLink 的**唯一身份**：TLS 1.3 握手
//! （双方互相出示证书）已经保证「对端持有该证书的私钥」，指纹只用于区分设备、展示短码。
//! 证书与私钥必须同时存在、必须互相匹配，且必须原子落盘。

use std::fs;
use std::path::Path;

use audiolink_types::NodeId;
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use p256::pkcs8::{DecodePrivateKey, DecodePublicKey};
use sha2::{Digest, Sha256};

use crate::atomic::write_atomic;
use crate::der::subject_public_key_info;
use crate::error::IdentityError;

/// 证书文件名（架构 §9 的表：`identity\cert.pem`）。
pub const CERT_FILE_NAME: &str = "cert.pem";
/// 私钥文件名（PKCS#8 PEM）。
pub const KEY_FILE_NAME: &str = "key.pem";

/// 自签证书里唯一的 SAN 条目，**固定常量**（rcgen 的 `CertificateParams::new` 只写 SAN、不写 CN，
/// 故证书里的人类可读标识就这一个）。
///
/// 必须与 `docs/11-m1-contract.md` §3 里 net 侧连接用的 `server_name` 一致（`"audiolink"`）：
/// 自签场景下 SNI 不参与任何判定（身份只看证书指纹），但两者保持一致才不会在将来启用真正的
/// 证书校验时埋雷。
///
/// **为什么不把 [`NodeIdentity::node_name`] 写进证书**：SAN 是 `dNSName`（IA5String，仅 ASCII），
/// 中文设备名会让 rcgen 直接报 `Invalid IA5String` —— 而本项目的设备名天然是中文（「我的手机」
/// 「客厅 PC」）。展示名本来就不参与身份判定（types §3），把它塞进证书只会制造两条错误路径：
/// 改个名字就要换身份、非 ASCII 名字直接生成失败。
pub const CERT_SUBJECT_ALT_NAME: &str = "audiolink";

/// 加载身份时的自检消息：固定常量，只用来证实「证书里的公钥 = 这把私钥的公钥」。
/// 它不参与任何协议语义，也不出网 —— 只在 [`NodeIdentity::from_pem`] 里签一次、验一次。
const SELF_CHECK_MESSAGE: &[u8] = b"audiolink-identity/self-check";

/// 本节点身份。
///
/// 只持有**落盘形式的字节**（[`NodeIdentity::cert_der`] / [`NodeIdentity::key_der_pkcs8`]）：
/// net 建 QUIC 端点时直接用它们装载密钥，本层不需要在内存里另留一份解析后的 `SigningKey`
/// （解析只在 [`NodeIdentity::from_pem`] 的自检里临时发生）。
pub struct NodeIdentity {
    /// SHA-256(cert DER)：构造时算一次（握手路径不做重复哈希）。
    id: NodeId,
    cert_der: Vec<u8>,
    cert_pem: String,
    key_der_pkcs8: Vec<u8>,
    node_name: String,
}

impl NodeIdentity {
    /// 从 `dir` 加载 `cert.pem` + `key.pem`；两者都不存在则生成一对并**原子落盘**
    /// （临时文件 + rename，架构 §9）。
    ///
    /// **只存在其中一个时返回 Err，不静默重新生成**：重新生成会换掉指纹，让对端眼里的
    /// 「同一台设备」变成另一台，比拒绝启动危险得多。
    pub fn load_or_create(dir: impl AsRef<Path>, node_name: &str) -> Result<Self, IdentityError> {
        let dir = dir.as_ref();
        let cert_path = dir.join(CERT_FILE_NAME);
        let key_path = dir.join(KEY_FILE_NAME);

        match (cert_path.exists(), key_path.exists()) {
            (true, true) => {
                let cert_pem = read_to_string(&cert_path, "读取 cert.pem 失败")?;
                let key_pem = read_to_string(&key_path, "读取 key.pem 失败")?;
                Self::from_pem(&cert_pem, &key_pem, node_name)
            }
            (false, false) => {
                let certified =
                    rcgen::generate_simple_self_signed(vec![CERT_SUBJECT_ALT_NAME.to_string()])
                        .map_err(|error| {
                            IdentityError::certificate_owned(format!("生成自签证书失败：{error}"))
                        })?;
                // rcgen 的默认算法即 ECDSA P-256（`KeyPair::generate()` → PKCS_ECDSA_P256_SHA256）；
                // `serialize_pem()` 与 `serialize_der()` 是同一份 PKCS#8 数据的两种包装。
                let key_pem = certified.signing_key.serialize_pem();
                let cert_pem = certified.cert.pem();

                // 先落私钥再落证书：私钥**不可再生**（丢了就永久失去这个身份），证书可以用同一
                // 私钥重签。崩溃窗口里宁可留下「有私钥、无证书」这种可人工恢复的状态，也不要
                // 留下「有证书、无私钥」这种身份永久不可用的状态。
                write_atomic(&key_path, key_pem.as_bytes())?;
                write_atomic(&cert_path, cert_pem.as_bytes())?;

                Self::from_pem(&cert_pem, &key_pem, node_name)
            }
            (true, false) => Err(IdentityError::io_owned(format!(
                "身份文件不完整：{} 存在但 {} 缺失（拒绝重新生成，重新生成会更换本机指纹）",
                cert_path.display(),
                key_path.display()
            ))),
            (false, true) => Err(IdentityError::io_owned(format!(
                "身份文件不完整：{} 存在但 {} 缺失（拒绝重新生成，重新生成会更换本机指纹）",
                key_path.display(),
                cert_path.display()
            ))),
        }
    }

    /// 由 PEM 文本构造身份（不落盘）。
    ///
    /// 加载即自检「证书公钥 == 私钥公钥」：不匹配的身份在 TLS 握手时就立不住（对端会拿
    /// **证书里的公钥**验证本机出示的密钥），必须在加载时就失败，而不是等握手时才暴露。
    pub fn from_pem(cert_pem: &str, key_pem: &str, node_name: &str) -> Result<Self, IdentityError> {
        if node_name.trim().is_empty() {
            return Err(IdentityError::invalid_config("节点名不能为空"));
        }

        let cert_der = decode_cert_pem(cert_pem)?;
        let key_der_pkcs8 = decode_key_pem(key_pem)?;
        let signing_key = SigningKey::from_pkcs8_der(&key_der_pkcs8).map_err(|error| {
            IdentityError::key_owned(format!("私钥不是合法的 P-256 PKCS#8 私钥：{error}"))
        })?;

        let spki = subject_public_key_info(&cert_der)?;
        let cert_public = VerifyingKey::from_public_key_der(spki).map_err(|error| {
            IdentityError::certificate_owned(format!("证书里的公钥不是合法的 P-256 公钥：{error}"))
        })?;
        let proof: Signature = signing_key
            .try_sign(SELF_CHECK_MESSAGE)
            .map_err(|error| IdentityError::key_owned(format!("自检签名失败：{error}")))?;
        cert_public
            .verify(SELF_CHECK_MESSAGE, &proof)
            .map_err(|_| {
                IdentityError::certificate("证书与私钥不匹配：证书公钥不是这把私钥的公钥")
            })?;

        Ok(Self {
            id: fingerprint(&cert_der),
            cert_der,
            cert_pem: cert_pem.to_owned(),
            key_der_pkcs8,
            node_name: node_name.to_owned(),
        })
    }

    /// 本节点指纹 = SHA-256(cert DER)（架构 §9：设备身份的唯一依据）。
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// 本节点证书 DER。
    pub fn cert_der(&self) -> &[u8] {
        &self.cert_der
    }

    /// 本节点私钥 PKCS#8 DER（交给 net 建 QUIC 端点时用；net 不解析它）。
    pub fn key_der_pkcs8(&self) -> &[u8] {
        &self.key_der_pkcs8
    }

    /// 展示名（**仅展示**，不参与任何身份判定，也不写进证书 —— 见
    /// [`CERT_SUBJECT_ALT_NAME`] 的说明）。
    pub fn node_name(&self) -> &str {
        &self.node_name
    }

    /// 证书 PEM 原文（落盘内容原样返回）。
    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }
}

impl std::fmt::Debug for NodeIdentity {
    /// 手写 `Debug`：**绝不打印私钥**，只暴露对外可见的身份信息。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NodeIdentity")
            .field("id", &self.id)
            .field("node_name", &self.node_name)
            .field("key_der_pkcs8", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// `NodeId` = SHA-256(cert DER)。
fn fingerprint(cert_der: &[u8]) -> NodeId {
    let digest = Sha256::digest(cert_der);
    let mut bytes = [0u8; NodeId::LEN];
    // 用 zip 而不是 `copy_from_slice`：长度恒等（SHA-256 = 32 B），但这样连「潜在的 panic 路径」
    // 都不存在。
    for (slot, byte) in bytes.iter_mut().zip(digest.iter()) {
        *slot = *byte;
    }
    NodeId::from_bytes(bytes)
}

/// 读文本文件，失败时把路径写进上下文。
fn read_to_string(path: &Path, what: &str) -> Result<String, IdentityError> {
    fs::read_to_string(path)
        .map_err(|error| IdentityError::io_owned(format!("{what}（{}）：{error}", path.display())))
}

/// 解析 `cert.pem` → DER。文件里有多张证书时取第一张（叶子证书在前）。
fn decode_cert_pem(cert_pem: &str) -> Result<Vec<u8>, IdentityError> {
    let mut reader = std::io::BufReader::new(cert_pem.as_bytes());
    let first = rustls_pemfile::certs(&mut reader)
        .next()
        .ok_or_else(|| IdentityError::certificate("cert.pem 里没有 CERTIFICATE 块"))?
        .map_err(|error| IdentityError::certificate_owned(format!("cert.pem 解析失败：{error}")))?;
    Ok(first.to_vec())
}

/// 解析 `key.pem` → PKCS#8 DER。
///
/// 只接受 PKCS#8（`PRIVATE KEY`）：rcgen 0.14 落盘的就是 PKCS#8，出现 SEC1 / PKCS#1 说明文件
/// 被外部工具改写过 —— 不做兼容猜测，直接报错，避免把「另一种格式」误判成同一身份。
fn decode_key_pem(key_pem: &str) -> Result<Vec<u8>, IdentityError> {
    let mut reader = std::io::BufReader::new(key_pem.as_bytes());
    let item = rustls_pemfile::read_one(&mut reader)
        .map_err(|error| IdentityError::key_owned(format!("key.pem 解析失败：{error}")))?
        .ok_or_else(|| IdentityError::key("key.pem 里没有任何 PEM 段"))?;
    match item {
        rustls_pemfile::Item::Pkcs8Key(der) => Ok(der.secret_pkcs8_der().to_vec()),
        _ => Err(IdentityError::key(
            "key.pem 不是 PKCS#8（PRIVATE KEY）段；本仓库只认 rcgen 落盘的 PKCS#8",
        )),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 空节点名被拒() {
        assert!(NodeIdentity::from_pem("x", "y", "  ").is_err());
    }

    #[test]
    fn cert_key_mismatch_errors_instead_of_panicking() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = NodeIdentity::load_or_create(dir_a.path(), "a").unwrap();
        NodeIdentity::load_or_create(dir_b.path(), "b").unwrap();

        // 把 B 的私钥搬到 A 的身份目录：两份 PEM 都合法，但公钥对不上。
        fs::copy(
            dir_b.path().join(KEY_FILE_NAME),
            dir_a.path().join(KEY_FILE_NAME),
        )
        .unwrap();

        let mixed = NodeIdentity::load_or_create(dir_a.path(), "a");
        assert!(matches!(mixed, Err(IdentityError::Certificate { .. })));
        // 失败不得顺手改写身份文件：证书内容必须还是 A 的。
        assert_eq!(
            fs::read_to_string(dir_a.path().join(CERT_FILE_NAME)).unwrap(),
            a.cert_pem()
        );
    }

    #[test]
    fn 只剩证书或只剩私钥时拒绝重新生成() {
        let dir = tempfile::tempdir().unwrap();
        NodeIdentity::load_or_create(dir.path(), "solo").unwrap();

        // 删掉私钥：必须报错，而不是「重新生成一份新身份」（那会换掉本机指纹）。
        fs::remove_file(dir.path().join(KEY_FILE_NAME)).unwrap();
        let outcome = NodeIdentity::load_or_create(dir.path(), "solo");
        assert!(
            matches!(outcome, Err(IdentityError::Io { .. })),
            "{outcome:?}"
        );
    }
}
