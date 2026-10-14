//! B 线（内核满队列拒收）的**复现装置与回归护栏**（docs/73 §2 / docs/74 / docs/75 的续集）。
//!
//! 现场症状（PC → Android，10 ms 低延迟档，Wi-Fi）：A（节拍超时）≈ 0，**B（满队列拒收）持续滴漏**
//! （1767 s 窗口 314 次），C（游标追赶）跟随其后。本机干净回环永远造不出 B（docs/74 §5：
//! 「发送节奏钉死供给速率」），因为回环上数据报是**匀速逐个**到达的。
//!
//! 真机 Wi-Fi 不是那样：节电/聚合让包**成批**到达（DTIM 节拍 ~100 ms 一批）。本装置用一个
//! 批式中继复现这个形状：音频大包攒够一个批窗一次性放行，控制流小包即时透传；另外每隔若干批
//! 整批丢弃一次（弱信号喷嚏）。批式到达做两件事：
//!
//! 1. 把到达抖动顶到 ~批窗量级 ⇒ 自适应深度升档（每批内的间隔 ≈ 0，批间 ≈ 批窗）；
//! 2. 每个批窗一次性向待播队列投递 ~批窗/帧长 帧 ⇒ 队列水位在批到达瞬间冲高。
//!
//! 深度顶高 + 批量投递叠加 16 帧的固定容量 ⇒ 溢出拒收。拒收留下永久序号缺口 ⇒ C 追赶 ⇒
//! C 也是欠载 ⇒ 欠载阻止深度降档 ⇒ 深度钉死 ⇒ 下一批继续溢出 —— 自锁环。
//!
//! 这条测试先复现（打印 A/B/C 与水位分布），修复落地后它就是回归护栏：
//! 批式到达 + 偶发整批丢失下 B 必须保持为零（或近零）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use audiolink_audio::{NullPlayout, PlayoutSink, SyntheticCapture};
use audiolink_engine::{Engine, EngineConfig, SessionState, UnderrunBreakdown};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

/// 批窗（~WiFi DTIM 节拍）：音频大包攒这么久后一次性放行。
const BATCH_WINDOW: Duration = Duration::from_millis(100);
/// 音频数据报的尺寸门槛（与 `group_sync_under_loss.rs` 同一口径：≥ 512 B 视为音频）。
const AUDIO_MIN_LEN: usize = 512;

/// 批式中继：音频大包按批窗攒批放行；每 `drop_every_batches` 批整批丢弃一次。
///
/// 小包（握手 / 控制流 / 时钟探测）与下行包即时透传 —— 批窗只作用于音频路径，
/// 否则握手与 §6 探测会被批窗拖死，测的就不是音频链路了。
struct BatchRelay {
    addr: SocketAddr,
    forwarded_audio: Arc<AtomicU64>,
    dropped_audio: Arc<AtomicU64>,
    task: JoinHandle<()>,
}

impl BatchRelay {
    async fn start(upstream: SocketAddr, drop_every_batches: u64) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("中继绑定");
        let addr = socket.local_addr().expect("中继地址");
        let socket = Arc::new(socket);
        let forwarded_audio = Arc::new(AtomicU64::new(0));
        let dropped_audio = Arc::new(AtomicU64::new(0));
        let forwarded = forwarded_audio.clone();
        let dropped_counter = dropped_audio.clone();
        let task = tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let mut client: Option<SocketAddr> = None;
            // 攒批：到达时刻 + 字节。只攒「客户端 → 服务端」的音频大包。
            let mut batch: Vec<Vec<u8>> = Vec::new();
            let mut batches_seen = 0_u64;
            let mut ticker = tokio::time::interval(BATCH_WINDOW);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    received = socket.recv_from(&mut buf) => {
                        let Ok((len, from)) = received else { break };
                        let client_to_server = from != upstream;
                        if client_to_server {
                            client = Some(from);
                            if len >= AUDIO_MIN_LEN {
                                batch.push(buf[..len].to_vec());
                                continue;
                            }
                            let _ = socket.send_to(&buf[..len], upstream).await;
                        } else if let Some(client) = client {
                            let _ = socket.send_to(&buf[..len], client).await;
                        }
                    }
                    _ = ticker.tick() => {
                        if batch.is_empty() {
                            continue;
                        }
                        batches_seen += 1;
                        let drop_this = drop_every_batches > 0
                            && batches_seen.is_multiple_of(drop_every_batches);
                        let count = batch.len() as u64;
                        if drop_this {
                            dropped_counter.fetch_add(count, Ordering::Relaxed);
                        } else {
                            for packet in batch.drain(..) {
                                let _ = socket.send_to(&packet, upstream).await;
                            }
                            forwarded.fetch_add(count, Ordering::Relaxed);
                        }
                        batch.clear();
                    }
                }
            }
        });
        Self {
            addr,
            forwarded_audio,
            dropped_audio,
            task,
        }
    }

    fn forwarded(&self) -> u64 {
        self.forwarded_audio.load(Ordering::Relaxed)
    }

    fn dropped(&self) -> u64 {
        self.dropped_audio.load(Ordering::Relaxed)
    }
}

impl Drop for BatchRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn median(values: &mut [u32]) -> u32 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[values.len() / 2]
}

/// 批式到达 + 每 40 批（≈ 4 s）整批丢弃一次，10 ms 帧，跑 30 s：
/// 打印 A/B/C 三分量与水位分布 —— 这条装置先回答「B 到底是怎么来的」。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queue_rejects_under_wifi_style_bursts() {
    const FRAME_MS: u32 = 10;
    /// 起播预热（攒帧 + 深度爬升）之后再开窗采样。
    const WARMUP: Duration = Duration::from_secs(5);
    const RUN: Duration = Duration::from_secs(30);

    let dir = tempfile::TempDir::new().expect("临时目录");
    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = FRAME_MS;
    send_config.capture = Some(Arc::new(|| {
        Ok(Box::new(SyntheticCapture::new(FRAME_MS, 440.0)?))
    }));
    let sender = Engine::start(send_config).await.expect("发送引擎");

    let mut recv_config = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_config.listen = "127.0.0.1:0".parse().unwrap();
    recv_config.codec.frame_ms = FRAME_MS;
    recv_config.playout = Some(Arc::new(|| {
        Ok(Box::new(NullPlayout::new(60)) as Box<dyn PlayoutSink>)
    }));
    let receiver = Engine::start(recv_config).await.expect("接收引擎");
    let accept = receiver.spawn_accept_loop();
    let receiver_id = receiver.info().id;
    let sender_id = sender.info().id;

    // 每 40 批（≈ 4 s）整批丢一次：一次丢 ~10 个音频包 ≈ 100 ms 的洞。
    let relay = BatchRelay::start(receiver.local_addr(), 40).await;

    let outcome = tokio::time::timeout(Duration::from_secs(120), async {
        sender
            .connect(relay.addr)
            .await
            .expect("连接应当直接成功（无认证）");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !sender
            .peers()
            .iter()
            .any(|peer| peer.id == receiver_id && peer.state == SessionState::Streaming)
        {
            assert!(
                Instant::now() < deadline,
                "会话没有在 20 s 内进入 streaming"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        sender.start_send(receiver_id).await.expect("开流");

        tokio::time::sleep(WARMUP).await;
        let base_stats = receiver.telemetry(sender_id).expect("遥测应在");
        let base_breakdown = receiver.underrun_breakdown(sender_id).expect("分量账应在");

        let mut levels: Vec<u32> = Vec::new();
        let deadline = Instant::now() + RUN;
        while Instant::now() < deadline {
            if let Some(stats) = receiver.telemetry(sender_id) {
                levels.push(stats.buffer_level_us);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        let last_stats = receiver.telemetry(sender_id).expect("遥测应在");
        let last_breakdown = receiver.underrun_breakdown(sender_id).expect("分量账应在");
        (
            levels,
            last_stats.late_drops.saturating_sub(base_stats.late_drops),
            last_stats.underruns.saturating_sub(base_stats.underruns),
            last_stats.plc_count.saturating_sub(base_stats.plc_count),
            last_breakdown.saturating_delta_since(base_breakdown),
        )
    })
    .await;

    let (mut levels, late_delta, underrun_delta, plc_delta, breakdown) =
        outcome.expect("120 s 内必须完成连接、开流与 30 s 采样");

    sender.shutdown().await;
    accept.abort();
    receiver.shutdown().await;

    let median_level = median(&mut levels);
    let peak_level = levels.iter().copied().max().unwrap_or(0);
    println!(
        "[queue-reject-burst] 10 ms 帧 × 批窗 {} ms × {RUN:?}：中继转发 {} / 丢弃 {} 个音频包；\
         水位 P50 {} µs · 峰值 {} µs；增量：underruns {underrun_delta} · late_drops {late_delta} · plc {plc_delta}；\
         A {} · B {} · C {}",
        BATCH_WINDOW.as_millis(),
        relay.forwarded(),
        relay.dropped(),
        median_level,
        peak_level,
        breakdown.schedule_slips,
        breakdown.queue_rejects,
        breakdown.cursor_catchups,
    );

    // 前提：中继真的在干活（转发与丢弃都发生了），否则这条测试什么也没证明。
    assert!(relay.forwarded() > 0, "中继没有转发任何音频包");
    assert!(
        relay.dropped() > 0,
        "中继没有丢弃任何音频包 —— 丢包注入失效"
    );
    assert!(!levels.is_empty(), "水位采样为空（遥测没更新？）");

    // 回归判据：批式到达 + 偶发整批丢失是 Wi-Fi 的常态，不是异常 —— 它不该撞出满队列拒收。
    // B 的每一次都是一帧真音频或掩盖帧被丢 + 一个永久序号缺口（docs/75 的恢复只是止血）。
    assert_eq!(
        breakdown.queue_rejects, 0,
        "批式到达下出现 {} 次满队列拒收（B 线）—— 队列容量对 10 ms 档的自适应深度没有余量",
        breakdown.queue_rejects
    );
    // A 线必须保持近零：播放节拍不该被批量投递打乱（本机回环，没有真机调度问题）。
    assert!(
        breakdown.schedule_slips <= 2,
        "批式到达下出现 {} 次节拍超时（A 线）—— 投递节奏影响了播放节拍",
        breakdown.schedule_slips
    );
    let _ = UnderrunBreakdown::default();
}
