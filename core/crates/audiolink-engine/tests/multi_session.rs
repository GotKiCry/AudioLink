//! M3 多会话：**一台发送端同时推给多台接收端**（真实 QUIC）。
//!
//! 这条测试盯的是本轮的结构改动：采集从「每会话一路」变成**引擎级共享的采集枢纽** ——
//! 所有会话拿到的是同一串帧、同一套 seq / sample_index。为什么这是正确性问题而不是优化：
//! 各会话各自采集时，读取起点与丢样彼此错开几十毫秒，接收端按**同一个 epoch** 播放时，
//! 这点错位会原封不动加到组内偏差上（M3 验收是组内 ±10 ms）。
//!
//! 三台设备同时出声的听感验收仍需真机；这里证明的是「一条采集喂三路会话，三路都在稳定出声」。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use audiolink_audio::{NullPlayout, PlayoutSink, SyntheticCapture};
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
        tokio::time::sleep(Duration::from_secs(6)).await;
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
    //
    // 注意（本轮实测发现，记在这里而不是悄悄绕开）：接收端的 `peers()` 里查不到发起端
    // （accept 一侧的会话表没有登记对方），所以不能用接收端的状态来断言这件事。
    // 这是既有行为、与本轮改动无关，已记进 docs/33 的「发现」一节待核。
    let sender_id = sender.info().id;
    for (receiver, _accept) in &receivers {
        let seen = receiver.peers().iter().any(|peer| peer.id == sender_id);
        println!(
            "接收端 {}：对端可见 {seen}，链路码率 {:?}",
            receiver.info().name,
            receiver.telemetry(sender_id).map(|stats| stats.bitrate_bps)
        );
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
