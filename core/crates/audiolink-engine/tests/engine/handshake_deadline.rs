//! §5 握手死线的回归测试。
//!
//! 无认证（`docs/71-remove-pairing.md`）之后握手只剩 `HELLO` / `HELLO_ACK` 一次往返，
//! 死线的职责收窄成一条：**不发 `HELLO`、也不探测的连接必须被回收** —— 半开连接不许占着会话。
//!
//! # 为什么这里用「缩短过的常量」而不是真等 10 s
//!
//! 真实 I/O 下 tokio 的 `time::pause()` 不可靠（auto-advance 会在真实等待里把定时器一起推掉），
//! 所以生产常量做成了 `EngineConfig::handshake_timeout`，测试把它压到 1 s；
//! 生产数字本身由 `production_defaults_keep_the_handshake_deadline` 钉住（不真等）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // 测试代码不受实时路径三条禁令约束

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use audiolink_engine::{Engine, EngineConfig, SessionState};
use audiolink_identity::NodeIdentity;
use audiolink_net::{AudioLinkEndpoint, EndpointConfig};
use audiolink_types::NodeId;

/// 压缩后的握手死线（生产值 10 s）。
const SHORT_HANDSHAKE: Duration = Duration::from_secs(1);

#[test]
fn production_defaults_keep_the_handshake_deadline() {
    // 无认证把握手压成一次机器时间往返（`HELLO` → `HELLO_ACK`），10 s 只剩「回收半开连接」这一个作用。
    let dir = std::env::temp_dir().join("audiolink-handshake-defaults");
    let config = EngineConfig::new("defaults", &dir);

    assert_eq!(
        config.handshake_timeout,
        Duration::from_secs(10),
        "握手死线仍是 10 s（半开连接的回收底线）"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connect_completes_on_the_first_try() {
    // 无认证的直接推论：第一次 `connect` 就该成功，没有「先要 PIN、再提交」的第二次机会。
    let dir = tempfile::TempDir::new().expect("临时目录");
    let engine_a = start_engine(dir.path(), "node-a", SHORT_HANDSHAKE).await;
    let engine_b = start_engine(dir.path(), "node-b", SHORT_HANDSHAKE).await;
    let _accept_a = engine_a.spawn_accept_loop();
    let _accept_b = engine_b.spawn_accept_loop();

    let peer_on_a = engine_a
        .connect(engine_b.local_addr())
        .await
        .expect("无认证：第一次 connect 就该成功");
    assert_eq!(peer_on_a, engine_b.info().id, "返回的对端身份是证书指纹");
    assert!(engine_a.peers().iter().all(|peer| peer.initiated_locally));
    assert!(engine_b.peers().iter().all(|peer| !peer.initiated_locally));

    wait_for_streaming(&engine_a, peer_on_a, Duration::from_secs(5)).await;

    engine_a.shutdown().await;
    engine_b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_that_neither_handshakes_nor_probes_is_reclaimed() {
    // 负例：连接建立后什么都不发（既不 `HELLO`、也不发探测）→ 必须按握手死线回收。
    let dir = tempfile::TempDir::new().expect("临时目录");
    let engine = start_engine(dir.path(), "node-a", SHORT_HANDSHAKE).await;
    let _accept = engine.spawn_accept_loop();

    let client =
        NodeIdentity::load_or_create(dir.path().join("idle-client"), "idle-client").expect("身份");
    let endpoint = AudioLinkEndpoint::bind(EndpointConfig {
        bind: "127.0.0.1:0".parse().expect("回环地址"),
        cert_der: client.cert_der().to_vec(),
        key_der_pkcs8: client.key_der_pkcs8().to_vec(),
        ..EndpointConfig::default()
    })
    .await
    .expect("绑定");
    // 绑定名刻意用 `_connection` 并保留到作用域末尾：句柄一 drop，quinn 就会关掉连接，
    // 会话任务会因**读错误**退出 —— 那样测到的就不是「握手死线回收」了。
    let _connection = endpoint
        .connect(engine.local_addr(), "audiolink")
        .await
        .expect("连接引擎");

    wait_until(Duration::from_secs(2), || {
        engine.clock_probe_stats(client.id()).is_some()
    })
    .await;
    assert!(
        engine.clock_probe_stats(client.id()).is_some(),
        "刚连上时会话必须在表里（否则下面的等待没有意义）"
    );

    // 什么都不发：握手死线到点后必须被回收。
    wait_until(Duration::from_secs(3), || {
        engine.clock_probe_stats(client.id()).is_none()
    })
    .await;
    assert!(
        engine.clock_probe_stats(client.id()).is_none(),
        "不发 HELLO、不发探测的连接必须被握手死线（{SHORT_HANDSHAKE:?}）回收"
    );
    assert!(engine.peers().is_empty(), "会话表必须清干净");

    endpoint.close(0, "test done");
    engine.shutdown().await;
}

// ---------------------------------------------------------------------------
// 助手
// ---------------------------------------------------------------------------

async fn start_engine(dir: &Path, name: &str, handshake_timeout: Duration) -> Arc<Engine> {
    let node_dir = dir.join(name);
    std::fs::create_dir_all(&node_dir).expect("建节点目录");
    let mut config = EngineConfig::new(name, &node_dir);
    config.listen = "127.0.0.1:0".parse().expect("回环地址");
    config.handshake_timeout = handshake_timeout;
    Engine::start(config).await.expect("启动引擎")
}

async fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "等待条件成立超时（{timeout:?}）");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_streaming(engine: &Arc<Engine>, peer: NodeId, timeout: Duration) {
    wait_until(timeout, || {
        engine
            .peers()
            .into_iter()
            .any(|status| status.id == peer && status.state == SessionState::Streaming)
    })
    .await;
}
