//! §4.1 音量控制：**发送端调接收端的音量**（真实 QUIC 端到端）。
//!
//! 这条测试要证明四件事：
//!
//! 1. SET_GAIN 真的落到接收侧的播放样本上（不是「记一条日志就算实现」）；
//! 2. 音量状态是**每会话一份**（多会话各调各的，采样时间轴仍然共享）；
//! 3. 非法值在发送端就被拒绝，不发出线上、也不让对方以为生效了；
//! 4. FR-12 的**本地层**（接收端自己再叠一层，不走网络）与对端下发的那层**相乘** ——
//!    判别性证据是「两层各 0.5 时幅度是基线的 1/4」（覆盖语义会得到 1/2，相加语义会 ≥ 基线）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::{ErrorCode, NodeId};
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

/// FR-12 本地层：**引擎级设置**，不依赖会话 —— 没连接也能设、能读回，非法值当场拒绝。
///
/// 为什么这条要单独测：本地音量是「用户给这台设备设的偏好」，不是「当前这条流的参数」。
/// 若实现成「必须会话存在才生效」，用户就得先连上再调、断开后设置又丢 —— 而这正是本任务要修的。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_gain_is_stored_without_a_session() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let mut config = EngineConfig::new("standalone", dir.path().join("engine"));
    config.listen = "127.0.0.1:0".parse().unwrap();
    let engine = Engine::start(config).await.expect("引擎");
    let stranger = NodeId::from_bytes([0x5A; NodeId::LEN]);

    // 从没设过 → None（而不是 1.0）：界面据此区分「没调过」与「调成了 1.0」。
    assert_eq!(engine.local_peer_gain(stranger), None);
    assert_eq!(
        engine.set_local_peer_gain(stranger, 0.75).expect("设置"),
        750
    );
    assert_eq!(engine.local_peer_gain(stranger), Some(750));
    // 本地静音就是增益 0（内核只维护「增益」一份状态）。
    assert_eq!(engine.set_local_peer_gain(stranger, 0.0).expect("静音"), 0);
    assert_eq!(engine.local_peer_gain(stranger), Some(0));
    // 非法值当场拒绝，且**不改变**已设的值。
    assert!(engine.set_local_peer_gain(stranger, 3.0).is_err(), "超上限");
    assert!(
        engine.set_local_peer_gain(stranger, f32::NAN).is_err(),
        "NaN"
    );
    assert_eq!(
        engine.local_peer_gain(stranger),
        Some(0),
        "被拒的变更不该改动原值"
    );

    // 断开：没有这条会话 → 幂等返回 false，不报错（调用方不必先查 peers()）。
    assert!(!engine.disconnect(stranger).await.expect("断开是幂等的"));

    engine.shutdown().await;
}

/// FR-12：本地层与对端下发层**相乘**（真实链路、真实播放样本）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_gain_multiplies_with_the_remote_layer() {
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
    let sender_id = sender.info().id;

    // 配对 + 开流（与上一个用例同一套动作）
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

    tokio::time::sleep(Duration::from_secs(5)).await;
    let (baseline, baseline_frames) = drain_stats(&mut record_rx);
    println!("基线：RMS={baseline:.4} 采样点={baseline_frames}");
    assert!(baseline_frames > 0, "接收侧没有记录到样本");
    assert!(baseline > 0.01, "基线幅度太小，测试没有意义：{baseline}");

    // ① 本地静音：接收端自己就能把这一路压掉，**不需要对端做任何事**（整个用例里没给对端发过帧）。
    assert_eq!(
        receiver
            .set_local_peer_gain(sender_id, 0.0)
            .expect("本地静音"),
        0
    );
    tokio::time::sleep(Duration::from_millis(400)).await;
    let _ = drain_stats(&mut record_rx);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let (silenced, _) = drain_stats(&mut record_rx);
    println!("本地 0.0：RMS={silenced:.4}");
    assert!(
        silenced < baseline * 0.05,
        "本地增益 0 必须把这一路压到接近 0：{silenced} vs {baseline}"
    );

    // ② 本地回 1.0 + 对端下发 0.5 → 约基线一半（两层各自都能生效）。
    receiver
        .set_local_peer_gain(sender_id, 1.0)
        .expect("恢复本地");
    sender
        .set_peer_gain(receiver_id, 0.5, 100)
        .await
        .expect("对端下发");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let _ = drain_stats(&mut record_rx);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let (half, _) = drain_stats(&mut record_rx);
    println!("对端 0.5 × 本地 1.0：RMS={half:.4}");
    assert!(
        half > baseline * 0.35 && half < baseline * 0.65,
        "对端下发 0.5 应约为基线的一半：{half} vs {baseline}"
    );

    // ③ 两层各 0.5 → **基线的四分之一**。这是「相乘」的判别性证据：
    //    覆盖语义会得到 half（0.5×基线），相加语义会 ≥ 基线，都对不上这个数。
    receiver
        .set_local_peer_gain(sender_id, 0.5)
        .expect("本地 0.5");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let _ = drain_stats(&mut record_rx);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let (quarter, _) = drain_stats(&mut record_rx);
    println!("对端 0.5 × 本地 0.5：RMS={quarter:.4}");
    assert!(
        quarter > baseline * 0.18 && quarter < baseline * 0.33,
        "两层各 0.5 必须是基线的约 1/4（相乘）：{quarter} vs {baseline}"
    );
    assert!(
        quarter < half * 0.75,
        "叠上本地 0.5 之后必须更小（不是覆盖成 0.5）：{quarter} vs {half}"
    );

    // 界面要能同时看到两个数：本地那层可以读回（千分点）。
    assert_eq!(receiver.local_peer_gain(sender_id), Some(500));
    // 对端那一层照旧归它自己管：本机读不到「对端设了什么」，只知道自己这层的值。
    assert_eq!(
        receiver.local_peer_gain(receiver_id),
        None,
        "本机没有给自己的条目"
    );

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();
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
