//! M3 多会话：**一台发送端同时推给多台接收端**（真实 QUIC）。
//!
//! 这条测试盯的是本轮的结构改动：采集从「每会话一路」变成**引擎级共享的采集枢纽** ——
//! 所有会话拿到的是同一串帧、同一套 seq / sample_index。为什么这是正确性问题而不是优化：
//! 各会话各自采集时，读取起点与丢样彼此错开几十毫秒，接收端按**同一个 epoch** 播放时，
//! 这点错位会原封不动加到组内偏差上（M3 验收是组内 ±10 ms）。
//!
//! 三台设备同时出声的听感验收仍需真机；这里证明的是「一条采集喂三路会话，三路都在稳定出声」。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::ErrorCode;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_capture_feeds_three_receivers() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));
    let sender = Engine::start(send_config).await.expect("发送引擎");

    let mut receivers = Vec::new();
    for index in 0..3 {
        let mut recv_config = EngineConfig::new(
            format!("receiver-{index}"),
            dir.path().join(format!("receiver-{index}")),
        );
        recv_config.listen = "127.0.0.1:0".parse().unwrap();
        recv_config.codec.frame_ms = frame_ms;
        recv_config.playout = Some(Arc::new(move || {
            Ok(Box::new(NullPlayout::new(60)) as Box<dyn PlayoutSink>)
        }));
        let receiver = Engine::start(recv_config).await.expect("接收引擎");
        let accept = receiver.spawn_accept_loop();
        receivers.push((receiver, accept));
    }

    let outcome = tokio::time::timeout(Duration::from_secs(90), async {
        let mut ids = Vec::new();
        for (receiver, _accept) in &receivers {
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
            ids.push(receiver_id);
        }

        // 等待三台都进入 streaming
        for id in &ids {
            while !sender
                .peers()
                .iter()
                .any(|peer| peer.id == *id && peer.state == SessionState::Streaming)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        // 一条采集喂三路会话
        let results = sender.start_send_many(&ids).await.expect("批量开流");
        assert_eq!(results.len(), 3, "三台都要有各自的结论");
        for (id, result) in &results {
            assert!(result.is_ok(), "{id:?} 开流失败：{result:?}");
        }

        // 三台各自都要真的收到流：接收侧看到的链路码率非零
        let sender_id = sender.info().id;
        tokio::time::sleep(Duration::from_secs(6)).await;

        // 接收端必须**看得见发起端**。
        //
        // 这条断言是补上的：前一轮我在测试**关闭之后**（shutdown 已经跑过）查过这张表，
        // 看到 0 个对端，就把它记成「接收端不登记对端」的疑点。本轮按时序采样后看清了 ——
        // 链路上时它一直是 1，0 只出现在 shutdown 之后，那是正常清理而不是缺陷。
        // 这里把它钉成正式断言，免得下次再被同一个时序骗一遍。
        for (receiver, _accept) in &receivers {
            let peers = receiver.peers();
            assert_eq!(peers.len(), 1, "接收端应当恰好看到发起端一条会话");
            let peer = peers.first().expect("发起端会话");
            assert_eq!(peer.id, sender_id, "看到的必须是发起端");
            assert!(
                matches!(peer.state, SessionState::Streaming | SessionState::Degraded),
                "会话应当在链路上，实际 {:?}",
                peer.state
            );
        }
        ids.iter()
            .map(|id| (*id, receiver_bitrate(&receivers, &sender, *id)))
            .collect::<Vec<_>>()
    })
    .await;

    let rates = outcome.expect("90 s 内必须完成三台配对与批量开流");

    for (_receiver, accept) in receivers.iter() {
        accept.abort();
    }
    sender.shutdown().await;
    for (receiver, _accept) in &receivers {
        receiver.shutdown().await;
    }

    println!("三台接收侧链路码率：{rates:?}");
    for (id, rate) in &rates {
        assert!(
            rate.unwrap_or(0) > 0,
            "{id:?} 没有收到流（链路码率 {rate:?}）"
        );
    }
    // 「在出声」的判据用**接收侧自己看到的链路码率**：三台都必须真的在收帧。
    // 关闭之后对端表必须清干净：会话都走了还留着「幽灵对端」，UI 上就会一直挂着不存在的设备。
    for (receiver, _accept) in &receivers {
        assert!(
            receiver.peers().is_empty(),
            "关闭后不该留下对端记录（{} 台还在）",
            receiver.peers().len()
        );
    }
}

/// 记「非静音样本数」与「首次非静音写出的时刻」的播放端（8 台压测用）。
#[derive(Default)]
struct Counters {
    audible: usize,
    first_audio: Option<Instant>,
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
        let audible = samples.iter().any(|sample| sample.abs() > 0.01);
        if audible && let Ok(mut counters) = self.counters.lock() {
            counters.audible += samples.len();
            if counters.first_audio.is_none() {
                counters.first_audio = Some(Instant::now());
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

/// 把广播缓冲里累积的「首次排播」事件数出来：非零 = 这一路真的进了 epoch 排播。
fn drain_scheduled(events: &mut tokio::sync::broadcast::Receiver<EngineEvent>) -> usize {
    let mut count = 0;
    while let Ok(event) = events.try_recv() {
        if let EngineEvent::PlayoutScheduled { .. } = event {
            count += 1;
        }
    }
    count
}

/// M3 交付物 1 的账面上写着「多会话（并行推 ≥ 8 台）未开始」—— 这条把它补上。
///
/// 盯三件事，都要是**直接证据**：
/// 1. 一路采集能同时喂 8 台（批量开流每台都有各自的结论）；
/// 2. 8 台**都在真的出声** —— 用非静音样本，而不是「链路码率非零」；
/// 3. 8 台都进了 epoch 排播，且播放量同量级 —— 「有出声」抓不住「某台几乎不出声」
///    （第 61 轮那个 join_group 缺陷就是这么漏过去的）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_capture_feeds_eight_receivers() {
    const COUNT: usize = 8;
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
    for index in 0..COUNT {
        let sink_counters = Arc::new(Mutex::new(Counters::default()));
        let mut recv_config = EngineConfig::new(
            format!("receiver-{index}"),
            dir.path().join(format!("receiver-{index}")),
        );
        recv_config.listen = "127.0.0.1:0".parse().unwrap();
        recv_config.codec.frame_ms = frame_ms;
        let moved = Arc::clone(&sink_counters);
        recv_config.playout = Some(Arc::new(move || {
            Ok(Box::new(CountingSink {
                inner: NullPlayout::new(60),
                counters: Arc::clone(&moved),
            }) as Box<dyn PlayoutSink>)
        }));
        let receiver = Engine::start(recv_config).await.expect("接收引擎");
        let accept = receiver.spawn_accept_loop();
        counters.push(sink_counters);
        receivers.push((receiver, accept));
    }

    let mut events: Vec<_> = receivers
        .iter()
        .map(|(receiver, _)| receiver.subscribe())
        .collect();

    let outcome = tokio::time::timeout(Duration::from_secs(120), async {
        let mut ids = Vec::new();
        for ((receiver, _), event_rx) in receivers.iter().zip(events.iter_mut()) {
            let receiver_id = receiver.info().id;
            let error = sender.connect(receiver.local_addr()).await.unwrap_err();
            assert_eq!(error.code(), ErrorCode::NotPaired, "首次连接必须先要 PIN");
            let pin = loop {
                if let EngineEvent::DisplayPin { pin, .. } = event_rx.recv().await.unwrap() {
                    break pin;
                }
            };
            sender
                .submit_pin(receiver_id, &pin)
                .await
                .expect("PIN 配对");
            ids.push(receiver_id);
        }

        for id in &ids {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !sender
                .peers()
                .iter()
                .any(|peer| peer.id == *id && peer.state == SessionState::Streaming)
            {
                assert!(
                    Instant::now() < deadline,
                    "{id:?} 没有在 30 s 内进入 streaming"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        // 8 台成一个组：它们要在同一个 epoch 基准上排播。
        let _group_id = sender.create_group(&ids, lead_ms).await.expect("建组");
        let results = sender.start_send_many(&ids).await.expect("批量开流");
        assert_eq!(results.len(), COUNT, "每台都要有各自的结论");
        for (id, result) in &results {
            assert!(result.is_ok(), "{id:?} 开流失败：{result:?}");
        }

        tokio::time::sleep(Duration::from_secs(5)).await;
        ids
    })
    .await;
    let ids = outcome.expect("120 s 内必须完成 8 台配对与批量开流");

    let audible: Vec<usize> = counters
        .iter()
        .map(|counters| counters.lock().unwrap().audible)
        .collect();
    let scheduled: Vec<usize> = events.iter_mut().map(drain_scheduled).collect();
    println!(
        "[multi-8] 8 台非静音样本 {audible:?}（{COUNT} 台，其中 {} 路在链路上）；排播事件数 {scheduled:?}",
        ids.len()
    );

    for (index, samples) in audible.iter().enumerate() {
        assert!(
            *samples > 0,
            "第 {index} 台一个非静音样本都没写出来（8 台压测下也不许有哑的）"
        );
    }
    for (index, count) in scheduled.iter().enumerate() {
        assert!(
            *count > 0,
            "第 {index} 台没有排播事件 —— 它没拿到组基准（8 台同组必须都排播）"
        );
    }
    let reference = audible[0];
    for (index, samples) in audible.iter().enumerate() {
        assert!(
            *samples * 10 >= reference * 9,
            "第 {index} 台的播放量 {samples} 比第 0 台的 {reference} 少了一成以上 —— 并行 8 路时某路被饿着了"
        );
    }

    for (_receiver, accept) in receivers.iter() {
        accept.abort();
    }
    sender.shutdown().await;
    for (receiver, _accept) in &receivers {
        receiver.shutdown().await;
    }
}
/// 某台接收端看到的链路码率（它自己的遥测口径）。
fn receiver_bitrate(
    receivers: &[(Arc<Engine>, tokio::task::JoinHandle<()>)],
    sender: &Arc<Engine>,
    id: audiolink_types::NodeId,
) -> Option<u32> {
    let sender_id = sender.info().id;
    receivers
        .iter()
        .find(|(receiver, _)| receiver.info().id == id)
        .and_then(|(receiver, _)| receiver.telemetry(sender_id).map(|stats| stats.bitrate_bps))
}

/// 各台「非静音样本累计数」的快照。
///
/// `Counters.audible` 累计的是**样本数**（交织的双声道各算一个样本）：20 ms 帧 @48 kHz = 960 帧，
/// 双声道即 1920 样本/帧、50 帧/s ⇒ 满速 ≈ 96000 样本/s，4 s 窗口的满额 = **384000**
/// （本机实测四台都精确落在满额上，见打印的 `[isolation]` 行）—— 绝对下限就建立在这个换算上。
fn audible_snapshot(counters: &[Arc<Mutex<Counters>>]) -> Vec<usize> {
    counters
        .iter()
        .map(|counters| counters.lock().unwrap().audible)
        .collect()
}

/// 一段窗口内的增量（累计计数单调递增，`saturating_sub` 只是防御）。
fn delta_since(now: &[usize], before: &[usize]) -> Vec<usize> {
    now.iter()
        .zip(before.iter())
        .map(|(now, before)| now.saturating_sub(*before))
        .collect()
}

/// **M3 并发验收的另一半：`互不影响`（故障隔离）。**
///
/// `docs/05-roadmap.md` §M3 的并发原文是「3 台接收端同时推流，**互不影响**」，而
/// `one_capture_feeds_three_receivers` 只钉住了前半句（三台各自在收帧、对端表干净）——
/// 后半句「一台出问题，其余不受影响」此前**没有任何测试在守**：它被声称，却没被测。
///
/// 场景与判据（每一条都是量级判据）：
///
/// 1. 1 发 + 4 收并发推流，先量**断开前 4 s** 各台的非静音样本增量（对照组）；
/// 2. 把 1 号接收端**整个引擎 shutdown** —— 等价于那台设备掉线/进程死掉；
/// 3. 再量**断开后 4 s** 的各台增量。幸存的三台必须仍在满量级出声，三条一起卡：
///    与自己的对照组比**不跌超四分之一**、绝对下限 **96000 样本/4 s**（满额 384000 的四分之一）、
///    幸存台之间**同量级**（最小不低于最大的一半）。
///    第 61 轮那个 `join_group` 缺陷正是靠量级对比才现形的（4 s 窗口里 A/B 各 384000、C 只有 30720）——
///    「几乎不出声」和「完全不出声」在 `> 0` 眼里一模一样，所以这里一条 `> 0` 都没有。
/// 4. 发送侧会话数必须**恰好少一个**：全掉说明一台拖垮了发送端，不变说明断开根本没被感知；
///    少掉的那台还必须**正是受害台**（否则「数目对了」也可能是别的会话被误摘）。
/// 5. 受害台自己必须真的哑了 —— 这只是「断开这个动作生效了」的健全性检查，
///    **不是**隔离判据（后者显然是），放在最后是为了不喧宾夺主。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_receiver_failure_does_not_stall_the_others() {
    const COUNT: usize = 4;
    /// 被断开的接收端：取中间那台，避开「首台/末台」在配对与批量开流里的特殊位置。
    const VICTIM: usize = 1;
    const WINDOW: Duration = Duration::from_secs(4);
    /// 4 s 窗口的绝对下限：满额 384000 的四分之一（= 96000）。
    /// 留四倍余量是为了不被后台负载带偏，同时仍然挡得住「只剩零头」：
    /// 某台要是只写出满额的零头（比如 10%），这条直接判红。
    const MIN_SAMPLES_PER_WINDOW: usize = 96_000;

    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));
    let sender = Engine::start(send_config).await.expect("发送引擎");

    let mut receivers: Vec<(Arc<Engine>, tokio::task::JoinHandle<()>)> = Vec::new();
    let mut counters: Vec<Arc<Mutex<Counters>>> = Vec::new();
    for index in 0..COUNT {
        let sink_counters = Arc::new(Mutex::new(Counters::default()));
        let mut recv_config = EngineConfig::new(
            format!("receiver-{index}"),
            dir.path().join(format!("receiver-{index}")),
        );
        recv_config.listen = "127.0.0.1:0".parse().unwrap();
        recv_config.codec.frame_ms = frame_ms;
        let moved = Arc::clone(&sink_counters);
        recv_config.playout = Some(Arc::new(move || {
            Ok(Box::new(CountingSink {
                inner: NullPlayout::new(60),
                counters: Arc::clone(&moved),
            }) as Box<dyn PlayoutSink>)
        }));
        let receiver = Engine::start(recv_config).await.expect("接收引擎");
        let accept = receiver.spawn_accept_loop();
        counters.push(sink_counters);
        receivers.push((receiver, accept));
    }

    let mut events: Vec<_> = receivers
        .iter()
        .map(|(receiver, _)| receiver.subscribe())
        .collect();

    // 配对 + 批量开流 + 两段采样。整体包一个总超时，避免任何一步挂死把测试变成吊死。
    let outcome = tokio::time::timeout(Duration::from_secs(120), async {
        let mut ids = Vec::new();
        for ((receiver, _), event_rx) in receivers.iter().zip(events.iter_mut()) {
            let receiver_id = receiver.info().id;
            let error = sender.connect(receiver.local_addr()).await.unwrap_err();
            assert_eq!(error.code(), ErrorCode::NotPaired, "首次连接必须先要 PIN");
            let pin = loop {
                if let EngineEvent::DisplayPin { pin, .. } = event_rx.recv().await.unwrap() {
                    break pin;
                }
            };
            sender
                .submit_pin(receiver_id, &pin)
                .await
                .expect("PIN 配对");
            ids.push(receiver_id);
        }

        for id in &ids {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !sender
                .peers()
                .iter()
                .any(|peer| peer.id == *id && peer.state == SessionState::Streaming)
            {
                assert!(
                    Instant::now() < deadline,
                    "{id:?} 没有在 30 s 内进入 streaming"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        let results = sender.start_send_many(&ids).await.expect("批量开流");
        assert_eq!(results.len(), COUNT, "每台都要有各自的结论");
        for (id, result) in &results {
            assert!(result.is_ok(), "{id:?} 开流失败：{result:?}");
        }

        // 预热：等四台都冒出第一段非静音，再走 1 s，让排播与抖动缓冲进入稳态。
        // 不等就采样，会把启动爬坡记进对照组。
        let warmup_deadline = Instant::now() + Duration::from_secs(20);
        while counters
            .iter()
            .any(|counters| counters.lock().unwrap().first_audio.is_none())
        {
            assert!(
                Instant::now() < warmup_deadline,
                "20 s 内并非四台都出了声，预热不成立"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;

        // ① 对照组：断开前 4 s。
        let before_window = audible_snapshot(&counters);
        tokio::time::sleep(WINDOW).await;
        let first_window = delta_since(&audible_snapshot(&counters), &before_window);

        // ② 断开 1 号接收端：整个引擎关掉 —— 那台设备就当作死了。
        //    发送侧会看到连接被关，随后摘表并按 FR-27 重拨；重拨必然失败（对端端口已释放），
        //    这段「重连折腾」正是本测试要压的负载：它不许把其余三台一起拖哑。
        receivers[VICTIM].0.shutdown().await;
        let cut_at = Instant::now();

        // 等发送侧摘掉受害会话（不在这里 panic：超时留给下面的断言去判，好让读数先落到输出里）。
        let peer_wait_deadline = Instant::now() + Duration::from_secs(25);
        while sender.peers().len() >= COUNT {
            if Instant::now() >= peer_wait_deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let peers_settle_ms = cut_at.elapsed().as_millis();

        // ③ 断开后 4 s（从发送侧摘掉受害会话之后起算，窗口干净）。
        let before_window = audible_snapshot(&counters);
        tokio::time::sleep(WINDOW).await;
        let second_window = delta_since(&audible_snapshot(&counters), &before_window);

        let sender_peers = sender.peers();
        let sender_peer_ids: Vec<_> = sender_peers.iter().map(|peer| peer.id).collect();
        let survivor_states: Vec<Option<SessionState>> = ids
            .iter()
            .map(|id| {
                sender_peers
                    .iter()
                    .find(|peer| peer.id == *id)
                    .map(|peer| peer.state)
            })
            .collect();
        (
            ids,
            first_window,
            second_window,
            sender_peer_ids,
            survivor_states,
            peers_settle_ms,
        )
    })
    .await
    .expect("120 s 内必须跑完四台配对、开流与两段采样");

    let (ids, first_window, second_window, sender_peer_ids, survivor_states, peers_settle_ms) =
        outcome;

    let survivors: Vec<usize> = (0..COUNT).filter(|index| *index != VICTIM).collect();
    let survivor_deltas: Vec<usize> = survivors
        .iter()
        .map(|index| second_window[*index])
        .collect();
    println!(
        "[isolation] 断开 1 号前后各 {WINDOW:?} 的非静音样本增量：断开前 {first_window:?} → 断开后 {second_window:?}（下标 {VICTIM} 是被 shutdown 的那台；幸存台 {survivors:?} 增量为 {survivor_deltas:?}）；发送侧会话数 {COUNT} → {}（感知耗时 {peers_settle_ms} ms）；发送侧对端状态 {survivor_states:?}",
        sender_peer_ids.len(),
    );

    // ① 对照组自身必须达标：断开前四台都在满量级出声，否则后面的比例没有意义。
    for (index, delta) in first_window.iter().enumerate() {
        assert!(
            *delta >= MIN_SAMPLES_PER_WINDOW,
            "断开前 4 s 第 {index} 台只有 {delta} 个非静音样本（下限 {MIN_SAMPLES_PER_WINDOW}）：对照组就没在正常出声，这条测试失去意义"
        );
    }

    // ② 隔离的主判据：幸存三台仍按满量级出声（绝对下限 + 不比自己跌超四分之一）。
    for index in &survivors {
        assert!(
            second_window[*index] >= MIN_SAMPLES_PER_WINDOW,
            "断开 1 号之后，第 {index} 台 4 s 只写出 {} 个非静音样本（下限 {MIN_SAMPLES_PER_WINDOW}，断开前它是 {}）—— 一台出问题把别的台拖哑了",
            second_window[*index],
            first_window[*index]
        );
        assert!(
            second_window[*index] * 4 >= first_window[*index] * 3,
            "断开 1 号之后，第 {index} 台的样本增量从 {} 掉到 {}（跌超四分之一）—— 故障没有被隔离",
            first_window[*index],
            second_window[*index]
        );
    }

    // ③ 幸存台彼此仍要同量级：抓「某台被饿着」而不是「全都哑了」这种更隐蔽的隔离失败。
    let weakest = *survivor_deltas.iter().min().expect("幸存台非空");
    let strongest = *survivor_deltas.iter().max().expect("幸存台非空");
    assert!(
        weakest * 2 >= strongest,
        "幸存台的样本增量 {survivor_deltas:?} 差了一整个档位（最小 {weakest} vs 最大 {strongest}）—— 断开一台之后幸存台之间出现了饿死"
    );

    // ④ 发送侧少掉的必须**恰好是受害台那一条**（不是 0 台，也不是 4 台）。
    assert_eq!(
        sender_peer_ids.len(),
        COUNT - 1,
        "断开一台之后发送侧应当还剩 {} 条会话，实际 {} 条（0 = 被一台拖垮，{COUNT} = 断开根本没被感知）",
        COUNT - 1,
        sender_peer_ids.len()
    );
    assert!(
        !sender_peer_ids.contains(&ids[VICTIM]),
        "受害台的会话仍挂在发送侧表里 —— 断开没被感知"
    );

    // ⑤ 健全性检查（不是隔离判据）：受害台自己确实哑了，否则「断开」这个动作没生效。
    assert!(
        second_window[VICTIM] * 2 < first_window[VICTIM],
        "被 shutdown 的那台 4 s 仍写出 {} 个非静音样本（断开前 {}）—— 断开动作没生效，上面的隔离结论无效",
        second_window[VICTIM],
        first_window[VICTIM]
    );

    for (_receiver, accept) in receivers.iter() {
        accept.abort();
    }
    sender.shutdown().await;
    for (index, (receiver, _accept)) in receivers.iter().enumerate() {
        if index == VICTIM {
            continue;
        }
        receiver.shutdown().await;
    }
}
