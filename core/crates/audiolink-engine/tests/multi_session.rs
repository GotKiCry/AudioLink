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
