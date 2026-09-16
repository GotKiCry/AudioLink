//! audiolink-tools —— 开发工具的可复用逻辑（纯函数 / 纯数据，便于单测）
//!
//! 规格：`docs/03-protocol.md`（§6 时钟同步、§3 报文格式）、`docs/02-architecture.md`
//!
//! - [`parse_hex_bytes`]：hex 文本 → 字节串（`alp2-dump` 用，容忍文档里的 `|` / `0x` / `<占位>` 写法）
//! - [`pcap`]：pcap / pcapng 抓包 → 逐包 UDP 载荷（`alp2-dump --pcap` 的输入层）
//! - [`netem`]：弱网注入内核（丢包 / 延迟 / 抖动 / 限速，`netem-sim` 与 `soak-runner --netem-*` 共用）
//! - [`soak`]：长跑采样的异常判定与 JSON 报告（`soak-runner`）
//! - [`ClockSamples`]：§6 的采样统计（最小 RTT 过滤 + 中位数 + 质量分级），`latency-probe` 用
//!
//! 注：§6 的稳态算法（200 样本滑动窗口 + 线性回归漂移估计）属 M3，本 crate 只做 M0/M1 需要的部分。

#![deny(unsafe_code)] // 必须使用 unsafe 的 crate（如 FFI 绑定）在文件顶部显式 #[allow] 并注明理由
#![deny(clippy::unwrap_used, clippy::expect_used)] // 实时路径禁止 panic；确需处用 #[allow] 并注明理由
#![deny(clippy::panic)]

pub mod license;
pub mod netem;
pub mod pcap;
pub mod soak;
pub mod syncmeasure;

use std::fmt;

// ---------------------------------------------------------------------------
// hex 文本解析（alp2-dump）
// ---------------------------------------------------------------------------

/// hex 文本解析失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HexParseError {
    /// 出现非 hex 字符。
    BadDigit {
        /// 该字符。
        ch: char,
        /// 第几列（1 起，按字符计）。
        column: usize,
    },
    /// hex 数字个数为奇数，无法组成整字节。
    OddDigits(usize),
    /// 一个字节都没解析出来。
    Empty,
}

impl fmt::Display for HexParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadDigit { ch, column } => write!(f, "第 {column} 列出现非 hex 字符 {ch:?}"),
            Self::OddDigits(count) => write!(f, "hex 数字个数为奇数（{count}）"),
            Self::Empty => write!(f, "没有解析出任何字节"),
        }
    }
}

impl std::error::Error for HexParseError {}

/// 把 hex 文本解析成字节串。
///
/// 容忍下列写法（都来自项目文档 / 抓包工具的常见输出）：
/// - `0x` / `0X` 前缀：`0x02 0x01`、`0201`、`0x0201` 等价；
/// - 分隔符：`|`、`-`、`:`、`,`、任意空白；
/// - 行内注释：`//` 或 `#` 起的剩余内容忽略；
/// - 占位符：`<...>`（例如 `docs/03-protocol.md` §12 的 `<10 字节 postcard 载荷>`）整段忽略。
pub fn parse_hex_bytes(text: &str) -> Result<Vec<u8>, HexParseError> {
    let mut digits: Vec<u8> = Vec::new();
    let mut placeholder = false;
    let mut chars = text.chars().enumerate().peekable();

    while let Some((index, ch)) = chars.next() {
        if placeholder {
            if ch == '>' {
                placeholder = false;
            }
            continue;
        }
        match ch {
            '<' => placeholder = true,
            '#' => break,
            '/' if matches!(chars.peek(), Some((_, '/'))) => break,
            c if c.is_ascii_hexdigit() => {
                // `0x` 前缀：把 `0` 与 `x` 一起吞掉，避免多算一位
                if c == '0' {
                    let followed_by_x =
                        matches!(chars.peek(), Some((_, next)) if *next == 'x' || *next == 'X');
                    if followed_by_x {
                        chars.next();
                        continue;
                    }
                }
                match c.to_digit(16) {
                    Some(value) => digits.push(value as u8),
                    None => {
                        return Err(HexParseError::BadDigit {
                            ch: c,
                            column: index + 1,
                        });
                    }
                }
            }
            c if c.is_whitespace() || matches!(c, '|' | '-' | ':' | ',') => {}
            other => {
                return Err(HexParseError::BadDigit {
                    ch: other,
                    column: index + 1,
                });
            }
        }
    }

    if digits.is_empty() {
        return Err(HexParseError::Empty);
    }
    if !digits.len().is_multiple_of(2) {
        return Err(HexParseError::OddDigits(digits.len()));
    }

    let mut out = Vec::with_capacity(digits.len() / 2);
    for pair in digits.chunks(2) {
        let high = pair.first().copied().unwrap_or(0);
        let low = pair.get(1).copied().unwrap_or(0);
        out.push((high << 4) | low);
    }
    Ok(out)
}

/// 把字节串渲染成 `DE AD BE EF` 形式（超出 `limit` 字节时省略并注明总长）。
pub fn hex_preview(bytes: &[u8], limit: usize) -> String {
    let shown = bytes.len().min(limit);
    let mut out = String::with_capacity(shown * 3 + 24);
    for byte in bytes.iter().take(shown) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("{byte:02X}"));
    }
    if bytes.len() > shown {
        out.push_str(&format!(" … (共 {} B)", bytes.len()));
    }
    if out.is_empty() {
        out.push_str("(空)");
    }
    out
}

// ---------------------------------------------------------------------------
// §6 时钟同步采样统计（latency-probe）
// ---------------------------------------------------------------------------

/// §6.3「取 RTT 最小的 N 个样本」中的 N。
pub const BEST_RTT_SAMPLES: usize = 8;

/// 单次时钟探测的原始结果（单位 µs）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    /// 往返时延：`(t4 - t1) - (t3 - t2)`（§6）。
    pub rtt_us: u64,
    /// 时钟偏移估计：`((t2 - t1) + (t3 - t4)) / 2`（§6），对端 − 本地。
    pub offset_us: i64,
}

/// §6 的采样窗口与统计口径：RTT 最小样本过滤 + 中位数（抗异常值）。
#[derive(Debug, Clone, Default)]
pub struct ClockSamples {
    samples: Vec<Sample>,
}

impl ClockSamples {
    /// 空窗口。
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一个样本。
    pub fn push(&mut self, sample: Sample) {
        self.samples.push(sample);
    }

    /// 样本数。
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// 全部样本（按到达顺序）。
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    /// RTT 的最近秩百分位（`pct` ∈ 0..=100）；空窗口 → `None`。
    pub fn rtt_percentile_us(&self, pct: u8) -> Option<u64> {
        let mut rtts: Vec<u64> = self.samples.iter().map(|s| s.rtt_us).collect();
        if rtts.is_empty() {
            return None;
        }
        rtts.sort_unstable();
        let len = rtts.len();
        let rank = (usize::from(pct.min(100)) * len).div_ceil(100).max(1);
        rtts.get(rank - 1).copied()
    }

    /// 相邻样本 RTT 变化绝对值的百分位（抖动代理指标，供缓冲深度决策参考）。
    pub fn rtt_delta_percentile_us(&self, pct: u8) -> Option<u64> {
        if self.samples.len() < 2 {
            return None;
        }
        let mut deltas: Vec<u64> = self
            .samples
            .windows(2)
            .map(|pair| match (pair.first(), pair.get(1)) {
                (Some(a), Some(b)) => a.rtt_us.abs_diff(b.rtt_us),
                _ => 0,
            })
            .collect();
        if deltas.is_empty() {
            return None;
        }
        deltas.sort_unstable();
        let len = deltas.len();
        let rank = (usize::from(pct.min(100)) * len).div_ceil(100).max(1);
        deltas.get(rank - 1).copied()
    }

    /// §6.3：RTT 最小的至多 [`BEST_RTT_SAMPLES`] 个样本（按 RTT 升序）。
    fn best_samples(&self) -> Vec<Sample> {
        let mut sorted = self.samples.clone();
        sorted.sort_by_key(|s| s.rtt_us);
        sorted.truncate(BEST_RTT_SAMPLES);
        sorted
    }

    /// 被选中样本里最大的 RTT（= 第 8 小的 RTT；样本不足时为最大 RTT）—— 质量分级的代表值。
    pub fn best_rtt_us(&self) -> Option<u64> {
        self.best_samples().iter().map(|s| s.rtt_us).max()
    }

    /// §6.3 的偏移估计：被选中样本 `offset` 的中位数。
    pub fn best_offset_us(&self) -> Option<i64> {
        let mut offsets: Vec<i64> = self.best_samples().iter().map(|s| s.offset_us).collect();
        if offsets.is_empty() {
            return None;
        }
        offsets.sort_unstable();
        let len = offsets.len();
        let mid = len / 2;
        if len.is_multiple_of(2) {
            let a = offsets.get(mid - 1).copied().unwrap_or(0);
            let b = offsets.get(mid).copied().unwrap_or(0);
            Some((a + b) / 2)
        } else {
            offsets.get(mid).copied()
        }
    }

    /// 被选中样本的 offset 极差 —— 收敛指标（§6：稳定后 offset 抖动 ≤ 2 ms）。
    pub fn best_offset_spread_us(&self) -> Option<i64> {
        let best = self.best_samples();
        let min = best.iter().map(|s| s.offset_us).min()?;
        let max = best.iter().map(|s| s.offset_us).max()?;
        Some(max - min)
    }

    /// §6.5 质量分级：`Good` 要求样本 ≥ [`BEST_RTT_SAMPLES`] 且代表 RTT ≤ 5 ms；`Fair` ≤ 20 ms；其余 `Poor`。
    ///
    /// 注：§6.5 只写「RTT ≤ 5 ms」，未指明统计量。本实现用「最小 RTT 过滤后的代表值」，
    /// 与 §6.3 的筛选口径一致 —— 单次高 RTT 抖动不应把长期质量判差。
    pub fn quality(&self) -> Quality {
        match self.best_rtt_us() {
            Some(rtt) if self.samples.len() >= BEST_RTT_SAMPLES && rtt <= 5_000 => Quality::Good,
            Some(rtt) if rtt <= 20_000 => Quality::Fair,
            _ => Quality::Poor,
        }
    }
}

/// §6.5 的时钟质量分级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// RTT ≤ 5 ms 且样本 ≥ 8。
    Good,
    /// RTT ≤ 20 ms。
    Fair,
    /// 其它。
    Poor,
}

impl Quality {
    /// 短标签（CLI / 遥测用）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Good => "Good",
            Self::Fair => "Fair",
            Self::Poor => "Poor",
        }
    }
}

#[cfg(test)]
mod tests {
    // 测试代码：断言失败即测试失败，不属实时路径纪律（docs/02-architecture.md §4 铁律 1）
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn parses_doc_vector_lines() {
        // docs/03-protocol.md §12 示例 1 的写法（含分隔符与行内注释）
        let line = "02 01 00 00 | 01 00 00 00 | 2A 00 00 00 | C0 03 00 00 | 88 77 66 55 44 33 22 11 | DE AD BE EF";
        let bytes = parse_hex_bytes(line).unwrap();
        assert_eq!(bytes.len(), 28);
        assert_eq!(&bytes[..4], &[0x02, 0x01, 0x00, 0x00]);
        assert_eq!(&bytes[24..], &[0xDE, 0xAD, 0xBE, 0xEF]);

        // 向量 3 的占位写法：`<10 字节 postcard 载荷>` 必须整段忽略
        let with_placeholder = "02 01 00 00 | 0A 00 00 00 | 01 00 00 00 | <10 字节 postcard 载荷>";
        let bytes = parse_hex_bytes(with_placeholder).unwrap();
        assert_eq!(
            bytes,
            vec![
                0x02, 0x01, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00
            ]
        );

        // `0x` 前缀 / 逗号分隔 / 注释
        let prefixed = "0x02, 0x01, 0x03, 0x07   // 4 字节";
        assert_eq!(
            parse_hex_bytes(prefixed).unwrap(),
            vec![0x02, 0x01, 0x03, 0x07]
        );
        assert_eq!(parse_hex_bytes("DE AD BE EF # 注释").unwrap().len(), 4);
    }

    #[test]
    fn hex_parse_rejects_bad_input() {
        assert_eq!(parse_hex_bytes(""), Err(HexParseError::Empty));
        assert_eq!(parse_hex_bytes("   // 只有注释"), Err(HexParseError::Empty));
        assert_eq!(parse_hex_bytes("0 2 0"), Err(HexParseError::OddDigits(3)));
        assert!(matches!(
            parse_hex_bytes("02 01 ZZ"),
            Err(HexParseError::BadDigit { ch: 'Z', .. })
        ));
    }

    #[test]
    fn hex_preview_truncates() {
        assert_eq!(hex_preview(&[0xDE, 0xAD], 8), "DE AD");
        assert_eq!(hex_preview(&[], 8), "(空)");
        assert_eq!(hex_preview(&[1, 2, 3, 4, 5], 3), "01 02 03 … (共 5 B)");
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let mut samples = ClockSamples::new();
        for rtt in [10_u64, 20, 30, 40] {
            samples.push(Sample {
                rtt_us: rtt,
                offset_us: 0,
            });
        }
        assert_eq!(samples.rtt_percentile_us(0), Some(10));
        assert_eq!(samples.rtt_percentile_us(50), Some(20));
        assert_eq!(samples.rtt_percentile_us(95), Some(40));
        assert_eq!(samples.rtt_percentile_us(100), Some(40));
        assert_eq!(ClockSamples::new().rtt_percentile_us(50), None);
    }

    #[test]
    fn best_offset_ignores_high_rtt_outliers() {
        let mut samples = ClockSamples::new();
        for i in 0..8_i64 {
            samples.push(Sample {
                rtt_us: 1_000 + i as u64,
                offset_us: 500 + i,
            });
        }
        // 两个高 RTT 异常样本（丢包重传 / 调度抖动）：offset 被污染得很厉害
        samples.push(Sample {
            rtt_us: 90_000,
            offset_us: 5_000,
        });
        samples.push(Sample {
            rtt_us: 120_000,
            offset_us: 9_000,
        });

        assert_eq!(samples.best_rtt_us(), Some(1_007), "取第 8 小的 RTT");
        assert_eq!(
            samples.best_offset_us(),
            Some(503),
            "中位数只由低 RTT 样本决定"
        );
        assert_eq!(samples.best_offset_spread_us(), Some(7));
        assert_eq!(samples.quality(), Quality::Good);
    }

    #[test]
    fn quality_thresholds_follow_spec() {
        // 样本不足 8：即使 RTT 很小也不能判 Good（§6.5）
        let mut few = ClockSamples::new();
        for _ in 0..6 {
            few.push(Sample {
                rtt_us: 2_000,
                offset_us: 0,
            });
        }
        assert_eq!(few.quality(), Quality::Fair);

        // 8 个样本、RTT ≤ 5 ms → Good
        let mut good = ClockSamples::new();
        for _ in 0..8 {
            good.push(Sample {
                rtt_us: 4_900,
                offset_us: 0,
            });
        }
        assert_eq!(good.quality(), Quality::Good);

        // RTT > 20 ms → Poor
        let mut poor = ClockSamples::new();
        for _ in 0..20 {
            poor.push(Sample {
                rtt_us: 30_000,
                offset_us: 0,
            });
        }
        assert_eq!(poor.quality(), Quality::Poor);
    }

    #[test]
    fn rtt_delta_tracks_jitter() {
        let mut samples = ClockSamples::new();
        for rtt in [1_000_u64, 1_100, 1_200, 5_000] {
            samples.push(Sample {
                rtt_us: rtt,
                offset_us: 0,
            });
        }
        // 相邻差：100, 100, 3800 → P50 = 100，P95 = 3800
        assert_eq!(samples.rtt_delta_percentile_us(50), Some(100));
        assert_eq!(samples.rtt_delta_percentile_us(95), Some(3_800));
        assert_eq!(ClockSamples::new().rtt_delta_percentile_us(50), None);
    }
}
