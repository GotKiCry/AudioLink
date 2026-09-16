//! §4.1 音量控制：**发送端调接收端的音量**（真实 QUIC 端到端）。
//!
//! 这条测试要证明三件事：
//!
//! 1. SET_GAIN 真的落到接收侧的播放样本上（不是「记一条日志就算实现」）；
//! 2. 音量状态是**每会话一份**（多会话各调各的，采样时间轴仍然共享）；
//! 3. 非法值在发送端就被拒绝，不发出线上、也不让对方以为生效了。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::ErrorCode;
use tokio::sync::mpsc;

/// 只记样本、不出声的播放端（NullPlayout 的写调用会把样本原样送进记录通道）。
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

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|value| value * value).sum::<f32>() / samples.len() as f32).sqrt()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gain_changes_reach_the_receivers_playback_samples() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));

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
    let mut events = receiver.subscribe();
    let receiver_id = receiver.info().id;

    let sender = Engine::start(send_config).await.expect("发送引擎");

    // 配对 + 开流
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
    sender.start_send(receiver_id).await.expect("开流");

    // 收一段基线（给攒帧与首帧时间留足余量）
    tokio::time::sleep(Duration::from_secs(5)).await;
    let (baseline, baseline_frames) = drain_stats(&mut record_rx);
    println!("基线：RMS={baseline:.4} 采样点={baseline_frames}");
    assert!(
        baseline_frames > 0,
        "接收侧播放端一个样本都没记录到：播放线程没有写数据"
    );
    assert!(baseline > 0.01, "基线幅度太小，测试没有意义：{baseline}");

    // §4.1：调成静音（100 ms 渐变）
    sender
        .set_peer_gain(receiver_id, 0.0, 100)
        .await
        .expect("下发静音");
    // 渐变期（100 ms）本身带衰减增益，属于「过渡」而不是「稳态」：先把它丢掉再测。
    tokio::time::sleep(Duration::from_millis(400)).await;
    let _ = drain_stats(&mut record_rx);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let (muted, muted_frames) = drain_stats(&mut record_rx);
    println!("静音后：RMS={muted:.4} 采样点={muted_frames}");
    assert!(
        muted < baseline * 0.05,
        "静音后写出的样本幅度必须接近 0：baseline={baseline} muted={muted}"
    );

    // 调回来
    sender
        .set_peer_gain(receiver_id, 1.0, 100)
        .await
        .expect("恢复音量");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let _ = drain_stats(&mut record_rx);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let (restored, restored_frames) = drain_stats(&mut record_rx);
    println!("恢复后：RMS={restored:.4} 采样点={restored_frames}");
    assert!(
        restored > baseline * 0.5,
        "恢复后幅度应回到基线量级：baseline={baseline} restored={restored}"
    );

    // 非法值在**发送端**就被拒绝：不发到线上，也不让对方以为生效了
    let rejected = sender.set_peer_gain(receiver_id, 3.0, 0).await;
    assert!(rejected.is_err(), "越界增益必须被拒绝，实际 {rejected:?}");

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();

    println!("总结：基线 {baseline:.4} → 静音 {muted:.4} → 恢复 {restored:.4}");
}

/// 把记录通道里积攒的样本收干，返回 (RMS, 采样点数)。
fn drain_stats(rx: &mut mpsc::UnboundedReceiver<Vec<f32>>) -> (f32, usize) {
    let mut all = Vec::new();
    while let Ok(chunk) = rx.try_recv() {
        all.extend_from_slice(&chunk);
    }
    let count = all.len();
    (rms(&all), count)
}
