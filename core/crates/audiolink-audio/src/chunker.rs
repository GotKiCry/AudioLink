//! 组帧器：把任意长度的交错样本流切成固定长度的音频帧（默认 20 ms = 960 样本/声道）。
//!
//! `docs/02-architecture.md` §4 的 `audio::chunker`。设计要点：
//! - **按槽（slot）环形**：容量是整数帧，写指针只在帧边界回绕 → 出帧永远是**一整块连续切片**（零拷贝）；
//! - 稳态零分配（构造时一次性分配）；
//! - 队列满时**丢最旧帧**（策略来自 §4：编码队列「满则丢最旧帧」），并计数供遥测；
//! - 不 panic：内部访问一律 `get` / 模式匹配，越界只会退化成为空结果。

use crate::error::AudioError;
use crate::format::{CHANNELS, frame_interleaved, is_supported_frame_ms};

/// 组帧结果计数器。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChunkerStats {
    /// 累计产出的完整帧数。
    pub frames_emitted: u64,
    /// 因队列满而被丢弃的最旧帧数（编码跟不上时的直接体现）。
    pub dropped_frames: u64,
    /// 累计吞入的交错样本数。
    pub samples_in: u64,
}

/// 固定帧长的环形组帧器。
#[derive(Debug)]
pub struct FrameChunker {
    slots: Vec<f32>,
    frame_len: usize,
    capacity: usize,
    write_slot: usize,
    write_offset: usize,
    read_slot: usize,
    ready: usize,
    stats: ChunkerStats,
}

impl FrameChunker {
    /// 构造：`frame_ms` 必须是协议允许的帧长（10/20/40/60），`capacity_frames` ≥ 1。
    pub fn new(frame_ms: u32, capacity_frames: usize) -> Result<Self, AudioError> {
        if !is_supported_frame_ms(frame_ms) {
            return Err(AudioError::invalid_config_owned(format!(
                "不支持的帧长 {frame_ms} ms（只允许 10/20/40/60）"
            )));
        }
        let capacity = capacity_frames.max(1);
        let frame_len = frame_interleaved(frame_ms);
        Ok(Self {
            slots: vec![0.0; frame_len * capacity],
            frame_len,
            capacity,
            write_slot: 0,
            write_offset: 0,
            read_slot: 0,
            ready: 0,
            stats: ChunkerStats::default(),
        })
    }

    /// 帧长（每声道样本数）。
    pub fn frame_samples(&self) -> usize {
        self.frame_len / CHANNELS as usize
    }

    /// 一帧的交错样本数。
    pub fn frame_len(&self) -> usize {
        self.frame_len
    }

    /// 队列容量（帧数）。
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// 可立即取出的完整帧数。
    pub fn ready_frames(&self) -> usize {
        self.ready
    }

    /// 统计计数。
    pub fn stats(&self) -> ChunkerStats {
        self.stats
    }

    /// 喂入交错样本；返回**本次因队列满而丢弃的最旧帧数**（0 = 没有丢）。
    ///
    /// 实时路径友好：无分配、无 panic（越界访问退化为提前结束）。
    pub fn push(&mut self, samples: &[f32]) -> u64 {
        let dropped_before = self.stats.dropped_frames;
        self.stats.samples_in += samples.len() as u64;
        let mut consumed = 0usize;
        while consumed < samples.len() {
            // 开始写新帧之前先腾位置：这样任何处于「可读」状态的帧都不会被**部分覆盖**
            // （否则 push 半帧后调用 next_frame 会读到半新半旧的垃圾数据）。
            if self.write_offset == 0 && self.ready == self.capacity {
                self.read_slot += 1;
                if self.read_slot >= self.capacity {
                    self.read_slot = 0;
                }
                self.ready -= 1;
                self.stats.dropped_frames += 1;
            }
            let remaining = self.frame_len - self.write_offset;
            let take = remaining.min(samples.len() - consumed);
            let start = self.write_slot * self.frame_len + self.write_offset;
            match samples.get(consumed..consumed + take) {
                Some(chunk) => match self.slots.get_mut(start..start + take) {
                    Some(dest) => dest.copy_from_slice(chunk),
                    None => return self.stats.dropped_frames - dropped_before,
                },
                None => return self.stats.dropped_frames - dropped_before,
            }
            consumed += take;
            self.write_offset += take;

            if self.write_offset == self.frame_len {
                // 一帧写满 → 入队（腾位置已在上一帧开始时完成）
                self.ready += 1;
                self.write_slot += 1;
                if self.write_slot >= self.capacity {
                    self.write_slot = 0;
                }
                self.write_offset = 0;
            }
        }
        self.stats.dropped_frames - dropped_before
    }

    /// 取下一帧（零拷贝）。不足一帧返回 `None`。
    pub fn next_frame(&mut self) -> Option<&[f32]> {
        if self.ready == 0 {
            return None;
        }
        let start = self.read_slot * self.frame_len;
        let frame = self.slots.get(start..start + self.frame_len)?;
        self.read_slot += 1;
        if self.read_slot >= self.capacity {
            self.read_slot = 0;
        }
        self.ready -= 1;
        self.stats.frames_emitted += 1;
        Some(frame)
    }

    /// 重置读写指针（音频流重建时用；不清零数据，避免无谓的内存写）。
    pub fn reset(&mut self) {
        self.write_slot = 0;
        self.write_offset = 0;
        self.read_slot = 0;
        self.ready = 0;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn 不足一帧时取不出帧() {
        let mut chunker = FrameChunker::new(20, 4).unwrap();
        assert_eq!(chunker.frame_len(), 1_920);
        assert_eq!(chunker.frame_samples(), 960);
        chunker.push(&[0.0; 1_919]);
        assert_eq!(chunker.ready_frames(), 0);
        assert!(chunker.next_frame().is_none());
        chunker.push(&[0.0; 1]);
        assert_eq!(chunker.ready_frames(), 1);
        assert_eq!(chunker.next_frame().map(<[f32]>::len), Some(1_920));
    }

    #[test]
    fn 跨多次_push_的样本能被正确组帧() {
        let mut chunker = FrameChunker::new(20, 4).unwrap();
        for _ in 0..5 {
            chunker.push(&[0.5; 384]); // 5 × 384 = 1920 = 正好一帧
        }
        assert_eq!(chunker.ready_frames(), 1);
        let frame = chunker.next_frame().unwrap();
        assert!(
            frame
                .iter()
                .all(|value| (*value - 0.5).abs() < f32::EPSILON)
        );
        assert_eq!(chunker.stats().frames_emitted, 1);
        assert_eq!(chunker.stats().samples_in, 1_920);
    }

    #[test]
    fn 帧内容不串槽() {
        let mut chunker = FrameChunker::new(10, 2).unwrap(); // 960 交错样本/帧
        chunker.push(&[1.0; 960]);
        chunker.push(&[2.0; 960]);
        assert_eq!(chunker.ready_frames(), 2);
        assert!(chunker.next_frame().unwrap().iter().all(|v| *v == 1.0));
        assert!(chunker.next_frame().unwrap().iter().all(|v| *v == 2.0));
    }

    #[test]
    fn 队列满时丢弃最旧帧并计数() {
        let mut chunker = FrameChunker::new(10, 2).unwrap();
        chunker.push(&[1.0; 960]);
        chunker.push(&[2.0; 960]);
        let dropped = chunker.push(&[3.0; 960]);
        assert_eq!(dropped, 1);
        assert_eq!(chunker.stats().dropped_frames, 1);
        assert_eq!(chunker.ready_frames(), 2);
        // 丢的是最旧的 1.0
        assert!(chunker.next_frame().unwrap().iter().all(|v| *v == 2.0));
        assert!(chunker.next_frame().unwrap().iter().all(|v| *v == 3.0));
    }

    #[test]
    fn 环形回绕后仍连续正确() {
        let mut chunker = FrameChunker::new(10, 3).unwrap();
        for round in 0..20u32 {
            chunker.push(&[round as f32; 960]);
            let frame = chunker.next_frame().unwrap();
            assert!(frame.iter().all(|v| *v == round as f32), "round {round}");
        }
        assert_eq!(chunker.stats().dropped_frames, 0);
        assert_eq!(chunker.stats().frames_emitted, 20);
    }

    #[test]
    fn 半帧写入不会污染可读帧() {
        // 容量 1：写满一帧后，再写半帧时必须先把旧的丢干净，绝不能出现「半新半旧」的可读帧
        let mut chunker = FrameChunker::new(10, 1).unwrap();
        chunker.push(&[1.0; 960]);
        chunker.push(&[2.0; 480]);
        assert_eq!(chunker.ready_frames(), 0);
        assert!(chunker.next_frame().is_none());
        assert_eq!(chunker.stats().dropped_frames, 1);
        chunker.push(&[2.0; 480]);
        let frame = chunker.next_frame().unwrap();
        assert!(frame.iter().all(|v| *v == 2.0));
    }

    #[test]
    fn 非法帧长被拒绝() {
        let err = FrameChunker::new(15, 4).unwrap_err();
        assert!(err.context().contains("15"), "{}", err.context());
    }
}
