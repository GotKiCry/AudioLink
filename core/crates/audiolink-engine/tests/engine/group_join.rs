//! M3 验收「动态」的**同步组**版本：运行中第 3 台加入同步组，它必须拿到组基准并排播，而前两台不中断。
//!
//! 为什么补这一条（第 61 轮）：`Engine::join_group`（成员动态加入同步组）此前**没有任何自动化证据** ——
//! `dynamic_join` 测的是「第 3 台加入**会话**」，`group_management` 测的是建组与退出。
//! 而 `join_group` 的语义正是「新成员除 JOIN 外补一条 GROUP_EPOCH —— 它需要组基准才能排播」（docs/30）：
//! 这条路径一旦断掉，新加入的设备就永远同步不上，而且不会有任何测试变红。
//!
//! 判据用两样东西，都要求是**直接证据**：
//! 1. 第 3 台必须收到 `PlayoutScheduled`（首次进入排播才报一次）—— 空就是没排播；
//! 2. 第 3 台必须**真的写出非静音样本** —— 只收到排播事件但目标时刻错位（一直等待或一直被丢弃）同样是坏的，
//!    而那种坏法只看事件是看不出来的。
//!
//! 顺序刻意选「**先加入组、再开流**」：此时播放句柄还没建好，组基准必须先被暂存、开流后再补应用
//! （第 59 轮修的那条路径），也正是真实场景「先把设备加进组、再推流」。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::NodeId;
use tokio::task::JoinHandle;

/// 播放端：分别数「写了多少样本」与「其中多少是非静音」。
#[derive(Clone, Default)]
struct Counters {
    written: usize,
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
        if let Ok(mut counters) = self.counters.lock() {
            counters.written += samples.len();
            if samples.iter().any(|sample| sample.abs() > 0.01) {
                counters.audible += samples.len();
            }
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

async fn connect_peer(sender: &Arc<Engine>, receiver: &Arc<Engine>) -> NodeId {
    let receiver_id = receiver.info().id;
    sender
        .connect(receiver.local_addr())
        .await
        .expect("连接应当直接成功（无认证）");
    receiver_id
}

async fn wait_streaming(sender: &Arc<Engine>, id: NodeId) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !sender
        .peers()
        .iter()
        .any(|peer| peer.id == id && peer.state == SessionState::Streaming)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "会话没有在 20 s 内进入 streaming"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn audible_of(counters: &Arc<Mutex<Counters>>) -> usize {
    counters.lock().unwrap().audible
}

/// 把广播缓冲里累积的「首次排播」事件扫出来：`(target_local_us, wait_us)`。
fn drain_scheduled(events: &mut tokio::sync::broadcast::Receiver<EngineEvent>) -> Vec<(i64, u64)> {
    let mut out = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let EngineEvent::PlayoutScheduled {
            target_local_us,
            wait_us,
            ..
        } = event
        {
            out.push((target_local_us, wait_us));
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn joining_member_receives_an_epoch_and_plays() {
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

    let counters_a = Arc::new(Mutex::new(Counters::default()));
    let counters_b = Arc::new(Mutex::new(Counters::default()));
    let counters_c = Arc::new(Mutex::new(Counters::default()));
    let (receiver_a, accept_a) =
        start_receiver(dir.path(), 0, Arc::clone(&counters_a), frame_ms).await;
    let (receiver_b, accept_b) =
        start_receiver(dir.path(), 1, Arc::clone(&counters_b), frame_ms).await;
    let (receiver_c, accept_c) =
        start_receiver(dir.path(), 2, Arc::clone(&counters_c), frame_ms).await;

    let id_a = connect_peer(&sender, &receiver_a).await;
    let id_b = connect_peer(&sender, &receiver_b).await;
    let id_c = connect_peer(&sender, &receiver_c).await;
    for id in [id_a, id_b, id_c] {
        wait_streaming(&sender, id).await;
    }

    // ① 先让 A、B 成一个组并开流 —— 它们是「已经在跑的会话」。
    let group_id = sender
        .create_group(&[id_a, id_b], lead_ms)
        .await
        .expect("建组");
    sender
        .start_send_many(&[id_a, id_b])
        .await
        .expect("两台一起开流");

    let mut events_a = receiver_a.subscribe();
    let mut events_b = receiver_b.subscribe();
    let mut events_c = receiver_c.subscribe();

    tokio::time::sleep(Duration::from_secs(3)).await;
    let before_a = audible_of(&counters_a);
    let before_b = audible_of(&counters_b);
    assert!(before_a > 0 && before_b > 0, "A、B 应当已经在出声");

    // ② 第 3 台**先加入组**（此时它还没开流 → 组基准必须先被暂存），再开流。
    sender
        .join_group(id_c, group_id)
        .await
        .expect("第 3 台加入同步组");
    sender.start_send(id_c).await.expect("第 3 台开流");

    tokio::time::sleep(Duration::from_secs(4)).await;
    let after_a = audible_of(&counters_a);
    let after_b = audible_of(&counters_b);
    let after_c = audible_of(&counters_c);
    let scheduled_c = drain_scheduled(&mut events_c);
    let scheduled_a = drain_scheduled(&mut events_a);
    let scheduled_b = drain_scheduled(&mut events_b);

    println!(
        "[group-join] 第 3 台加入后：A 新增 {} · B 新增 {} · C 新增 {} 个非静音样本；C 排播事件 {:?}",
        after_a - before_a,
        after_b - before_b,
        after_c,
        scheduled_c
    );

    // ③ 前两台不许被新成员打断（这是「动态」验收的原话）。
    assert!(after_a > before_a, "第 3 台加入后 A 停了");
    assert!(after_b > before_b, "第 3 台加入后 B 停了");
    // ④ 第 3 台必须真的排播（空 = 没拿到组基准 / 没走排播路径）。
    assert!(
        !scheduled_c.is_empty(),
        "第 3 台加入同步组后必须进入排播（C={scheduled_c:?}）—— 空说明它没拿到组基准"
    );
    // ⑤ 而且必须**真的出声**，且量与老成员同量级 —— 只收到排播事件、但目标时刻错位
    //    （一直等待或一直被丢弃）同样是坏的，而且这种坏法只看「有没有事件」看不出来：
    //    第 61 轮实测第 3 台拿到「当前时刻」当基准时，4 s 里只写出 0.32 s 音频（A、B 各 4 s）。
    let added_a = after_a - before_a;
    assert!(
        after_c * 2 >= added_a,
        "第 3 台只写出 {after_c} 个非静音样本，远少于同一窗口里 A 的 {added_a} —— 目标时刻与实际流位置对不上"
    );
    // 老成员的排播不受影响（它们早在建组时就生效了）。
    assert!(
        !scheduled_a.is_empty() && !scheduled_b.is_empty(),
        "老成员的排播事件不该消失（A={scheduled_a:?} · B={scheduled_b:?}）"
    );

    accept_a.abort();
    accept_b.abort();
    accept_c.abort();
    sender.shutdown().await;
    receiver_a.shutdown().await;
    receiver_b.shutdown().await;
    receiver_c.shutdown().await;
}
