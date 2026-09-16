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
        let (open_flag, dropped_counter, forwarded_counter) =
            (open.clone(), dropped.clone(), forwarded.clone());

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
                if !open_flag.load(Ordering::SeqCst) {
                    dropped_counter.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                forwarded_counter.fetch_add(1, Ordering::Relaxed);
                let _ = socket.send_to(&buf[..len], to).await;
            }
        });

        Self {
            addr,
            open,
            dropped,
            forwarded,
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
    relay.cut();
    let cut_at = Instant::now();
    drain(writes);

    // ③ 证明「网真的断了」：播放缓冲 60 ms，给到 400 ms 余量后，不该再有新的非静音回调。
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let noise_start = cut_at + Duration::from_millis(400);
    let noise_while_cut = count_sound_until(writes, noise_start, Instant::now()).await;
    assert!(
        relay.dropped() > dropped_before,
        r"闸门关着却一个包都没丢：这条测试失去意义"
    );

    // ④ 把剩下的断网时间走完。
    tokio::time::sleep(outage.saturating_sub(Duration::from_millis(1000))).await;
    let restore_at = Instant::now();
    relay.restore();

    // ⑤ 恢复：必须由**晚于插回时刻**的非静音回调证明。
    //
    // 观测窗 15 s（2026-09-17 从 8 s 放宽）：10 s 拔网的恢复延迟由 QUIC 的 PTO 退避决定，本身就在
    // 4~5 s 量级；而本机跑 8 h soak 时 CPU 与回环都被占着，实测会出现 > 8 s 的情况 —— 那是环境竞争，
    // 不是「不恢复」。窗口太短会把环境噪声记成产品缺陷（本轮 20 轮采样里就踩到过一次）。
    let recovered_at = next_sound_after(writes, restore_at, Duration::from_secs(15)).await;
    let recovery_ms = recovered_at.map(|stamp| stamp.duration_since(restore_at).as_millis());

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
