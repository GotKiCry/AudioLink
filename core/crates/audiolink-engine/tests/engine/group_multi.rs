//! M3 边界：**一个发送端同时服务两个同步组**（真实 QUIC）。
//!
//! 真实场景：一台 PC 同时给「客厅」和「书房」两组设备推流 —— 两个组各有自己的 epoch 基准，
//! 组内必须一致、组间必须互不干扰。
//!
//! 为什么补这一条（第 64 轮）：组管理此前覆盖了「建组」（group_management）、「组内同步」（group_sync）、
//! 「动态加入」（group_join），但**两个组同时存在**这条边界没有任何测试 —— 而它是最容易被写坏的一种：
//! 组基准若是按「发送端」而不是按「组」存/取，第二个 `create_group` 就会把第一个组的基准冲掉，
//! 表现是「先建的组突然不同步了」，且组内单组测试全都抓不到。
//!
//! 判据是**排播时间线**（`PlayoutScheduled.target_local_us`），三条一起看：
//! 1. 同组成员的目标时刻必须相同（±1 ms 的时钟估计误差）；
//! 2. 不同组的目标时刻必须不同（两个组各有自己的 epoch）；
//! 3. 四台都必须真的出声。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::{ErrorCode, NodeId};
use tokio::task::JoinHandle;

/// 数非静音样本的播放端。
#[derive(Default)]
struct Counters {
    audible: usize,
}

struct CountingSink {
    inner: NullPlayout,
    counters: Arc<Mutex<Counters>>,
}

impl PlayoutSink for CountingSink {
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
        if samples.iter().any(|sample| sample.abs() > 0.01)
            && let Ok(mut counters) = self.counters.lock()
        {
            counters.audible += samples.len();
        }
        Ok(())
    }

    fn stats(&self) -> PlayoutStats {
        self.inner.stats()
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn backend_name(&self) -> &'static str {
        "counting"
    }
}

async fn start_receiver(
    dir: &Path,
    index: usize,
    counters: Arc<Mutex<Counters>>,
    frame_ms: u32,
) -> (Arc<Engine>, JoinHandle<()>) {
    let mut config = EngineConfig::new(
        format!("receiver-{index}"),
        dir.join(format!("receiver-{index}")),
    );
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.codec.frame_ms = frame_ms;
    config.playout = Some(Arc::new(move || {
        Ok(Box::new(CountingSink {
            inner: NullPlayout::new(60),
            counters: Arc::clone(&counters),
        }) as Box<dyn PlayoutSink>)
    }));
    let engine = Engine::start(config).await.expect("接收引擎");
    let accept = engine.spawn_accept_loop();
    (engine, accept)
}

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
    let deadline = Instant::now() + std::time::Duration::from_secs(20);
    while !sender
        .peers()
        .iter()
        .any(|peer| peer.id == id && peer.state == SessionState::Streaming)
    {
        assert!(
            Instant::now() < deadline,
            "会话没有在 20 s 内进入 streaming"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// 首次排播的 `(epoch_id, target_local_us)`；`None` = 这一路压根没进排播。
///
/// **为什么必须带上 `epoch_id`**：两组各自建组时取的 `now_monotonic_us()` 只差几十微秒，
/// 实测两组的目标时刻只差 27 µs —— 拿「目标时刻不同」当「各有各的基准」的判据根本立不住。
fn first_scheduled(
    events: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
) -> Option<(u64, i64)> {
    let mut found = None;
    while let Ok(event) = events.try_recv() {
        if let EngineEvent::PlayoutScheduled {
            epoch_id,
            target_local_us,
            ..
        } = event
        {
            found = Some((epoch_id, target_local_us));
        }
    }
    found
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_groups_keep_their_own_epochs() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;
    let lead_ms = 120_u32;

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));
    let sender = Engine::start(send_config).await.expect("发送引擎");

    let mut receivers = Vec::new();
    let mut counters: Vec<Arc<Mutex<Counters>>> = Vec::new();
    for index in 0..4 {
        let sink_counters = Arc::new(Mutex::new(Counters::default()));
        let (receiver, accept) =
            start_receiver(dir.path(), index, Arc::clone(&sink_counters), frame_ms).await;
        counters.push(sink_counters);
        receivers.push((receiver, accept));
    }

    let mut events: Vec<_> = receivers
        .iter()
        .map(|(receiver, _)| receiver.subscribe())
        .collect();

    let outcome = tokio::time::timeout(std::time::Duration::from_secs(90), async {
        let mut ids = Vec::new();
        for ((receiver, _), event_rx) in receivers.iter().zip(events.iter_mut()) {
            let id = connect_and_pair(&sender, receiver).await;
            // 配对阶段会收到 DisplayPin 等事件，订阅是复用的，这里只取排播事件所以不冲突。
            let _ = event_rx;
            ids.push(id);
        }
        for id in &ids {
            wait_streaming(&sender, *id).await;
        }

        // 两个组：前两台一组、后两台一组。两条 `create_group` 各有自己的 epoch。
        let group_one = sender
            .create_group(&[ids[0], ids[1]], lead_ms)
            .await
            .expect("建组 1");
        let group_two = sender
            .create_group(&[ids[2], ids[3]], lead_ms)
            .await
            .expect("建组 2");
        assert_ne!(group_one, group_two, "两个组应当是不同的组号");

        let results = sender.start_send_many(&ids).await.expect("四台一起开流");
        assert_eq!(results.len(), 4);
        for (id, result) in &results {
            assert!(result.is_ok(), "{id:?} 开流失败：{result:?}");
        }

        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        ids
    })
    .await
    .expect("90 s 内必须完成四台配对、两个组与批量开流");
    assert_eq!(outcome.len(), 4);

    let scheduled: Vec<Option<(u64, i64)>> = events.iter_mut().map(first_scheduled).collect();
    let audible: Vec<usize> = counters
        .iter()
        .map(|counters| counters.lock().unwrap().audible)
        .collect();
    println!("[group-multi] 四台 (epoch_id, target) {scheduled:?}；非静音样本 {audible:?}");

    for (index, entry) in scheduled.iter().enumerate() {
        assert!(
            entry.is_some(),
            "第 {index} 台没有排播事件 —— 它没拿到组基准（两个组都必须各自生效）"
        );
    }
    for (index, samples) in audible.iter().enumerate() {
        assert!(*samples > 0, "第 {index} 台没有出声");
    }

    let (epoch_a, a) = scheduled[0].unwrap();
    let (epoch_b, b) = scheduled[1].unwrap();
    let (epoch_c, c) = scheduled[2].unwrap();
    let (epoch_d, d) = scheduled[3].unwrap();
    assert_eq!(epoch_a, epoch_b, "组 1 的两台必须用同一个 epoch");
    assert_eq!(epoch_c, epoch_d, "组 2 的两台必须用同一个 epoch");
    assert_ne!(
        epoch_a, epoch_c,
        "两个组必须各有自己的 epoch —— 相同说明后建的组把先建的冲掉了"
    );
    // 组内一致（只容忍两端时钟估计各自的误差）。
    assert!(
        (a - b).abs() <= 1_000,
        "组 1 的两台必须锚到同一个目标时刻，实际 {a} vs {b}"
    );
    assert!(
        (c - d).abs() <= 1_000,
        "组 2 的两台必须锚到同一个目标时刻，实际 {c} vs {d}"
    );
    let _ = (a, c); // 目标时刻本身不参与断言，见上面关于「27 µs」的说明。

    for (_receiver, accept) in receivers.iter() {
        accept.abort();
    }
    sender.shutdown().await;
    for (receiver, _accept) in &receivers {
        receiver.shutdown().await;
    }
}
