//! §7 组基准（GROUP_EPOCH）的端到端回归：**真实双 Engine + 真实 QUIC**。
//!
//! 这条测试要证明的是**协议通道**，不是换算公式（公式由 epoch 模块的 8 项单测覆盖）：
//!
//! 1. 发送端 announce_group_epoch → 真的发出 0x43 控制帧；
//! 2. 接收端解出载荷（epoch_id 原样到达）→ 用本机时钟偏移换算 → 排播生效；
//! 3. 生效从 EngineEvent::PlayoutScheduled 出来 —— 验收「组内同步」看的就是这条时间线。
//!
//! 组内两台设备是否真的同时在同一个组时刻出声，要等第二台真机（属 M3 的真机验收项）；
//! 这里先把「发送端指定 → 接收端排播」这条线钉死。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use audiolink_audio::{NullPlayout, PlayoutSink, SyntheticCapture};
use audiolink_engine::epoch::EpochSchedule;
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState, now_monotonic_us};
use audiolink_types::ErrorCode;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn group_epoch_reaches_the_receiver_and_schedules_playout() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;
    let lead_ms = 120_u32;
    let epoch_id = 0x0BAD_C0DE_1234_5678_u64;

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));

    let mut recv_config = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_config.listen = "127.0.0.1:0".parse().unwrap();
    recv_config.codec.frame_ms = frame_ms;
    recv_config.playout = Some(Arc::new(move || {
        Ok(Box::new(NullPlayout::new(60)) as Box<dyn PlayoutSink>)
    }));

    let receiver = Engine::start(recv_config).await.expect("接收引擎");
    let accept = receiver.spawn_accept_loop();
    let mut events = receiver.subscribe();
    let receiver_id = receiver.info().id;

    let sender = Engine::start(send_config).await.expect("发送引擎");
    let sender_id = sender.info().id;

    let outcome = tokio::time::timeout(Duration::from_secs(45), async {
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
        // 等接收侧真的开始播：首帧数据报建立起序号 → 样本序号的换算基准
        tokio::time::sleep(Duration::from_secs(2)).await;

        sender
            .announce_group_epoch(receiver_id, epoch_id, lead_ms)
            .await
            .expect("广播 GROUP_EPOCH");

        loop {
            if let Ok(EngineEvent::PlayoutScheduled {
                epoch_id: got,
                target_local_us,
                wait_us,
            }) = events.recv().await
            {
                break (got, target_local_us, wait_us);
            }
        }
    })
    .await;

    let (got_epoch, target_local_us, wait_us) =
        outcome.expect("45 s 内必须完成配对、开流并观察到排播时间线");

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();

    println!(
        "GROUP_EPOCH epoch={got_epoch:#x} target={target_local_us} µs wait={wait_us} µs（lead={lead_ms} ms）"
    );
    let _ = sender_id;
    assert_eq!(got_epoch, epoch_id, "载荷里的 epoch_id 必须原样到达接收端");
    assert!(target_local_us > 0, "目标时刻必须是换算后的本机 µs");
    // 等待量的语义（这里有个容易想错的地方）：目标是
    //   local(epoch) + sample_index / 48000 + lead_ms
    // 所以它**不只等于 lead** —— 还要加上这一帧在流里的时刻。测试里推流了 2 s，
    // 于是等待量 ≈ 2 s + 120 ms，这正是公式在正常工作（而不是"等得太久"）。
    assert!(
        wait_us >= u64::from(lead_ms) * 1_000,
        "等待量至少要包含提前量：{wait_us} µs"
    );
    assert!(
        wait_us <= 15_000_000,
        "等待量应落在「帧的流内时刻 + 提前量」的量级：{wait_us} µs"
    );
}
/// `Engine::schedule_playout` 的**显式 API 语义**：设定基准后必须报出排播时间线。
///
/// 为什么补这一条（第 65 轮）：第 59 轮动过这条路径的实现（把「播放句柄未就绪」从报
/// `cap_unsupported` 改成暂存、开流后补应用），但那次改动**没有测试**。
///
/// **待查（本轮实测到的行为，不写成契约）**：在「配对完成、但还没开流」的时刻调用这条 API，
/// 会拿到 `BadRequest { context: "session task is gone" }` —— 说明那一刻**接受侧的会话任务已经不在**
/// （`send_command` 只在 command 通道关闭时报这句）。它究竟是「无流会话被提前回收」的缺陷，
/// 还是「会话任务只在流期间存在」的既定设计，需要单独查；查清之前，这条测试只覆盖开流之后的语义。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn schedule_playout_reports_the_timeline() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;
    let lead_ms = 120_u32;
    let epoch_id = 0x5EED_1234_ABCD_0001_u64;

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));
    let sender = Engine::start(send_config).await.expect("发送引擎");

    let mut recv_config = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_config.listen = "127.0.0.1:0".parse().unwrap();
    recv_config.codec.frame_ms = frame_ms;
    recv_config.playout = Some(Arc::new(move || {
        Ok(Box::new(NullPlayout::new(60)) as Box<dyn PlayoutSink>)
    }));
    let receiver = Engine::start(recv_config).await.expect("接收引擎");
    let accept = receiver.spawn_accept_loop();
    let mut events = receiver.subscribe();
    let receiver_id = receiver.info().id;
    let sender_id = sender.info().id;

    let outcome = tokio::time::timeout(Duration::from_secs(45), async {
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

        // 配对完成还不等于可以开流：会话要先进入 streaming（握手走完）。
        let ready = Instant::now() + Duration::from_secs(20);
        while !sender
            .peers()
            .iter()
            .any(|peer| peer.id == receiver_id && peer.state == SessionState::Streaming)
        {
            assert!(Instant::now() < ready, "会话没有在 20 s 内进入 streaming");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // 再把流开起来，并等首帧建立「序号 → 样本序号」的换算基准。
        sender.start_send(receiver_id).await.expect("开流");
        let deadline = Instant::now() + Duration::from_secs(20);
        while receiver.peers().is_empty() {
            assert!(Instant::now() < deadline, "接收侧没有在 20 s 内登记到会话");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;

        // ① 设定排播。基准取**未来**（10 s 后）：落在过去的 target 会走 Drop 分支，
        //    那条路不发时间线事件，于是测试会误以为「排播没生效」。
        let schedule = EpochSchedule::new(epoch_id, now_monotonic_us() + 10_000_000, lead_ms);
        receiver
            .schedule_playout(sender_id, Some(schedule))
            .await
            .expect("设定排播");

        loop {
            if let Ok(EngineEvent::PlayoutScheduled { epoch_id: got, .. }) = events.recv().await {
                break got;
            }
        }
    })
    .await;

    let got = outcome.expect("45 s 内必须看到「开流后补应用」出来的排播时间线");
    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();

    println!("schedule_playout 设定之后报出排播时间线：epoch={got:#x}");
    assert_eq!(got, epoch_id, "补应用的必须是当初暂存的那份基准");
}
