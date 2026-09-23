//! M4 汇聚：**两个发送端 → 一个接收端，软件侧混音**（真实 QUIC）。
//!
//! 证明的是混音器的接线，不是混音算法本身（算法有 11 项单测）：
//!
//! 1. 同一台接收端只打开一次播放设备（第一个会话是 owner，其余会话把帧混进来）；
//! 2. 两路都真的进了输出 —— 把其中一路静音，输出幅度必须可见地降下来；
//! 3. 每路的音量仍然独立（§4.1 的 SET_GAIN 落到它自己那一路的混音输入上）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, SessionState};
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

fn drain_rms(rx: &mut mpsc::UnboundedReceiver<Vec<f32>>) -> f32 {
    let mut all = Vec::new();
    while let Ok(chunk) = rx.try_recv() {
        all.extend_from_slice(&chunk);
    }
    if all.is_empty() {
        return 0.0;
    }
    (all.iter().map(|value| value * value).sum::<f32>() / all.len() as f32).sqrt()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_senders_converge_into_one_output() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;

    let (record_tx, mut record_rx) = mpsc::unbounded_channel::<Vec<f32>>();
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

    // 两个发送端用**不同频率**：混音后不会有相位抵消，幅度证据才站得住
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
        // 两个发送端各自连接接收端
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

        // 两路同时开流 → 接收端只有一次设备打开，两路在软件侧求和
        for sender in &senders {
            sender.start_send(receiver_id).await.expect("开流");
        }
        tokio::time::sleep(Duration::from_secs(6)).await;
        let both = drain_rms(&mut record_rx);

        // 把 A 路在接收侧的混音输入静音（§4.1：发送端调接收端音量，落到它自己那一路）
        let sender_a = senders.first().expect("发送端 A");
        sender_a
            .set_peer_gain(receiver_id, 0.0, 100)
            .await
            .expect("静音 A 路");
        tokio::time::sleep(Duration::from_millis(400)).await;
        let _ = drain_rms(&mut record_rx); // 丢掉渐变期
        tokio::time::sleep(Duration::from_secs(1)).await;
        let only_b = drain_rms(&mut record_rx);
        (both, only_b)
    })
    .await;

    let (both, only_b) = outcome.expect("90 s 内必须完成连接、开流与静音验证");

    for sender in &senders {
        sender.shutdown().await;
    }
    accept.abort();
    receiver.shutdown().await;

    println!("两路混音 RMS={both:.4} → 静音 A 路后 RMS={only_b:.4}");
    assert!(both > 0.01, "两路混音的输出幅度太小，测试没有意义：{both}");
    assert!(
        only_b < both * 0.8,
        "静音一路之后输出必须可见地降下来：两路 {both} → 单路 {only_b}"
    );
    assert!(only_b > 0.005, "剩下那一路还应当在出声：{only_b}");
}
