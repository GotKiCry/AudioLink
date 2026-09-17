//! M3 × M2 的交叉边界：**组内同步在链路丢包下还成不成立**（真实 QUIC + 真实丢包注入）。
//!
//! 为什么补这一条（第 67 轮）：M3 的「组内 ±10 ms」在干净回环上已经钉住了（`group_sync.rs`），
//! M2 的「丢包下不出洞」也钉住了（`nack_retransmit.rs`）。但两者**相交**的那一格没人测：
//! 链路丢包时，接收侧的抖动缓冲、PLC、NACK 重传都会动播放时刻 —— 而组内同步要求两台设备
//! 在**同一时刻**播同一帧。丢包会不会把这条验收线打穿，此前没有任何证据。
//!
//! 拓扑：发送端 → 两个**独立的有损中继** → 两台接收端（各自一条链路、各自丢包）。
//! 中继只丢「客户端 → 服务端」方向、且长度 ≥ 512 B 的包（音频数据报约 400 B 级的加密包），
//! 于是控制流与握手不受影响，命中的正是音频路径。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::{ErrorCode, NodeId};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

/// 一次写出的时刻（这条测试比的是「第 k 次写出」，不需要区分静音）。
#[derive(Clone, Copy)]
struct Stamp {
    at: Instant,
    /// 全是零样本 = 排播补的静音（不是真实音频）。
    silence: bool,
}

/// 每次写出都盖时间戳的播放端（同一进程、同一单调时钟，两端可比）。
struct StampingSink {
    inner: NullPlayout,
    stamps: Arc<Mutex<Vec<Stamp>>>,
}

impl PlayoutSink for StampingSink {
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
        if let Ok(mut list) = self.stamps.lock() {
            list.push(Stamp {
                at: Instant::now(),
                silence: samples.iter().all(|sample| sample.abs() < 0.01),
            });
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
        "stamping"
    }
}

/// 有损 UDP 中继：客户端连它，它把字节转给服务端；每 `burst_every` 个「大包」丢连续的 `burst_len` 个。
struct LossRelay {
    addr: SocketAddr,
    dropped: Arc<AtomicU64>,
    task: JoinHandle<()>,
}

impl LossRelay {
    async fn start(upstream: SocketAddr, min_len: usize, burst_every: u64, burst_len: u64) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("中继绑定");
        let addr = socket.local_addr().expect("中继地址");
        let socket = Arc::new(socket);
        let dropped = Arc::new(AtomicU64::new(0));
        let dropped_counter = dropped.clone();
        let task = tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let mut client: Option<SocketAddr> = None;
            let mut seen = 0_u64;
            let mut burst_left = 0_u64;
            while let Ok((len, from)) = socket.recv_from(&mut buf).await {
                let client_to_server = from != upstream;
                let to = if client_to_server {
                    client = Some(from);
                    upstream
                } else {
                    match client {
                        Some(client) => client,
                        None => continue,
                    }
                };
                if client_to_server && len >= min_len {
                    seen += 1;
                    if burst_left == 0 && burst_every > 0 && seen.is_multiple_of(burst_every) {
                        burst_left = burst_len;
                    }
                    if burst_left > 0 {
                        burst_left -= 1;
                        dropped_counter.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                }
                let _ = socket.send_to(&buf[..len], to).await;
            }
        });
        Self {
            addr,
            dropped,
            task,
        }
    }

    fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Drop for LossRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start_receiver(
    dir: &Path,
    index: usize,
    stamps: Arc<Mutex<Vec<Stamp>>>,
    frame_ms: u32,
) -> (Arc<Engine>, JoinHandle<()>) {
    let mut config = EngineConfig::new(
        format!("receiver-{index}"),
        dir.join(format!("receiver-{index}")),
    );
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.codec.frame_ms = frame_ms;
    config.playout = Some(Arc::new(move || {
        Ok(Box::new(StampingSink {
            inner: NullPlayout::new(60),
            stamps: Arc::clone(&stamps),
        }) as Box<dyn PlayoutSink>)
    }));
    let engine = Engine::start(config).await.expect("接收引擎");
    let accept = engine.spawn_accept_loop();
    (engine, accept)
}

async fn connect_and_pair(
    sender: &Arc<Engine>,
    receiver: &Arc<Engine>,
    addr: SocketAddr,
) -> NodeId {
    let mut events = receiver.subscribe();
    let receiver_id = receiver.info().id;
    let error = sender.connect(addr).await.unwrap_err();
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

/// 同一「第 k 次写出」在两端的时刻差（毫秒），返回 (样本数, P50, P95)。
fn deviation_ms(a: &[Instant], b: &[Instant]) -> (usize, f64, f64) {
    let skip = 50;
    let n = a.len().min(b.len());
    assert!(n > skip + 20, "采样点太少（{n} 个），这条护栏没有意义");
    let mut diffs: Vec<f64> = (skip..n)
        .map(|i| {
            let delta = if a[i] >= b[i] {
                a[i] - b[i]
            } else {
                b[i] - a[i]
            };
            delta.as_secs_f64() * 1000.0
        })
        .collect();
    diffs.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let pick = |pct: f64| diffs[((diffs.len() - 1) as f64 * pct).round() as usize];
    (n - skip, pick(0.50), pick(0.95))
}

fn drain_scheduled(events: &mut tokio::sync::broadcast::Receiver<EngineEvent>) -> usize {
    let mut count = 0;
    while let Ok(event) = events.try_recv() {
        if let EngineEvent::PlayoutScheduled { .. } = event {
            count += 1;
        }
    }
    count
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn group_sync_survives_link_loss() {
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

    let stamps_a = Arc::new(Mutex::new(Vec::new()));
    let stamps_b = Arc::new(Mutex::new(Vec::new()));
    let (receiver_a, accept_a) =
        start_receiver(dir.path(), 0, Arc::clone(&stamps_a), frame_ms).await;
    let (receiver_b, accept_b) =
        start_receiver(dir.path(), 1, Arc::clone(&stamps_b), frame_ms).await;

    // 每 50 个音频数据报成串丢 2 个（约 4%），两条链路各自独立注入。
    let relay_a = LossRelay::start(receiver_a.local_addr(), 512, 50, 2).await;
    let relay_b = LossRelay::start(receiver_b.local_addr(), 512, 50, 2).await;

    let mut events_a = receiver_a.subscribe();
    let mut events_b = receiver_b.subscribe();

    let outcome = tokio::time::timeout(Duration::from_secs(90), async {
        let id_a = connect_and_pair(&sender, &receiver_a, relay_a.addr).await;
        let id_b = connect_and_pair(&sender, &receiver_b, relay_b.addr).await;
        wait_streaming(&sender, id_a).await;
        wait_streaming(&sender, id_b).await;

        let _group_id = sender
            .create_group(&[id_a, id_b], lead_ms)
            .await
            .expect("建组");
        sender
            .start_send_many(&[id_a, id_b])
            .await
            .expect("两台一起开流");

        tokio::time::sleep(Duration::from_secs(6)).await;
        (id_a, id_b)
    })
    .await
    .expect("90 s 内必须完成配对、建组与开流");
    let (_id_a, _id_b) = outcome;

    let a: Vec<Instant> = stamps_a
        .lock()
        .unwrap()
        .iter()
        .map(|stamp| stamp.at)
        .collect();
    let b: Vec<Instant> = stamps_b
        .lock()
        .unwrap()
        .iter()
        .map(|stamp| stamp.at)
        .collect();
    let (samples, absolute_p50, absolute_p95) = deviation_ms(&a, &b);
    let scheduled_a = drain_scheduled(&mut events_a);
    let scheduled_b = drain_scheduled(&mut events_b);
    let dropped = relay_a.dropped() + relay_b.dropped();
    println!(
        "[group-sync-loss] 注入丢弃 {dropped} 个音频包；两类排播事件 {scheduled_a}/{scheduled_b}；样本 {samples} · 绝对偏差 P50 {absolute_p50:.2} ms · P95 {absolute_p95:.2} ms"
    );

    // 前提：注入真的发生了，否则这条测试什么也没证明。
    assert!(dropped > 0, "中继一个音频包都没丢 —— 这条测试失去意义");
    // 组内排播在丢包下仍要生效（空 = 退回本地游标，那就与组同步无关了）。
    assert!(
        scheduled_a > 0 && scheduled_b > 0,
        "丢包下两端都必须仍然进入排播（A={scheduled_a} · B={scheduled_b}）"
    );
    // 决定性断言：M3 验收线不因链路丢包而失守。
    assert!(
        absolute_p95 <= 10.0,
        "链路丢包时组内偏差 P95 = {absolute_p95:.2} ms，超过 M3 的 ±10 ms 验收线"
    );

    accept_a.abort();
    accept_b.abort();
    sender.shutdown().await;
    receiver_a.shutdown().await;
    receiver_b.shutdown().await;
}

/// 更狠的一档：每 10 个音频数据报成串丢 3 个（约 30%）。
///
/// 上一轮（第 67 轮）只做到 4% 并把「更狠的丢包档」记为未做。这一档会真正触发**自适应降码率**
/// 与丢包隐藏（PLC），所以它要回答一个更尖锐的问题：修正机制在极限下会不会把**播放时刻**也拖歪？
/// —— 组内同步关心的正是时刻，不是「有没有丢音」。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn group_sync_survives_heavy_loss() {
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
    let sender_id = sender.info().id;

    let stamps_a = Arc::new(Mutex::new(Vec::new()));
    let stamps_b = Arc::new(Mutex::new(Vec::new()));
    let (receiver_a, accept_a) =
        start_receiver(dir.path(), 0, Arc::clone(&stamps_a), frame_ms).await;
    let (receiver_b, accept_b) =
        start_receiver(dir.path(), 1, Arc::clone(&stamps_b), frame_ms).await;

    // 每 10 个音频数据报成串丢 3 个 ≈ 30%，两条链路各自独立注入。
    let relay_a = LossRelay::start(receiver_a.local_addr(), 512, 10, 3).await;
    let relay_b = LossRelay::start(receiver_b.local_addr(), 512, 10, 3).await;

    let mut events_a = receiver_a.subscribe();
    let mut events_b = receiver_b.subscribe();

    let outcome = tokio::time::timeout(Duration::from_secs(90), async {
        let id_a = connect_and_pair(&sender, &receiver_a, relay_a.addr).await;
        let id_b = connect_and_pair(&sender, &receiver_b, relay_b.addr).await;
        wait_streaming(&sender, id_a).await;
        wait_streaming(&sender, id_b).await;

        let _group_id = sender
            .create_group(&[id_a, id_b], lead_ms)
            .await
            .expect("建组");
        sender
            .start_send_many(&[id_a, id_b])
            .await
            .expect("两台一起开流");

        tokio::time::sleep(Duration::from_secs(8)).await;
    })
    .await;
    outcome.expect("90 s 内必须完成配对、建组与开流");

    let a: Vec<Instant> = stamps_a
        .lock()
        .unwrap()
        .iter()
        .map(|stamp| stamp.at)
        .collect();
    let b: Vec<Instant> = stamps_b
        .lock()
        .unwrap()
        .iter()
        .map(|stamp| stamp.at)
        .collect();
    let (samples, absolute_p50, absolute_p95) = deviation_ms(&a, &b);
    let scheduled_a = drain_scheduled(&mut events_a);
    let scheduled_b = drain_scheduled(&mut events_b);
    let dropped = relay_a.dropped() + relay_b.dropped();
    let stats = receiver_a.telemetry(sender_id);
    println!(
        "[group-sync-loss] 重丢包：注入丢弃 {dropped} 个包；排播事件 {scheduled_a}/{scheduled_b}；样本 {samples} · 绝对偏差 P50 {absolute_p50:.2} ms · P95 {absolute_p95:.2} ms；接收侧遥测 {:?}",
        stats.map(|s| (s.nack_count, s.plc_count, s.underruns))
    );

    assert!(dropped > 0, "中继一个音频包都没丢 —— 这条测试失去意义");
    assert!(
        scheduled_a > 0 && scheduled_b > 0,
        "重丢包下两端都必须仍然进入排播（A={scheduled_a} · B={scheduled_b}）"
    );
    // 决定性断言（与 4% 档同一条线）：M3 的验收线不因链路恶劣而失守。
    assert!(
        absolute_p95 <= 10.0,
        "重丢包时组内偏差 P95 = {absolute_p95:.2} ms，超过 M3 的 ±10 ms 验收线"
    );

    accept_a.abort();
    accept_b.abort();
    sender.shutdown().await;
    receiver_a.shutdown().await;
    receiver_b.shutdown().await;
}
/// 动态加入 × 链路丢包：新成员在**链路已经在丢包**时加入同步组，还能拿到基准并跟上吗？
///
/// 这是 `docs/33` §10.2 记的「丢包与动态加入的组合」。新成员拿组基准走的是**控制流**（不受数据报丢包影响），
/// 但它的**播放**要靠数据报 —— 所以要问的是「丢包中新加入的那台能不能与老成员对齐」。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn late_joiner_aligns_under_loss() {
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

    let stamps: Vec<Arc<Mutex<Vec<Stamp>>>> =
        (0..3).map(|_| Arc::new(Mutex::new(Vec::new()))).collect();
    let (receiver_a, accept_a) =
        start_receiver(dir.path(), 0, Arc::clone(&stamps[0]), frame_ms).await;
    let (receiver_b, accept_b) =
        start_receiver(dir.path(), 1, Arc::clone(&stamps[1]), frame_ms).await;
    let (receiver_c, accept_c) =
        start_receiver(dir.path(), 2, Arc::clone(&stamps[2]), frame_ms).await;

    // 三条链路各自注入：每 50 个音频数据报成串丢 2 个（约 4%）。
    let relay_a = LossRelay::start(receiver_a.local_addr(), 512, 50, 2).await;
    let relay_b = LossRelay::start(receiver_b.local_addr(), 512, 50, 2).await;
    let relay_c = LossRelay::start(receiver_c.local_addr(), 512, 50, 2).await;

    let mut events_a = receiver_a.subscribe();
    let mut events_b = receiver_b.subscribe();
    let mut events_c = receiver_c.subscribe();

    let outcome = tokio::time::timeout(Duration::from_secs(120), async {
        let id_a = connect_and_pair(&sender, &receiver_a, relay_a.addr).await;
        let id_b = connect_and_pair(&sender, &receiver_b, relay_b.addr).await;
        let id_c = connect_and_pair(&sender, &receiver_c, relay_c.addr).await;
        wait_streaming(&sender, id_a).await;
        wait_streaming(&sender, id_b).await;
        wait_streaming(&sender, id_c).await;

        // ① 老成员先成组、开流，并在丢包链路上跑一会儿。
        let group_id = sender
            .create_group(&[id_a, id_b], lead_ms)
            .await
            .expect("建组");
        sender
            .start_send_many(&[id_a, id_b])
            .await
            .expect("两台一起开流");
        tokio::time::sleep(Duration::from_secs(3)).await;

        // ② 第 3 台**在丢包进行中**加入同步组，再开流。
        sender
            .join_group(id_c, group_id)
            .await
            .expect("第 3 台加入同步组");
        sender.start_send(id_c).await.expect("第 3 台开流");

        tokio::time::sleep(Duration::from_secs(5)).await;
    })
    .await;
    outcome.expect("120 s 内必须完成配对、建组、动态加入与开流");

    let mut audible = Vec::new();
    let mut times: Vec<Vec<Instant>> = Vec::new();
    for entry in &stamps {
        let guard = entry.lock().unwrap();
        audible.push(guard.iter().filter(|stamp| !stamp.silence).count());
        times.push(guard.iter().map(|stamp| stamp.at).collect());
    }
    let scheduled = [
        drain_scheduled(&mut events_a),
        drain_scheduled(&mut events_b),
        drain_scheduled(&mut events_c),
    ];
    let dropped = relay_a.dropped() + relay_b.dropped() + relay_c.dropped();
    let (samples, absolute_p50, absolute_p95) = deviation_ms(&times[0], &times[1]);
    println!(
        "[group-join-loss] 注入丢弃 {dropped} 个包；三台排播 {scheduled:?} · 非静音写出 {audible:?}；A/B 绝对偏差 P50 {absolute_p50:.2} ms · P95 {absolute_p95:.2} ms（样本 {samples}）"
    );

    assert!(dropped > 0, "中继一个包都没丢 —— 这条测试失去意义");
    assert!(
        scheduled.iter().all(|count| *count > 0),
        "三台都必须排播：{scheduled:?}"
    );
    assert!(
        audible.iter().all(|count| *count > 0),
        "三台都必须出声：{audible:?}"
    );
    assert!(
        absolute_p95 <= 10.0,
        "丢包 + 动态加入时老成员仍在 M3 验收线上：P95 = {absolute_p95:.2} ms"
    );

    accept_a.abort();
    accept_b.abort();
    accept_c.abort();
    sender.shutdown().await;
    receiver_a.shutdown().await;
    receiver_b.shutdown().await;
    receiver_c.shutdown().await;
}
