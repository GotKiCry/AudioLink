//! audiolink-identity —— 自签证书与节点身份（指纹）
//!
//! 规格：`docs/03-protocol.md` §2（TLS 与身份）、§5（连接时序）、§9/§11、
//! `docs/02-architecture.md` §9（配置与持久化）；
//! 接口形状冻结于 `docs/11-m1-contract.md` §4。
//!
//! # 这一层负责什么
//!
//! | 关注点 | 类型 | 规格依据 |
//! |---|---|---|
//! | 身份（自签证书 + 私钥 + 指纹） | [`NodeIdentity`] | §2、架构 §9 |
//!
//! **没有信任裁决**：本层只产出身份，不回答「这个指纹值不值得信任」—— 局域网内任何节点直连即可
//! （`docs/71-remove-pairing.md`）。对端身份由 TLS 1.3 握手证明（双方互相出示证书，各自持私钥），
//! 指纹 = `SHA-256(cert DER)` 只用于区分设备、展示短码。
//!
//! # 一条刻意的严格性
//!
//! **身份文件只存在一半时报错，不静默重建**：重建会换掉指纹，让对端眼里的「同一台设备」
//! 变成另一台（用户看到的却只是一次正常启动）。
//!
//! # 错误码
//!
//! [`IdentityError::code`] 全部收敛到 `1008 BAD_REQUEST` —— §11 表里唯一中性的「本端拒绝」码：
//! 证书解析、IO、参数非法都是本机自身的问题。engine 侧不得据此判定对端异常，且必须先看
//! `is_statistical()`（本 crate 恒为 `false`：身份失败绝不能「计数后继续」）。

#![deny(unsafe_code)] // 本 crate 不需要任何 unsafe
#![deny(clippy::unwrap_used, clippy::expect_used)] // 实时路径禁止 panic；确需处用 #[allow] 并注明理由
#![deny(clippy::panic)]

mod atomic;
mod der;
mod error;
mod identity;

pub use error::IdentityError;
pub use identity::{CERT_FILE_NAME, CERT_SUBJECT_ALT_NAME, KEY_FILE_NAME, NodeIdentity};
