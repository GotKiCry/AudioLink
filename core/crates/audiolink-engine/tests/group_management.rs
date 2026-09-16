//! §7 临时同步组的端到端回归：**真实双 Engine + 真实 QUIC**。
//!
//! 证明的是**组管理链路**：发送端建组 → GROUP_CREATE（0x40）携带组基准送达成员 →
//! 成员据此排播（PlayoutScheduled）→ 成员表可查 → 成员退出后账本收敛。
//!
//! 三台设备同时出声这一层（组内 ±10 ms P95）要等真机，属 M3 的真机验收项。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use audiolink_audio::{NullPlayout, PlayoutSink, SyntheticCapture};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::ErrorCode;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn creating_a_group_schedules_the_member_and_leaving_converges() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;
    let lead_ms = 120_u32;

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

    let outcome = tokio::time::timeout(Duration::from_secs(60), async {
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
        tokio::time::sleep(Duration::from_secs(2)).await;

        let group_id = sender
            .create_group(&[receiver_id], lead_ms)
            .await
            .expect("建组");
        let after_create = sender.groups();

        // 成员侧必须因此排播（建组帧里带着组基准）
        let scheduled = loop {
            if let Ok(EngineEvent::PlayoutScheduled { epoch_id, .. }) = events.recv().await {
                break epoch_id;
            }
        };

        sender
            .leave_group(receiver_id, group_id)
            .await
            .expect("成员退出");
        let after_leave = sender.groups();
        (group_id, after_create, after_leave, scheduled)
    })
    .await;

    let (group_id, after_create, after_leave, scheduled_epoch) =
        outcome.expect("60 s 内必须完成配对、开流、建组、排播与退出");

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();

    println!(
        "组 {group_id}：建组后 {} 个组 / 成员 {:?}；退出后 {} 个组；成员排播 epoch={scheduled_epoch:#x}",
        after_create.len(),
        after_create.first().map(|snapshot| snapshot.members.len()),
        after_leave.len()
    );

    assert_eq!(after_create.len(), 1, "建组后账本里应有且只有一个组");
    let snapshot = after_create.first().expect("组快照");
    assert_eq!(snapshot.group_id, group_id);
    assert_eq!(snapshot.lead_ms, lead_ms);
    assert_eq!(
        snapshot.members,
        vec![receiver_id],
        "成员表应记下被点名的成员"
    );
    assert_eq!(
        scheduled_epoch, snapshot.epoch_id,
        "成员排播用的基准必须就是建组帧里的那个 epoch"
    );
    assert!(after_leave.is_empty(), "成员退光后组条目应该被清掉");
}
