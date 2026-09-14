//! 分位数统计：延迟分解、抖动、包间隔等**离线/遥测**口径的统一算法。
//!
//! 设计取舍：
//! - **固定容量**（环形覆盖最旧样本）：8 h soak 也不会把内存吃光；
//! - [`SampleStats::push`] 是 O(1) 且不分配，可被音频线程调用；
//! - [`SampleStats::summary`] 会复制一份排序（允许分配）—— 只许在遥测/工具路径调用，不许在音频回调里调用。

/// 一个指标（如「编码耗时 μs」）的样本集合。
#[derive(Debug, Clone)]
pub struct SampleStats {
    samples: Vec<u32>,
    /// 显式记下容量：**不能**用 `Vec::capacity()` 判断是否写满 —— `Vec::with_capacity(n)`
    /// 只保证「至少 n」，分配器给多了环就永远不覆盖最旧样本，8 h soak 会把内存吃光。
    capacity: usize,
    next: usize,
    full: bool,
}

/// 分位数汇总（单位与喂进来的样本一致，通常是微秒）。
///
/// 注意：含 `mean: f64`，因此只实现 `PartialEq`（不实现 `Eq`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    /// 样本数。
    pub count: usize,
    /// 最小值。
    pub min: u32,
    /// 中位数。
    pub p50: u32,
    /// 95 分位。
    pub p95: u32,
    /// 99 分位。
    pub p99: u32,
    /// 最大值。
    pub max: u32,
    /// 算术平均（μs 口径，保留小数）。
    pub mean: f64,
}

impl SampleStats {
    /// 建一个最多保留 `capacity` 个样本的统计器（容量 0 会被夹到 1）。
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            samples: Vec::with_capacity(capacity),
            capacity,
            next: 0,
            full: false,
        }
    }

    /// 记录一个样本（O(1)，不分配；超出容量时覆盖最旧样本）。
    pub fn push(&mut self, value: u32) {
        if self.full {
            if let Some(slot) = self.samples.get_mut(self.next) {
                *slot = value;
            }
            self.next += 1;
            if self.next >= self.capacity {
                self.next = 0;
            }
        } else {
            self.samples.push(value);
            if self.samples.len() >= self.capacity {
                self.full = true;
                self.next = 0;
            }
        }
    }

    /// 当前样本数。
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// 是否还没有样本。
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// 容量（构造时声明值，不是 `Vec` 的分配容量）。
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// 清空。
    pub fn clear(&mut self) {
        self.samples.clear();
        self.next = 0;
        self.full = false;
    }

    /// 汇总（复制 + 排序，允许分配；**非实时路径**）。
    pub fn summary(&self) -> Option<Summary> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        let count = sorted.len();
        let total: u64 = sorted.iter().map(|value| u64::from(*value)).sum();
        Some(Summary {
            count,
            min: percentile_of_sorted(&sorted, 0.0),
            p50: percentile_of_sorted(&sorted, 50.0),
            p95: percentile_of_sorted(&sorted, 95.0),
            p99: percentile_of_sorted(&sorted, 99.0),
            max: percentile_of_sorted(&sorted, 100.0),
            mean: if count == 0 {
                0.0
            } else {
                total as f64 / count as f64
            },
        })
    }
}

impl Default for SampleStats {
    fn default() -> Self {
        Self::new(4096)
    }
}

/// 最近秩法（nearest-rank）取分位数：`p = 0` 取最小、`p = 100` 取最大。
fn percentile_of_sorted(sorted: &[u32], p: f64) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    let clamped = p.clamp(0.0, 100.0);
    let rank = (clamped / 100.0 * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted.get(index).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 空统计器没有汇总() {
        let stats = SampleStats::new(8);
        assert!(stats.is_empty());
        assert!(stats.summary().is_none());
    }

    #[test]
    fn 最近秩分位数() {
        // 1..=100
        let mut stats = SampleStats::new(100);
        for value in 1..=100u32 {
            stats.push(value);
        }
        let summary = stats.summary().unwrap();
        assert_eq!(summary.count, 100);
        assert_eq!(summary.min, 1);
        assert_eq!(summary.p50, 50);
        assert_eq!(summary.p95, 95);
        assert_eq!(summary.p99, 99);
        assert_eq!(summary.max, 100);
        assert!((summary.mean - 50.5).abs() < 1e-9);
    }

    #[test]
    fn 容量满后覆盖最旧样本() {
        let mut stats = SampleStats::new(4);
        for value in [10u32, 20, 30, 40, 50, 60] {
            stats.push(value);
        }
        assert_eq!(stats.len(), 4);
        let summary = stats.summary().unwrap();
        // 保留的是 30/40/50/60（最旧的 10、20 被覆盖）
        assert_eq!(summary.min, 30);
        assert_eq!(summary.max, 60);
    }

    #[test]
    fn 单样本各分位相同() {
        let mut stats = SampleStats::new(4);
        stats.push(7);
        let summary = stats.summary().unwrap();
        assert_eq!(
            (
                summary.min,
                summary.p50,
                summary.p95,
                summary.p99,
                summary.max
            ),
            (7, 7, 7, 7, 7)
        );
    }

    /// 长跑护栏：声明容量必须被真正遵守。
    ///
    /// `Vec::with_capacity(n)` 只保证「至少 n」，早先的实现用 `Vec::capacity()` 判断是否写满
    /// → 分配器给多了就**永不覆盖最旧样本**，8 h soak 会把内存涨爆。这里用远超容量的写入压住它。
    #[test]
    fn 长跑时样本数不超过声明容量() {
        let mut stats = SampleStats::new(64);
        assert_eq!(stats.capacity(), 64);
        for value in 0..100_000u32 {
            stats.push(value);
            assert!(
                stats.len() <= 64,
                "样本数 {} 超过声明容量 64（第 {value} 次 push）",
                stats.len()
            );
        }
        assert_eq!(stats.len(), 64);
        let summary = stats.summary().unwrap();
        assert_eq!(summary.count, 64);
        assert_eq!(summary.max, 99_999, "最后写入的样本必须在集合里");
    }
}
