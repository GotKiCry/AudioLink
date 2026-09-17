//! 移除设备（FR-18）的回归：撤信任必须**先断会话**，撤掉之后也不许自己回来。
//!
//! 这一组测试守的是 `Engine::revoke_trust` 的**顺序不变量**（不是口味问题）：
//!
//! 1. **先断会话**：信任只在握手期判定一次，撤信任不会让活会话自己结束 —— 只撤不断的话，
//!    用户看到「已移除」而音频还在流，隐私说明里那句承诺的行为没有兑现；
//! 2. **撤完不许被写回**：握手成功时引擎会 remember_peer 把记录写回白名单，
//!    所以「先撤信任、后断会话」会被那条活会话的下一次成功握手静默撤销。
//!
//! 判据都落在**对端可观察的事实**上：会话表、磁盘上的信任库、以及「它再连回来要不要重新配对」。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

/// 起一个只监听、不推流的引擎。
async fn engine(dir: &std::path::Path, name: &str) -> Arc<Engine> {
    let mut config = EngineConfig::new(name, dir.join(name));
    config.listen = "127.0.0.1:0".parse().unwrap();
    Engine::start(config).await.unwrap()
}

/// 走完 PIN 配对，返回接收端 id（发起端视角的对端 id）。
async fn pair(sender: &Arc<Engine>, receiver: &Arc<Engine>) -> NodeId {
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
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let sender_streaming = sender
            .peers()
            .iter()
            .any(|peer| peer.id == receiver_id && peer.state == SessionState::Streaming);
        let receiver_sees_sender = receiver
            .peers()
            .iter()
            .any(|peer| peer.id == sender.info().id);
        if sender_streaming && receiver_sees_sender {
            return receiver_id;
        }
        assert!(Instant::now() < deadline, "握手没有在 10 s 内完成");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// 有界等待：条件在窗口内成立返回 true，否则 false（**不 panic** —— 让「不该发生」的断言自己说话）。
async fn settles(condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// 该指纹是否在信任库里 —— **重新从磁盘加载**（不相信内存副本）。
fn trusted_on_disk(engine: &Arc<Engine>, id: NodeId) -> bool {
    let path = engine.inner.config.trust_store_path.clone();
    TrustStore::load(&path).unwrap().is_trusted(id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn 移除设备先断会话再撤信任且撤后不会自己回来() {
    let dir = tempfile::tempdir().unwrap();
    let receiver = engine(dir.path(), "receiver").await;
    let accept = receiver.spawn_accept_loop();
    let sender = engine(dir.path(), "sender").await;
    let _ = pair(&sender, &receiver).await;
    let sender_id = sender.info().id;

    // 前提：配对成功这件事落在接收端的信任库里（否则这条测试没有意义）。
    assert!(
        trusted_on_disk(&receiver, sender_id),
        "配对成功后接收端应当信任发起端"
    );

    // 移除设备。
    let removed = receiver.revoke_trust(sender_id).await.unwrap();
    assert!(removed, "第一次撤销应当真的删掉了记录");

    // ① **先断会话**：撤完之后那条会话不能还挂在表里。
    let gone = settles(|| !receiver.peers().iter().any(|peer| peer.id == sender_id)).await;
    assert!(
        gone,
        "撤信任必须断开该对端的会话 —— 只改信任库会留下「已移除但音频还在流」的假状态"
    );

    // ② 落盘的信任库里不含该指纹。
    assert!(
        !trusted_on_disk(&receiver, sender_id),
        "撤销后落盘必须不含该指纹"
    );

    // ③ 「移除生效」的可判定证据：该设备再连回来必须**重新配对**，而不是免 PIN 直连。
    let again = sender.connect(receiver.local_addr()).await.unwrap_err();
    assert_eq!(
        again.code(),
        ErrorCode::NotPaired,
        "移除后该设备再连接必须重新配对（这是「移除真的生效」的可判定证据）"
    );

    // ④ **顺序不变量**：它又握了一次手（走到「需要 PIN」那一步）也不许把记录写回白名单 ——
    //    这正是「先撤信任、后断会话」会踩的坑。
    let wrote_back = settles(|| trusted_on_disk(&receiver, sender_id)).await;
    assert!(
        !wrote_back,
        "被移除的设备不得因为再次握手而被写回信任库（先撤信任后断会话就会这样）"
    );

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn 重复撤销是幂等的() {
    let dir = tempfile::tempdir().unwrap();
    let receiver = engine(dir.path(), "receiver").await;
    let accept = receiver.spawn_accept_loop();
    let sender = engine(dir.path(), "sender").await;
    let _ = pair(&sender, &receiver).await;
    let sender_id = sender.info().id;

    assert!(receiver.revoke_trust(sender_id).await.unwrap());
    assert!(
        !receiver.revoke_trust(sender_id).await.unwrap(),
        "第二次撤销应当返回 false（本来就不在信任库里）"
    );
    assert!(!trusted_on_disk(&receiver, sender_id));

    // 从未信任过、也没有会话的指纹：同样 Ok(false)，不报错（幂等的另一半）。
    let stranger = NodeId::from_bytes([7; NodeId::LEN]);
    assert!(
        !receiver.revoke_trust(stranger).await.unwrap(),
        "从未信任过的指纹也该是 Ok(false)，而不是错误"
    );

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn 移除一台设备不影响其它对端() {
    let dir = tempfile::tempdir().unwrap();
    let receiver = engine(dir.path(), "receiver").await;
    let accept = receiver.spawn_accept_loop();
    let first = engine(dir.path(), "first").await;
    let second = engine(dir.path(), "second").await;
    let _ = pair(&first, &receiver).await;
    let _ = pair(&second, &receiver).await;
    let first_id = first.info().id;
    let second_id = second.info().id;

    // 撤掉第一台。
    assert!(receiver.revoke_trust(first_id).await.unwrap());

    // 第二台：会话还在、仍是 Streaming、信任记录还在。
    let untouched = receiver
        .peers()
        .into_iter()
        .any(|peer| peer.id == second_id && peer.state == SessionState::Streaming);
    assert!(untouched, "移除一台设备不该影响另一台的会话");
    assert!(
        trusted_on_disk(&receiver, second_id),
        "另一台的信任记录不该被删"
    );
    // 被移除的那台：没记录、也没会话。
    assert!(!trusted_on_disk(&receiver, first_id));
    assert!(
        !receiver.peers().iter().any(|peer| peer.id == first_id),
        "被移除的那台不该还挂在会话表里"
    );

    first.shutdown().await;
    second.shutdown().await;
    receiver.shutdown().await;
    accept.abort();
}
