//! P0 探针（docs/73 §4 / docs/74 §3）：引擎播放循环 + **真实 WASAPI 阻塞 sink**，10 ms 帧。
//!
//! 回答的问题：soak-runner（NullPlayout，从不阻塞）在 debug/release 下都零滑拍，
//! 而现场欠载飙升 —— 差异是否来自「write_frame 真阻塞 + 本地节拍网格」？
//! 本装置把每次 `write` 的阻塞时长与 A/B/C 三分量同时量出来（本机实测：零滑拍，
//! write 均值 ≈ 8 µs，见 docs/74 §3）。
//!
//! 用法：
//!     cargo test -p audiolink-engine --test p0_realsink_probe -- --ignored --nocapture
//!     cargo test --release -p audiolink-engine --test p0_realsink_probe -- --ignored --nocapture

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use audiolink_audio::wasapi::{DeviceSelector, RenderSink};
use audiolink_audio::{AudioError, DeviceFormat, PlayoutSink, PlayoutStats, SyntheticCapture};
use audiolink_engine::{Engine, EngineConfig, SessionState};

const FRAME_MS: u32 = 10;
/// 与桌面端一致（engine_bridge.rs 的 WASAPI_BUFFER_MS）。
const BUFFER_MS: u32 = 20;
const RUN_SECS: u64 = 60;

/// 每次 write 的阻塞时长（µs）：playout 拍内成本主嫌的直接读数。
#[derive(Default)]
struct WriteProbe {
    count: u64,
    total_us: u64,
    max_us: u64,
    samples: Vec<u32>,
}

/// 真实 sink 外套计时层；其余全部转发。
struct TimingSink {
    inner: RenderSink,
    probe: Arc<Mutex<WriteProbe>>,
}

impl PlayoutSink for TimingSink {
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
        let started = Instant::now();
        let out = self.inner.write(samples);
        let elapsed = started.elapsed().as_micros() as u32;
        let mut probe = self.probe.lock().unwrap();
        probe.count += 1;
        probe.total_us += u64::from(elapsed);
        probe.max_us = probe.max_us.max(u64::from(elapsed));
        probe.samples.push(elapsed);
        out
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "手动测量装置：显式 --ignored 才跑（用真实声卡）"]
async fn real_sink_schedule_slip_probe() {
    let probe = Arc::new(Mutex::new(WriteProbe::default()));

    let dir = tempfile::TempDir::new().expect("临时目录");

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = FRAME_MS;
    send_config.capture = Some(Arc::new(|| {
        Ok(Box::new(SyntheticCapture::new(FRAME_MS, 440.0)?)
            as Box<dyn audiolink_audio::CaptureSource>)
    }));

    let mut recv_config = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_config.listen = "127.0.0.1:0".parse().unwrap();
    recv_config.codec.frame_ms = FRAME_MS;
    {
        let probe = Arc::clone(&probe);
        recv_config.playout = Some(Arc::new(move || {
            let inner = RenderSink::open(&DeviceSelector::Default, BUFFER_MS)?;
            Ok(Box::new(TimingSink {
                inner,
                probe: Arc::clone(&probe),
            }) as Box<dyn PlayoutSink>)
        }));
    }

    let receiver = Engine::start(recv_config).await.expect("接收引擎");
    let _accept = receiver.spawn_accept_loop();
    let receiver_id = receiver.info().id;

    let sender = Engine::start(send_config).await.expect("发送引擎");
    let sender_id = sender.info().id;

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
    // 静音本机输出（增益 0），探针照常计时。
    receiver
        .set_local_peer_gain(sender_id, 0.0)
        .expect("设本地增益");
    sender.start_send(receiver_id).await.expect("开流");

    println!("t_s,underruns,A_schedule_slip,B_queue_reject,C_cursor_catchup,buffer_us,depth_drops");
    let t0 = Instant::now();
    let mut next = Duration::from_secs(1);
    while t0.elapsed() < Duration::from_secs(RUN_SECS) {
        tokio::time::sleep_until(tokio::time::Instant::from_std(t0 + next)).await;
        next += Duration::from_secs(1);
        let stats = receiver.telemetry(sender_id);
        let b = receiver.underrun_breakdown(sender_id).unwrap_or_default();
        println!(
            "{},{},{},{},{},{},{}",
            t0.elapsed().as_secs(),
            stats.as_ref().map_or(0, |s| s.underruns),
            b.schedule_slips,
            b.queue_rejects,
            b.cursor_catchups,
            stats.as_ref().map_or(0, |s| s.buffer_level_us),
            receiver.depth_drops(sender_id).unwrap_or(0),
        );
    }

    sender.shutdown().await;
    receiver.shutdown().await;

    let probe = probe.lock().unwrap();
    let mut sorted = probe.samples.clone();
    sorted.sort_unstable();
    let pick = |pct: f64| -> u32 {
        if sorted.is_empty() {
            return 0;
        }
        let rank = (pct / 100.0 * sorted.len() as f64).ceil() as usize;
        sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
    };
    println!(
        "write 阻塞：次数 {} · 均值 {:.1} µs · p50 {} µs · p95 {} µs · max {} µs",
        probe.count,
        probe.total_us as f64 / probe.count.max(1) as f64,
        pick(50.0),
        pick(95.0),
        probe.max_us,
    );
    println!(
        "拍数对账：write 次数 {} / {} s ⇒ {:.2} 拍/s（标称 {}）",
        probe.count,
        RUN_SECS,
        probe.count as f64 / RUN_SECS as f64,
        1000 / FRAME_MS,
    );
}
