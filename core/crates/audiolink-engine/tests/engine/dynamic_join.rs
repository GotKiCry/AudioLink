//! M3 验收「动态」：**运行中第 3 台加入，前两台不中断**（真实 QUIC）。
//!
//! 为什么单独测这一条：M3 的验收表里，「并发」已由 `multi_session` 覆盖，「同步」要真机同期录音，
//! 「时钟」由 `clock_sync` 覆盖 —— 只有「动态加入」此前**没有任何自动化证据**，而它恰恰是最容易出问题的：
//! 新会话加入时要新建采集订阅、新建编码器、新建 QT 会话，任何一处争用都可能让**已经在跑的会话**停顿。
//!
//! 判据用「接收侧 sink 的样本增量」，而不是「状态还是 Streaming」—— 状态是采样出来的，
//! 断流几百毫秒可能采不到；样本数不增长则是**直接证据**。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::{ErrorCode, NodeId};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// 只记样本、不出声的播放端（写调用把样本送进记录通道）。
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

/// 取走通道里攒下的**样本数**（顺便清空），用来量「这段时间里它到底有没有出声」。
fn drain_samples(rx: &mut mpsc::UnboundedReceiver<Vec<f32>>) -> usize {
    let mut total = 0;
    while let Ok(chunk) = rx.try_recv() {
        total += chunk.len();
    }
    total
}

async fn start_receiver(
    dir: &Path,
    index: usize,
    samples: mpsc::UnboundedSender<Vec<f32>>,
    frame_ms: u32,
) -> (Arc<Engine>, JoinHandle<()>) {
    let mut config = EngineConfig::new(
        format!("receiver-{index}"),
        dir.join(format!("receiver-{index}")),
    );
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.codec.frame_ms = frame_ms;
    config.playout = Some(Arc::new(move || {
        Ok(Box::new(RecordingSink {
            inner: NullPlayout::new(60),
            samples: samples.clone(),
        }) as Box<dyn PlayoutSink>)
    }));
    let engine = Engine::start(config).await.expect("接收引擎");
    let accept = engine.spawn_accept_loop();
    (engine, accept)
}

/// 首次连接必须走 PIN 配对（§5）。
async fn connect_and_pair(sender: &Arc<Engine>, receiver: &Arc<Engine>) -> NodeId {
    let mut events = receiver.subscribe();
    let receiver_id = receiver.info().id;
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
    receiver_id
}

async fn wait_streaming(sender: &Arc<Engine>, id: NodeId) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !sender
        .peers()
        .iter()
        .any(|peer| peer.id == id && peer.state == SessionState::Streaming)
    {
        assert!(
            Instant::now() < deadline,
            "会话没有在 20 s 内进入 streaming"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_third_receiver_joins_without_interrupting_the_first_two() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));
    let sender = Engine::start(send_config).await.expect("发送引擎");

    // 前两台先上线并跑起来。
    let mut first_two = Vec::new();
    let mut accepts = Vec::new();
    let mut sinks = Vec::new();
    let mut ids = Vec::new();
    for index in 0..2 {
        let (tx, rx) = mpsc::unbounded_channel();
        let (receiver, accept) = start_receiver(dir.path(), index, tx, frame_ms).await;
        let id = connect_and_pair(&sender, &receiver).await;
        wait_streaming(&sender, id).await;
        first_two.push(receiver);
        accepts.push(accept);
        sinks.push(rx);
        ids.push(id);
    }
    sender.start_send_many(&ids).await.expect("前两台开流");

    tokio::time::sleep(Duration::from_secs(4)).await;
    let before: Vec<usize> = sinks.iter_mut().map(drain_samples).collect();
    for (index, samples) in before.iter().enumerate() {
        assert!(
            *samples > 0,
            "第 {index} 台在前 4 s 就该在出声，实际 {samples} 个样本"
        );
    }

    // 第三台**在运行中加入**（M3 验收表里的「动态」）。
    let (tx3, mut rx3) = mpsc::unbounded_channel();
    let (third, accept3) = start_receiver(dir.path(), 2, tx3, frame_ms).await;
    let id3 = connect_and_pair(&sender, &third).await;
    wait_streaming(&sender, id3).await;
    sender.start_send(id3).await.expect("第三台开流");

    tokio::time::sleep(Duration::from_secs(4)).await;
    let after: Vec<usize> = sinks.iter_mut().map(drain_samples).collect();
    for (index, samples) in after.iter().enumerate() {
        assert!(
            *samples > 0,
            "第三台加入后第 {index} 台停了（新增 {samples} 个样本）—— 动态加入不该打断已有会话"
        );
    }
    let third_samples = drain_samples(&mut rx3);
    assert!(
        third_samples > 0,
        "第三台自己也要在出声，实际 {third_samples} 个样本"
    );

    // 三台都还在链路上（状态是采样值，只作辅助证据）。
    for id in [ids[0], ids[1], id3] {
        assert!(
            sender
                .peers()
                .iter()
                .any(|peer| peer.id == id && peer.state == SessionState::Streaming),
            "第三台加入后 {id:?} 不在 streaming"
        );
    }

    println!("动态加入：前两台新增 {after:?} 个样本，第三台 {third_samples} 个样本");

    accept3.abort();
    third.shutdown().await;
    for accept in accepts {
        accept.abort();
    }
    sender.shutdown().await;
    for receiver in &first_two {
        receiver.shutdown().await;
    }
}
