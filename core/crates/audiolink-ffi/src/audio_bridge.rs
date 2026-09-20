//! Kotlin ↔ 内核的音频出入口桥接（`docs/11-m1-contract.md` §7、`docs/02-architecture.md` §1 第 3 条）
//!
//! # 为什么是「回调」而不是「内核托管播放线程」
//!
//! 架构 §1 第 3 条冻结了一件事：`AudioTrack` 的生命周期与线程亲和性**必须留在 Kotlin 侧**，
//! 内核只输出「PCM + 目标播放时刻 + 期望速率」。落地方式就是本模块：
//!
//! ```text
//! 播放方向（M1 Android 的主路径）
//!   网络 → engine 解码 → PlayoutSink::write ──► PcmFeed.feedPcm ──► AudioLinkService.feedPcm
//!                                                                  └─► PcmRingBuffer ─► AudioTrack
//! 发送方向（M1 Android 不用；内录推流属后续里程碑）
//!   CaptureSource::read ◄── PcmPull.readPcm ◄── Kotlin（AudioRecord / AudioPlaybackCapture）
//! ```
//!
//! 两侧都实现了 `audiolink_audio` 的平台抽象（[`PlayoutSink`] / [`CaptureSource`]），
//! 所以 engine 侧完全看不出音频是从 Kotlin 来的 —— 工厂只在目标线程里被调用
//! （`audiolink-engine::runtime` 的「谁用谁建」纪律）。
//!
//! # 实时路径的代价（**已知成本，别当没这回事**）
//!
//! UniFFI 0.29 的 Kotlin 绑定里 `Vec<f32>` 映射为 `List<Float>`（不是 `FloatArray`）：
//! 每帧 20 ms（1920 个交错样本）会在 Kotlin 侧产生 1920 个装箱对象 ≈ 30 KB/帧、1.5 MB/s 垃圾。
//! 这是本桥接**已知**的性能代价，选它的理由是**唯一可编译可验收的路径**：
//! 要拿到零装箱的 `ByteArray`，只能用 UDL 模式的 `bytes` 类型（proc-macro 模式没有对应 FFI 类型），
//! 那会引入 UDL + build.rs 的第二套脚手架生成链路。
//! M1 无真机（`adb devices` 为空），**抖动影响未验证**；真机接入后若 GC 抖动进不了预算，
//! 逃生通道是：把 `PcmFeed` 换成 UDL `bytes`（`ByteArray` + `ByteBuffer.asFloatBuffer()` 零拷贝视图）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use audiolink_audio::{
    AudioError, CHANNELS, CaptureSource, CaptureStats, CapturedPacket, DEFAULT_FRAME_MS,
    DeviceFormat, PlayoutSink, PlayoutStats, SAMPLE_RATE_HZ, SampleFormat,
};

/// Kotlin 实现的 PCM 灌入口（`AudioLinkService.feedPcm` 就是它）。
///
/// 契约（Kotlin 侧必须遵守，否则声音会错位）：
/// 1. `samples` 是 **48 kHz / f32 / 2ch 交错**样本，长度恒为 `frames * 2`；
/// 2. **从任意线程调用**：内核的播放线程是 Rust 线程（`audiolink-playout`，不是 UI 线程、也不是
///    Kotlin 的主线程），实现必须线程安全（落到 `PcmRingBuffer` 上时要自己加锁）；
/// 3. 返回值 = 实际接收的帧数，仅供诊断；`< frames` 表示 Kotlin 侧环满。
#[uniffi::export(callback_interface)]
pub trait PcmFeed: Send + Sync {
    /// 交付一段 PCM。
    fn feed_pcm(&self, samples: Vec<f32>, frames: i32) -> i32;
}

/// Kotlin 实现的 PCM 拉取口（发送方向用，M1 Android 不用）。
///
/// 契约：
/// 1. 返回 48 kHz / f32 / 2ch 交错样本，长度 **偶数**、且 ≤ `max_samples`；
/// 2. 返回空 = 暂无数据（不要忙等返回 0，那会在采集线程里变成热循环）；
/// 3. 同样可能从 Rust 音频线程调用，实现必须线程安全。
#[uniffi::export(callback_interface)]
pub trait PcmPull: Send + Sync {
    /// 取最多 `max_samples` 个交错样本。
    fn read_pcm(&self, max_samples: i32) -> Vec<f32>;
}

/// 统一内部格式（48 kHz / f32 / 2ch）—— engine 的「零重采样」硬闸会拿它做断言。
fn unified_format() -> DeviceFormat {
    DeviceFormat::new(SAMPLE_RATE_HZ, CHANNELS, SampleFormat::F32)
}

/// 播放输出 → Kotlin 回调。
pub(crate) struct KotlinPlayoutSink {
    feed: Arc<dyn PcmFeed>,
    stats: PlayoutStats,
}

impl KotlinPlayoutSink {
    pub(crate) fn new(feed: Arc<dyn PcmFeed>) -> Self {
        Self {
            feed,
            stats: PlayoutStats::default(),
        }
    }
}

impl PlayoutSink for KotlinPlayoutSink {
    fn device_format(&self) -> DeviceFormat {
        unified_format()
    }

    fn requested_buffer_ms(&self) -> u32 {
        DEFAULT_FRAME_MS
    }

    fn effective_buffer_ms(&self) -> u32 {
        // 实际缓冲由 Kotlin 侧的 `PcmRingBuffer` + `AudioTrack` 决定，这里只报内核一帧的长度。
        DEFAULT_FRAME_MS
    }

    fn buffered_frames(&mut self) -> u32 {
        // 内核看不见 Kotlin 的环深度。**刻意不编一个数字**：要么真知道，要么说不知道。
        0
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), AudioError> {
        let frames = samples.len() / CHANNELS as usize;
        if frames == 0 {
            return Ok(());
        }
        self.stats.writes += 1;

        let accepted = self.feed.feed_pcm(samples.to_vec(), frames as i32);
        if accepted < 0 {
            // Kotlin 侧明确报错（例如播放器已释放）→ 记为写入失败；`write_errors` 是唯一能表达它的字段。
            self.stats.write_errors += 1;
            return Ok(());
        }
        let accepted = accepted as usize;
        // 环满导致的部分接收：`PlayoutStats` 没有「丢弃」字段，就近记为一次写入异常，
        // 免得这个数字**无声消失**（engine 自己的 `late_drops` 只统计网络队列，不覆盖这一段）。
        if accepted < frames {
            self.stats.write_errors += 1;
        }
        self.stats.frames_written += accepted as u64;
        Ok(())
    }

    fn stats(&self) -> PlayoutStats {
        self.stats
    }

    fn stop(&mut self) {
        // 刻意什么都不做：`AudioTrack` 的生命周期归 Kotlin（架构 §1 第 3 条），
        // 内核停掉一路流不该顺手释放别人的播放设备。
    }

    fn backend_name(&self) -> &'static str {
        "kotlin-callback"
    }
}

/// Kotlin 回调 → 采集源。
pub(crate) struct KotlinCaptureSource {
    pull: Arc<dyn PcmPull>,
    stats: CaptureStats,
    /// 一次拉取的上限（交错样本数）= 一帧。
    max_samples: usize,
}

impl KotlinCaptureSource {
    pub(crate) fn new(pull: Arc<dyn PcmPull>) -> Self {
        Self {
            pull,
            stats: CaptureStats::default(),
            max_samples: audiolink_audio::DEFAULT_FRAME_INTERLEAVED,
        }
    }
}

impl CaptureSource for KotlinCaptureSource {
    fn device_format(&self) -> DeviceFormat {
        unified_format()
    }

    fn requested_buffer_ms(&self) -> u32 {
        DEFAULT_FRAME_MS
    }

    fn effective_buffer_ms(&self) -> u32 {
        DEFAULT_FRAME_MS
    }

    fn read(
        &mut self,
        dst: &mut Vec<f32>,
        timeout: Duration,
    ) -> Result<Option<CapturedPacket>, AudioError> {
        dst.clear();

        let samples = self.pull.read_pcm(self.max_samples as i32);
        // 奇数长度说明 Kotlin 侧给的字节数不是整数帧：截到帧边界，绝不把半帧塞给编码器。
        let usable = samples.len() - samples.len() % CHANNELS as usize;
        if usable == 0 {
            self.stats.read_timeouts += 1;
            // 「回调立即返回空」会把采集线程变成热循环，所以这里给一个**下限**睡眠。
            // 正常情况下 Kotlin 侧自己会阻塞到有数据，这个睡眠只是保险丝。
            let nap = timeout.min(Duration::from_millis(5));
            if !nap.is_zero() {
                std::thread::sleep(nap);
            }
            return Ok(None);
        }

        dst.extend_from_slice(&samples[..usable]);
        let frames = usable / CHANNELS as usize;
        self.stats.packets += 1;
        self.stats.frames += frames as u64;

        Ok(Some(CapturedPacket {
            frames,
            // 设备时间戳要平台侧给；Kotlin 不提供时保持 `None`（不许编 0 假装有）。
            device_timestamp_100ns: None,
            read_at: Instant::now(),
            silent: false,
            discontinuity: false,
        }))
    }

    fn stats(&self) -> CaptureStats {
        self.stats
    }

    fn stop(&mut self) {
        // 同播放侧：采集设备的生命周期归 Kotlin。
    }

    fn backend_name(&self) -> &'static str {
        "kotlin-callback"
    }
}

/// 构造 engine 需要的播放工厂（每路流新建一个 sink，共享同一个 Kotlin 回调）。
pub(crate) fn playout_factory(feed: Arc<dyn PcmFeed>) -> audiolink_engine::PlayoutFactory {
    Arc::new(move || Ok(Box::new(KotlinPlayoutSink::new(Arc::clone(&feed)))))
}

/// 构造 engine 需要的采集工厂。
pub(crate) fn capture_factory(pull: Arc<dyn PcmPull>) -> audiolink_engine::CaptureFactory {
    Arc::new(move || Ok(Box::new(KotlinCaptureSource::new(Arc::clone(&pull)))))
}

/// 把 `Box<dyn PcmFeed>`（UniFFI 传入的外来实现）折成可克隆的 `Arc`。
pub(crate) fn share_feed(feed: Box<dyn PcmFeed>) -> Arc<dyn PcmFeed> {
    Arc::from(feed)
}

/// 采集方向同理。
pub(crate) fn share_pull(pull: Box<dyn PcmPull>) -> Arc<dyn PcmPull> {
    Arc::from(pull)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use std::sync::Mutex;

    /// 用 Rust 实现一遍回调接口 —— 证明桥接两端在**无 Kotlin** 的情况下也能端到端跑通。
    #[derive(Default)]
    struct RecordingFeed {
        received: Mutex<Vec<f32>>,
        calls: Mutex<usize>,
        accept_limit: Option<i32>,
    }

    impl PcmFeed for RecordingFeed {
        fn feed_pcm(&self, samples: Vec<f32>, _frames: i32) -> i32 {
            *self.calls.lock().unwrap() += 1;
            let frames = (samples.len() / 2) as i32;
            let accepted = self.accept_limit.unwrap_or(frames);
            self.received.lock().unwrap().extend_from_slice(&samples);
            accepted
        }
    }

    #[derive(Default)]
    struct ScriptedPull {
        chunks: Mutex<Vec<Vec<f32>>>,
    }

    impl PcmPull for ScriptedPull {
        fn read_pcm(&self, _max_samples: i32) -> Vec<f32> {
            let mut chunks = self.chunks.lock().unwrap();
            if chunks.is_empty() {
                Vec::new()
            } else {
                chunks.remove(0)
            }
        }
    }

    #[test]
    fn 播放桥把整帧原样交给_kotlin_回调() {
        let feed = Arc::new(RecordingFeed::default());
        let mut sink = KotlinPlayoutSink::new(Arc::clone(&feed) as Arc<dyn PcmFeed>);

        assert!(sink.device_format().is_unified(), "必须过零重采样硬闸");
        assert_eq!(sink.backend_name(), "kotlin-callback");

        let frame: Vec<f32> = (0..1_920).map(|i| i as f32 / 1_920.0).collect();
        sink.write(&frame).unwrap();

        assert_eq!(*feed.calls.lock().unwrap(), 1);
        assert_eq!(*feed.received.lock().unwrap(), frame);

        let stats = sink.stats();
        assert_eq!(stats.writes, 1);
        assert_eq!(stats.frames_written, 960);
        assert_eq!(stats.write_errors, 0);
    }

    #[test]
    fn 回调少接收时计入写入异常而不静默() {
        let feed = Arc::new(RecordingFeed {
            accept_limit: Some(0),
            ..RecordingFeed::default()
        });
        let mut sink = KotlinPlayoutSink::new(Arc::clone(&feed) as Arc<dyn PcmFeed>);

        sink.write(&[0.0f32; 1_920]).unwrap();

        let stats = sink.stats();
        assert_eq!(stats.writes, 1);
        assert_eq!(stats.frames_written, 0);
        assert_eq!(stats.write_errors, 1, "环满必须留下痕迹");
    }

    #[test]
    fn 空帧不触发回调() {
        let feed = Arc::new(RecordingFeed::default());
        let mut sink = KotlinPlayoutSink::new(Arc::clone(&feed) as Arc<dyn PcmFeed>);
        sink.write(&[]).unwrap();
        assert_eq!(*feed.calls.lock().unwrap(), 0);
        assert_eq!(sink.stats().writes, 0);
    }

    #[test]
    fn 采集桥按帧交付且把奇数长度截到帧边界() {
        let pull = Arc::new(ScriptedPull::default());
        pull.chunks.lock().unwrap().push(vec![0.25f32; 1_920]);
        // 半帧（1921 个样本）→ 必须截成 1920
        pull.chunks.lock().unwrap().push(vec![0.5f32; 1_921]);

        let mut source = KotlinCaptureSource::new(Arc::clone(&pull) as Arc<dyn PcmPull>);
        assert!(source.device_format().is_unified());

        let mut buf: Vec<f32> = Vec::new();
        let first = source
            .read(&mut buf, Duration::from_millis(200))
            .unwrap()
            .unwrap();
        assert_eq!(first.frames, 960);
        assert_eq!(buf.len(), 1_920);
        assert_eq!(buf[0], 0.25);
        assert_eq!(source.stats().packets, 1);

        let second = source
            .read(&mut buf, Duration::from_millis(200))
            .unwrap()
            .unwrap();
        assert_eq!(second.frames, 960, "1921 个样本必须截到帧边界");
        assert_eq!(buf.len(), 1_920);
    }

    #[test]
    fn 采集无数据时返回_none_而不是空帧() {
        let pull = Arc::new(ScriptedPull::default());
        let mut source = KotlinCaptureSource::new(Arc::clone(&pull) as Arc<dyn PcmPull>);
        let mut buf: Vec<f32> = vec![1.0; 8];

        let packet = source.read(&mut buf, Duration::from_millis(0)).unwrap();
        assert!(packet.is_none());
        assert!(buf.is_empty(), "超时返回时缓冲必须被清空");
        assert_eq!(source.stats().read_timeouts, 1);
    }

    #[test]
    fn 工厂每次都给一个独立_sink() {
        let feed = Arc::new(RecordingFeed::default());
        let factory = playout_factory(Arc::clone(&feed) as Arc<dyn PcmFeed>);
        let first = factory().unwrap();
        let second = factory().unwrap();
        assert_eq!(first.backend_name(), second.backend_name());
        assert!(first.device_format().is_unified());
    }
}
