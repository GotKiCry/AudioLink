#![allow(clippy::unwrap_used, clippy::expect_used)]
use audiolink_engine::{Engine, EngineConfig};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_drains_an_outbound_handshake_and_prevents_late_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = EngineConfig::new("shutdown", dir.path());
    cfg.listen = "127.0.0.1:0".parse().unwrap();
    let engine = Engine::start(cfg).await.unwrap();
    let addr = engine.local_addr();
    let accept = engine.spawn_accept_loop();
    let blackhole = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let peer_addr = blackhole.local_addr().unwrap();
    let connecting_engine = Arc::clone(&engine);
    let connecting = tokio::spawn(async move { connecting_engine.connect(peer_addr).await });
    let mut initial = [0; 2048];
    tokio::time::timeout(Duration::from_secs(5), blackhole.recv(&mut initial))
        .await
        .unwrap()
        .unwrap();
    // 已发出 QUIC Initial，但对端从不回应；停止不能等待完整网络连接超时。
    tokio::time::timeout(Duration::from_secs(5), engine.shutdown())
        .await
        .unwrap();
    assert!(connecting.await.unwrap().is_err());
    accept.await.unwrap();
    assert!(engine.peers().is_empty());
    assert!(engine.connect(peer_addr).await.is_err());
    std::net::UdpSocket::bind(addr).expect("保留旧 Engine 也不能占用原端口");
    engine.spawn_accept_loop().await.unwrap();
    engine.shutdown().await;
}
