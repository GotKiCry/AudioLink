//! 水位上涨的本机复现实验（task-7 Phase 2 装置）。
//!
//! # 它回答什么
//!
//! 「上游供帧速率」与「接收侧播放线程取帧速率」之间是否长期存在 ppm 级残差，
//! 以及这个残差是否会把引擎播放环水位（buffer_level_us）推着单调上涨。
//! 详见 target/evidence/water-level/MECHANISM.md §5-A / §6。
//!
//! # 与既有测试的区别（为什么不复用 playout_watermark.rs）
//!
//! playout_watermark 只看「水位不得堆积」，且只跑 20 s；本实验要**同时**拿到三个绝对速率：
//!   ① 发送侧采集实际产出的包/帧（TapCapture 计数）
//!   ② 接收侧播放线程实际写设备的圈数/帧数（TapSink 计数 —— 这是「播放线程跑了几拍」的唯一外部可观测量）
//!   ③ 1 Hz 遥测的 buffer_level_us（引擎播放环深度）
//! 只有 ①② 之差才能判定「入队/出队速率残差」，只有 ③ 才能看水位形态。
//!
//! # 纪律
//!
//! - 不改生产代码：TapCapture / TapSink 都是本文件内的测试替身，只做转发 + 计数。
//! - 判据先写死：见常量 REPRO_TOLERANCE_US 与结尾的判定打印。
//! - 默认 #[ignore]：只有显式设了 WATER_SECS 并加 --ignored 才会跑（避免拖慢常规回归）。
//!
//! # 用法
//!
//!     $env:WATER_SECS = "600"
//!     $env:WATER_CSV  = "C:\_Project\AudioLink\target\evidence\water-level\phase2\water-run.csv"
//!     cargo test -p audiolink-engine --test water_level_repro -- --ignored --nocapture
//!
//! 或直接用 target/evidence/water-level/run-phase2.ps1（它负责设环境变量并做判定）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use audiolink_audio::{
    AudioError, CaptureSource, CaptureStats, CapturedPacket, DeviceFormat, NullPlayout,
    PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, SessionState};

/// 判据（先写死，避免事后解释）：后段水位中位数比前段高出这么多 ⇒ 判「复现成功」。
const REPRO_TOLERANCE_US: u32 = 20_000;
/// 采样周期：比 1 Hz 遥测快一倍，取到重复值是正常的。
const SAMPLE_MS: u64 = 500;
/// 声道数（M1 契约固定 48 kHz / 2ch / f32 交错）。
const CHANNELS: usize = 2;

/// 采集侧探针：转发给 SyntheticCapture，另记「实际产出的包/帧」。
struct TapCapture {
    inner: SyntheticCapture,
    packets: Arc<AtomicU64>,
    frames: Arc<AtomicU64>,
    timeouts: Arc<AtomicU64>,
}

impl CaptureSource for TapCapture {
    fn device_format(&self) -> DeviceFormat {
        self.inner.device_format()
    }

    fn requested_buffer_ms(&self) -> u32 {
        self.inner.requested_buffer_ms()
    }

    fn effective_buffer_ms(&self) -> u32 {
        self.inner.effective_buffer_ms()
    }

    fn read(
        &mut self,
        dst: &mut Vec<f32>,
        timeout: Duration,
    ) -> Result<Option<CapturedPacket>, AudioError> {
        let out = self.inner.read(dst, timeout)?;
        match &out {
            Some(packet) => {
                self.frames
                    .fetch_add(packet.frames as u64, Ordering::Relaxed);
                self.packets.fetch_add(1, Ordering::Relaxed);
            }
            None => {
                self.timeouts.fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(out)
    }

    fn stats(&self) -> CaptureStats {
        self.inner.stats()
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
}

/// 播放侧探针：转发给 NullPlayout，另记「播放线程实际写了几圈、多少帧」。
///
/// writes 就是播放线程的**拍数**（playout_main 每圈必定 write 一次），
/// 因此 writes / elapsed 直接给出「取帧速率」，与发送侧 packets / elapsed 对照即得残差。
struct TapSink {
    inner: NullPlayout,
    writes: Arc<AtomicU64>,
    frames: Arc<AtomicU64>,
}

impl PlayoutSink for TapSink {
    fn device_format(&self) -> DeviceFormat {
        self.inner.device_format()
    }

    fn requested_buffer_ms(&self) -> u32 {
        self.inner.requested_buffer_ms()
    }

    fn effective_buffer_ms(&self) -> u32 {
        self.inner.effective_buffer_ms()
    }

    fn buffered_frames(&mut self) -> u32 {
        self.inner.buffered_frames()
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), AudioError> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.frames
            .fetch_add((samples.len() / CHANNELS) as u64, Ordering::Relaxed);
        self.inner.write(samples)
    }

    fn stats(&self) -> PlayoutStats {
        self.inner.stats()
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
}

fn median(values: &mut [u32]) -> u32 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[values.len() / 2]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "长跑实验装置：设 WATER_SECS 后手动跑（task-7 Phase 2）"]
async fn water_level_growth_repro() {
    let secs: u64 = std::env::var("WATER_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let csv_path = std::env::var("WATER_CSV").unwrap_or_else(|_| {
        format!(
            "{}/../../../target/evidence/water-level/phase2/water-run.csv",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    if let Some(parent) = std::path::Path::new(&csv_path).parent() {
        std::fs::create_dir_all(parent).expect("建证据目录");
    }

    let frame_ms = 20_u32;
    let sent_packets = Arc::new(AtomicU64::new(0));
    let sent_frames = Arc::new(AtomicU64::new(0));
    let send_timeouts = Arc::new(AtomicU64::new(0));
    let sink_writes = Arc::new(AtomicU64::new(0));
    let sink_frames = Arc::new(AtomicU64::new(0));

    let dir = tempfile::TempDir::new().expect("临时目录");

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    {
        let packets = Arc::clone(&sent_packets);
        let frames = Arc::clone(&sent_frames);
        let timeouts = Arc::clone(&send_timeouts);
        send_config.capture = Some(Arc::new(move || {
            Ok(Box::new(TapCapture {
                inner: SyntheticCapture::new(frame_ms, 440.0)?,
                packets: Arc::clone(&packets),
                frames: Arc::clone(&frames),
                timeouts: Arc::clone(&timeouts),
            }) as Box<dyn CaptureSource>)
        }));
    }

    let mut recv_config = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_config.listen = "127.0.0.1:0".parse().unwrap();
    recv_config.codec.frame_ms = frame_ms;
    {
        let writes = Arc::clone(&sink_writes);
        let frames = Arc::clone(&sink_frames);
        recv_config.playout = Some(Arc::new(move || {
            Ok(Box::new(TapSink {
                inner: NullPlayout::new(60),
                writes: Arc::clone(&writes),
                frames: Arc::clone(&frames),
            }) as Box<dyn PlayoutSink>)
        }));
    }

    let receiver = Engine::start(recv_config).await.expect("接收引擎");
    let accept = receiver.spawn_accept_loop();
    let receiver_id = receiver.info().id;

    let sender = Engine::start(send_config).await.expect("发送引擎");
    let sender_id = sender.info().id;

    // 无认证：一次 connect 即完成握手。
    sender
        .connect(receiver.local_addr())
        .await
        .expect("连接应当直接成功（无认证）");

    let deadline = Instant::now() + Duration::from_secs(15);
    while !sender
        .peers()
        .iter()
        .any(|peer| peer.id == receiver_id && peer.state == SessionState::Streaming)
    {
        assert!(Instant::now() < deadline, "握手没有在 15 s 内完成");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    sender.start_send(receiver_id).await.expect("开流");

    // 丢掉起播阶段（攒帧、sink 建立）后再开始采样。
    tokio::time::sleep(Duration::from_secs(3)).await;

    let t0 = Instant::now();
    let base_sent = sent_packets.load(Ordering::Relaxed);
    let base_writes = sink_writes.load(Ordering::Relaxed);
    let mut rows: Vec<String> = Vec::new();
    let mut samples: Vec<u32> = Vec::new();

    loop {
        let elapsed = t0.elapsed();
        if elapsed >= Duration::from_secs(secs) {
            break;
        }
        let stats = receiver.telemetry(sender_id);
        let buffer_us = stats.as_ref().map_or(0, |s| s.buffer_level_us);
        let underruns = stats.as_ref().map_or(0, |s| s.underruns);
        let late = stats.as_ref().map_or(0, |s| s.late_drops);
        // depth_drops 不在 StreamStats 里（它是引擎的独立出口），必须单独读。
        // 口径：它只有**一个**来源 —— 抖动深度**降档**的主动丢帧（`PlayoutDepthAction::DropOldest`）。
        // （2026-09-18 的「水位护栏」曾给它加过第二个来源，但两版护栏的真机验证都没过代价判据
        // （计数 6.9× / 103×，见 `docs/12` §11.12–§11.14），已于 `b6b0f90` 整体移除。）
        // 在本装置里只要 underruns == 0 ⇒ target 从未升档 ⇒ 30 个稳定窗口后也不会有降档 ⇒
        // 这一格应恒为 0。
        let depth_drops = receiver.depth_drops(sender_id).unwrap_or(0);
        let sent = sent_packets.load(Ordering::Relaxed) - base_sent;
        let writes = sink_writes.load(Ordering::Relaxed) - base_writes;
        rows.push(format!(
            "{:.3},{},{},{},{},{},{},{},{},{}",
            elapsed.as_secs_f64(),
            buffer_us,
            underruns,
            late,
            depth_drops,
            sent,
            sent_frames.load(Ordering::Relaxed),
            writes,
            sink_frames.load(Ordering::Relaxed),
            send_timeouts.load(Ordering::Relaxed),
        ));
        samples.push(buffer_us);
        tokio::time::sleep(Duration::from_millis(SAMPLE_MS)).await;
    }

    let elapsed = t0.elapsed();
    let sent_total = sent_packets.load(Ordering::Relaxed) - base_sent;
    let writes_total = sink_writes.load(Ordering::Relaxed) - base_writes;
    let timeouts_total = send_timeouts.load(Ordering::Relaxed);

    // 终值行：CSV 的最后一行必须是**结束时刻**的累计值，否则用最后一行算速率残差会带上
    // 「最后一次采样 → 循环结束」这半个采样周期的误差（45 s 窗口里它能到几百 ppm，
    // 而本实验要判的正是 ppm 级残差）。
    let final_stats = receiver.telemetry(sender_id);
    rows.push(format!(
        "{:.3},{},{},{},{},{},{},{},{},{}",
        elapsed.as_secs_f64(),
        samples.last().copied().unwrap_or(0),
        final_stats.as_ref().map_or(0, |s| s.underruns),
        final_stats.as_ref().map_or(0, |s| s.late_drops),
        receiver.depth_drops(sender_id).unwrap_or(0),
        sent_total,
        sent_frames.load(Ordering::Relaxed),
        writes_total,
        sink_frames.load(Ordering::Relaxed),
        timeouts_total,
    ));

    // 写 CSV（原始数据，供 analyze-phase2.ps1 复算）。
    {
        let mut out = String::from(
            "t_s,buffer_us,underruns,late_drops,depth_drops,sent_packets,sent_frames,sink_writes,sink_frames,send_timeouts\n",
        );
        for row in &rows {
            out.push_str(row);
            out.push('\n');
        }
        std::fs::write(&csv_path, out).expect("写 CSV");
    }

    let elapsed_s = elapsed.as_secs_f64();
    let half = samples.len() / 2;
    let mut early = samples[..half].to_vec();
    let mut late = samples[half..].to_vec();
    let early_us = median(&mut early);
    let late_us = median(&mut late);
    let peak = samples.iter().copied().max().unwrap_or(0);
    let first = samples.first().copied().unwrap_or(0);

    // 绝对速率：发送侧包率（每包 = 20 ms 音频）与接收侧播放线程圈率（每圈 = 20 ms）。
    let sent_rate = sent_total as f64 / elapsed_s;
    let writes_rate = writes_total as f64 / elapsed_s;
    let realtime_rate = 1000.0 / f64::from(frame_ms);
    let ppm = if writes_total > 0 {
        (sent_total as f64 - writes_total as f64) / writes_total as f64 * 1e6
    } else {
        f64::NAN
    };
    let drift_frames = (f64::from(late_us) - f64::from(early_us)) / f64::from(frame_ms);
    let implied_ppm = drift_frames / elapsed_s * 1e6;

    println!(
        "[water] 时长 {elapsed_s:.1}s  样本 {}  水位首值 {first} us -> 前段 P50 {early_us} us / 后段 P50 {late_us} us / 峰值 {peak} us",
        samples.len()
    );
    println!(
        "[water] 绝对速率：发送侧 {sent_rate:.3} 包/s（实时参考 {realtime_rate:.1}） | 播放线程 {writes_rate:.3} 圈/s | 残差 {ppm:.1} ppm"
    );
    println!(
        "[water] 水位斜率 = {drift_frames:.2} 帧 / {elapsed_s:.0}s（{implied_ppm:.1} ppm 量级）  CSV: {csv_path}"
    );
    println!(
        "[water] 其它账：underruns {} · late_drops {} · depth_drops {}（本装置里 underruns==0 ⇒ 无降档 ⇒ 即护栏触发次数）· 采集超时 {} · 发送帧 {} · 设备帧 {}",
        receiver.telemetry(sender_id).map_or(0, |s| s.underruns),
        receiver.telemetry(sender_id).map_or(0, |s| s.late_drops),
        receiver.depth_drops(sender_id).unwrap_or(0),
        timeouts_total,
        sent_frames.load(Ordering::Relaxed),
        sink_frames.load(Ordering::Relaxed),
    );

    if late_us >= early_us.saturating_add(REPRO_TOLERANCE_US) {
        println!(
            "[water] 判定：**复现成功** —— 后段水位中位数比前段高 {} us（阈值 {REPRO_TOLERANCE_US} us）",
            late_us - early_us
        );
    } else {
        println!(
            "[water] 判定：**未复现** —— 后段水位中位数仅比前段高 {} us（阈值 {REPRO_TOLERANCE_US} us）",
            late_us.saturating_sub(early_us)
        );
    }

    sender.shutdown().await;
    accept.abort();
    receiver.shutdown().await;
}
