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

/// 读侧接口：信任库列表必须反映**白名单的全部条目**，包括从来没有会话的那些。
///
/// 「换机后残留的旧记录」就是这样进来的：直接落在 trust.json 里，界面上从来没有过它的卡片。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn 信任库列表反映没有会话的历史记录() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("receiver");

    // 启动**之前**就把两条记录写进信任库：一条「很久没连过的设备」，一条「残留的旧记录」。
    let stale = NodeId::from_bytes([0x11; NodeId::LEN]);
    let leftover = NodeId::from_bytes([0x22; NodeId::LEN]);
    {
        let mut store = TrustStore::load(data_dir.join("trust.json")).unwrap();
        store
            .trust(TrustEntry {
                id: stale,
                name: "很久没连过的设备".to_string(),
                platform: Platform::Windows,
                paired_at_unix: 1_700_000_000,
            })
            .unwrap();
        store
            .trust(TrustEntry {
                id: leftover,
                name: "换机后残留".to_string(),
                platform: Platform::Android,
                paired_at_unix: 1_700_000_100,
            })
            .unwrap();
    }

    let receiver = engine(dir.path(), "receiver").await;
    let accept = receiver.spawn_accept_loop();

    // 前提：这两条都**没有会话**（否则这条测试就退化成 task-33 已经覆盖的场景）。
    assert!(
        receiver.peers().is_empty(),
        "还没有任何连接，会话表应当是空的"
    );
    let listed: Vec<NodeId> = receiver
        .trusted_peers()
        .iter()
        .map(|entry| entry.id)
        .collect();
    assert_eq!(listed.len(), 2, "两条历史记录都该在列表里：{listed:?}");
    assert!(listed.contains(&stale) && listed.contains(&leftover));
    // 名字也要带出来：界面上「移除哪一台」靠它认。
    assert!(
        receiver
            .trusted_peers()
            .iter()
            .any(|entry| entry.id == leftover && entry.name == "换机后残留")
    );

    // 无会话也能移除，且列表立刻少一条。
    assert!(receiver.revoke_trust(leftover).await.unwrap());
    let after: Vec<NodeId> = receiver
        .trusted_peers()
        .iter()
        .map(|entry| entry.id)
        .collect();
    assert_eq!(after, vec![stale], "撤回后列表应当只剩下另一条");
    assert!(!trusted_on_disk(&receiver, leftover));
    assert!(
        trusted_on_disk(&receiver, stale),
        "没被移除的那条不该受影响"
    );

    receiver.shutdown().await;
    accept.abort();
}

/// **task-35 的成功判据**：白名单里有、但**当前没有会话**的设备也能被移除。
///
/// 这正是 task-33 做不到的场景：桌面 UI 的对端卡片来自会话表，会话没了就解析不出 id，
/// 于是「换机后残留的旧信任记录」只能去删 trust.json —— 隐私说明承诺的另一半没兑现。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn 无会话的已配对设备也能被移除() {
    let dir = tempfile::tempdir().unwrap();
    let receiver = engine(dir.path(), "receiver").await;
    let accept = receiver.spawn_accept_loop();
    let sender = engine(dir.path(), "sender").await;
    let _ = pair(&sender, &receiver).await;
    let sender_id = sender.info().id;
    assert!(
        trusted_on_disk(&receiver, sender_id),
        "配对成功后接收端应当信任发起端"
    );

    // 让发起端**整个退出**：接收侧的会话随之消失，但信任记录还在 —— 这就是「无会话的已配对设备」。
    sender.shutdown().await;
    let session_gone = settles(|| !receiver.peers().iter().any(|peer| peer.id == sender_id)).await;
    assert!(session_gone, "发起端退出后接收侧的会话表应当清空");
    assert!(
        receiver
            .trusted_peers()
            .iter()
            .any(|entry| entry.id == sender_id),
        "会话没了但信任记录还在 —— 读侧必须能列出它（本任务要覆盖的正是这一类设备）"
    );
    assert!(
        trusted_on_disk(&receiver, sender_id),
        "读侧列表不是内存幻觉：磁盘上也还在"
    );

    // 走同一个移除入口：没有会话也照样能移除。
    assert!(
        receiver.revoke_trust(sender_id).await.unwrap(),
        "无会话设备也该能移除"
    );
    assert!(
        !trusted_on_disk(&receiver, sender_id),
        "移除后信任库必须干净"
    );
    assert!(
        !receiver
            .trusted_peers()
            .iter()
            .any(|entry| entry.id == sender_id),
        "读侧列表也不该再有它（撤回后列表要立刻更新）"
    );

    // 「移除生效」的可判定证据：同一身份再连回来必须**重新配对**。
    let sender_again = engine(dir.path(), "sender").await;
    assert_eq!(
        sender_again.info().id,
        sender_id,
        "同一个 data_dir 应当是同一身份（否则后半段断言就没意义了）"
    );
    let again = sender_again
        .connect(receiver.local_addr())
        .await
        .unwrap_err();
    assert_eq!(
        again.code(),
        ErrorCode::NotPaired,
        "被移除的设备再连接必须重新配对"
    );

    sender_again.shutdown().await;
    receiver.shutdown().await;
    accept.abort();
}
