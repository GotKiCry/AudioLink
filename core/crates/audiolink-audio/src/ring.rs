//! 跨线程无锁采样环（采集线程 → 编码线程）
//!
//! `docs/02-architecture.md` §4：采集侧只做「抓帧 + 投递」，编码侧才允许分配与阻塞。
//! 本模块把 `ringbuf` 的 SPSC 环形缓冲包成一对窄接口，并**显式统计溢出**：
//! 采集回调里 `push` 写不下时按「丢最新样本」处理并计数 —— 溢出一旦发生就说明消费跟不上，
//! 必须出现在遥测里，而不是悄悄吞掉。
//!
//! 容量口径：调用方给的是**交错样本数**（`f32` 个数）。建议按「若干帧」给，
//! 例如 4 帧 × 20 ms = 3840 帧 = 7 680 个样本。

use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

/// 写入端（采集线程持有）。
pub struct SampleRingWriter {
    producer: HeapProd<f32>,
    dropped_samples: u64,
}

/// 读出端（编码线程持有）。
pub struct SampleRingReader {
    consumer: HeapCons<f32>,
}

impl std::fmt::Debug for SampleRingWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SampleRingWriter")
            .field("buffered", &self.producer.occupied_len())
            .field("capacity", &self.producer.capacity().get())
            .field("dropped_samples", &self.dropped_samples)
            .finish()
    }
}

impl std::fmt::Debug for SampleRingReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SampleRingReader")
            .field("buffered", &self.consumer.occupied_len())
            .finish()
    }
}

/// 建一对 SPSC 端点；`capacity_samples` 为 0 时按 1 处理（容量必须大于 0）。
///
/// 注意：`HeapRb::new` 在**分配失败**时会 panic（不是音频线程路径，属于启动期失败）；
/// 实时路径上的所有操作都不会 panic。
pub fn sample_ring(capacity_samples: usize) -> (SampleRingWriter, SampleRingReader) {
    let (producer, consumer) = HeapRb::<f32>::new(capacity_samples.max(1)).split();
    (
        SampleRingWriter {
            producer,
            dropped_samples: 0,
        },
        SampleRingReader { consumer },
    )
}

impl SampleRingWriter {
    /// 写入交错样本；返回**本次没能写入（被丢弃）的样本数**（0 = 全部写入）。
    pub fn push(&mut self, samples: &[f32]) -> usize {
        let pushed = self.producer.push_slice(samples);
        let dropped = samples.len().saturating_sub(pushed);
        if dropped > 0 {
            self.dropped_samples += dropped as u64;
        }
        dropped
    }

    /// 环内当前待消费样本数。
    pub fn buffered(&self) -> usize {
        self.producer.occupied_len()
    }

    /// 环容量（样本数）。
    pub fn capacity(&self) -> usize {
        self.producer.capacity().get()
    }

    /// 累计丢弃样本数。
    pub fn dropped_samples(&self) -> u64 {
        self.dropped_samples
    }
}

impl SampleRingReader {
    /// 把样本搬进 `dst`（**覆盖** `dst` 原内容），最多搬 `max_samples` 个；返回搬到的样本数。
    ///
    /// 复用 `dst` 的容量：稳态下不分配（`resize` 只在首次会增长容量）。
    pub fn drain_into(&mut self, dst: &mut Vec<f32>, max_samples: usize) -> usize {
        let available = self.consumer.occupied_len();
        let want = available.min(max_samples);
        dst.clear();
        dst.resize(want, 0.0);
        let popped = self.consumer.pop_slice(dst.as_mut_slice());
        dst.truncate(popped);
        popped
    }

    /// 环内当前待消费样本数。
    pub fn buffered(&self) -> usize {
        self.consumer.occupied_len()
    }

    /// 丢弃最旧的 `count` 个样本（链路重建 / 追赶时用），返回实际丢弃数。
    pub fn skip(&mut self, count: usize) -> usize {
        self.consumer.skip(count)
    }

    /// 清空环内所有样本。
    pub fn clear(&mut self) -> usize {
        self.consumer.clear()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 写入与读出保持顺序() {
        let (mut writer, mut reader) = sample_ring(64);
        let input: Vec<f32> = (0..32).map(|value| value as f32).collect();
        assert_eq!(writer.push(&input), 0);
        assert_eq!(reader.buffered(), 32);

        let mut buffer = Vec::with_capacity(32);
        let read = reader.drain_into(&mut buffer, 32);
        assert_eq!(read, 32);
        assert_eq!(buffer, input);
        assert_eq!(reader.buffered(), 0);
    }

    #[test]
    fn 溢出丢最新样本并计数() {
        let (mut writer, _reader) = sample_ring(16);
        let input = [1.0f32; 20];
        let dropped = writer.push(&input);
        assert_eq!(dropped, 4);
        assert_eq!(writer.dropped_samples(), 4);
        assert_eq!(writer.buffered(), 16);
        assert_eq!(writer.capacity(), 16);
    }

    #[test]
    fn 可以跳过最旧样本() {
        let (mut writer, mut reader) = sample_ring(16);
        writer.push(&(0..10).map(|value| value as f32).collect::<Vec<_>>());
        assert_eq!(reader.skip(4), 4);
        let mut buffer = Vec::new();
        let read = reader.drain_into(&mut buffer, 16);
        assert_eq!(read, 6);
        assert_eq!(buffer.first().copied(), Some(4.0));
    }

    #[test]
    fn 跨线程搬运完整无损() {
        let (mut writer, mut reader) = sample_ring(4096);
        let total = 20_000usize;
        let producer = std::thread::spawn(move || {
            let mut sent = 0usize;
            while sent < total {
                let take = 256.min(total - sent);
                let chunk: Vec<f32> = (0..take).map(|i| ((sent + i) % 97) as f32).collect();
                let dropped = writer.push(&chunk);
                sent += chunk.len() - dropped;
                if dropped > 0 {
                    std::thread::sleep(std::time::Duration::from_micros(50));
                }
            }
        });
        let mut received = Vec::with_capacity(total);
        let mut buffer = Vec::with_capacity(512);
        while received.len() < total {
            let read = reader.drain_into(&mut buffer, 512);
            received.extend_from_slice(&buffer);
            if read == 0 {
                std::thread::sleep(std::time::Duration::from_micros(50));
            }
        }
        producer.join().unwrap();
        assert_eq!(received.len(), total);
        for (index, value) in received.iter().enumerate() {
            assert_eq!(*value, (index % 97) as f32, "索引 {index} 处数据错位");
        }
    }
}
