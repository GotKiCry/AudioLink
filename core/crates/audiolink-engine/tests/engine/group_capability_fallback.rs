//! §13 可选能力缺失时内核的兜底（task-27）：**功能调用**必须报错，**连接**不受影响。
//!
//! # 缺口
//!
//! handshake.rs 的两处判定只管**连接**：交集缺 Capabilities::REQUIRED（只有 Opus）就拒绝，
//! 缺**可选位**则连接照旧建立（audiolink-types 的 Capabilities 文档原文）。但 create_group /
//! join_group 路径此前**没有任何 capability 判定** —— 向一个协商结果里明确不含 §7 排播位
//! （GROUP_EPOCH）的成员建组，会得到一个 Ok 与一条与成功时一模一样的 GroupUpdated：
//! **降级完全依赖界面把复选框禁用**，非 UI 调用方（FFI / CLI / 测试）拿不到任何错误，只能拿着
//! 成功回执去期待一个不会发生的同步效果（第 103 轮审计清单 B）。
//!
//! # 现在的语义
//!
//! GROUP_EPOCH 是「组内排播」的**硬前提**（界面侧早已据此置灰建组入口），所以两个**主动入口**
//! 都拒绝：create_group 与 join_group 返回 Err(CapUnsupported / 1005)，且 context **点名是哪个
//! 成员**缺这一位 —— 拒绝要能让人行动。拒绝零副作用：组账本、广播帧都不动。
//!
//! 这**不**改 §13 的连接语义：缺可选位时连接照旧建立，本文件把它也钉成断言。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use audiolink_engine::{Engine, EngineConfig, SessionState};
use audiolink_types::{Capabilities, ErrorCode, NodeId};

/// 起一个接收端：能力位可定制（缺省 = 内核完整能力，含 §7 排播）。
async fn start_receiver(
    dir: &Path,
    name: &str,
    capabilities: u32,
) -> (Arc<Engine>, tokio::task::JoinHandle<()>) {
    let mut config = EngineConfig::new(name, dir.join(name));
    config.listen = r"127.0.0.1:0".parse().unwrap();
    config.capabilities = capabilities;
    let engine = Engine::start(config).await.expect(r"接收引擎");
    let accept = engine.spawn_accept_loop();
    (engine, accept)
}

/// 有界等待：条件在预算内成立返回 true（不 panic，读数交给调用方）。
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

/// 连接接收端（无认证：一次 connect 即完成握手），返回接收端 id（同时等到发送侧进入 Streaming）。
async fn connect_peer(sender: &Arc<Engine>, receiver: &Arc<Engine>) -> NodeId {
    let receiver_id = receiver.info().id;

    sender
        .connect(receiver.local_addr())
        .await
        .expect("连接应当直接成功（无认证）");
    let streaming = wait_for(Duration::from_secs(10), || {
        sender
            .peers()
            .iter()
            .any(|peer| peer.id == receiver_id && peer.state == SessionState::Streaming)
    })
    .await;
    assert!(streaming, r"发送侧没有在 10 s 内进入 Streaming");
    receiver_id
}

/// 读某个对端协商结果里的交集位图。
fn agreed_of(engine: &Engine, peer: NodeId) -> u32 {
    engine
        .peers()
        .into_iter()
        .find(|status| status.id == peer)
        .and_then(|status| status.capabilities)
        .expect(r"握手完成后应当有协商结果")
        .agreed
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn group_calls_reject_members_without_group_epoch() {
    let dir = tempfile::TempDir::new().expect(r"临时目录");

    // 发送端：完整能力。
    let mut send_config = EngineConfig::new(r"sender", dir.path().join(r"sender"));
    send_config.listen = r"127.0.0.1:0".parse().unwrap();
    let sender = Engine::start(send_config).await.expect(r"发送引擎");

    // 两个接收端：一个支持 §7 排播，一个**故意不声明**（模拟尚未实现同步组的对端）。
    let (receiver_ok, accept_ok) =
        start_receiver(dir.path(), r"receiver-ok", Capabilities::CURRENT).await;
    let (receiver_bad, accept_bad) = start_receiver(
        dir.path(),
        r"receiver-bad",
        Capabilities::CURRENT & !Capabilities::GROUP_EPOCH,
    )
    .await;

    let ok = connect_peer(&sender, &receiver_ok).await;
    let bad = connect_peer(&sender, &receiver_bad).await;

    // ⓪ 前提：协商结果里「谁支持」是确定的 —— 内核手里本来就有这个事实。
    assert_ne!(
        agreed_of(&sender, ok) & Capabilities::GROUP_EPOCH,
        0,
        r"支持的一端协商结果里应当有 §7 排播位"
    );
    assert_eq!(
        agreed_of(&sender, bad) & Capabilities::GROUP_EPOCH,
        0,
        r"不支持的一端协商结果里不该有 §7 排播位（这是本测试的输入前提）"
    );

    // ① §13 的连接语义不变：缺**可选位**照样连上、照样 Streaming。
    for peer in [ok, bad] {
        let state = sender
            .peers()
            .into_iter()
            .find(|status| status.id == peer)
            .map(|status| status.state);
        assert_eq!(
            state,
            Some(SessionState::Streaming),
            r"缺可选位不该影响连接：该对端应当仍是 Streaming"
        );
    }

    // ② 正向：支持的一端可以建组（正常路径没有被兜底误伤）。
    let group_id = sender
        .create_group(&[ok], 150)
        .await
        .expect(r"支持 §7 排播的成员应当能建组");

    // ③ 反向：不支持的成员 —— 必须报错，且错误里点名是**谁**缺**哪一位**。
    let error = sender
        .create_group(&[bad], 150)
        .await
        .expect_err(r"向不支持 §7 排播的成员建组必须报错，而不是静默建一个不会同步的组");
    assert_eq!(
        error.code(),
        ErrorCode::CapUnsupported,
        r"错误码应当是 1005"
    );
    assert!(
        error.context().contains(&bad.short()),
        r"错误必须点名是哪个成员：{}",
        error.context()
    );
    assert!(
        error.context().contains(r"同步组排播"),
        r"错误必须说清缺的是哪一位能力：{}",
        error.context()
    );

    // ④ 混合名单：只有不支持的那个被点名，支持的不该被冤枉。
    let error = sender
        .create_group(&[ok, bad], 150)
        .await
        .expect_err(r"名单里混入不支持的成员也必须整单拒绝");
    assert_eq!(error.code(), ErrorCode::CapUnsupported);
    assert!(
        error.context().contains(&bad.short()),
        r"{}",
        error.context()
    );
    assert!(
        !error.context().contains(&ok.short()),
        r"支持 §7 排播的成员不该出现在拒绝原因里：{}",
        error.context()
    );

    // ⑤ 第二个入口：join_group 同样拒绝，且同样点名。
    let error = sender
        .join_group(bad, group_id)
        .await
        .expect_err(r"join_group 是同一个根因的第二个入口，也必须报错");
    assert_eq!(error.code(), ErrorCode::CapUnsupported);
    assert!(
        error.context().contains(&bad.short()),
        r"{}",
        error.context()
    );

    // ⑥ 支持的一端照旧能加入（第二个入口的正常路径）。
    sender
        .join_group(ok, group_id)
        .await
        .expect(r"支持 §7 排播的成员应当能加入");

    // ⑦ 拒绝必须**零副作用**：账本里只有一个组、只有支持的那个成员。
    let groups = sender.groups();
    assert_eq!(groups.len(), 1, r"只有一次成功的建组，账本里应当恰好一个组");
    let members: Vec<NodeId> = groups[0].members.iter().map(|member| member.id).collect();
    assert_eq!(
        members,
        vec![ok],
        r"被拒绝的成员绝不能出现在组账本里（拒绝发生在写账本之前）"
    );

    sender.shutdown().await;
    receiver_ok.shutdown().await;
    receiver_bad.shutdown().await;
    accept_ok.abort();
    accept_bad.abort();
}
