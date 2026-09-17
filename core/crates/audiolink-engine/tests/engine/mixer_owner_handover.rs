//! FR-27 / M4：**引擎级混音器的 owner 让位与接管**（`docs/51-fr27-reconnect-audit.md`）。
//!
//! 审计 §2.2 记录的缺口是：`acquire_playout_mixer` 曾把「混音器槽里已经有东西」当成
//! 「我不是 owner」，而槽**从第一次建起永不置空** —— 于是接收侧第一条会话结束、第二条会话建立时，
//! 新会话拿到 `is_owner = false`，不建 sink，每帧只把 PCM 混进混音器、**从不写播放设备**。
//! 这就是「两侧 Streaming 却没有声音」的机制。
//!
//! 修复（同文档 §3.1 方案 1 的落地）把 owner 从「一次性事实」变成**可让位、可接管**的身份：
//! `PlayoutMixSlot.owner` 是当前 owner 的**源号**（0 = 无人持有设备），新会话用
//! `compare_exchange(0 -> 我的源号)` 抢，owner 的退出路径用 `compare_exchange(我的源号 -> 0)` 让位 ——
//! 只有仍是当前 owner 的那一路清得掉，旧 owner 迟到的退出不会误清新 owner。
//!
//! 但这条修复此前**没有任何测试守着**（`tests/engine/` 全目录 grep `is_owner|owner_slot|factory`
//! 零命中），任何后续改动都能静默把它改回去。本文件补上审计 §3.5 判据 ⑧ 与 §7 第 189 行
//! 点名要求的两条回归：
//!
//! * `owner_handover_reopens_the_playout_device` —— 判据 ⑧：owner 会话退出后，下一条会话建立播放管线时
//!   播放设备必须被**重新打开**（playout factory 调用计数 +1），且混音器对象是**复用**的（槽不置空）；
//! * `other_sources_keep_playing_after_the_owner_leaves` —— §189：owner 先断，另一路仍要出声。
//!   现有 `mixer_convergence.rs` 只测了 `set_peer_gain(0)` 静音，没覆盖 owner 退出。
//!
//! 判据 ⑧ 的唯一口径是「设备被打开的次数」：`spawn_playout_thread` 只在 `is_owner` 分支调用
//! `EngineConfig.playout` 工厂，所以测试侧在工厂里自增一个 `AtomicU64` 就是那条读数。
//! 与之配对的是 `CountingSink` 的 `write` 计数 —— 「打开了」与「真的在写」是两件事，两个都要卡。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::ErrorCode;
use tokio::sync::mpsc;

/// 「播放设备被打开的次数」（判据 ⑧）。
type OpenCounter = Arc<AtomicU64>;

/// 只记样本与写入次数、不出声的播放端。
struct CountingSink {
    inner: NullPlayout,
    writes: Arc<AtomicU64>,
    samples: mpsc::UnboundedSender<Vec<f32>>,
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
        self.writes.fetch_add(1, Ordering::SeqCst);
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
        "counting-handover"
    }
}

/// 接收端 + 它的三个读数：设备打开次数、sink 写入次数、写出的样本通道。
struct Receiver {
    engine: Arc<Engine>,
    accept: tokio::task::JoinHandle<()>,
    samples: mpsc::UnboundedReceiver<Vec<f32>>,
    opens: OpenCounter,
    writes: Arc<AtomicU64>,
}

impl Receiver {
    fn opens(&self) -> u64 {
        self.opens.load(Ordering::SeqCst)
    }

    fn writes(&self) -> u64 {
        self.writes.load(Ordering::SeqCst)
    }

    fn sources(&self) -> Option<usize> {
        self.engine.mixer_stats().map(|stats| stats.sources)
    }
}

async fn start_receiver(dir: &Path, frame_ms: u32) -> Receiver {
    let opens: OpenCounter = Arc::new(AtomicU64::new(0));
    let writes = Arc::new(AtomicU64::new(0));
    let (samples_tx, samples_rx) = mpsc::unbounded_channel::<Vec<f32>>();

    let mut config = EngineConfig::new("receiver", dir.join("receiver"));
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.codec.frame_ms = frame_ms;
    let (opens_factory, writes_factory) = (Arc::clone(&opens), Arc::clone(&writes));
    config.playout = Some(Arc::new(move || {
        // 判据 ⑧ 的唯一口径：工厂**只在 owner 分支**被调用，所以这里自增就是
        // 「播放设备被打开的次数」。
        opens_factory.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(CountingSink {
            inner: NullPlayout::new(60),
            writes: Arc::clone(&writes_factory),
            samples: samples_tx.clone(),
        }) as Box<dyn PlayoutSink>)
    }));

    let engine = Engine::start(config).await.expect("接收引擎");
    let accept = engine.spawn_accept_loop();
    Receiver {
        engine,
        accept,
        samples: samples_rx,
        opens,
        writes,
    }
}

fn sender_config(dir: &Path, name: &str, frequency: f32, frame_ms: u32) -> EngineConfig {
    let mut config = EngineConfig::new(name, dir.join(name));
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.codec.frame_ms = frame_ms;
    config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, frequency)?))
    }));
    config
}

/// 与接收端走完 PIN 配对，返回接收端 id。
async fn pair_with_receiver(
    sender: &Arc<Engine>,
    receiver: &Arc<Engine>,
) -> audiolink_types::NodeId {
    let receiver_id = receiver.info().id;
    let mut events = receiver.subscribe();
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
    let paired = wait_for(Duration::from_secs(10), || {
        sender
            .peers()
            .iter()
            .any(|peer| peer.id == receiver_id && peer.state == SessionState::Streaming)
    })
    .await;
    assert!(paired, "发送侧没有在 10 s 内进入 Streaming");
    receiver_id
}

/// 有界等待：条件在 `budget` 内成立返回 `true`，否则返回 `false`。
///
/// 刻意**不在这里 panic**：条件不成立时把结论交给调用方的断言，失败信息里才带得上实测读数
/// （「打开次数停在 1」比「10 s 超时」有用得多）。
async fn wait_for(budget: Duration, condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// 在一段窗口里把 sink 写出的样本收干，返回 RMS（0 = 这一窗里设备一个样本都没写出来）。
async fn rms_over(rx: &mut mpsc::UnboundedReceiver<Vec<f32>>, window: Duration) -> f32 {
    let deadline = Instant::now() + window;
    let mut all = Vec::new();
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        match tokio::time::timeout(deadline - now, rx.recv()).await {
            Ok(Some(chunk)) => all.extend_from_slice(&chunk),
            Ok(None) | Err(_) => break,
        }
    }
    if all.is_empty() {
        return 0.0;
    }
    (all.iter().map(|value| value * value).sum::<f32>() / all.len() as f32).sqrt()
}

/// **判据 ⑧**：owner 会话退出后，下一条会话建立播放管线时**播放设备必须被重新打开**。
///
/// 修复前这条必然失败：槽永不清空 ⇒ 恒 `is_owner = false` ⇒ 工厂一次都不会被再调用
/// （计数停在 1，且新会话是个「活着的哑巴」：帧进了混音器，设备没人写）。
///
/// 三轮接管而不是一轮，是为了顺带压住判据 ⑨ 的一半 —— **源号不泄漏**：每轮 owner 退出后
/// `mixer_stats().sources` 必须回到 0，三轮下来既不单调增长、也不会逼近 FR-12 的 8 路上限。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn owner_handover_reopens_the_playout_device() {
    /// 接管轮数：判据 ⑧ 只要求 1 → 2，这里连做三轮。
    const ROUNDS: u64 = 3;
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;
    let receiver = start_receiver(dir.path(), frame_ms).await;
    let engine = Arc::clone(&receiver.engine);

    assert_eq!(receiver.opens(), 0, "还没有任何会话，播放设备不该被打开");

    for round in 1..=ROUNDS {
        let name = format!("sender-{round}");
        let sender = Engine::start(sender_config(
            dir.path(),
            &name,
            300.0 + round as f32 * 100.0,
            frame_ms,
        ))
        .await
        .expect("发送引擎");
        let receiver_id = pair_with_receiver(&sender, &engine).await;
        let opens_before = receiver.opens();
        sender.start_send(receiver_id).await.expect("开流");

        // 判据 ⑧ 的核心：**每一条**接管会话都必须重新打开播放设备。
        let reopened = wait_for(Duration::from_secs(10), || receiver.opens() == round).await;
        assert_eq!(
            receiver.opens(),
            opens_before + 1,
            "第 {round} 轮：owner 会话退出后新会话没有重新打开播放设备（打开次数 {opens_before} → {}，期望 {}）。\
             这就是 docs/51 §2.2 的缺口复现：槽非空 ⇒ is_owner=false ⇒ 不建 sink ⇒ 从不写设备",
            receiver.opens(),
            opens_before + 1
        );
        assert!(reopened, "第 {round} 轮：10 s 内都没等到设备被重新打开");

        // 「打开了」与「真的在写」是两件事，两个都要卡。
        let writes_before = receiver.writes();
        let written = wait_for(Duration::from_secs(10), || {
            receiver.writes() > writes_before
        })
        .await;
        assert!(
            written,
            "第 {round} 轮：设备被打开了却没有任何写入（写入次数停在 {writes_before}）—— 那是「有 sink 不写」"
        );
        println!(
            "[owner-handover] 第 {round} 轮接管：播放设备打开次数 {opens_before} → {}（判据 ⑧：每一次接管都必须重新打开设备），sink 写入 {writes_before} → {}，活跃混音源 {:?}",
            receiver.opens(),
            receiver.writes(),
            receiver.sources(),
        );

        // M4 多路语义不变：混音器**对象**被复用（槽只在第一条会话时新建，此后不重建、不置空），
        // 此刻只有这一路是活跃混音源。
        assert_eq!(
            receiver.sources(),
            Some(1),
            "第 {round} 轮：此刻应当恰好有一条活跃混音源"
        );

        // 让这一路 —— 当前的 owner —— 退出。
        sender.shutdown().await;
        let emptied = wait_for(Duration::from_secs(10), || engine.peers().is_empty()).await;
        assert!(emptied, "第 {round} 轮：接收侧会话没有在 10 s 内清空");

        // owner 让位的直接证据：槽**仍然在**（没有把混音器置空 —— 那会把 M4 多路拆成两摊，
        // 见 docs/51 §3.3），但源号已经还回去。
        let _ = wait_for(Duration::from_secs(10), || receiver.sources() == Some(0)).await;
        let idle = receiver.engine.mixer_stats().expect(
            "owner 退出后混音器槽必须仍在：置空槽 = 把多路混音拆成两摊，用一个静音换另一个静音",
        );
        assert_eq!(
            idle.sources, 0,
            "第 {round} 轮：owner 退出后活跃混音源应当归零（实测 {}）—— 源号泄漏会逼近 FR-12 的 8 路上限",
            idle.sources
        );
    }

    assert_eq!(
        receiver.opens(),
        ROUNDS,
        "{ROUNDS} 轮接管合计应当恰好打开播放设备 {ROUNDS} 次（每次接管一次，不多不少）"
    );

    receiver.accept.abort();
    receiver.engine.shutdown().await;
}

/// **§189 的回归**：接收端挂两路，**先建的那路（owner）断开**，另一路必须仍然出声。
///
/// 判据是量级而不是「没崩」：断开前先量两路混音的 RMS（> 0.01，证明对照组真的在出声），
/// 断开后量剩下那一路的 RMS（> 0.005，与 `mixer_convergence.rs` 单路口径一致），
/// 并且**播放设备必须被重新打开**（计数 1 → 2）—— 两者缺一，另一路就是「活着的哑巴」。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn other_sources_keep_playing_after_the_owner_leaves() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;
    let mut receiver = start_receiver(dir.path(), frame_ms).await;
    let engine = Arc::clone(&receiver.engine);

    // 两路用不同频率：混音后不会有相位抵消，「仍在出声」这条判据才不会被抵消骗过去。
    let sender_a = Engine::start(sender_config(dir.path(), "sender-a", 440.0, frame_ms))
        .await
        .expect("发送引擎 A");
    let receiver_id = pair_with_receiver(&sender_a, &engine).await;
    sender_a.start_send(receiver_id).await.expect("A 开流");
    let first_opens = wait_for(Duration::from_secs(10), || receiver.opens() == 1).await;
    assert!(
        first_opens,
        "先建的那一路没有打开播放设备（计数 {}）",
        receiver.opens()
    );

    // 第二路后加入：**不再打开第二台设备**，只把自己的帧混进同一个混音器（M4 语义）。
    let sender_b = Engine::start(sender_config(dir.path(), "sender-b", 660.0, frame_ms))
        .await
        .expect("发送引擎 B");
    pair_with_receiver(&sender_b, &engine).await;
    // 注意：这里是**发送端 B 自己的 id** —— 接收端 peers 表以对端（发送端）id 为键，
    // 用它才读得到「B 路还在不在被接收」。（pair_with_receiver 返回的是接收端 id，别混用。）
    let sender_b_id = sender_b.info().id;
    sender_b.start_send(receiver_id).await.expect("B 开流");
    let mixed = wait_for(Duration::from_secs(10), || receiver.sources() == Some(2)).await;
    assert!(
        mixed,
        "两路没有都进混音器（活跃源 {}）",
        receiver.sources().unwrap_or(0)
    );
    assert_eq!(
        receiver.opens(),
        1,
        "第二条会话不该再打开一台设备（多路只开一次）"
    );

    let _ = rms_over(&mut receiver.samples, Duration::from_millis(400)).await; // 丢掉启动期
    let both = rms_over(&mut receiver.samples, Duration::from_secs(2)).await;
    assert!(both > 0.01, "两路混音的 RMS 太小，测试没有意义：{both}");

    // owner（先建的 A）断开。
    sender_a.shutdown().await;
    let lone = wait_for(Duration::from_secs(10), || engine.peers().len() == 1).await;
    assert!(
        lone,
        "接收侧没有在 10 s 内落到只剩 B 一条会话（当前 {} 条）",
        engine.peers().len()
    );

    // 给接管一个有界窗口（3 s），然后**断言**结果：失败信息里要能同时看到
    // 「设备没被重新打开」与「另一路哑了」两个读数。
    let _ = wait_for(Duration::from_secs(3), || receiver.opens() >= 2).await;
    let _ = rms_over(&mut receiver.samples, Duration::from_millis(300)).await; // 丢掉切换期
    let only_b = rms_over(&mut receiver.samples, Duration::from_secs(2)).await;
    let after = receiver.opens();

    // 因果链的最后一块：B 的帧**还在进来**吗？如果连帧都收不到，那这条红就不是隔离缺口，
    // 而是流本身断了 —— 两个原因必须分开，否则会拿错结论去改代码。
    let incoming = engine.telemetry(sender_b_id).map(|stats| stats.bitrate_bps);
    println!(
        "[owner-handover] 两路混音 RMS {both:.4} → owner（先建的 A）退出后仅剩 B 路 RMS {only_b:.4}；播放设备打开次数 1 → {after}；会话数 {}；活跃混音源 {:?}；B 路仍在被接收的码率 {incoming:?} bps",
        engine.peers().len(),
        receiver.sources(),
    );

    assert!(
        incoming.unwrap_or(0) > 0,
        "owner 退出后 B 路连帧都收不到了（接收侧遥测 {incoming:?}）—— 那要查的是流本身，不是混音器 owner 缺口"
    );
    assert!(
        only_b > 0.005,
        "owner 退出后另一路必须仍在出声（RMS {only_b}，断开前两路 {both}）—— 这正是 docs/51 §7 第 189 行要守的缺口"
    );
    assert!(
        after >= 2,
        "owner 退出后播放设备必须被重新打开（打开次数仍是 {after}）—— 没有接管就没有声音"
    );
    assert_eq!(after, 2, "接管只该多打开一次设备（实测 {after}）");

    sender_b.shutdown().await;
    receiver.accept.abort();
    receiver.engine.shutdown().await;
}
