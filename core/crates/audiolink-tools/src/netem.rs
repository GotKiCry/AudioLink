//! 弱网注入（`tools/netem-sim` 与 `soak-runner --netem-*` 的共用内核）。
//!
//! 路线图 M2 的验收条件是「弱网（**5 Mbps / 2% 丢包 / 30 ms 抖动**）可听」；
//! 本模块就是那句话的可执行版本：按概率丢包、按令牌桶限带宽、给每个包加「固定延迟 + 抖动」。
//!
//! 三条纪律：
//!
//! 1. **确定性**：所有随机数来自自带的小 PRNG（xorshift64*），同一个种子给出同一串结果 ——
//!    弱网验收必须可复现，否则「这次卡顿是不是网络造成的」永远说不清；
//! 2. **纯逻辑**：这里只做「这个包该丢还是该延迟多少」的判定，不碰套接字（收发在 bin 里）；
//! 3. **只丢不改**：不重排、不改写字节 —— 注入器不该在链路上再当一次实现者。
//!    乱序由「抖动的随机性」自然产生（延迟不同的包会换序到达）。

use std::time::{Duration, Instant};

/// 注入参数（百分比一律用「×100」的整数表达，避免浮点让结果不可复现）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetemConfig {
    /// 丢包率（百分比 ×100：`200` = 2%）。
    pub loss_pct_x100: u16,
    /// 单向固定延迟（ms）。
    pub delay_ms: u32,
    /// 抖动幅度（ms）：实际延迟在 `delay_ms ± jitter_ms` 之间均匀取值。
    pub jitter_ms: u32,
    /// 带宽上限（kbps）；`0` = 不限速。
    pub bandwidth_kbps: u32,
}

impl Default for NetemConfig {
    fn default() -> Self {
        // M2 验收口径：5 Mbps / 2% 丢包 / 30 ms 抖动（延迟取抖动的一半作为基准）。
        Self {
            loss_pct_x100: 200,
            delay_ms: 15,
            jitter_ms: 15,
            bandwidth_kbps: 5_000,
        }
    }
}

impl NetemConfig {
    /// 是否完全没有注入（此时可以直接透传）。
    pub const fn is_passthrough(&self) -> bool {
        self.loss_pct_x100 == 0
            && self.delay_ms == 0
            && self.jitter_ms == 0
            && self.bandwidth_kbps == 0
    }

    /// 人类可读描述（报告 / 日志）。
    pub fn describe(&self) -> String {
        let bandwidth = if self.bandwidth_kbps == 0 {
            "不限".to_string()
        } else {
            format!("{} kbps", self.bandwidth_kbps)
        };
        format!(
            "丢包 {}.{:02}% · 延迟 {} ± {} ms · 带宽 {bandwidth}",
            self.loss_pct_x100 / 100,
            self.loss_pct_x100 % 100,
            self.delay_ms,
            self.jitter_ms,
        )
    }
}

/// 确定性小 PRNG（xorshift64*）：只为可复现，不做密码学用途。
#[derive(Debug, Clone)]
pub struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    /// 建一个固定种子的 PRNG（种子为 0 时换成非零常数 —— xorshift 不能从 0 起步）。
    pub const fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    /// 下一个 64 位随机数。
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// `[0, bound)` 内的均匀整数；`bound == 0` 时返回 0。
    pub fn next_below(&mut self, bound: u32) -> u32 {
        if bound == 0 {
            return 0;
        }
        (self.next_u64() % u64::from(bound)) as u32
    }
}

/// 令牌桶（按字节限速）。
#[derive(Debug)]
pub struct TokenBucket {
    rate_bytes_per_sec: u64,
    capacity_bytes: u64,
    tokens: u64,
    last_refill: Instant,
}

impl TokenBucket {
    /// 按 kbps 建桶；`0` → `None`（不限速）。
    pub fn new(bandwidth_kbps: u32, now: Instant) -> Option<Self> {
        if bandwidth_kbps == 0 {
            return None;
        }
        let rate = u64::from(bandwidth_kbps).saturating_mul(1_000) / 8;
        let rate = rate.max(1);
        Some(Self {
            rate_bytes_per_sec: rate,
            capacity_bytes: rate,
            tokens: rate,
            last_refill: now,
        })
    }

    /// 取 `bytes` 个令牌；不够就整包丢弃（返回 `false`）。
    pub fn allow(&mut self, now: Instant, bytes: usize) -> bool {
        let elapsed = now.saturating_duration_since(self.last_refill);
        self.last_refill = now;
        let refill = elapsed
            .as_nanos()
            .saturating_mul(u128::from(self.rate_bytes_per_sec))
            / 1_000_000_000;
        let refill = u64::try_from(refill).unwrap_or(u64::MAX);
        self.tokens = self.tokens.saturating_add(refill).min(self.capacity_bytes);

        let need = u64::try_from(bytes).unwrap_or(u64::MAX);
        if self.tokens >= need {
            self.tokens -= need;
            true
        } else {
            false
        }
    }
}
/// 一个包的处置结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetemOutcome {
    /// 转发，`delay` 之后到达。
    Forward {
        /// 注入的额外延迟。
        delay: Duration,
    },
    /// 丢弃。
    Drop(DropReason),
}

/// 丢弃原因（报告里分开计数：丢包是配置的，掉速是被带宽卡掉的）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    /// 按丢包率丢的。
    Loss,
    /// 令牌桶不够（超过带宽上限）。
    Bandwidth,
}

impl DropReason {
    /// 稳定名字（报告用）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Loss => "loss",
            Self::Bandwidth => "bandwidth",
        }
    }
}

/// 累计统计。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NetemStats {
    /// 观察到的包数。
    pub observed: u64,
    /// 转发出去的包数。
    pub forwarded: u64,
    /// 按丢包率丢掉的包数。
    pub dropped_loss: u64,
    /// 被带宽上限丢掉的包数。
    pub dropped_bandwidth: u64,
    /// 转发出去的字节数。
    pub bytes_forwarded: u64,
}

impl NetemStats {
    /// 实际丢包比例（百分比 ×100）；没有包时返回 0。
    pub fn loss_pct_x100(&self) -> u16 {
        if self.observed == 0 {
            return 0;
        }
        let dropped = self.dropped_loss + self.dropped_bandwidth;
        u16::try_from(dropped.saturating_mul(10_000) / self.observed).unwrap_or(u16::MAX)
    }
}

/// 注入决策器：给定「此刻来了一个 `bytes` 字节的包」，回答「丢还是延迟多少」。
#[derive(Debug)]
pub struct NetemShaper {
    config: NetemConfig,
    rng: DeterministicRng,
    bucket: Option<TokenBucket>,
    stats: NetemStats,
}

impl NetemShaper {
    /// 建决策器（`seed` 固定则整条注入序列可复现）。
    pub fn new(config: NetemConfig, seed: u64, now: Instant) -> Self {
        Self {
            config,
            rng: DeterministicRng::new(seed),
            bucket: TokenBucket::new(config.bandwidth_kbps, now),
            stats: NetemStats::default(),
        }
    }

    /// 当前参数。
    pub const fn config(&self) -> NetemConfig {
        self.config
    }

    /// 累计统计。
    pub const fn stats(&self) -> NetemStats {
        self.stats
    }

    /// 判定一个包。
    pub fn decide(&mut self, now: Instant, bytes: usize) -> NetemOutcome {
        self.stats.observed = self.stats.observed.saturating_add(1);

        if self.config.loss_pct_x100 > 0
            && self.rng.next_below(10_000) < u32::from(self.config.loss_pct_x100)
        {
            self.stats.dropped_loss = self.stats.dropped_loss.saturating_add(1);
            return NetemOutcome::Drop(DropReason::Loss);
        }

        if let Some(bucket) = self.bucket.as_mut()
            && !bucket.allow(now, bytes)
        {
            self.stats.dropped_bandwidth = self.stats.dropped_bandwidth.saturating_add(1);
            return NetemOutcome::Drop(DropReason::Bandwidth);
        }

        // 延迟 = 固定值 + 抖动偏移（在 ±jitter 内均匀取值，结果不低于 0）。
        let offset = if self.config.jitter_ms > 0 {
            let span = self.config.jitter_ms.saturating_mul(2).saturating_add(1);
            i64::from(self.rng.next_below(span)) - i64::from(self.config.jitter_ms)
        } else {
            0
        };
        let delay_ms = (i64::from(self.config.delay_ms) + offset).max(0);
        let delay = Duration::from_millis(u64::try_from(delay_ms).unwrap_or(0));

        self.stats.forwarded = self.stats.forwarded.saturating_add(1);
        self.stats.bytes_forwarded = self
            .stats
            .bytes_forwarded
            .saturating_add(u64::try_from(bytes).unwrap_or(0));
        NetemOutcome::Forward { delay }
    }

    /// 渲染 JSON 报告（`elapsed_secs` 用于算实测吞吐）。
    pub fn report_json(&self, elapsed_secs: f64) -> String {
        let stats = self.stats;
        let throughput_kbps = if elapsed_secs > 0.0 {
            (stats.bytes_forwarded as f64 * 8.0 / elapsed_secs / 1000.0).round()
        } else {
            0.0
        };
        serde_json::json!({
            "tool": "netem-sim",
            "config": {
                "loss_pct_x100": self.config.loss_pct_x100,
                "delay_ms": self.config.delay_ms,
                "jitter_ms": self.config.jitter_ms,
                "bandwidth_kbps": self.config.bandwidth_kbps,
            },
            "stats": {
                "observed": stats.observed,
                "forwarded": stats.forwarded,
                "dropped_loss": stats.dropped_loss,
                "dropped_bandwidth": stats.dropped_bandwidth,
                "bytes_forwarded": stats.bytes_forwarded,
                "observed_loss_pct_x100": stats.loss_pct_x100(),
                "forwarded_throughput_kbps": throughput_kbps,
            },
            "elapsed_secs": elapsed_secs,
        })
        .to_string()
    }
}
/// 弱网中继：客户端连它，它按 [`NetemShaper`] 的判决把字节转发给上游。
///
/// 「客户端 / 上游」由**来源地址**自动区分：第一个非上游来源就是客户端。
/// QUIC 是端到端加密的，中继只搬字节 —— 它既不需要也不应该理解协议内容。
pub struct NetemRelay {
    addr: std::net::SocketAddr,
    shaper: std::sync::Arc<std::sync::Mutex<NetemShaper>>,
    task: tokio::task::JoinHandle<()>,
}

impl NetemRelay {
    /// 起中继：`upstream` 是真正的服务端地址（例如接收端 Engine 的监听地址）。
    pub async fn start(
        upstream: std::net::SocketAddr,
        config: NetemConfig,
        seed: u64,
    ) -> std::io::Result<Self> {
        let socket = std::sync::Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await?);
        let addr = socket.local_addr()?;
        let shaper = std::sync::Arc::new(std::sync::Mutex::new(NetemShaper::new(
            config,
            seed,
            Instant::now(),
        )));
        let worker = std::sync::Arc::clone(&socket);
        let judgement = std::sync::Arc::clone(&shaper);

        let task = tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let mut client: Option<std::net::SocketAddr> = None;
            while let Ok((len, from)) = worker.recv_from(&mut buf).await {
                let to = if from == upstream {
                    match client {
                        Some(client) => client,
                        None => continue,
                    }
                } else {
                    client = Some(from);
                    upstream
                };

                let delay = match judgement.lock() {
                    Ok(mut guard) => match guard.decide(Instant::now(), len) {
                        NetemOutcome::Forward { delay } => delay,
                        NetemOutcome::Drop(_) => continue,
                    },
                    Err(_) => continue,
                };

                let bytes = buf[..len].to_vec();
                let socket = std::sync::Arc::clone(&worker);
                if delay.is_zero() {
                    let _ = socket.send_to(&bytes, to).await;
                } else {
                    // 延迟注入：一个包一个定时任务。弱网下包速率本来就被限住了，
                    // 这条路径上的并发任务数是有界的（几十到几百），不会失控。
                    tokio::spawn(async move {
                        tokio::time::sleep(delay).await;
                        let _ = socket.send_to(&bytes, to).await;
                    });
                }
            }
        });

        Ok(Self { addr, shaper, task })
    }

    /// 中继监听地址（把它交给发起方连接）。
    pub const fn addr(&self) -> std::net::SocketAddr {
        self.addr
    }

    /// 当前统计。
    pub fn stats(&self) -> NetemStats {
        self.shaper
            .lock()
            .map(|guard| guard.stats())
            .unwrap_or_default()
    }

    /// 渲染 JSON 报告。
    pub fn report_json(&self, elapsed_secs: f64) -> String {
        self.shaper
            .lock()
            .map(|guard| guard.report_json(elapsed_secs))
            .unwrap_or_else(|_| "{}".to_string())
    }
}

impl Drop for NetemRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn shaper(config: NetemConfig) -> NetemShaper {
        NetemShaper::new(config, 0x5EED, Instant::now())
    }

    #[test]
    fn passthrough_forwards_everything_with_zero_delay() {
        let mut shaper = shaper(NetemConfig {
            loss_pct_x100: 0,
            delay_ms: 0,
            jitter_ms: 0,
            bandwidth_kbps: 0,
        });
        for _ in 0..100 {
            assert_eq!(
                shaper.decide(Instant::now(), 400),
                NetemOutcome::Forward {
                    delay: Duration::ZERO
                }
            );
        }
        assert_eq!(shaper.stats().forwarded, 100);
        assert_eq!(shaper.stats().dropped_loss, 0);
    }

    #[test]
    fn the_same_seed_reproduces_the_same_sequence() {
        let config = NetemConfig::default();
        let mut first = NetemShaper::new(config, 42, Instant::now());
        let mut second = NetemShaper::new(config, 42, Instant::now());
        let now = Instant::now();
        for _ in 0..500 {
            assert_eq!(first.decide(now, 400), second.decide(now, 400));
        }
        assert_eq!(first.stats(), second.stats());
    }

    #[test]
    fn configured_loss_rate_shows_up_in_the_statistics() {
        let config = NetemConfig {
            loss_pct_x100: 200,
            delay_ms: 0,
            jitter_ms: 0,
            bandwidth_kbps: 0,
        };
        let mut shaper = shaper(config);
        let now = Instant::now();
        for _ in 0..20_000 {
            let _ = shaper.decide(now, 400);
        }
        let observed = shaper.stats().loss_pct_x100();
        assert!(
            (150..=250).contains(&observed),
            "配置 2.00%，实测 {}.{:02}% 偏离过大",
            observed / 100,
            observed % 100
        );
    }

    #[test]
    fn injected_delay_stays_inside_the_configured_band() {
        let config = NetemConfig {
            loss_pct_x100: 0,
            delay_ms: 15,
            jitter_ms: 15,
            bandwidth_kbps: 0,
        };
        let mut shaper = shaper(config);
        let now = Instant::now();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..2_000 {
            match shaper.decide(now, 400) {
                NetemOutcome::Forward { delay } => {
                    assert!(delay <= Duration::from_millis(30), "延迟越界：{delay:?}");
                    seen.insert(delay.as_millis());
                }
                NetemOutcome::Drop(reason) => panic!("不该丢包：{reason:?}"),
            }
        }
        assert!(seen.len() > 10, "抖动必须真的散开，而不是常数延迟");
        assert_eq!(seen.iter().min().copied(), Some(0));
        assert_eq!(seen.iter().max().copied(), Some(30));
    }

    #[test]
    fn bandwidth_limit_caps_the_forwarded_rate() {
        let config = NetemConfig {
            loss_pct_x100: 0,
            delay_ms: 0,
            jitter_ms: 0,
            bandwidth_kbps: 8,
        };
        let start = Instant::now();
        let mut shaper = NetemShaper::new(config, 7, start);
        for step in 0..1_000u64 {
            let now = start + Duration::from_millis(step * 10);
            let _ = shaper.decide(now, 400);
        }
        let stats = shaper.stats();
        assert!(stats.dropped_bandwidth > 900, "限速必须真的丢：{stats:?}");
        assert!(stats.forwarded >= 20, "一秒的桶该放行约 1000 B：{stats:?}");
        let throughput_kbps = stats.bytes_forwarded as f64 * 8.0 / 10.0 / 1000.0;
        assert!(
            throughput_kbps <= 16.0,
            "转发吞吐 {throughput_kbps} kbps 超过上限的两倍"
        );
    }

    #[test]
    fn report_json_carries_config_and_stats() {
        let mut shaper = shaper(NetemConfig::default());
        let now = Instant::now();
        for _ in 0..1_000 {
            let _ = shaper.decide(now, 400);
        }
        let text = shaper.report_json(10.0);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["tool"], "netem-sim");
        assert_eq!(parsed["config"]["loss_pct_x100"], 200);
        assert_eq!(parsed["stats"]["observed"], 1_000);
        assert!(
            parsed["stats"]["observed_loss_pct_x100"]
                .as_u64()
                .unwrap_or(0)
                > 0
        );
        assert!(
            parsed["stats"]["forwarded_throughput_kbps"]
                .as_f64()
                .unwrap_or(0.0)
                > 0.0
        );
    }

    #[test]
    fn default_config_matches_the_m2_acceptance_profile() {
        let config = NetemConfig::default();
        assert_eq!(config.loss_pct_x100, 200, "2% 丢包");
        assert_eq!(config.bandwidth_kbps, 5_000, "5 Mbps");
        assert_eq!(config.jitter_ms, 15, "±15 ms 覆盖 30 ms 抖动");
        assert!(!config.is_passthrough());
    }
}
