//! PCM 单位回归（Issue #1）：真实 Engine → QUIC → Opus → 播放线程 → sink。
//! 合成正弦源和记录 sink 不访问声卡；断言消费方实际收到的样本，覆盖封装边界。

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
        "recording-null"
    }
}

async fn check_pcm_delivery(frame_ms: u32, expected_samples: usize) {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));

    let (samples_tx, mut samples_rx) = mpsc::unbounded_channel();
    let mut recv_config = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_config.listen = "127.0.0.1:0".parse().unwrap();
    recv_config.codec.frame_ms = frame_ms;
    recv_config.playout = Some(Arc::new(move || {
        Ok(Box::new(RecordingSink {
            inner: NullPlayout::new(60),
            samples: samples_tx.clone(),
        }))
    }));

    let sender = Engine::start(send_config).await.expect("发送引擎");
    let receiver = Engine::start(recv_config).await.expect("接收引擎");
    let accept = receiver.spawn_accept_loop();
    let mut events = receiver.subscribe();
    let peer = receiver.info().id;
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let error = sender.connect(receiver.local_addr()).await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotPaired);
        let pin = loop {
            if let EngineEvent::DisplayPin { pin, .. } = events.recv().await.unwrap() {
                break pin;
            }
        };
        sender.submit_pin(peer, &pin).await.expect("PIN 配对");
        while !sender
            .peers()
            .iter()
            .any(|p| p.id == peer && p.state == SessionState::Streaming)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        sender.start_send(peer).await.expect("开流");

        // 等实际输出，避免固定 sleep 让 CI 调度速度决定测试成败。
        // 单独保留欠载静音：它也必须保持一包的长度，但不用于正弦能量断言。
        let mut writes = Vec::new();
        let mut audio_packets = 0;
        while audio_packets < 30 {
            let samples = samples_rx.recv().await.expect("播放回调");
            if samples.iter().any(|sample| sample.abs() > 0.01) {
                audio_packets += 1;
            }
            writes.push(samples);
        }
        writes
    })
    .await;

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();
    let writes = result.expect("10 s 内必须配对、开流并收到 30 包音频");
    let mut audio_samples = 0;
    for samples in writes {
        assert_eq!(
            samples.len(),
            expected_samples,
            "{frame_ms} ms 双声道输出不可重复乘声道数，也不可携带未写入的缓冲尾部"
        );
        assert!(samples.iter().all(|sample| sample.is_finite()));
        if samples.iter().any(|sample| sample.abs() > 0.01) {
            audio_samples += samples.len();
            let tail = &samples[expected_samples / 2..];
            let power = tail.iter().map(|sample| sample * sample).sum::<f32>() / tail.len() as f32;
            assert!(power > 0.001, "连续正弦包的后半段不应被追加为静音");
        }
    }
    assert_eq!(audio_samples, 30 * expected_samples);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stereo_20ms_delivers_1920_samples_per_packet() {
    check_pcm_delivery(20, 1920).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stereo_10ms_delivers_960_samples_per_packet() {
    check_pcm_delivery(10, 960).await;
}
