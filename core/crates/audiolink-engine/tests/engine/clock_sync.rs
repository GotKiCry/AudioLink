//! §6 时钟同步的**真 QUIC** 集成测试。
//!
//! 接法照抄 `core/crates/audiolink-tools/src/bin/link_loop.rs`：同一进程里起**两个真实 Engine**，
//! 让它们走真实 QUIC（TLS 握手 + 证书指纹互认 + §5 握手 + PIN 配对实跑），
//! 而不是拿假 Connection 去测。
//!
//! # 为什么断言 `|offset| ≤ 2 ms` 是有意义的
//!
//! 两个引擎在**同一进程**里，`now_monotonic_us()` 共享同一个单调时钟基准 ⇒ 真实偏移为 0。
//! 于是这条断言量的正是「估计器 + 收发节奏」引入的全部误差（调度、编码、锁）。
//! 若哪天把 t1 取早了、或把 t3 算错了，误差会立刻从微秒级跳到毫秒级，这里就会红。
//!
//! # 引擎配置里为什么没有采集/播放工厂
//!
//! 时钟探针走**数据报**、不吃声卡：会话可通信（§5 握手完成）就开始探测。
//! 用空配置把音频路径整条排除在测试之外，断言只落在 §6 上；真机那侧的音频链路由
//! `link-loop` / 真机验收各自覆盖。
//!
//! # 未验证（不许推断）
//!
//! 这里是 **127.0.0.1** 回环：Wi-Fi 下的抖动、丢包与调度差异**未验证**；
//! PC→Android 真机链路**未验证**（本机 adb 无设备）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // 测试代码不受实时路径的三条禁令约束

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use audiolink_engine::runtime::DEFAULT_HANDSHAKE_TIMEOUT;
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState, now_monotonic_us};
use audiolink_identity::NodeIdentity;
use audiolink_net::{AudioLinkEndpoint, Connection, EndpointConfig};
use audiolink_proto::{AudioDatagram, ClockProbe, ClockReply};
use audiolink_types::{ErrorCode, NodeId, StreamStats};

/// 起一台**纯控制面**引擎（无采集 / 无播放）。
async fn start_engine(dir: &Path, name: &str, frame_ms: u32) -> Arc<Engine> {
    start_engine_with_handshake(dir, name, frame_ms, DEFAULT_HANDSHAKE_TIMEOUT).await
}

/// 同上，但显式指定**非配对阶段**的握手死线。
///
/// 测量连接那条用例要「越过死线还活着」，把 10 s 压到 1 s 就能秒级验完同一段逻辑
/// （真实 I/O 下 tokio 时钟暂停不可靠，改常量比 pause/advance 诚实）。
async fn start_engine_with_handshake(
    dir: &Path,
    name: &str,
    frame_ms: u32,
    handshake_timeout: Duration,
) -> Arc<Engine> {
    let node_dir = dir.join(name);
    std::fs::create_dir_all(&node_dir).expect("建节点目录");
    let mut config = EngineConfig::new(name, &node_dir);
    config.listen = "127.0.0.1:0".parse().expect("回环地址");
    config.codec.frame_ms = frame_ms;
    config.handshake_timeout = handshake_timeout;
    Engine::start(config).await.expect("启动引擎")
}

/// 连接 + （必要时）走完 §5 的 PIN 配对，返回本端会话表里的对端 id。
async fn connect_and_pair(engine_a: &Arc<Engine>, engine_b: &Arc<Engine>) -> NodeId {
    // 第二个对端时 peers() 里已经有别的会话了，所以先记下「连之前有谁」，再认新面孔。
    let before: Vec<NodeId> = engine_a.peers().into_iter().map(|peer| peer.id).collect();
    let mut events = engine_b.subscribe();
    let addr = engine_b.local_addr();

    match engine_a.connect(addr).await {
        Ok(peer) => peer,
        Err(error) if error.code() == ErrorCode::NotPaired => {
            let pin = wait_for_pin(&mut events, Duration::from_secs(5)).await;
            let peer_on_a = engine_a
                .peers()
                .into_iter()
                .map(|peer| peer.id)
                .find(|id| !before.contains(id))
                .expect("新会话必须已经登记进会话表");
            engine_a
                .submit_pin(peer_on_a, &pin)
                .await
                .expect("提交 PIN");
            wait_for_streaming(engine_a, peer_on_a, Duration::from_secs(5)).await;
            peer_on_a
        }
        Err(error) => panic!("连接失败（{} {}）", error.code().as_u16(), error.context()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clock_sync_converges_over_real_quic_within_two_seconds() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let engine_a = start_engine(dir.path(), "node-a", 20).await;
    let engine_b = start_engine(dir.path(), "node-b", 20).await;
    let _accept_a = engine_a.spawn_accept_loop();
    let _accept_b = engine_b.spawn_accept_loop();

    let peer_on_a = connect_and_pair(&engine_a, &engine_b).await;
    let peer_on_b = engine_b
        .peers()
        .into_iter()
        .next()
        .map(|peer| peer.id)
        .expect("node-b 侧必须已建会话");
    assert_eq!(peer_on_a, engine_b.info().id, "对端身份 = 证书指纹");
    assert_eq!(peer_on_b, engine_a.info().id);

    // ---- §6.0 首连快速同步：100 ms × 50 ⇒ 2 s 内必须攒够 8 个样本 ----
    // 计时起点 = 会话刚可用（与下面的死线同一时刻），量的是「**收敛**要多久」。
    let started = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(2);
    let estimate = loop {
        if let Some(estimate) = engine_a.clock_estimate(peer_on_a)
            && estimate.samples >= 8
        {
            break estimate;
        }
        assert!(
            Instant::now() < deadline,
            "2 s 内未收敛到 8 个样本：{:?}",
            engine_a.clock_probe_stats(peer_on_a)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    assert!(estimate.samples >= 8, "samples = {}", estimate.samples);
    assert!(
        estimate.offset_us.abs() <= 2_000,
        "|offset| = {} µs 超过 2 ms（同进程真值 ≈ 0）",
        estimate.offset_us
    );
    println!(
        "[clock-sync] node-a 会话可用后 {:.0} ms 收敛 · samples={} · offset={} µs · drift={} ppm · quality={:?} · 代表 RTT={} µs",
        started.elapsed().as_secs_f64() * 1000.0,
        estimate.samples,
        estimate.offset_us,
        estimate.drift_ppm,
        estimate.quality,
        estimate.rtt_us
    );

    // ---- 计数口径：发出 / 收到 / 配对 / 配不上 ----
    let stats_a = engine_a
        .clock_probe_stats(peer_on_a)
        .expect("会话在，统计必须在");
    assert!(stats_a.sent >= 8, "sent = {}", stats_a.sent);
    assert!(stats_a.replies >= 8, "replies = {}", stats_a.replies);
    assert_eq!(
        stats_a.unmatched, 0,
        "每个应答都该配上未决探测：{stats_a:?}"
    );
    assert!(
        stats_a.received >= 8,
        "两端都探测：node-a 也该收到并应答 node-b 的探测（received = {}）",
        stats_a.received
    );

    // ---- RTT 环：≥ 8 个样本后必须给分位数（不是 Some(0)、也不是 None）----
    assert!(
        stats_a.rtt_samples >= 8,
        "rtt_samples = {}",
        stats_a.rtt_samples
    );
    let p50 = stats_a.rtt_p50_us.expect("≥ 8 个样本后必须给出 P50");
    let p95 = stats_a.rtt_p95_us.expect("≥ 8 个样本后必须给出 P95");
    assert!(p50 <= p95, "P50 {p50} µs 不应大于 P95 {p95} µs");
    assert!(p95 < 1_000_000, "回环 RTT P95 = {p95} µs 量级不合理");
    println!(
        "[clock-sync] node-a 计数 sent={} received={} replies={} unmatched={} · RTT 环 {} 样本 P50={} P95={} µs",
        stats_a.sent,
        stats_a.received,
        stats_a.replies,
        stats_a.unmatched,
        stats_a.rtt_samples,
        p50,
        p95
    );

    // ---- 响应方也探测：node-b 侧必须有**自己的**估计（M1 跨机测量靠它）----
    let estimate_b = engine_b
        .clock_estimate(peer_on_b)
        .expect("node-b 也在探测，必须收敛");
    assert!(estimate_b.samples >= 8, "samples = {}", estimate_b.samples);
    assert!(
        estimate_b.offset_us.abs() <= 2_000,
        "node-b 侧 |offset| = {} µs",
        estimate_b.offset_us
    );

    // ---- 对端 1 Hz STREAM_STATS 最近快照（按 peer 隔离）----
    //
    // 注意键：会话表按**对端** id 建，所以这里要用本端视角的 peer_on_a；
    // 用 node-b 视角的 peer_on_b（= node-a 自己的指纹）会查不到——那正是本接口按 peer 隔离的证据。
    let peer_stats = wait_for_peer_stats(&engine_a, peer_on_a, Duration::from_secs(3)).await;
    assert_eq!(peer_stats.stream_id, 1);
    assert!(
        engine_a.peer_stats(peer_on_b).is_none(),
        "用本机自己的指纹查对端快照必须查不到"
    );

    engine_a.shutdown().await;
    engine_b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn peer_stats_is_isolated_per_peer() {
    let dir = tempfile::TempDir::new().expect("临时目录");

    // 三台引擎：A 同时连 B 与 C。B / C 的帧长不同 ⇒ 快照里的 codec.frame_ms 就是「这份快照来自谁」的指纹。
    let engine_a = start_engine(dir.path(), "node-a", 20).await;
    let engine_b = start_engine(dir.path(), "node-b", 20).await;
    let engine_c = start_engine(dir.path(), "node-c", 10).await;
    let _accept_a = engine_a.spawn_accept_loop();
    let _accept_b = engine_b.spawn_accept_loop();
    let _accept_c = engine_c.spawn_accept_loop();

    let peer_b = connect_and_pair(&engine_a, &engine_b).await;
    let peer_c = connect_and_pair(&engine_a, &engine_c).await;
    assert_ne!(peer_b, peer_c, "两条会话必须是对端各异的");

    let stats_b = wait_for_peer_stats(&engine_a, peer_b, Duration::from_secs(3)).await;
    let stats_c = wait_for_peer_stats(&engine_a, peer_c, Duration::from_secs(3)).await;

    assert_eq!(stats_b.codec.frame_ms, 20, "node-b 的快照被 node-c 覆盖了");
    assert_eq!(stats_c.codec.frame_ms, 10, "node-c 的快照被 node-b 覆盖了");

    // 未知对端：三个 API 都必须给 None（**不是** default / 全 0 —— 那会把「没这回事」伪装成「测到了 0」）。
    let ghost = NodeId::from_bytes([0u8; 32]);
    assert!(engine_a.peer_stats(ghost).is_none());
    assert!(engine_a.clock_estimate(ghost).is_none());
    assert!(engine_a.clock_probe_stats(ghost).is_none());

    engine_a.shutdown().await;
    engine_b.shutdown().await;
    engine_c.shutdown().await;
}

/// 一条**只发数据报、不跑 §5 握手**的测量连接（= `tools/latency-probe` 的接法）也必须拿到 REPLY。
///
/// 这是真机验收量「PC→手机网络 RTT」的既定手段，所以它不能依赖会话状态。
/// 测试刻意越过握手死线再探一次：真机验收要量的是 30 s 级别的 RTT，
/// 死线一到就把连接收回去的话，那份测量会在第 10 秒静默截断。
/// 死线在本用例里压到 1 s（生产默认 10 s，由 `DEFAULT_HANDSHAKE_TIMEOUT` 钉住），
/// 于是整套用例 2 s 多就跑完，不必真等 11 s。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn measurement_connection_gets_replies_without_the_session_handshake() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let engine =
        start_engine_with_handshake(dir.path(), "node-a", 20, Duration::from_secs(1)).await;
    let _accept = engine.spawn_accept_loop();

    // 客户端身份自签（net 只认真实证书链，所以必须是一份真证书，不能用假指纹）。
    let client = NodeIdentity::load_or_create(dir.path().join("probe-client"), "probe-client")
        .expect("生成测量端身份");
    let endpoint = AudioLinkEndpoint::bind(EndpointConfig {
        bind: "127.0.0.1:0".parse().expect("回环地址"),
        cert_der: client.cert_der().to_vec(),
        key_der_pkcs8: client.key_der_pkcs8().to_vec(),
        ..EndpointConfig::default()
    })
    .await
    .expect("绑定测量端点");
    let connection = endpoint
        .connect(engine.local_addr(), "audiolink")
        .await
        .expect("连接引擎");

    // ① 握手前：必须立刻有应答。
    let t1 = now_monotonic_us();
    assert_probe_reply(&connection, 7, t1).await;

    // ② 越过握手死线（本用例把它压到 1 s）：连接仍活、仍应答。
    tokio::time::sleep(Duration::from_secs(2)).await;
    let t1 = now_monotonic_us();
    assert_probe_reply(&connection, 8, t1).await;

    // 引擎侧：这两个探测都记在账上，且**没有**建立会话（状态仍是 Handshaking）。
    let stats = engine
        .clock_probe_stats(client.id())
        .expect("入站连接必须已经登记进会话表");
    assert!(stats.received >= 2, "received = {}", stats.received);
    assert_eq!(stats.sent, 0, "纯测量客户端：本机没探测");
    let peer = engine
        .peers()
        .into_iter()
        .find(|peer| peer.id == client.id())
        .expect("对端必须在会话表里");
    assert_eq!(
        peer.state,
        SessionState::Handshaking,
        "这条连接从头到尾没跑 §5 握手"
    );
    println!(
        "[clock-sync] 裸测量连接（不跑 §5 握手）：received={} sent={} · 越过握手死线仍应答（用例压到 1 s；生产 10 s）",
        stats.received, stats.sent
    );

    endpoint.close(0, "test done");
    engine.shutdown().await;
}

/// 发一个 CLOCK_PROBE 并断言拿回来的 REPLY 逐字段正确（§6：长度 28 B、`t2 == t3`、序号/t1 原样回填）。
async fn assert_probe_reply(connection: &Connection, probe_seq: u32, t1: u64) {
    let t1_i64 = i64::try_from(t1).expect("时间戳在 i64 范围内");
    let wire = ClockProbe {
        probe_seq,
        t1: t1_i64,
    }
    .to_datagram_bytes()
    .expect("编码 CLOCK_PROBE");
    connection.send_datagram(&wire).await.expect("发送探测");

    let mut buf = vec![0u8; 1200];
    let len = tokio::time::timeout(
        Duration::from_secs(2),
        connection.read_datagram_into(&mut buf),
    )
    .await
    .expect("2 s 内必须收到 CLOCK_REPLY")
    .expect("读数据报");
    let datagram = AudioDatagram::decode(&buf[..len]).expect("解整包");
    assert_eq!(datagram.payload.len(), 28, "CLOCK_REPLY 载荷必须恰好 28 B");

    let reply = ClockReply::from_datagram(&datagram).expect("解 CLOCK_REPLY");
    assert_eq!(reply.probe_seq, probe_seq, "probe_seq 必须原样回填");
    assert_eq!(reply.t1, t1_i64, "t1 必须原样回填");
    assert_eq!(reply.t2, reply.t3, "§6：t3 = t2");
}

// ---------------------------------------------------------------------------
// 等待助手（照 link-loop 的口径）
// ---------------------------------------------------------------------------

async fn wait_for_peer_stats(engine: &Arc<Engine>, peer: NodeId, timeout: Duration) -> StreamStats {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(stats) = engine.peer_stats(peer) {
            return stats;
        }
        assert!(Instant::now() < deadline, "等待对端 STREAM_STATS 超时");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_for_pin(
    events: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
    timeout: Duration,
) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        assert!(Instant::now() < deadline, "等待对端展示 PIN 超时");
        match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
            Ok(Ok(EngineEvent::DisplayPin { pin, .. })) => return pin,
            Ok(Ok(_)) => continue,
            Ok(Err(error)) => panic!("事件订阅中断：{error}"),
            Err(_) => continue,
        }
    }
}

async fn wait_for_streaming(engine: &Arc<Engine>, peer: NodeId, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if engine
            .peers()
            .into_iter()
            .any(|status| status.id == peer && status.state == SessionState::Streaming)
        {
            return;
        }
        assert!(Instant::now() < deadline, "等待会话进入 Streaming 超时");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// M3 验收「时钟」的回环版本：**稳定后 offset 抖动 ≤ 2 ms**。
///
/// 与上一条测试的分工很清楚：
/// * 上一条量的是**收敛**（2 s 内攒够 8 个样本、单次 `|offset| ≤ 2 ms`）；
/// * 这一条量的是**稳定之后抖不抖** —— 抓的是缓慢漂移、周期性跳变这类问题，
///   它们不会体现在「一次快照」上。
///
/// 真机上的抖动还包含晶振漂移（靠 `drift_ppm` 做速率补偿），回环里没有硬件漂移，
/// 所以这条主要证明**估计器本身是稳的**；硬件那部分仍属 M3 的真机验收（见 `docs/05`）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offset_jitter_stays_within_two_milliseconds_after_convergence() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let engine_a = start_engine(dir.path(), "node-a", 20).await;
    let engine_b = start_engine(dir.path(), "node-b", 20).await;
    let _accept_a = engine_a.spawn_accept_loop();
    let _accept_b = engine_b.spawn_accept_loop();
    let peer_on_a = connect_and_pair(&engine_a, &engine_b).await;

    // 每 100 ms 采一次（探测间隔也是 100 ms，所以这等于「每个新估计采一次」）。
    let mut offsets: Vec<i64> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if let Some(estimate) = engine_a.clock_estimate(peer_on_a) {
            offsets.push(estimate.offset_us);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // 丢掉前 2 s：那是收敛期，收敛过程本身不该算进「稳定后的抖动」。
    let skip = 20;
    assert!(
        offsets.len() > skip + 50,
        "稳定期样本太少：{}",
        offsets.len()
    );
    let settled = &offsets[skip..];
    let min = *settled.iter().min().expect("非空");
    let max = *settled.iter().max().expect("非空");
    let jitter = max - min;
    println!(
        "[clock-jitter] 稳定期 {} 个样本 · offset {}..{} µs · 抖动 {} µs（验收线 2000 µs）",
        settled.len(),
        min,
        max,
        jitter
    );
    assert!(
        jitter <= 2_000,
        "offset 抖动 {jitter} µs 超过 2 ms（区间 {min}..{max}，样本 {}）",
        settled.len()
    );
}
