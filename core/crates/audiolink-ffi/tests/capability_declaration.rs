//! §13：FFI 调用方声明的**平台能力位**必须真的进到协商结果里 —— 而不是「字段加上了」。
//!
//! # 为什么这条测试必须存在
//!
//! audiolink-engine 侧的 capability_negotiation.rs 只能证明「EngineConfig.capabilities 会进协商」，
//! 证明不了 **FFI 这一跳**（EngineStartConfig → EngineConfig）。历史上 Android 侧的能力位就是在这
//! 一跳上缺失的：EngineStartConfig 没有 capabilities 字段，注释写死「Android 侧不声明系统内录」，
//! 于是采集已经可用、对端却永远不知道（第 106 轮决策 (a) 的挂账）。
//!
//! # 两个对照点（缺一不可）
//!
//! 1. declared = MICROPHONE 时：从**对端**视角读到的 peer 位图必须含该位；
//! 2. declared = 0（缺省）时：同样读不到该位 —— 否则这条断言只是「一直都有」的同义反复。
//!
//! 两种情形下都断言必需位（OPUS）在：并集换算的意义就是**不可能**抹掉 REQUIRED。
//!
//! 独立测试进程隔离 FFI 全局引擎（进程级静态状态）；不访问声卡。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{SocketAddr, UdpSocket};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use audiolink_engine::{Engine, EngineConfig};
use audiolink_ffi::{EngineStartConfig, displayed_pin, engine_start, engine_stop};
use audiolink_types::{Capabilities, ErrorCode, NodeId};

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

/// 起一个原生对端引擎；它自己声明 MICROPHONE（模拟 PC 端也能采麦克风）。
async fn peer_engine(dir: &Path, name: &str) -> Arc<Engine> {
    let mut config = EngineConfig::new(name, dir.join(name));
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.capabilities = Capabilities::CURRENT | Capabilities::MICROPHONE;
    Engine::start(config).await.unwrap()
}

/// 与 FFI 引擎走完配对（首次要 PIN；已受信则直接建立）。
async fn pair(peer: &Arc<Engine>, addr: SocketAddr, ffi_id: NodeId) {
    match peer.connect(addr).await {
        Ok(_) => {}
        Err(error) if error.code() == ErrorCode::NotPaired => {
            until("FFI 引擎显示 PIN", || {
                displayed_pin().unwrap().is_some()
            })
            .await;
            let pin = displayed_pin().unwrap().unwrap();
            peer.submit_pin(ffi_id, &pin).await.expect("PIN 配对");
        }
        Err(error) => panic!("连接 FFI 引擎失败：{error:?}"),
    }
    until("对端把 FFI 引擎标为受信", || {
        peer.peers()
            .iter()
            .any(|status| status.id == ffi_id && status.trusted)
    })
    .await;
}

/// 从**对端**视角读 FFI 引擎声明的能力位与交集。
fn caps_seen_by_peer(peer: &Arc<Engine>, ffi_id: NodeId) -> (u32, u32) {
    let status = peer
        .peers()
        .into_iter()
        .find(|status| status.id == ffi_id)
        .expect("对端应当有 FFI 引擎的会话");
    let caps = status.capabilities.expect("握手完成后应当有协商结果");
    (caps.peer, caps.agreed)
}

/// 启动 FFI 引擎并返回 (addr, id, 对端视角读数)。
async fn run_case(dir: &Path, name: &str, declared: u32) -> (u32, u32) {
    let port = free_port();
    let config = EngineStartConfig {
        node_name: "phone".into(),
        data_dir: dir.join("phone").to_string_lossy().into(),
        listen_port: port,
        capabilities: declared,
    };
    let local = engine_start(config, None, None).await.unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let ffi_id = NodeId::from_hex(&local.id_hex).unwrap();

    let peer = peer_engine(dir, name).await;
    pair(&peer, addr, ffi_id).await;
    let (peer_seen, agreed_seen) = caps_seen_by_peer(&peer, ffi_id);

    peer.shutdown().await;
    engine_stop().await.unwrap();
    (peer_seen, agreed_seen)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn declared_capabilities_reach_the_negotiation() {
    let dir = tempfile::tempdir().unwrap();
    let declared = Capabilities::MICROPHONE;

    // ① 声明了平台能力位：对端必须看得到。
    let (peer_seen, agreed_seen) = run_case(dir.path(), "pc-declared", declared).await;
    println!(
        "[caps-ffi] declared=0x{declared:04X} → 对端看到 peer=0x{peer_seen:04X} agreed=0x{agreed_seen:04X}"
    );
    assert_ne!(
        peer_seen & declared,
        0,
        "FFI 调用方声明的能力位没有传到对端：peer=0x{peer_seen:04X}"
    );
    assert_ne!(
        agreed_seen & declared,
        0,
        "交集里也应当有这一位（对端也声明了它）：agreed=0x{agreed_seen:04X}"
    );
    // 并集换算的意义：必需位不可能被调用方抹掉。
    assert_ne!(peer_seen & Capabilities::OPUS, 0, "必需位 OPUS 必须还在");
    assert_ne!(
        agreed_seen & Capabilities::OPUS,
        0,
        "交集里必须还有 REQUIRED"
    );

    // ② 对照组：缺省 0 → 与加这个字段之前逐位一致，看不到任何额外能力位。
    let (quiet_seen, quiet_agreed) = run_case(dir.path(), "pc-quiet", 0).await;
    println!(
        "[caps-ffi] declared=0x0000 → 对端看到 peer=0x{quiet_seen:04X} agreed=0x{quiet_agreed:04X}"
    );
    assert_eq!(
        quiet_seen & declared,
        0,
        "缺省 0 不该凭空多出能力位：peer=0x{quiet_seen:04X}"
    );
    assert_eq!(
        quiet_agreed & declared,
        0,
        "交集同理：agreed=0x{quiet_agreed:04X}"
    );
    assert_eq!(
        quiet_seen,
        Capabilities::CURRENT & (Capabilities::CURRENT | Capabilities::MICROPHONE),
        "缺省时的位图应当恰好等于内核默认（CURRENT）：peer=0x{quiet_seen:04X}"
    );
    assert_ne!(quiet_seen & Capabilities::OPUS, 0, "必需位照旧在");
}
