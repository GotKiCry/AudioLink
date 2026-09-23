//! 真 QUIC 回环集成测试（`docs/11-m1-contract.md` §3 验收 1–5）。
//!
//! 这份测试**只用 crate 的公开面**：它同时充当「冻结 API 在 crate 外真的可用」的证明。
//! 验收 6（时钟估计）是纯逻辑，放在 `src/clock.rs` 的单测里。
//!
//! 测试代码不受实时路径的 unwrap / expect / panic 禁令约束（那三条针对运行时音频路径）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use audiolink_net::{AudioLinkEndpoint, Connection, EndpointConfig, NetError};
use audiolink_types::{
    CONTROL_MAX_PAYLOAD, DATAGRAM_HEADER_LEN, DATAGRAM_MAX_LEN, ErrorCode, NodeId, OpCode,
};
use rustls::pki_types::CertificateDer;
use sha2::{Digest, Sha256};

#[tokio::test]
async fn shutdown_waits_for_live_handles_and_can_resume_after_cancellation() {
    let Pair {
        _server_endpoint: server_endpoint,
        _client_endpoint: client_endpoint,
        server,
        client,
        ..
    } = pair().await;
    let addr = server_endpoint.local_addr().unwrap();
    // 故意保留一个 Connection，使 socket 仍有使用者，验证不能提前返回成功。
    let mut stop = Box::pin(server_endpoint.shutdown());
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut stop)
            .await
            .is_err()
    );
    drop(stop);
    drop(server);
    tokio::time::timeout(Duration::from_secs(5), server_endpoint.shutdown())
        .await
        .unwrap();
    let rebound = std::net::UdpSocket::bind(addr).expect("旧 Endpoint 句柄仍在也应能复用端口");
    server_endpoint.shutdown().await;
    assert!(server_endpoint.accept().await.is_err());
    drop(rebound);
    drop(client);
    client_endpoint.shutdown().await;
}

// ---------------------------------------------------------------------------
// 脚手架（与 crate 内 `testing` 模块同构，此处刻意只用公开面）
// ---------------------------------------------------------------------------

/// 测试用节点身份（生产路径的证书来自 `audiolink-identity`）。
struct TestIdentity {
    cert_der: Vec<u8>,
    key_der_pkcs8: Vec<u8>,
}

fn identity() -> TestIdentity {
    let certified =
        rcgen::generate_simple_self_signed(vec!["audiolink".to_owned()]).expect("生成自签证书");
    let key_der_pkcs8 = certified.signing_key.serialize_der();
    let cert: CertificateDer<'static> = certified.cert.into();
    let leaf: &[u8] = cert.as_ref();
    TestIdentity {
        cert_der: leaf.to_vec(),
        key_der_pkcs8,
    }
}

/// 独立计算指纹（**不调被测代码**，避免「用被测实现验证被测实现」）。
fn sha256_node_id(bytes: &[u8]) -> NodeId {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    NodeId::from_bytes(hasher.finalize().into())
}

/// 两端点 + 双向连接（端点必须一起持有：`quinn::Endpoint` 一 drop，连接也就没了）。
struct Pair {
    _server_endpoint: AudioLinkEndpoint,
    _client_endpoint: AudioLinkEndpoint,
    server: Connection,
    client: Connection,
    server_identity: TestIdentity,
    client_identity: TestIdentity,
}

/// 在 127.0.0.1 上起两个端点并完成一次真实 QUIC 握手（双向认证）。
async fn pair() -> Pair {
    let server_identity = identity();
    let client_identity = identity();

    let server_endpoint = AudioLinkEndpoint::bind(EndpointConfig {
        bind: "127.0.0.1:0".parse().expect("回环地址"),
        cert_der: server_identity.cert_der.clone(),
        key_der_pkcs8: server_identity.key_der_pkcs8.clone(),
        ..EndpointConfig::default()
    })
    .await
    .expect("绑定服务端端点");

    let client_endpoint = AudioLinkEndpoint::bind(EndpointConfig {
        bind: "127.0.0.1:0".parse().expect("回环地址"),
        cert_der: client_identity.cert_der.clone(),
        key_der_pkcs8: client_identity.key_der_pkcs8.clone(),
        ..EndpointConfig::default()
    })
    .await
    .expect("绑定客户端端点");

    let addr = server_endpoint.local_addr().expect("读取服务端地址");
    let (server, client) = tokio::join!(
        async { server_endpoint.accept().await.expect("接受入站连接") },
        async {
            client_endpoint
                .connect(addr, "audiolink")
                .await
                .expect("主动连接")
        },
    );

    Pair {
        _server_endpoint: server_endpoint,
        _client_endpoint: client_endpoint,
        server,
        client,
        server_identity,
        client_identity,
    }
}

/// 带超时的数据报读：回环上不该等超过 5 s，超时说明实现坏了 —— 让测试快速失败而不是挂住。
async fn read_datagram(conn: &Connection, buf: &mut [u8]) -> Result<usize, NetError> {
    tokio::time::timeout(Duration::from_secs(5), conn.read_datagram_into(buf))
        .await
        .expect("等待数据报超时（5 s）")
}

// ---------------------------------------------------------------------------
// 验收 ①：握手成对 + 双方指纹互认
// ---------------------------------------------------------------------------

#[tokio::test]
async fn 握手后双方指纹互认且等于证书der的sha256() {
    let pair = pair().await;

    // 双方各自看到的对端指纹 = 对端证书 DER 的 SHA-256（用 sha2 现算，不调被测代码）
    let client_sees = pair.client.peer_id().expect("客户端读取对端指纹");
    let server_sees = pair.server.peer_id().expect("服务端读取对端指纹");
    assert_eq!(
        client_sees,
        sha256_node_id(&pair.server_identity.cert_der),
        "客户端看到的指纹必须等于服务端证书的 SHA-256"
    );
    assert_eq!(
        server_sees,
        sha256_node_id(&pair.client_identity.cert_der),
        "服务端看到的指纹必须等于客户端证书的 SHA-256（双向认证，§2）"
    );
    assert_ne!(client_sees, server_sees, "两端是两份不同证书");

    // 契约修订 #2：`peer_cert_der()` 与 `peer_id()` 必须永远指向同一份字节
    assert_eq!(
        pair.client.peer_cert_der().expect("客户端读对端证书"),
        pair.server_identity.cert_der
    );
    assert_eq!(
        pair.server.peer_cert_der().expect("服务端读对端证书"),
        pair.client_identity.cert_der
    );
    assert_eq!(
        sha256_node_id(&pair.client.peer_cert_der().expect("对端证书")),
        client_sees,
        "peer_id() 必须恒等于 SHA-256(peer_cert_der())"
    );
    assert_eq!(
        sha256_node_id(&pair.server.peer_cert_der().expect("对端证书")),
        server_sees,
        "peer_id() 必须恒等于 SHA-256(peer_cert_der())"
    );
}

// ---------------------------------------------------------------------------
// 验收 ②：数据报边界（刚好成功 / 多一字节被拒）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn 数据报边界刚好成功且多一字节被拒() {
    let pair = pair().await;

    // 口径：「刚好」= 一整条 `min(DATAGRAM_MAX_LEN, max_datagram_size())` 字节的数据报
    //（24 B 帧头 + `max_audio_payload()` 字节音频载荷）。契约验收里的
    //「发 1162 B 成功、发 max_audio_payload()+1 被拒」说的正是这两个**整包**长度。
    let max_datagram = pair.client.max_datagram_size().expect("QUIC 数据报上限");
    println!("实测 max_datagram_size = {max_datagram} B");
    assert!(
        max_datagram > DATAGRAM_HEADER_LEN,
        "数据报上限必须大于 24 B 帧头，实得 {max_datagram}"
    );

    // §3 实测修正：载荷上限 = min(DATAGRAM_MAX_LEN, max_datagram_size) - 24
    let budget = max_datagram.min(DATAGRAM_MAX_LEN);
    assert_eq!(
        pair.client.max_audio_payload(),
        budget - DATAGRAM_HEADER_LEN,
        "载荷上限必须是 min(1200, max_datagram_size) - 24"
    );
    assert_eq!(
        pair.server.max_audio_payload(),
        pair.client.max_audio_payload(),
        "两端协商出的数据报上限应当一致"
    );

    // ---- 越界 1 字节：必须返回 Err（不是 panic，也不是静默截断）----
    //
    // 为什么要循环：`max_datagram_size()` 不是常量 —— DPLPMTUD 会随会话继续上探 MTU
    //（本机实测从握手时的 **1162 B 涨到 1288 B**，被 §3 的 1200 B 协议预算截住）。
    // 所以每一步都重读「发送前那一刻」的上限，绝不缓存几毫秒前的旧值。
    // 上限单调不减且封顶 1200，因此循环必然收敛；长度检查若真失效，这里会一直在 Ok 上打转，
    // 最终由 `rounds` 断言炸掉，仍然是有效测试。
    let mut rounds = 0_u32;
    loop {
        let current = pair
            .client
            .max_datagram_size()
            .expect("数据报上限")
            .min(DATAGRAM_MAX_LEN);
        assert_eq!(
            pair.client.max_audio_payload(),
            current - DATAGRAM_HEADER_LEN
        );
        let over = vec![0u8; current + 1];
        match pair.client.send_datagram(&over).await {
            Err(error) => {
                assert!(
                    matches!(error, NetError::DatagramTooLarge { .. }),
                    "实得 {error}"
                );
                assert_eq!(error.code(), ErrorCode::BadRequest, "§11：长度越界归 1008");
                assert!(
                    error.is_statistical(),
                    "丢一帧不影响会话语义 → 属统计类（计数后继续）"
                );
                break;
            }
            Ok(()) => {
                rounds += 1;
                assert!(
                    rounds < 8,
                    "上限 {current} B 之上 1 字节被接受了 —— 长度检查失效"
                );
                println!("上限已上探，重读：新一轮预算 {} B", current + 1);
            }
        }
    }

    // ---- 协议预算的硬边界（§3「单包总长 ≤ 1200 B」）----
    //
    // `min(DATAGRAM_MAX_LEN, max_datagram_size()) <= 1200` 恒成立，所以 1201 B 是
    // **永远**越界的确定值：这条不依赖 MTU 探测的时序，是真正钉死协议预算的断言。
    let over_hard_cap = vec![0u8; DATAGRAM_MAX_LEN + 1];
    let error = pair
        .client
        .send_datagram(&over_hard_cap)
        .await
        .expect_err("超过 1200 B 协议预算必须拒绝");
    assert!(
        matches!(error, NetError::DatagramTooLarge { .. }),
        "实得 {error}"
    );

    // ---- 刚好到上限：必须成功，且字节原样送达 ----
    // 再读一次上限（MTU 只会变大，所以取新值一定不会小于之前那条）
    let budget = pair
        .client
        .max_datagram_size()
        .expect("数据报上限")
        .min(DATAGRAM_MAX_LEN);
    let datagram: Vec<u8> = (0..budget).map(|i| (i % 251) as u8).collect();
    pair.client
        .send_datagram(&datagram)
        .await
        .expect("刚好到上限必须成功");

    let mut buf = vec![0u8; budget];
    let read = read_datagram(&pair.server, &mut buf)
        .await
        .expect("读回数据报");
    assert_eq!(read, budget);
    assert_eq!(
        &buf[..read],
        &datagram[..],
        "§3 载荷是不透明字节串：必须逐字节原样送达"
    );
}

// ---------------------------------------------------------------------------
// 验收 ③：read_datagram_into 缓冲不足 → Err（不得静默截断）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn 读数据报缓冲不足返回err() {
    let pair = pair().await;

    let payload = vec![0x5A_u8; 64];
    pair.client
        .send_datagram(&payload)
        .await
        .expect("发送 64 B 数据报");

    let mut small = [0u8; 32];
    let error = read_datagram(&pair.server, &mut small)
        .await
        .expect_err("缓冲不足必须报错，不得截断静默成功");
    assert!(
        matches!(error, NetError::DatagramBufferTooSmall { .. }),
        "实得 {error}"
    );
    assert_eq!(error.code(), ErrorCode::BadRequest);
    assert!(error.is_statistical(), "丢一个数据报不改变会话语义");
    assert_eq!(small, [0u8; 32], "被拒绝时不得往调用方缓冲里写半个数据报");

    // 连接没有因此受损：下一个数据报用够大的缓冲仍能读回
    pair.client.send_datagram(&payload).await.expect("再次发送");
    let mut enough = [0u8; 64];
    let read = read_datagram(&pair.server, &mut enough)
        .await
        .expect("够大的缓冲必须成功");
    assert_eq!(read, 64);
    assert_eq!(&enough[..read], &payload[..]);
}

// ---------------------------------------------------------------------------
// 验收 ④：24 个 OpCode 全扫往返一致
// ---------------------------------------------------------------------------

#[tokio::test]
async fn 控制帧20个opcode全扫往返一致() {
    let pair = pair().await;

    // 每条命令一个可区分的 (request_id, 载荷)：长度覆盖 0 / 1 / 中间值，内容含 0x00 与 0xFF 边界。
    // 数量跟着 `OpCode::ALL` 走 —— 新增协议帧会被这条测试自动扫到。
    let expected: Vec<(OpCode, u32, Vec<u8>)> = OpCode::ALL
        .iter()
        .enumerate()
        .map(|(index, op)| {
            let request_id = u32::try_from(index).expect("序号") + 1;
            let len = index * 7 % 41;
            let payload: Vec<u8> = (0..len).map(|i| ((i * 31 + index) % 256) as u8).collect();
            (*op, request_id, payload)
        })
        .collect();
    // M4 新增 RECEIVER_EPOCH（0x44）、再删掉配对用的 0x03–0x07 五条之后是 20 条；
    // 这条断言是「协议表变化」的显式确认点。
    assert_eq!(expected.len(), 20, "§4.1 命令表共 20 条");

    let mut client = pair
        .client
        .open_control()
        .await
        .expect("客户端打开控制流 #0");

    // 先把全部帧写出去：QUIC 的 `open_bi()` 是惰性的，服务端的 `accept_bi()` 要等
    // **第一个字节**到达才返回 —— 所以「先写后接管」是这条测试能顺序执行的前提，
    // 也正是真实会话的顺序（发起方先发 HELLO）。
    for (op, request_id, payload) in &expected {
        client
            .send(*op, *request_id, payload)
            .await
            .expect("发送控制帧");
    }

    let mut server = pair
        .server
        .open_control()
        .await
        .expect("服务端接管控制流 #0");
    for (index, (op, request_id, payload)) in expected.iter().enumerate() {
        let message = server.recv().await.expect("服务端读帧");
        assert_eq!(message.op, Some(*op), "第 {index} 帧命令码");
        assert_eq!(message.raw_type, op.as_u8(), "第 {index} 帧原始 type 字节");
        assert_eq!(
            message.flags, 0,
            "v1 的 flags 全保留：net 发送端必须置 0（§4）"
        );
        assert_eq!(message.request_id, *request_id, "第 {index} 帧 request_id");
        assert_eq!(message.payload, *payload, "第 {index} 帧载荷字节");

        // 反方向：回填同一 request_id 与同一载荷（§4：响应必须回填同一 request_id）
        server
            .send(*op, *request_id, payload)
            .await
            .expect("回填控制帧");
    }

    for (index, (op, request_id, payload)) in expected.iter().enumerate() {
        let message = client.recv().await.expect("客户端读回填");
        assert_eq!(message.op, Some(*op), "回填第 {index} 帧命令码");
        assert_eq!(
            message.request_id, *request_id,
            "回填第 {index} 帧 request_id"
        );
        assert_eq!(message.payload, *payload, "回填第 {index} 帧载荷字节");
    }
}

// ---------------------------------------------------------------------------
// 验收 ⑤：控制帧载荷超 CONTROL_MAX_PAYLOAD → 发送侧 Err
// ---------------------------------------------------------------------------

#[tokio::test]
async fn 控制帧载荷超过上限发送侧返回err() {
    let pair = pair().await;
    let mut client = pair.client.open_control().await.expect("打开控制流 #0");

    let oversized = vec![0u8; CONTROL_MAX_PAYLOAD + 1];
    let error = client
        .send(OpCode::Hello, 1, &oversized)
        .await
        .expect_err("超过 64 KiB 必须被发送侧拒绝");
    assert!(
        matches!(error, NetError::ControlPayloadTooLarge { .. }),
        "实得 {error}"
    );
    assert_eq!(error.code(), ErrorCode::BadRequest);
    assert!(
        !error.is_statistical(),
        "控制帧是状态机的输入：丢一帧等于卡住，必须让上层看见"
    );

    // 恰好等于上限必须放行（§4 的约束是「> 65536」）。
    // 带超时：万一 64 KiB 撞上 QUIC 流控，测试要失败而不是挂住。
    let exact = vec![0x3C_u8; CONTROL_MAX_PAYLOAD];
    tokio::time::timeout(
        Duration::from_secs(10),
        client.send(OpCode::Hello, 2, &exact),
    )
    .await
    .expect("恰好 64 KiB 的发送不该被流控挂住（初始接收窗口远大于 64 KiB）")
    .expect("恰好 CONTROL_MAX_PAYLOAD 必须放行");
}
