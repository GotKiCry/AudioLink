//! M4 · 多路混音的**对齐观测**：两个发送端 → 一个接收端。
//!
//! 这一轮补齐的是「看得见」：混音器的累计观测（MixSnapshot）现在通过 Engine::mixer_stats 暴露，
//! 于是「两路都在稳定供帧」不再是猜测 —— partial_frames 直接告诉你有没有哪一路掉队。
//!
//! 已知缺口（写在这里，不让它藏在文档角落）：**两个发送端之间没有共同的时间基准**。
//! 协议 §7 的 epoch 是「发送端指定、接收端排播」，M4 的拓扑（2 部手机 → 1 台 PC）里
//! 缺少一个平级的基准来源；要做到真正的多源采样级对齐，需要「接收端广播 epoch +
//! 发送端按其对齐发送时间轴（预约发送）」，那是尚未实现的一块。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::ErrorCode;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_aligned_sources_keep_feeding_the_mixer() {
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
            let mut events = receiver.subscribe();
            let error = sender.connect(receiver.local_addr()).await.unwrap_err();
            assert_eq!(error.code(), ErrorCode::NotPaired, "首次连接必须先要 PIN");
            let pin = loop {
                if let EngineEvent::DisplayPin { pin, .. } = events.recv().await.unwrap() {
                    break pin;
                }
            };
            sender
                .submit_pin(receiver_id, &pin)
                .await
                .expect("PIN 配对");
            while !sender
                .peers()
                .iter()
                .any(|peer| peer.id == receiver_id && peer.state == SessionState::Streaming)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        for sender in &senders {
            sender.start_send(receiver_id).await.expect("开流");
        }
        tokio::time::sleep(Duration::from_secs(6)).await;
        receiver.mixer_stats()
    })
    .await;

    let stats = outcome.expect("90 s 内必须完成配对与开流");

    for sender in &senders {
        sender.shutdown().await;
    }
    accept.abort();
    receiver.shutdown().await;

    let stats = stats.expect("混音器应当在第一个接收会话时就建好");
    println!(
        "混音观测：源 {} 路 · 帧 {} · 掉队帧 {} · 限幅样本 {}",
        stats.sources, stats.total_frames, stats.partial_frames, stats.limited_samples
    );

    assert_eq!(stats.sources, 2, "两条会话都应当在混音器里各占一路");
    assert!(
        stats.total_frames > 50,
        "六秒里应当混出足够多的帧：{}",
        stats.total_frames
    );
    // 两路都持续推流时，绝大多数帧应当是「两路到齐」的：掉队帧比例必须很低。
    let partial_ratio = stats.partial_frames as f64 / stats.total_frames as f64;
    assert!(
        partial_ratio < 0.05,
        "掉队帧比例过高（{partial_ratio:.3}）：说明有一路没有持续供帧"
    );
    // 两路各约 0.4 幅度求和后接近软限幅拐点，但不该出现大面积削顶式的限幅。
    let limited_ratio = stats.limited_samples as f64 / (stats.total_frames as f64 * 960.0 * 2.0);
    assert!(
        limited_ratio < 0.5,
        "被限幅的样本比例过高（{limited_ratio:.3}）"
    );
}
