//! PIN 门禁（`docs/03-protocol.md` §5「PIN 安全」）
//!
//! 规格原文：**6 位数字 + 60 s 有效 + 失败 5 次锁定 5 分钟**；PIN 通过加密通道提交（防窃听），
//! 并绑定双方指纹（防转发，落在 `AUTH_RESPONSE` 的报文里，见 `identity` 模块）。
//!
//! 这条防线针对的是「同网段静默配对」（架构 §8）：`audiolink-engine` 在未配对时显示本结构持有的
//! PIN，对端必须在 PIN 有效期内、以正确数字回应。三个不变量：
//!
//! 1. **一次性**：校验成功即消耗该 PIN —— 否则被录下来的 `PAIR_SUBMIT` 可以重放；
//! 2. **可计数**：错误次数递减，第 5 次失败立即进入 5 分钟锁定；
//! 3. **锁定不吞尝试次数**：锁定期间的提交一律返回 `Locked` 且**不消耗**剩余次数
//!    （否则一旦锁定到期，合法用户也只剩 0 次可用）。

use std::fmt;
use std::time::{Duration, Instant};

use rand::Rng;

/// PIN 有效期（§5：60 s）。
pub const PIN_TTL: Duration = Duration::from_secs(60);
/// PIN 允许的失败次数上限（§5：5 次）。
pub const PIN_MAX_ATTEMPTS: u8 = 5;
/// 连续失败触发后的锁定时长（§5：5 分钟）。
pub const PIN_LOCKOUT: Duration = Duration::from_secs(300);
/// PIN 位数（§5：6 位数字）。
pub const PIN_DIGITS: usize = 6;

/// PIN 校验失败的原因（`1003 PAIR_REJECTED`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairRejection {
    /// PIN 已过期，或已被成功使用过（一次性）。
    Expired,
    /// PIN 不对；`remaining` = 本次失败之后还剩几次机会。
    WrongPin {
        /// 剩余尝试次数。
        remaining: u8,
    },
    /// 失败次数过多，处于锁定期；`retry_after` = 还需等待多久。
    Locked {
        /// 剩余锁定时长。
        retry_after: Duration,
    },
}

impl fmt::Display for PairRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expired => write!(formatter, "1003 PAIR_REJECTED: PIN 已过期或已被使用"),
            Self::WrongPin { remaining } => {
                write!(
                    formatter,
                    "1003 PAIR_REJECTED: PIN 错误，剩余 {remaining} 次"
                )
            }
            Self::Locked { retry_after } => write!(
                formatter,
                "1003 PAIR_REJECTED: 失败次数过多，{} 秒后可重试",
                retry_after.as_secs()
            ),
        }
    }
}

impl std::error::Error for PairRejection {}

/// 接收端持有的配对门禁：生成一枚 PIN，校验对端提交的 PIN。
pub struct PinGate {
    pin: String,
    expires_at: Instant,
    remaining_attempts: u8,
    /// 已成功使用过 —— 一次性，防止重放。
    consumed: bool,
    lockout_until: Option<Instant>,
}

impl PinGate {
    /// 生成并持有 6 位 PIN，`now` 为生效起点（单调时钟，§1：不用挂钟做时间运算）。
    pub fn new(now: Instant) -> Self {
        let value: u32 = rand::rng().random_range(0..1_000_000);
        Self {
            // 补零到 6 位：`random_range` 内部是拒绝采样，取值在 [0, 10^6) 上均匀，
            // 因此 000000–999999 每个码概率相同（用取模会引入偏差，让部分码概率翻倍）。
            pin: format!("{value:0width$}", width = PIN_DIGITS),
            expires_at: now.checked_add(PIN_TTL).unwrap_or(now),
            remaining_attempts: PIN_MAX_ATTEMPTS,
            consumed: false,
            lockout_until: None,
        }
    }

    /// 当前 PIN（接收端 UI 展示用）。
    ///
    /// 注意：本方法不判断有效期 —— 展示时机由调用方（engine）掌握，便于 UI 显示「已过期」态。
    pub fn pin(&self) -> &str {
        &self.pin
    }

    /// 该 PIN 的失效时刻。
    pub fn expires_at(&self) -> Instant {
        self.expires_at
    }

    /// 剩余尝试次数（锁定后为 0）。
    pub fn remaining_attempts(&self) -> u8 {
        self.remaining_attempts
    }

    /// 提交校验。成功**消耗**该 PIN（不可重放）。
    pub fn verify(&mut self, submitted: &str, now: Instant) -> Result<(), PairRejection> {
        // 顺序刻意如此：先判锁定，再判过期。
        // 若先判过期，则「锁定期间 PIN 自然过期」会退化成 `Expired`，UI 就只能说「PIN 过期了」
        // 而说不出「你被锁定到什么时候」—— 锁定状态是安全事件，必须能如实上报。
        if let Some(until) = self.lockout_until {
            if now < until {
                return Err(PairRejection::Locked {
                    retry_after: until.saturating_duration_since(now),
                });
            }
            // 锁定到期：解除锁定并恢复尝试次数。锁定期间不消耗次数（见模块文档第 3 条），
            // 所以这里恢复的是「本来就还剩下的次数」的语义等价值。
            self.lockout_until = None;
            self.remaining_attempts = PIN_MAX_ATTEMPTS;
        }

        if self.consumed || now >= self.expires_at {
            return Err(PairRejection::Expired);
        }

        if constant_time_eq(submitted.as_bytes(), self.pin.as_bytes()) {
            self.consumed = true;
            return Ok(());
        }

        self.remaining_attempts = self.remaining_attempts.saturating_sub(1);
        if self.remaining_attempts == 0 {
            let until = now.checked_add(PIN_LOCKOUT).unwrap_or(now);
            self.lockout_until = Some(until);
            return Err(PairRejection::Locked {
                retry_after: PIN_LOCKOUT,
            });
        }
        Err(PairRejection::WrongPin {
            remaining: self.remaining_attempts,
        })
    }

    /// 锁定中则返回剩余时长（`None` = 未锁定）。
    pub fn lockout_remaining(&self, now: Instant) -> Option<Duration> {
        match self.lockout_until {
            Some(until) if now < until => Some(until.saturating_duration_since(now)),
            _ => None,
        }
    }
}

impl fmt::Debug for PinGate {
    /// 手写 `Debug`：**不打印 PIN 本身**，避免调试日志把配对码漏到磁盘上。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PinGate")
            .field("pin", &"<redacted>")
            .field("remaining_attempts", &self.remaining_attempts)
            .field("consumed", &self.consumed)
            .field("locked", &self.lockout_until.is_some())
            .finish()
    }
}

/// 常量时间比较：不因「第几个字符不同」提前返回。
///
/// 本门禁已有 5 次上限，时序侧信道价值有限；但实现代价近乎为零，就没有理由留下可按字节爆破的接口。
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        // 长度不是秘密（PIN 恒为 6 位）。
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 生成的_pin_是_6_位数字() {
        let now = Instant::now();
        for _ in 0..64 {
            let gate = PinGate::new(now);
            assert_eq!(gate.pin().len(), PIN_DIGITS, "pin={}", gate.pin());
            assert!(gate.pin().bytes().all(|byte| byte.is_ascii_digit()));
            assert_eq!(gate.remaining_attempts(), PIN_MAX_ATTEMPTS);
            assert_eq!(gate.expires_at(), now + PIN_TTL);
            assert_eq!(gate.lockout_remaining(now), None);
        }
    }

    #[test]
    fn 调试输出不泄露_pin() {
        let gate = PinGate::new(Instant::now());
        let rendered = format!("{gate:?}");
        assert!(!rendered.contains(gate.pin()), "{rendered}");
    }

    #[test]
    fn 常量时间比较覆盖长短与内容差异() {
        assert!(constant_time_eq(b"123456", b"123456"));
        assert!(!constant_time_eq(b"123456", b"123457"));
        assert!(!constant_time_eq(b"123456", b"12345"));
        assert!(!constant_time_eq(b"", b"a"));
    }
}
