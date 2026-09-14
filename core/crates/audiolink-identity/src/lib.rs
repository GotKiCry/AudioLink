//! audiolink-identity —— 自签证书、指纹、信任库、PIN 配对状态机
//!
//! 规格：`docs/03-protocol.md` §5（连接与认证时序、认证强度、PIN 安全）、§9/§11、
//! `docs/02-architecture.md` §8（发现与配对）、§9（配置与持久化）；
//! 接口形状冻结于 `docs/11-m1-contract.md` §4。
//!
//! # 这一层负责什么
//!
//! | 关注点 | 类型 | 规格依据 |
//! |---|---|---|
//! | 身份（自签证书 + 私钥 + 指纹） | [`NodeIdentity`] | §5、架构 §9 |
//! | 信任边界（白名单落盘） | [`TrustStore`] / [`TrustEntry`] | 架构 §8、FR-18 |
//! | 配对门禁（6 位 PIN / 60 s / 5 次 / 锁 5 min） | [`PinGate`] / [`PairRejection`] | §5「PIN 安全」 |
//!
//! # 两条刻意的严格性
//!
//! 1. **身份文件只存在一半时报错，不静默重建**：重建会换指纹，等于把对端白名单里还留着的旧指纹
//!    悄悄作废（用户看到的只是一次正常启动）。
//! 2. **信任库读不懂就 `Err`**：静默重置信任库 = 悄悄清空信任边界。
//!
//! # 认证报文的排布（契约的一处实现解读，供 engine 对齐）
//!
//! 契约 §4 写的是 `ECDSA-P256(privkey, nonce ‖ fp_local ‖ fp_peer)`，这是**签名者视角**：
//! 签名者把自己放在前、把对话方放在后。验签方拿到的 `local` / `peer` 是**验签者视角**，与签名者
//! 正好互换，因此 [`NodeIdentity::verify_challenge`] 按 `peer ‖ local` 还原报文 —— 不这样读，
//! 两个诚实节点互相验签必然失败（契约验收第 3 组「A 签 B 验通过」也就无从成立）。
//! 报文同时绑定双方指纹，这正是 §5「PIN 安全：绑定双方指纹（防转发）」的落点。
//!
//! # 错误码
//!
//! [`IdentityError::code`] 把验签失败映射到 `1004 AUTH_FAILED`（§11，可能是中间人）；其余
//! 本机问题（证书解析、IO、信任库损坏）收敛到 `1008 BAD_REQUEST` —— §11 表里唯一中性的
//! 「本端拒绝」码。engine 侧不得据此判定对端异常，且必须先看 `is_statistical()`
//! （本 crate 恒为 `false`：身份与认证失败绝不能「计数后继续」）。

#![deny(unsafe_code)] // 本 crate 不需要任何 unsafe
#![deny(clippy::unwrap_used, clippy::expect_used)] // 实时路径禁止 panic；确需处用 #[allow] 并注明理由
#![deny(clippy::panic)]

mod atomic;
mod der;
mod error;
mod identity;
mod pin;
mod trust;

pub use error::IdentityError;
pub use identity::{CERT_FILE_NAME, CERT_SUBJECT_ALT_NAME, KEY_FILE_NAME, NodeIdentity};
pub use pin::{PIN_DIGITS, PIN_LOCKOUT, PIN_MAX_ATTEMPTS, PIN_TTL, PairRejection, PinGate};
pub use trust::{TrustEntry, TrustStore};
