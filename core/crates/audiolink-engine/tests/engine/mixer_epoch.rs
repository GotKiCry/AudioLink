//! M4 共同时间基准：**接收端广播 → 各发送端对齐编号**（真实 QUIC）。
//!
//! 场景就是 M4 的「多台手机 → 一台 PC 混音」：PC 是接收端、也是混音方，两台手机各自采集。
//! 若各手机从**自己启流那一刻**开始编号，PC 按编号混音就会错开几十毫秒（M4 的「多路时钟对齐」缺口）。
//! 协议上的解法是由接收端广播一个共同原点：`Engine::broadcast_epoch` → `0x44` → 发送端 `apply_epoch`。
//!
//! 为什么补这一条（第 63 轮）：这条链路此前**零测试覆盖**，而且对齐与否原本没有任何当场可读的观测点
//! （只能事后在混音音频里看出来）。本轮加了 `Engine::capture_epoch_us()`（采集枢纽那个原子量的出口），
//! 于是可以在回环里直接断言：广播前两端都停在 `i64::MIN`，广播后两端变成**同一个**值。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use audiolink_audio::{NullPlayout, PlayoutSink, SyntheticCapture};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::{ErrorCode, NodeId};
use tokio::task::JoinHandle;

/// 一台手机（发送端）：有采集源，没有播放。
async fn start_phone(dir: &Path, index: usize, frame_ms: u32) -> Arc<Engine> {
    let mut config =
        EngineConfig::new(format!("phone-{index}"), dir.join(format!("phone-{index}")));
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.codec.frame_ms = frame_ms;
    config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));
    Engine::start(config).await.expect("手机引擎")
}

/// 一台 PC（接收端 + 混音方）：只收不放。
async fn start_pc(dir: &Path, frame_ms: u32) -> (Arc<Engine>, JoinHandle<()>) {
    let mut config = EngineConfig::new("pc", dir.join("pc"));
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.codec.frame_ms = frame_ms;
    config.playout = Some(Arc::new(move || {
        Ok(Box::new(NullPlayout::new(60)) as Box<dyn PlayoutSink>)
    }));
    let engine = Engine::start(config).await.expect("PC 引擎");
    let accept = engine.spawn_accept_loop();
    (engine, accept)
}

/// 手机连上 PC 并完成 PIN 配对；PIN 从 **PC 侧**的事件里读（PC 是显示方）。
async fn pair_phone(
    pc: &Arc<Engine>,
    pc_events: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
    phone: &Arc<Engine>,
) -> NodeId {
    let pc_id = pc.info().id;
    let error = phone.connect(pc.local_addr()).await.unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotPaired, "首次连接必须先要 PIN");
    let pin = loop {
        if let EngineEvent::DisplayPin { pin, .. } = pc_events.recv().await.unwrap() {
            break pin;
        }
    };
    phone.submit_pin(pc_id, &pin).await.expect("PIN 配对");
    pc_id
}

async fn wait_streaming(sender: &Arc<Engine>, id: NodeId) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !sender
        .peers()
        .iter()
        .any(|peer| peer.id == id && peer.state == SessionState::Streaming)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "会话没有在 20 s 内进入 streaming"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn broadcast_epoch_aligns_every_sender_to_one_origin() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;
    let lead_ms = 80_u32;

    let (pc, accept) = start_pc(dir.path(), frame_ms).await;
    let mut pc_events = pc.subscribe();
    let phone_a = start_phone(dir.path(), 0, frame_ms).await;
    let phone_b = start_phone(dir.path(), 1, frame_ms).await;

    let outcome = tokio::time::timeout(Duration::from_secs(60), async {
        let pc_id = pair_phone(&pc, &mut pc_events, &phone_a).await;
        pair_phone(&pc, &mut pc_events, &phone_b).await;
        wait_streaming(&phone_a, pc_id).await;
        wait_streaming(&phone_b, pc_id).await;

        // 两台手机各自开流：采集枢纽这时才建起来（共同基准要落到枢纽上）。
        phone_a.start_send(pc_id).await.expect("手机 A 开流");
        phone_b.start_send(pc_id).await.expect("手机 B 开流");

        // 等 PC 侧把两条会话都登记上（广播按会话表遍历）。
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while pc.peers().len() < 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "PC 没有登记到两条会话"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        // ① 广播**之前**：两端都应当是「未对齐」。
        let before_a = phone_a.capture_epoch_us();
        let before_b = phone_b.capture_epoch_us();
        assert_eq!(before_a, Some(i64::MIN), "还没广播时手机 A 不该有共同基准");
        assert_eq!(before_b, Some(i64::MIN), "还没广播时手机 B 不该有共同基准");

        // ② 接收端广播共同原点。
        let sent = pc.broadcast_epoch(lead_ms).await.expect("广播共同基准");
        assert_eq!(sent, 2, "两台发送端都该收到广播");
        tokio::time::sleep(Duration::from_secs(2)).await;

        // ③ 广播**之后**：两端都必须对齐，而且是**同一个**原点。
        let after_a = phone_a.capture_epoch_us().expect("手机 A 有采集枢纽");
        let after_b = phone_b.capture_epoch_us().expect("手机 B 有采集枢纽");
        (before_a, before_b, after_a, after_b)
    })
    .await
    .expect("60 s 内必须完成配对、开流、广播与对齐");

    let (before_a, before_b, after_a, after_b) = outcome;
    println!(
        "[mixer-epoch] 广播前 A={before_a:?} · B={before_b:?}；广播后 A={after_a} · B={after_b}"
    );

    assert_ne!(after_a, i64::MIN, "广播之后手机 A 仍然没对齐");
    assert_ne!(after_b, i64::MIN, "广播之后手机 B 仍然没对齐");
    assert_eq!(
        after_a, after_b,
        "两台发送端必须对齐到**同一个**原点 —— 否则 PC 按编号混音时两路会错开（M4 的「多路时钟对齐」）"
    );
    assert!(
        after_a > 0,
        "共同基准应当是正的本端单调时刻，实际 {after_a}"
    );

    accept.abort();
    pc.shutdown().await;
    phone_a.shutdown().await;
    phone_b.shutdown().await;
}
