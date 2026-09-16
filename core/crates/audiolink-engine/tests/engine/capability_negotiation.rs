//! §13 能力协商：**本机声明的能力**真的到了对端（真实 QUIC 端到端）。
//!
//! 这条测试要证明的是「平台能力注入确实生效」：`EngineConfig::capabilities` 不只是个字段 ——
//! 它会被带进握手、参与协商，并出现在**对端**的 `peers()` 里，也就是界面读到的那份数据。
//!
//! 场景刻意选了「一端有内录、另一端没有」：这正是能力协商要处理的那种不对称，
//! 也是界面按 `missingKeys` 置灰的输入。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::{Capabilities, ErrorCode};

/// 等两端都看到对方进入会话（握手完成）。
async fn wait_established(desktop: &Engine, plain: &Engine, plain_id: audiolink_types::NodeId) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let forward = desktop
            .peers()
            .iter()
            .any(|peer| peer.id == plain_id && peer.state == SessionState::Streaming);
        let backward = !plain.peers().is_empty();
        if forward && backward {
            return;
        }
        assert!(Instant::now() < deadline, "握手没有在 15 s 内完成");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn declared_platform_capabilities_reach_the_peer() {
    let dir = tempfile::TempDir::new().expect("临时目录");

    // 桌面端形态：内核能力 + 系统内录（Windows WASAPI loopback，外壳已声明）。
    let mut desktop_config = EngineConfig::new("desktop", dir.path().join("desktop"));
    desktop_config.listen = "127.0.0.1:0".parse().unwrap();
    desktop_config.capabilities = Capabilities::CURRENT | Capabilities::SYSTEM_LOOPBACK;

    // 另一端故意不带内录：模拟尚未接上内录的平台（例如现在的 Android）。
    let mut plain_config = EngineConfig::new("plain", dir.path().join("plain"));
    plain_config.listen = "127.0.0.1:0".parse().unwrap();
    plain_config.capabilities = Capabilities::CURRENT;

    let plain = Engine::start(plain_config).await.expect("plain 引擎");
    let _accept = plain.spawn_accept_loop();
    let mut events = plain.subscribe();
    let plain_id = plain.info().id;

    let desktop = Engine::start(desktop_config).await.expect("desktop 引擎");

    // 首次连接必须先走 PIN 配对（§5）。
    let error = desktop.connect(plain.local_addr()).await.unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotPaired, "首次连接必须先要 PIN");
    let pin = loop {
        if let EngineEvent::DisplayPin { pin, .. } = events.recv().await.unwrap() {
            break pin;
        }
    };
    desktop.submit_pin(plain_id, &pin).await.expect("PIN 配对");

    wait_established(&desktop, &plain, plain_id).await;

    // 关键断言：plain 看到 desktop 的能力里**有**系统内录 —— 说明注入的位真的出网了。
    let desktop_id = desktop.info().id;
    let seen = plain
        .peers()
        .into_iter()
        .find(|peer| peer.id == desktop_id)
        .and_then(|peer| peer.capabilities)
        .expect("握手完成后应当有协商结果");
    assert_eq!(
        seen.peer & Capabilities::SYSTEM_LOOPBACK,
        Capabilities::SYSTEM_LOOPBACK,
        "对端声明的系统内录没传过来：平台能力注入没有生效"
    );
    assert_eq!(
        seen.local & Capabilities::SYSTEM_LOOPBACK,
        0,
        "本端声明的是不带内录的能力集"
    );
    assert_eq!(
        seen.agreed & Capabilities::SYSTEM_LOOPBACK,
        0,
        "交集里不含内录（本端没有）—— 「对端有、本端没有」正是界面要说明的那种情况"
    );
    assert_eq!(
        seen.local & Capabilities::OPUS,
        Capabilities::OPUS,
        "必需能力必须在交集里，否则会话根本建不起来"
    );

    // 反方向同理：desktop 看 plain 也不带内录。
    let seen_back = desktop
        .peers()
        .into_iter()
        .find(|peer| peer.id == plain_id)
        .and_then(|peer| peer.capabilities)
        .expect("反向也应有协商结果");
    assert_eq!(seen_back.peer & Capabilities::SYSTEM_LOOPBACK, 0);
    assert_eq!(
        seen_back.local & Capabilities::SYSTEM_LOOPBACK,
        Capabilities::SYSTEM_LOOPBACK
    );
}
