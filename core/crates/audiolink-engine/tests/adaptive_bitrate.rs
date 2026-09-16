//! §8 自适应码率的端到端回归：**真实双 Engine + 真实 QUIC**。
//!
//! 这条测试要证明的是**接线**，不是规则表（规则表由 `adaptive` 模块的 8 项单测覆盖）：
//!
//! 1. 策略在真实会话里以 1 Hz 跑起来，判据取自**对端**的 `STREAM_STATS`（不是拍脑袋的本地量）；
//! 2. 决策结果真的落到编码器上 —— 接收侧的链路码率可见地抬起来；
//! 3. 变更从 `EngineEvent::CodecAdapted` 出来（验收「自适应生效」看的就是这条时间线）。
//!
//! 为了让它**可复现**，这里从 §8 的码率下限（96 kbps）起步：健康链路上 10 s 后必然触发一次恢复，
//! 于是不需要任何脆弱的丢包注入就能看到端到端的真实变更。
//!
//! 降级分支为什么不做端到端：在回环上（RTT < 1 ms）§8.1 的双发 + NACK 会把人为丢包修得一个洞不剩，
//! 接收侧上报的丢包率就是 0 —— 「修好了就不该降码率」是**正确行为**，却让降级在本地不可复现。
//! 真正要降级的弱网场景（5 Mbps / 2% 丢包 / 30 ms 抖动）属 M2 的弱网验收项。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use audiolink_audio::{NullPlayout, PlayoutSink, SyntheticCapture};
use audiolink_engine::adaptive::MIN_BITRATE_BPS;
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::ErrorCode;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn healthy_link_recovers_from_the_floor_and_the_change_reaches_the_wire() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    // 从 §8 的下限起步：健康链路上必然触发一次恢复，端到端因此可复现。
    send_config.codec.bitrate_bps = MIN_BITRATE_BPS;
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
    let mut sender_events = sender.subscribe();

    let changes: Arc<std::sync::Mutex<Vec<(i32, i32, String)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = changes.clone();
    let collector = tokio::spawn(async move {
        loop {
            match sender_events.recv().await {
                Ok(EngineEvent::CodecAdapted {
                    from_bps,
                    to_bps,
                    reason,
                }) => {
                    if let Ok(mut slot) = sink.lock() {
                        slot.push((from_bps, to_bps, reason));
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

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

        // 起步阶段的链路码率（双发后 ≈ 2 × 目标）。
        tokio::time::sleep(Duration::from_secs(4)).await;
        let before = receiver.telemetry(sender_id).map(|stats| stats.bitrate_bps);

        // §8 的恢复窗口是 10 s 无丢包：再多等一会儿，让它至少升一级。
        tokio::time::sleep(Duration::from_secs(12)).await;
        let after = receiver.telemetry(sender_id).map(|stats| stats.bitrate_bps);
        (before, after)
    })
    .await;

    let (before, after) = outcome.expect("45 s 内必须完成配对、开流并观察到一次恢复");
    let captured = changes.lock().map(|slot| slot.clone()).unwrap_or_default();

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();
    collector.abort();

    println!("起步链路 {before:?} bps → 现在 {after:?} bps；变更 {captured:?}");

    assert!(
        !captured.is_empty(),
        "健康链路上从下限起步，必须出现恢复事件"
    );
    let (from_bps, to_bps, reason) = captured[0].clone();
    assert_eq!(from_bps, MIN_BITRATE_BPS, "起点是 §8 的码率下限");
    assert_eq!(to_bps, 105_600, "+10%");
    assert!(reason.contains("恢复"), "原因必须写明恢复：{reason}");

    // 端到端生效的硬证据：**接收侧看得见的链路码率真的抬起来了**（双发所以约为目标的两倍）。
    let before = before.expect("接收侧遥测");
    let after = after.expect("接收侧遥测");
    assert!(
        before >= MIN_BITRATE_BPS as u32,
        "起步链路码率 {before} 不该低于目标下限的两倍量级"
    );
    assert!(
        after > before,
        "码率变更必须落到线上：{before} → {after} bps"
    );
}
