//! M4 · 共同时间基准：接收端广播 epoch（`RECEIVER_EPOCH`，`0x44`），各发送端据此把样本编号
//! 对齐到同一条时间轴上。
//!
//! 为什么需要（上一轮留下的缺口）：`GROUP_EPOCH` 是「发送端指定、接收端排播」，只能让
//! 「同一发送端 → 多台接收端」对齐；而 M4 的拓扑是「多台发送端 → 一台接收端混音」，
//! 两个发送端彼此独立，`sample_index` 的原点分别是各自启流的瞬间 —— 接收端按编号取样本时，
//! 两路取到的不是同一时刻的声音。这一轮把方向反过来：**接收端作为基准来源广播 `0x44`**，
//! 发送端换算到自己轴上之后把编号 0 点钉到它（`HubClock::align_epoch`）。
//!
//! 本测试验证的是「广播到达 + 对齐不打断供帧」这一层；**采样级对齐的直接读数还缺一个观测入口**
//! （接收端目前不暴露每路流的 `sample_index` ↔ 到达时刻），这一点写在 docs/39 里，不藏。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, SessionState, StreamAxis};
use tokio::sync::mpsc;

struct RecordingSink {
    inner: NullPlayout,
    samples: mpsc::UnboundedSender<Vec<f32>>,
}

impl PlayoutSink for RecordingSink {
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
        self.inner.write(samples)?;
        let _ = self.samples.send(samples.to_vec());
        Ok(())
    }

    fn stats(&self) -> PlayoutStats {
        self.inner.stats()
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn backend_name(&self) -> &'static str {
        "recording"
    }
}

/// 用**同一个**「现在」推算两路编号，返回它们之间的跨度（样本）。
///
/// 这是「对齐」的直接读数：跨度接近 0 = 两路的同一编号落在同一时刻；跨度几十毫秒 = 时间轴错位。
fn spread(axes: &[StreamAxis], now_ms: u32) -> Option<u32> {
    let indices: Vec<u32> = axes
        .iter()
        .filter_map(|axis| axis.index_at(now_ms))
        .collect();
    let min = indices.iter().min()?;
    let max = indices.iter().max()?;
    Some(max.wrapping_sub(*min))
}

/// 接收端广播共同基准之后：两条发送端都必须收到，且混音不能被对齐打断。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn receiver_broadcasts_the_common_start() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;

    let (record_tx, _record_rx) = mpsc::unbounded_channel::<Vec<f32>>();
    let mut recv_config = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_config.listen = "127.0.0.1:0".parse().unwrap();
    recv_config.codec.frame_ms = frame_ms;
    recv_config.playout = Some(Arc::new(move || {
        Ok(Box::new(RecordingSink {
            inner: NullPlayout::new(60),
            samples: record_tx.clone(),
        }) as Box<dyn PlayoutSink>)
    }));
    let receiver = Engine::start(recv_config).await.expect("接收引擎");
    let accept = receiver.spawn_accept_loop();
    let receiver_id = receiver.info().id;

    let mut senders = Vec::new();
    for (name, frequency) in [("sender-a", 440.0f32), ("sender-b", 660.0f32)] {
        let mut config = EngineConfig::new(name, dir.path().join(name));
        config.listen = "127.0.0.1:0".parse().unwrap();
        config.codec.frame_ms = frame_ms;
        config.capture = Some(Arc::new(move || {
            Ok(Box::new(SyntheticCapture::new(frame_ms, frequency)?))
        }));
        senders.push(Engine::start(config).await.expect("发送引擎"));
    }

    let outcome = tokio::time::timeout(Duration::from_secs(90), async {
        for sender in &senders {
            sender
                .connect(receiver.local_addr())
                .await
                .expect("连接应当直接成功（无认证）");
            while !sender
                .peers()
                .iter()
                .any(|peer| peer.id == receiver_id && peer.state == SessionState::Streaming)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        // 故意制造一个**明显**的错位：先让 A 单独跑起来，1.5 s 后再让 B 加入 ——
        // 两路的编号原点（各自启流瞬间）因此相差约 1.5 s，也就是 72 000 个样本。
        senders[0].start_send(receiver_id).await.expect("开流 A");
        tokio::time::sleep(Duration::from_millis(1500)).await;
        senders[1].start_send(receiver_id).await.expect("开流 B");
        tokio::time::sleep(Duration::from_secs(3)).await;
        let before = receiver.mixer_stats().expect("混音器已建好");
        let axis_before = spread(&receiver.stream_axes(), receiver.monotonic_ms());

        // 本机作为接收端：把所有发送端共用的原点广播出去（提前量 200 ms）。
        let sent = receiver.broadcast_epoch(200).await.expect("广播共同基准");
        tokio::time::sleep(Duration::from_secs(3)).await;
        let after = receiver.mixer_stats().expect("混音器仍在");
        let axis_after = spread(&receiver.stream_axes(), receiver.monotonic_ms());
        (sent, before, after, axis_before, axis_after)
    })
    .await;

    for sender in &senders {
        sender.shutdown().await;
    }
    accept.abort();
    receiver.shutdown().await;

    let (sent, before, after, axis_before, axis_after) =
        outcome.expect("90 s 内必须完成连接、开流与广播");
    println!(
        "M4 共同基准：广播 {} 条会话 · 对齐前 {} 帧（掉队 {}）· 对齐后 {} 帧（掉队 {}）",
        sent, before.total_frames, before.partial_frames, after.total_frames, after.partial_frames
    );
    println!(
        "M4 编号跨度：对齐前 {:?} 样本 · 对齐后 {:?} 样本（1 帧 = 960 样本）",
        axis_before, axis_after
    );

    assert_eq!(sent, 2, "两条已连接的会话都应当收到共同基准");
    assert_eq!(after.sources, 2, "对齐之后两路仍应各占一路");
    assert!(
        after.total_frames > before.total_frames + 50,
        "对齐不该打断供帧：{} → {}",
        before.total_frames,
        after.total_frames
    );
    // 对齐的直接判据：用**同一个 now** 推算两路编号，跨度应当从「几十毫秒」塌到「1 帧以内」。
    let axis_before = axis_before.expect("对齐前两路都应当已经收到音频");
    let axis_after = axis_after.expect("对齐后两路都应当仍在收到音频");
    assert!(
        axis_before > 20_000,
        "对齐前两路本该明显错位（启流相差 1.5 s），实测跨度只有 {axis_before} 样本"
    );
    assert!(
        axis_after < 1_920,
        "对齐后两路的编号跨度应当落在 2 帧以内，实测 {axis_after} 样本"
    );

    let partial_ratio = after.partial_frames as f64 / after.total_frames as f64;
    assert!(
        partial_ratio < 0.05,
        "对齐后掉队帧比例过高（{partial_ratio:.3}）：对齐把某一路的编号打崩了"
    );
}
