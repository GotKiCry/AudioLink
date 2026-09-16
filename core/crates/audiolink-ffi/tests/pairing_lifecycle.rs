//! 真 QUIC 经 FFI 查询 PIN：断开、重连、重启以及双向配对。
//! 独立测试进程隔离 FFI 全局引擎；不访问声卡或 Android 设备。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use audiolink_engine::{Engine, EngineConfig, EngineEvent};
use audiolink_ffi::{
    EngineStartConfig, connect, displayed_pin, engine_start, engine_stop, peers, submit_pin,
};
use audiolink_types::ErrorCode;

async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("配对状态应在 5 s 内更新");
}

async fn remote(dir: &std::path::Path, name: &str) -> Arc<Engine> {
    let mut config = EngineConfig::new(name, dir.join(name));
    config.listen = "127.0.0.1:0".parse().unwrap();
    Engine::start(config).await.unwrap()
}

fn free_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ffi_pin_tracks_the_current_connection_and_engine() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = EngineStartConfig {
        node_name: "phone".into(),
        data_dir: dir.path().join("phone").to_string_lossy().into(),
        listen_port: free_port(),
    };
    assert!(displayed_pin().unwrap().is_none());
    let local = engine_start(config.clone(), None, None).await.unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], config.listen_port));
    let phone_id = audiolink_types::NodeId::from_hex(&local.id_hex).unwrap();

    // 连接后只靠同步查询取 PIN，不消费引擎广播。对端退出后不能遗留数字。
    let first = remote(dir.path(), "first").await;
    assert_eq!(
        first.connect(addr).await.unwrap_err().code(),
        ErrorCode::NotPaired
    );
    until(|| displayed_pin().unwrap().is_some()).await;
    let pin = displayed_pin().unwrap().unwrap();
    assert_eq!(pin.len(), 6);
    assert!(
        peers()
            .unwrap()
            .iter()
            .any(|p| p.name == "first" && !p.trusted)
    );
    first.shutdown().await;
    until(|| peers().unwrap().is_empty()).await;
    assert!(displayed_pin().unwrap().is_none(), "断开后不能返回旧 PIN");
    drop(first);

    // 同一身份重连。错误 PIN 仍可重试；成功后 PIN 消失且双方受信。
    let first = remote(dir.path(), "first").await;
    assert_eq!(
        first.connect(addr).await.unwrap_err().code(),
        ErrorCode::NotPaired
    );
    until(|| displayed_pin().unwrap().is_some()).await;
    let pin = displayed_pin().unwrap().unwrap();
    let bad = if pin == "000000" { "000001" } else { "000000" };
    let mut events = first.subscribe();
    first.submit_pin(phone_id, bad).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !matches!(
            events.recv().await.unwrap(),
            EngineEvent::PairCompleted { ok: false, .. }
        ) {}
    })
    .await
    .unwrap();
    assert_eq!(displayed_pin().unwrap().as_deref(), Some(pin.as_str()));
    first.submit_pin(phone_id, &pin).await.unwrap();
    until(|| first.peers().iter().any(|p| p.trusted)).await;
    until(|| displayed_pin().unwrap().is_none()).await;
    assert!(peers().unwrap().iter().any(|p| p.trusted));
    first.shutdown().await;
    until(|| peers().unwrap().is_empty()).await;

    // 未配对时停止再重启；旧连接/旧引擎不得把 PIN 回写到新实例。
    let second = remote(dir.path(), "second").await;
    assert_eq!(
        second.connect(addr).await.unwrap_err().code(),
        ErrorCode::NotPaired
    );
    until(|| displayed_pin().unwrap().is_some()).await;
    engine_stop().await.unwrap();
    assert!(displayed_pin().unwrap().is_none());
    second.shutdown().await;
    // PIN 生命周期独立于 QUIC 端口释放：同端口立即重启的既存 10048 竞态由单独看板项追踪。
    config.listen_port = free_port();
    engine_start(config, None, None).await.unwrap();
    assert!(displayed_pin().unwrap().is_none());
    assert!(peers().unwrap().is_empty());

    // FFI 作为发起端也从当前会话定位待输入 PIN 的对端。
    let third = remote(dir.path(), "third").await;
    let accept = third.spawn_accept_loop();
    assert_eq!(
        connect(third.local_addr().to_string())
            .await
            .unwrap_err()
            .code(),
        ErrorCode::NotPaired.as_u16()
    );
    until(|| third.displayed_pin().is_some()).await;
    submit_pin(third.displayed_pin().unwrap()).await.unwrap();
    until(|| peers().unwrap().iter().any(|p| p.trusted)).await;
    until(|| third.displayed_pin().is_none()).await;
    engine_stop().await.unwrap();
    third.shutdown().await;
    accept.abort();
    assert!(displayed_pin().unwrap().is_none());
}
