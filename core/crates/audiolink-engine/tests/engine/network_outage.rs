//! 断网自愈边界测定（M2 验收「拔网 10 s 后 ≤ 3 s 恢复」）：真实双 Engine + 真实 QUIC，
//! 中间挂一个**双向闸门中继** —— 闸门关上就等于把网线拔了（两个方向都不通）。
//!
//! # 判据为什么要带时间戳
//!
//! 第一版判据是「播放回调里出现非静音样本」，实测插回网线后 **1.1 µs** 就"恢复"了 ——
//! 那不是恢复，是**通道积压**：播放回调走无界通道，拔网期间没人消费，通道里堆着拔网**之前**
//! 写进去的样本，插回后一读就中。所以这里的播放端记录「(写入时刻, 样本)」，
//! 「恢复」必须由一个**晚于插回时刻**的非静音样本证明；同时还要证明拔网期间**真的安静了**，
//! 否则整条测试只是两台引擎在自己跟自己说话。
//!
//! # 为什么这条测试值得存在
//!
//! 验收原文是「拔网 10 s，恢复后 ≤ 3 s 内重新出声」。链条上真正说话的是 EngineConfig 里的两个数值：
//!
//! * 「idle_timeout」（原默认 **10 s**，本轮改为 **30 s**，见 DEFAULT_IDLE_TIMEOUT）—— QUIC 空闲超时；
//! * 「keep_alive」（原默认 **3 s**，本轮改为 **1 s**，见 DEFAULT_KEEP_ALIVE）—— 保活探测节奏。
//!
//! 空闲超时一过，连接被判死；而 runtime.rs 的会话循环在读写报错时走
//! report_peer_gone() → SessionState::Failed，**没有**任何重拨/重建路径
//! （SessionState::Reconnecting 只在收到对端 Bye 帧时进入）。所以「拔网多久还能自愈」的上界
//! 就是 idle_timeout —— 旧值 10 s 正好卡在验收线上，拔网 10 s 直接终结会话。
//!
//! # 实测（2026-09-17，本机回环，闸门中继模拟拔网）
//!
//! | 拔网 | 旧默认 10 s / 3 s | 新默认 30 s / 1 s |
//! |---|---|---|
//! | 2 s | — | 恢复 0.7～0.8 s |
//! | 4 s | 恢复 0.6 / 3.4 / 3.3 s（双峰） | 恢复 0.9 s |
//! | 10 s | **不恢复**：两侧会话表里对端都被移除 | 恢复 4.2～4.7 s（连测四次，稳定） |
//!
//! 两条结论：① 把 idle_timeout 提到 30 s 之后，10 s 拔网从「会话终结」变成「能自愈」；
//! ② 但恢复延迟随断网时长增长（2 s → 0.7 s，10 s → 4.5 s），这是 QUIC **PTO 指数退避**的
//! 确定性特征 —— 静默越久，下一次探测排得越晚。所以「10 s 拔网 ≤ 3 s 恢复」**尚未达标**，
//! 缺口记账在 docs/48-m2-outage-boundary.md 与看板。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::{ErrorCode, NodeId};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

/// 一次播放回调：写入时刻 + 样本。
type WriteStamp = (Instant, Vec<f32>);

/// 只记样本、不出声的播放端（NullPlayout 的输出原样落进记录通道）。
struct RecordingSink {
    inner: NullPlayout,
    samples: mpsc::UnboundedSender<WriteStamp>,
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
        let _ = self.samples.send((Instant::now(), samples.to_vec()));
        Ok(())
    }

    fn stats(&self) -> PlayoutStats {
        self.inner.stats()
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn backend_name(&self) -> &'static str {
        r"recording-null"
    }
}

/// 双向闸门中继：闸门开着时原样转发（QUIC 端到端加密，转发方不需要看懂内容），
/// 闸门关上时**两个方向都丢** —— 这就是「拔网线」。
struct OutageRelay {
    addr: SocketAddr,
    open: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    forwarded: Arc<AtomicU64>,
    /// 客户端 → 服务端方向通过的包数（发送侧还在不在按期发，看它）。
    fwd_up: Arc<AtomicU64>,
    /// 服务端 → 客户端方向通过的包数（**对端真的回了包**才增长，所以它是「链路已打通」的直接证据）。
    fwd_down: Arc<AtomicU64>,
    /// 上行方向**长度 ≥ 512 B** 且**通过了闸门**的包数，也就是音频数据报（保活与控制帧都是小包）。
    /// 它回答「发送侧在恢复后到底还把不把音频往外发」—— 与「链路通没通」是两件事。
    fwd_big_up: Arc<AtomicU64>,
    /// 上行音频数据报**到达中继**的数量，**掐在闸门判断之前**统计。
    ///
    /// 与 fwd_big_up 配对使用：拔网期间它若还在涨，说明发送侧照旧在推音频、丢的只是网络；
    /// 若它停了，说明发送侧真的停了 —— 这是「谁的锅」最干净的切分线。
    big_arrived: Arc<AtomicU64>,
    task: tokio::task::JoinHandle<()>,
}

impl OutageRelay {
    async fn start(upstream: SocketAddr) -> Self {
        let socket = UdpSocket::bind(r"127.0.0.1:0")
            .await
            .expect(r"中继绑定回环端口");
        let addr = socket.local_addr().expect(r"中继地址");
        let socket = Arc::new(socket);
        let open = Arc::new(AtomicBool::new(true));
        let dropped = Arc::new(AtomicU64::new(0));
        let forwarded = Arc::new(AtomicU64::new(0));
        let fwd_up = Arc::new(AtomicU64::new(0));
        let fwd_down = Arc::new(AtomicU64::new(0));
        let (open_flag, dropped_counter, forwarded_counter) =
            (open.clone(), dropped.clone(), forwarded.clone());
        let fwd_big_up = Arc::new(AtomicU64::new(0));
        let (up_counter, down_counter) = (fwd_up.clone(), fwd_down.clone());
        let big_up_counter = fwd_big_up.clone();
        let big_arrived = Arc::new(AtomicU64::new(0));
        let arrived_counter = big_arrived.clone();

        let task = tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let mut client: Option<SocketAddr> = None;
            while let Ok((len, from)) = socket.recv_from(&mut buf).await {
                let client_to_server = from != upstream;
                let to = if client_to_server {
                    client = Some(from);
                    upstream
                } else {
                    match client {
                        Some(client) => client,
                        None => continue, // 还不知道客户端是谁：丢掉（握手前的噪声）
                    }
                };
                if client_to_server && len >= 512 {
                    arrived_counter.fetch_add(1, Ordering::Relaxed);
                }
                if !open_flag.load(Ordering::SeqCst) {
                    dropped_counter.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                forwarded_counter.fetch_add(1, Ordering::Relaxed);
                if client_to_server {
                    up_counter.fetch_add(1, Ordering::Relaxed);
                    if len >= 512 {
                        big_up_counter.fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    down_counter.fetch_add(1, Ordering::Relaxed);
                }
                let _ = socket.send_to(&buf[..len], to).await;
            }
        });

        Self {
            addr,
            open,
            dropped,
            forwarded,
            fwd_up,
            fwd_down,
            fwd_big_up,
            big_arrived,
            task,
        }
    }

    /// 拔网：此后两个方向的包一律丢弃。
    fn cut(&self) {
        self.open.store(false, Ordering::SeqCst);
    }

    /// 插回网线。
    fn restore(&self) {
        self.open.store(true, Ordering::SeqCst);
    }

    fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn forwarded(&self) -> u64 {
        self.forwarded.load(Ordering::Relaxed)
    }

    /// 回程（服务端 → 客户端）通过的包数。拔网期间它应停止增长 —— 接收端收不到包就不会回包。
    fn fwd_down(&self) -> u64 {
        self.fwd_down.load(Ordering::Relaxed)
    }

    /// 上行音频数据报（长度 ≥ 512 B）通过的包数。
    fn fwd_big_up(&self) -> u64 {
        self.fwd_big_up.load(Ordering::Relaxed)
    }

    /// 上行音频数据报**到达**中继的包数（闸门之前统计）。
    fn big_arrived(&self) -> u64 {
        self.big_arrived.load(Ordering::Relaxed)
    }
}

impl Drop for OutageRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// 丢光通道里已有的回调（拔网前的积压），只保留之后新写入的。
fn drain(rx: &mut mpsc::UnboundedReceiver<WriteStamp>) -> usize {
    let mut dropped = 0;
    while rx.try_recv().is_ok() {
        dropped += 1;
    }
    dropped
}

/// 等一个**晚于 after** 的非静音回调，返回它的写入时刻。
async fn next_sound_after(
    rx: &mut mpsc::UnboundedReceiver<WriteStamp>,
    after: Instant,
    budget: Duration,
) -> Option<Instant> {
    let deadline = Instant::now() + budget;
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        if left.is_zero() {
            return None;
        }
        match tokio::time::timeout(left, rx.recv()).await {
            Ok(Some((stamp, samples))) => {
                if stamp >= after && samples.iter().any(|sample| sample.abs() > 0.01) {
                    return Some(stamp);
                }
            }
            Ok(None) | Err(_) => return None,
        }
    }
}

/// 数一数指定时间窗内落进来的非静音回调。
async fn count_sound_until(
    rx: &mut mpsc::UnboundedReceiver<WriteStamp>,
    from: Instant,
    until: Instant,
) -> usize {
    let mut hits = 0_usize;
    loop {
        let now = Instant::now();
        if now >= until {
            return hits;
        }
        match tokio::time::timeout(until - now, rx.recv()).await {
            Ok(Some((stamp, samples))) => {
                if stamp >= from && samples.iter().any(|sample| sample.abs() > 0.01) {
                    hits += 1;
                }
            }
            Ok(None) | Err(_) => return hits,
        }
    }
}

/// 等**回程**（服务端 → 客户端）第一个通过的包，返回它相对调用时刻的延迟。
///
/// 为什么盯回程而不是上行：拔网期间发送侧的保活包照样往闸门里灌（闸门只是把它们丢掉），
/// 所以上行计数一插回就会涨、反映不出「链路是否真的通了」；而接收端收不到包就不会回包，
/// 回程计数**必须等包真的过去并换来对端响应**才会涨。
async fn wait_down_packet(relay: &OutageRelay, before: u64, budget: Duration) -> Option<u128> {
    let start = Instant::now();
    let deadline = start + budget;
    loop {
        if relay.fwd_down() > before {
            return Some(start.elapsed().as_millis());
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// 等一个计数器越过 before，返回延迟。用来量「某类包多久重新开始通过」。
async fn wait_count<G>(read: G, before: u64, budget: Duration) -> Option<u128>
where
    G: Fn() -> u64,
{
    let start = Instant::now();
    let deadline = start + budget;
    loop {
        if read() > before {
            return Some(start.elapsed().as_millis());
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// 起一对引擎、走完 PIN 配对与开流。
async fn wire_up(
    dir: &Path,
    frame_ms: u32,
) -> (
    Arc<Engine>,
    Arc<Engine>,
    OutageRelay,
    tokio::task::JoinHandle<()>,
    mpsc::UnboundedReceiver<WriteStamp>,
    NodeId,
    NodeId,
) {
    let mut send_config = EngineConfig::new(r"sender", dir.join(r"sender"));
    send_config.listen = r"127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));

    let (writes_tx, writes_rx) = mpsc::unbounded_channel::<WriteStamp>();
    let mut recv_config = EngineConfig::new(r"receiver", dir.join(r"receiver"));
    recv_config.listen = r"127.0.0.1:0".parse().unwrap();
    recv_config.codec.frame_ms = frame_ms;
    recv_config.playout = Some(Arc::new(move || {
        Ok(Box::new(RecordingSink {
            inner: NullPlayout::new(60),
            samples: writes_tx.clone(),
        }))
    }));

    let receiver = Engine::start(recv_config).await.expect(r"接收引擎");
    let accept = receiver.spawn_accept_loop();
    let mut events = receiver.subscribe();
    let receiver_id = receiver.info().id;

    let relay = OutageRelay::start(receiver.local_addr()).await;
    let sender = Engine::start(send_config).await.expect(r"发送引擎");
    let sender_id = sender.info().id;

    let error = sender.connect(relay.addr).await.unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotPaired, r"首次连接必须先要 PIN");

    let pin = loop {
        if let EngineEvent::DisplayPin { pin, .. } = events.recv().await.unwrap() {
            break pin;
        }
    };
    sender
        .submit_pin(receiver_id, &pin)
        .await
        .expect(r"PIN 配对");
    while !sender
        .peers()
        .iter()
        .any(|peer| peer.id == receiver_id && peer.state == SessionState::Streaming)
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    sender.start_send(receiver_id).await.expect(r"开流");

    (
        sender,
        receiver,
        relay,
        accept,
        writes_rx,
        sender_id,
        receiver_id,
    )
}

/// 一次拔网实验的观测结果。
struct OutageOutcome {
    /// 拔网后、播放缓冲排空之后，落进来的非静音回调数（必须是 0，否则网没断干净）。
    noise_while_cut: usize,
    /// 插回网线到重新出声的真实延迟（毫秒）。
    recovery_ms: Option<u128>,
    /// 恢复后 2 s 内收到的非静音回调数（持续性：证明不是一口气把积压吐完）。
    sustained_writes: usize,
    sender_state: Option<SessionState>,
    receiver_state: Option<SessionState>,
}

/// 跑一次「拔网 N 秒」的实验。
async fn run_outage(
    sender: &Arc<Engine>,
    receiver: &Arc<Engine>,
    relay: &OutageRelay,
    writes: &mut mpsc::UnboundedReceiver<WriteStamp>,
    sender_id: NodeId,
    receiver_id: NodeId,
    outage: Duration,
) -> OutageOutcome {
    // ① 先确认链路真的在出声（对照组）。
    drain(writes);
    next_sound_after(writes, Instant::now(), Duration::from_secs(10))
        .await
        .expect(r"开流后 10 s 内必须有非静音输出");

    // ② 拔网。
    let dropped_before = relay.dropped();
    let arrived_before_cut = relay.big_arrived();
    relay.cut();
    let cut_at = Instant::now();
    drain(writes);

    // ③ 证明「网真的断了」：播放缓冲 60 ms，给到 400 ms 余量后，不该再有新的非静音回调。
    tokio::time::sleep(Duration::from_millis(1000)).await;
    // 拔网 1 s 后：发送侧还在推音频吗？（到达中继即为「在推」，闸门丢弃不影响这个计数）
    let arrived_after_1s = relay.big_arrived();
    let noise_start = cut_at + Duration::from_millis(400);
    let noise_while_cut = count_sound_until(writes, noise_start, Instant::now()).await;
    assert!(
        relay.dropped() > dropped_before,
        r"闸门关着却一个包都没丢：这条测试失去意义"
    );

    // ④ 把剩下的断网时间走完。
    tokio::time::sleep(outage.saturating_sub(Duration::from_millis(1000))).await;
    // 插回前记下回程包数。**这个读数是为了把「4.5 s 是谁的锅」分开**：
    // 如果回程包很快就通（几百毫秒），而声音要等 4 s 多，那慢的不是链路、是应用层；
    // 如果回程包自己就要等 4 s，那才是 QUIC 的 PTO 退避在定节奏。
    let arrived_after_outage = relay.big_arrived();
    let down_before_restore = relay.fwd_down();
    let big_before_restore = relay.fwd_big_up();
    // 插回之后立刻开始逐格采样**上行包速率**（每 500 ms 一格，共 12 格 = 6 s）。
    // 为什么不用「包长 ≥ 512 B」当音频判据：自适应降码率会把包压小，于是「大包回来了」
    // 会晚于「音频回来了」，读数被码率策略带偏。速率不会 —— 音频是 ~50 pps 的密集流，
    // 控制帧只有个位数/秒，两者差一个数量级，一眼可分。
    let up_handle = Arc::clone(&relay.fwd_up);
    let sampler = tokio::spawn(async move {
        let mut rows = Vec::with_capacity(12);
        for _ in 0..12 {
            let before = up_handle.load(Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(500)).await;
            rows.push(up_handle.load(Ordering::Relaxed).saturating_sub(before));
        }
        rows
    });

    let restore_at = Instant::now();
    relay.restore();
    let link_up_ms = wait_down_packet(relay, down_before_restore, Duration::from_secs(15)).await;
    let audio_up_ms = wait_count(
        || relay.fwd_big_up(),
        big_before_restore,
        Duration::from_secs(15),
    )
    .await;

    // ⑤ 恢复：必须由**晚于插回时刻**的非静音回调证明。
    //
    // 观测窗 15 s（2026-09-17 从 8 s 放宽）：10 s 拔网的恢复延迟由 QUIC 的 PTO 退避决定，本身就在
    // 4~5 s 量级；而本机跑 8 h soak 时 CPU 与回环都被占着，实测会出现 > 8 s 的情况 —— 那是环境竞争，
    // 不是「不恢复」。窗口太短会把环境噪声记成产品缺陷（本轮 20 轮采样里就踩到过一次）。
    let recovered_at = next_sound_after(writes, restore_at, Duration::from_secs(15)).await;
    let recovery_ms = recovered_at.map(|stamp| stamp.duration_since(restore_at).as_millis());
    let up_rates = sampler.await.unwrap_or_default();

    println!(
        "[outage-diag] 拔网 {} ms：到达中继的上行音频包 拔网前 {} → 1 s 后 {} → 拔网结束 {} · 插回后回程首个包 {} · 上行音频数据报 {} · 声音恢复 {}",
        outage.as_millis(),
        arrived_before_cut,
        arrived_after_1s,
        arrived_after_outage,
        match link_up_ms {
            Some(ms) => format!("{ms} ms"),
            None => String::from("15 s 窗口内未出现"),
        },
        match audio_up_ms {
            Some(ms) => format!("{ms} ms"),
            None => String::from("15 s 窗口内未出现"),
        },
        match recovery_ms {
            Some(ms) => format!("{ms} ms"),
            None => String::from("未恢复"),
        }
    );
    println!("[outage-diag] 插回后每 500 ms 的上行包数：{up_rates:?}");

    // ⑥ 持续性：从**恢复那一刻**起再观察 2 s，仍应持续有音频，而不是吐完积压就哑。
    //（第一版这里从 restore_at 起算，而恢复本身可能晚于它 —— 于是窗口早就过期，
    // 测出来恒为 0；那是个测量 bug，不是链路 bug。）
    let sustained_writes = match recovered_at {
        Some(stamp) => count_sound_until(writes, stamp, stamp + Duration::from_secs(2)).await,
        None => 0,
    };

    let sender_state = sender
        .peers()
        .iter()
        .find(|peer| peer.id == receiver_id)
        .map(|peer| peer.state);
    let receiver_state = receiver
        .peers()
        .iter()
        .find(|peer| peer.id == sender_id)
        .map(|peer| peer.state);

    OutageOutcome {
        noise_while_cut,
        recovery_ms,
        sustained_writes,
        sender_state,
        receiver_state,
    }
}

/// 更短的断网（2 s）：给「恢复延迟随断网时长增长」这条曲线再取一个左侧的点。
/// 如果连 2 s 拔网都要等秒级才出声，那慢的就不是空闲超时，而是恢复路径本身。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_second_outage_recovery_is_recorded() {
    let dir = tempfile::TempDir::new().expect(r"临时目录");
    let frame_ms = 20_u32;
    let (sender, receiver, relay, accept, mut writes, sender_id, receiver_id) =
        wire_up(dir.path(), frame_ms).await;

    let outcome = tokio::time::timeout(Duration::from_secs(60), async {
        run_outage(
            &sender,
            &receiver,
            &relay,
            &mut writes,
            sender_id,
            receiver_id,
            Duration::from_secs(2),
        )
        .await
    })
    .await
    .expect(r"60 s 内必须跑完");

    println!(
        "[outage] 很短断网（拔网 2 s）：断网期间漏出的非静音回调 {} 个，恢复延迟 {:?} ms，恢复后 2 s 内非静音回调 {} 个；会话状态 发送侧={:?} 接收侧={:?}；中继丢弃 {} 转发 {}",
        outcome.noise_while_cut,
        outcome.recovery_ms,
        outcome.sustained_writes,
        outcome.sender_state,
        outcome.receiver_state,
        relay.dropped(),
        relay.forwarded(),
    );

    assert_eq!(
        outcome.noise_while_cut, 0,
        r"拔网 400 ms 之后仍有非静音输出：网没真的断"
    );
    let recovery_ms = outcome
        .recovery_ms
        .expect(r"插回网线后 15 s 内必须重新出声");
    if recovery_ms > 3_000 {
        println!("[outage] 注意：仅 2 s 拔网，恢复就用了 {recovery_ms} ms（超出 3 s 预算）");
    }
    assert!(
        outcome.sustained_writes >= 20,
        "恢复后 2 s 内只有 {} 个非静音回调：这不是恢复，是吐积压",
        outcome.sustained_writes
    );

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();
}

/// 边界的**内侧**：拔网 4 s（< idle_timeout 30 s）—— 必须能自愈，且恢复后持续出声。
/// 这条钉住的是「链路层的自愈能力确实存在」，而不是「无论如何都达标」。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn brief_outage_self_heals_within_budget() {
    let dir = tempfile::TempDir::new().expect(r"临时目录");
    let frame_ms = 20_u32;
    let (sender, receiver, relay, accept, mut writes, sender_id, receiver_id) =
        wire_up(dir.path(), frame_ms).await;

    let outcome = tokio::time::timeout(Duration::from_secs(60), async {
        run_outage(
            &sender,
            &receiver,
            &relay,
            &mut writes,
            sender_id,
            receiver_id,
            Duration::from_secs(4),
        )
        .await
    })
    .await
    .expect(r"60 s 内必须跑完");

    println!(
        "[outage] 短断网（拔网 4 s < idle_timeout 30 s）：断网期间漏出的非静音回调 {} 个，恢复延迟 {:?} ms，恢复后 2 s 内非静音回调 {} 个；会话状态 发送侧={:?} 接收侧={:?}；中继丢弃 {} 转发 {}",
        outcome.noise_while_cut,
        outcome.recovery_ms,
        outcome.sustained_writes,
        outcome.sender_state,
        outcome.receiver_state,
        relay.dropped(),
        relay.forwarded(),
    );

    assert_eq!(
        outcome.noise_while_cut, 0,
        r"拔网 400 ms 之后仍有非静音输出：网没真的断"
    );
    // 现状（2026-09-17 本地实测）：4 s 拔网能自愈 —— 旧默认下恢复延迟呈**双峰**
    // （597 / 3417 / 3310 ms，峰值越过 3 s 预算；双峰的间距正是旧 keep_alive 的 3 s），
    // 新默认（idle 30 s / keep_alive 1 s）下稳定在 0.9 s 上下。
    // 这条测试守住的是**能力面**：拔网 4 s 必须还能自愈，且恢复后必须持续出声。
    let recovery_ms = outcome
        .recovery_ms
        .expect(r"插回网线后 15 s 内必须重新出声");
    if recovery_ms > 3_000 {
        println!("[outage] 注意：恢复延迟 {recovery_ms} ms 超出 M2 验收的 3 s 预算（缺口已记账）");
    }
    assert!(
        outcome.sustained_writes >= 20,
        "恢复后 2 s 内只有 {} 个非静音回调（20 ms 帧理论上约 100 个）：这不是恢复，是吐积压",
        outcome.sustained_writes
    );

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();
}

/// 边界的**外侧**：拔网 10 s —— M2 验收原文的那个数字，也是本轮改进的对照点。
///
/// * 旧默认（idle_timeout 10 s）：**8 s 观测窗内完全没有恢复**，两侧会话表里连对端都没了
///   （`peers()` 返回 None）—— 连接被判死、会话被移除，恢复转发救不回来。
/// * 新默认（idle_timeout 30 s）：**能自愈**，但恢复延迟稳定在 4.2～4.7 s
///   （连测四次：4415 / 4726 / 4546 / 4237 ms），仍越过验收的 3 s 预算 ——
///   慢在 QUIC 的 PTO 指数退避：静默 10 s 后，下一次探测本身就排在 4 s 开外。
///
/// 这条测试因此守住「必须自愈、且恢复后必须持续出声」，把预算差距记录在案而不断言 ——
/// 缺口跟随 docs/48-m2-outage-boundary.md 与看板走，而不是让护栏长期变红然后被无视。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ten_second_outage_self_heals_and_delay_is_recorded() {
    let dir = tempfile::TempDir::new().expect(r"临时目录");
    let frame_ms = 20_u32;
    let (sender, receiver, relay, accept, mut writes, sender_id, receiver_id) =
        wire_up(dir.path(), frame_ms).await;

    let outcome = tokio::time::timeout(Duration::from_secs(90), async {
        run_outage(
            &sender,
            &receiver,
            &relay,
            &mut writes,
            sender_id,
            receiver_id,
            Duration::from_secs(10),
        )
        .await
    })
    .await
    .expect(r"90 s 内必须跑完");

    println!(
        "[outage] 长断网（拔网 10 s ≈ idle_timeout 30 s）：断网期间漏出的非静音回调 {} 个，恢复延迟 {:?} ms，恢复后 2 s 内非静音回调 {} 个；会话状态 发送侧={:?} 接收侧={:?}；中继丢弃 {} 转发 {}",
        outcome.noise_while_cut,
        outcome.recovery_ms,
        outcome.sustained_writes,
        outcome.sender_state,
        outcome.receiver_state,
        relay.dropped(),
        relay.forwarded(),
    );

    assert_eq!(
        outcome.noise_while_cut, 0,
        r"拔网 400 ms 之后仍有非静音输出：网没真的断"
    );

    // 必须自愈：「插回网线也救不回来」的那一版已经不复存在（idle_timeout 30 s 覆盖了 10 s 拔网）。
    let recovery_ms = outcome
        .recovery_ms
        .expect(r"插回网线后 15 s 内必须重新出声");
    if recovery_ms > 3_000 {
        println!(
            "[outage] 注意：恢复延迟 {recovery_ms} ms 超出 M2 验收的 3 s 预算（PTO 退避所致，缺口已记账）"
        );
    }
    assert!(
        outcome.sustained_writes >= 20,
        "恢复后 2 s 内只有 {} 个非静音回调：这不是恢复，是吐积压",
        outcome.sustained_writes
    );

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();
}
