//! 最小 DER 读取器 —— 只为「从 X.509 证书里取出 SubjectPublicKeyInfo」服务。
//!
//! **为什么自己走位**：本 crate 不引入 x509 解析库（根清单没有该依赖，且本仓库拒绝为依赖
//! 付 CMake/NASM 代价）。这里只做**结构走位**：不校验签名、不解析名字、不认扩展 —— 语义校验
//! 交给 `p256`（公钥必须是合法的 P-256 点，否则 `from_public_key_der` 会 Err）。
//!
//! RFC 5280 的相关结构：
//!
//! ```text
//! Certificate    ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signatureValue }
//! TBSCertificate ::= SEQUENCE { [0] version?, serialNumber, signature, issuer, validity,
//!                               subject, subjectPublicKeyInfo, ... }
//! ```
//!
//! 走位规则：TBSCertificate 的字段顺序固定，故「用 tag 判断可选的 version，然后按位置数」是
//! 严格可靠的 —— 不依赖任何实现细节，也不做启发式搜索（搜索会在证书里出现同构字节串时误判）。

use crate::error::IdentityError;

/// `SEQUENCE` 的 tag。
const TAG_SEQUENCE: u8 = 0x30;
/// `[0] EXPLICIT` 的 tag（TBSCertificate 的 `version`，v3 证书必带）。
const TAG_CONTEXT_0: u8 = 0xA0;
/// 长形式长度域最多允许 4 字节：既排除 `n == 0` 的不定长形式，也够容纳任何真实证书。
const MAX_LEN_OCTETS: usize = 4;

/// 一个已定位的 TLV（tag / 内容 / 完整原文）。
#[derive(Debug, Clone, Copy)]
struct Tlv<'a> {
    tag: u8,
    value: &'a [u8],
    raw: &'a [u8],
}

/// 前向游标式 DER 读取器。
#[derive(Debug, Clone, Copy)]
struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// 看下一个 TLV 的 tag（用于识别可选字段），不消费。
    fn peek_tag(&self) -> Option<u8> {
        self.bytes.first().copied()
    }

    /// 读出一个 TLV 并前移游标。
    fn read_tlv(&mut self) -> Result<Tlv<'a>, IdentityError> {
        let remaining = self.bytes;
        let (&tag, rest) = remaining
            .split_first()
            .ok_or_else(|| truncated("缺少 tag"))?;
        let (&length_first, rest) = rest.split_first().ok_or_else(|| truncated("缺少长度域"))?;

        let (length, rest) = if length_first < 0x80 {
            // 短形式：长度直接就是这一个字节。
            (usize::from(length_first), rest)
        } else {
            // 长形式：低 7 位是长度域的字节数。
            let octets = usize::from(length_first & 0x7f);
            if octets == 0 || octets > MAX_LEN_OCTETS {
                return Err(IdentityError::certificate(
                    "证书 DER 长度域非法：不支持不定长或超过 4 字节的长度",
                ));
            }
            let (length_bytes, rest) = rest
                .split_at_checked(octets)
                .ok_or_else(|| truncated("长度域不完整"))?;
            let mut length = 0usize;
            for &byte in length_bytes {
                length = (length << 8) | usize::from(byte);
            }
            (length, rest)
        };

        let (value, tail) = rest
            .split_at_checked(length)
            .ok_or_else(|| truncated("内容长度越界"))?;
        let raw_len = remaining.len() - tail.len();
        self.bytes = tail;
        Ok(Tlv {
            tag,
            value,
            raw: &remaining[..raw_len],
        })
    }
}

/// 统一的「DER 截断」错误（带上具体位置，便于定位被改坏的文件）。
fn truncated(detail: &'static str) -> IdentityError {
    IdentityError::certificate_owned(format!("证书 DER 截断：{detail}"))
}

/// 从 X.509 证书 DER 里取出 `SubjectPublicKeyInfo` 的**完整 DER**（含自身的 SEQUENCE 头）。
///
/// 返回的切片可直接交给 `p256::ecdsa::VerifyingKey::from_public_key_der`。
pub(crate) fn subject_public_key_info(cert_der: &[u8]) -> Result<&[u8], IdentityError> {
    let mut certificate_reader = Reader::new(cert_der);
    let certificate = certificate_reader.read_tlv()?;
    if certificate.tag != TAG_SEQUENCE {
        return Err(IdentityError::certificate(
            "证书 DER 顶层不是 SEQUENCE（不是 X.509 证书）",
        ));
    }

    let mut certificate_fields = Reader::new(certificate.value);
    let tbs = certificate_fields.read_tlv()?;
    if tbs.tag != TAG_SEQUENCE {
        return Err(IdentityError::certificate(
            "证书 DER 的 tbsCertificate 不是 SEQUENCE",
        ));
    }

    let mut tbs_fields = Reader::new(tbs.value);
    // 可选字段 version：v1 证书没有它，v2/v3 是 `[0] EXPLICIT INTEGER`。按 tag 判断而不是
    // 按位置跳过 —— 这是唯一「字段个数可变」的位置。
    if tbs_fields.peek_tag() == Some(TAG_CONTEXT_0) {
        tbs_fields.read_tlv()?;
    }
    // serialNumber / signature / issuer / validity / subject：顺序固定，逐个跳过。
    for _ in 0..5 {
        tbs_fields.read_tlv()?;
    }

    let spki = tbs_fields.read_tlv()?;
    if spki.tag != TAG_SEQUENCE {
        return Err(IdentityError::certificate(
            "证书 DER 的 subjectPublicKeyInfo 不是 SEQUENCE",
        ));
    }
    Ok(spki.raw)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// 生成一对真实的自签证书/密钥（测试夹具，非被测逻辑）。
    fn sample_cert_der() -> Vec<u8> {
        let certified = rcgen::generate_simple_self_signed(vec!["audiolink-test".to_string()])
            .expect("生成自签证书失败");
        certified.cert.der().to_vec()
    }

    #[test]
    fn 能从真实证书里取出可用公钥() {
        use p256::ecdsa::VerifyingKey;
        use p256::pkcs8::DecodePublicKey;

        let cert_der = sample_cert_der();
        let spki = subject_public_key_info(&cert_der).expect("应能从证书里定位 SPKI");
        // 语义校验交给 p256：取出来的必须是合法 P-256 点。
        VerifyingKey::from_public_key_der(spki).expect("SPKI 应能解析成 P-256 公钥");
    }

    #[test]
    fn truncated_or_invalid_certs_error_instead_of_panicking() {
        let cert_der = sample_cert_der();

        assert!(subject_public_key_info(&[]).is_err());
        assert!(subject_public_key_info(&cert_der[..1]).is_err());
        // 砍掉尾巴：外层长度域声明的长度超出实际字节数。
        assert!(subject_public_key_info(&cert_der[..cert_der.len() / 2]).is_err());
        // 顶层 tag 不是 SEQUENCE。
        assert!(subject_public_key_info(&[0x02, 0x01, 0x00]).is_err());
        // 不定长形式（0x80）。
        assert!(subject_public_key_info(&[0x30, 0x80, 0x00, 0x00]).is_err());
    }
}
