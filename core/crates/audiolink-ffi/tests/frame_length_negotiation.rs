//! 帧长联动（FFI 一跳）：协商生效的帧长必须到达**对端可见的** PeerView。
//!
//! # 为什么这条测试必须存在
//!
//! audiolink-engine 侧的 frame_length_negotiation.rs 能证明「内核会协商并跟随」，但证明不了
//! **FFI 这一跳**：`PeerStatus.negotiated_frame_ms` → `PeerView.negotiated_frame_ms`。
//! 这一跳正是历史上最常掉链子的地方（capability_declaration.rs 记过同一个教训：
//! 能力位就是在这条边界上丢掉的），而且它**没有** record 级 checksum 兜底
//! （见 kotlin_guard.rs 护栏 4 的实测结论）—— 漏传字段不会有任何编译错误。
//!
//! # 场景
//!
//! FFI 引擎当**接收端**（配置 playout、`low_latency = true` 让它本地档是 10 ms），
//! 原生对端当**发送端**并推 20 ms 的流。协商生效时：
//!
//! - FFI 侧 `peers()[0].negotiated_frame_ms == Some(20)`（跟随了发送端，不是本地 10 ms）；
//! - 未开流时该字段是 `None`（`connect()` / `peers()` 在开流前就能读）。
//!
//! 两个对照点一起构成「这个字段真的在反映协商」，而不是「一直都是个常数」。
//!
//! 独立测试进程隔离 FFI 全局引擎（进程级静态状态）；不访问声卡。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{SocketAddr, UdpSocket};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use audiolink_audio::{CodecConfig, SyntheticCapture};
use audiolink_engine::{Engine, EngineConfig};
use audiolink_ffi::{EngineStartConfig, PcmFeed, engine_start, engine_stop, peers};
use audiolink_types::NodeId;

/// 收到 PCM 就计数；本用例不检查音频内容（那是 engine 侧的事），只借它满足「接收端」形态。
#[derive(Default)]
struct CountingFeed {
    calls: AtomicUsize,
}

impl PcmFeed for CountingFeed {
    fn feed_pcm(&self, samples: Vec<f32>, _frames: i32) -> i32 {
        self.calls.fetch_add(1, Ordering::Relaxed);
        (samples.len() / 2) as i32
    }
}

/// 有界等待：条件在 5 s 内成立即可，否则 panic 并说明卡在哪一步。
async fn until(what: &str, mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("5 s 内没有等到：{what}"));
}

/// 挑一个空闲 UDP 端口（理由同 engine_bridge.rs 的 free_udp_port：默认端口随时可能被占）。
fn free_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// 起一个原生对端（本用例里它是**发送端**）。
async fn peer_engine(dir: &Path, name: &str, codec: CodecConfig) -> Arc<Engine> {
    let mut config = EngineConfig::new(name, dir.join(name));
    config.listen = "127.0.0.1:0".parse().unwrap();
    config = config.with_codec(codec);
    config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(codec.frame_ms, 440.0)?))
    }));
    Engine::start(config).await.unwrap()
}

/// 与 FFI 引擎建立连接（**连上即通**：没有 PIN 交互，没有等待用户输码的中间态）。
async fn pair(peer: &Arc<Engine>, addr: SocketAddr, ffi_id: NodeId) {
    peer.connect(addr).await.expect("连接 FFI 引擎");

    until("对端把 FFI 引擎记进会话表", || {
        peer.peers().iter().any(|status| status.id == ffi_id)
    })
    .await;
}

/// 读 FFI 侧那**唯一**一路对端的 `negotiated_frame_ms`。
///
/// 对端还没进会话表时是 `None`（**不是** panic）：本用例的 ① 阶段本来就要在那之前读一次 ——
/// 「字段在开流前为空」与「字段一直为空」必须分得开，否则这条断言证明不了任何事。
///
/// 注意 `localStatus().id_hex` 是**本机**指纹，而 `peers()` 列的是**对端** —— 两者不可混用
/// （混用会得到空结果，看起来就像「协商没生效」）。本用例只有一路对端，取首条即可。
fn ffi_negotiated() -> Option<u8> {
    peers()
        .unwrap()
        .into_iter()
        .next()
        .and_then(|view| view.negotiated_frame_ms)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn negotiated_frame_ms_reaches_the_ffi_peer_view() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    // FFI 引擎当**接收端**，本地档是 10 ms —— 这样「协商后变成 20」才有区分度。
    let config = EngineStartConfig {
        node_name: "phone".into(),
        data_dir: dir.path().join("phone").to_string_lossy().into(),
        listen_port: port,
        capabilities: 0,
        low_latency: true,
    };
    let local = engine_start(config, Some(Box::new(CountingFeed::default())), None)
        .await
        .unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let ffi_id = NodeId::from_hex(&local.id_hex).unwrap();

    // 发送端推 20 ms 的流（与 FFI 本地 10 ms 档**不同档**）。
    let send_codec = CodecConfig::m1_default();
    assert_eq!(send_codec.frame_ms, 20, "用例前提：发送端是 20 ms 档");
    let peer = peer_engine(dir.path(), "pc-sender", send_codec).await;
    pair(&peer, addr, ffi_id).await;

    // ① 开流之前：没有流，就没有协商结果。
    until("对端进入 streaming", || {
        peer.peers()
            .iter()
            .any(|status| status.id == ffi_id && status.state.is_streaming())
    })
    .await;
    assert_eq!(
        ffi_negotiated(),
        None,
        "本条流还没开，negotiated_frame_ms 必须是 None（否则它只是个常数，不反映协商）"
    );

    // ② 开流：发送端把 20 ms 写进 OPEN_STREAM.codec_prefs。
    peer.start_send(ffi_id).await.expect("开流");

    // ③ 协商结果到达 FFI 侧的 PeerView。
    let deadline = Instant::now() + Duration::from_secs(5);
    let negotiated = loop {
        if let Some(frame_ms) = ffi_negotiated() {
            break frame_ms;
        }
        assert!(
            Instant::now() < deadline,
            "5 s 内 FFI 侧的 negotiated_frame_ms 一直是 None —— 协商结果没有走出内核"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    println!("[frame-ffi] FFI 本地档 10 ms，发送端 20 ms → negotiated_frame_ms = {negotiated}");
    assert_eq!(
        negotiated, 20,
        "FFI 侧的协商帧长必须是**发送端**的 20 ms（本地档是 10 ms，跟随才会得到 20）"
    );

    peer.shutdown().await;
    engine_stop().await.unwrap();
}
