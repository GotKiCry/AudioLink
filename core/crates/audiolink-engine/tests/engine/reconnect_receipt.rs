//! 重连回执（审计 §2.6-④）：**重连成功这件事必须能被 `Engine::peers()` 读到**。
//!
//! 缺口（`docs/51-fr27-reconnect-audit.md` §2.6-④）：`reconnect_once` 成功后把
//! `SessionEvent::ReconnectOk` 与 `SessionState::Streaming` 打在**刚从 `inner.peers` 表里
//! 摘下来的旧会话对象**上：`ReconnectOk` 的迁移会让状态机 `reconnects += 1`，但计数记在了
//! 谁也看不到的对象上；UI 读的是 `connect_inner` 新建、插在表里的**新会话**，它的
//! `reconnects()` 仍是 0。「重连成功」因此在界面上没有回执。
//!
//! 本文件用一条**真链路**复现：真 QUIC + 双向闸门中继（关闸 = 拔网线）→ 发起方静默看门狗
//! 触发重连 → 插回 → 断言 `peers()` 里那个 peer 的 `reconnects() >= 1` 且状态 `Streaming`。
//!
//! # 为什么一定要从 `peers()` 读
//!
//! `reconnects` 只存在于会话状态机里，而重连**会把状态机对象换掉**（旧会话摘表 + Shutdown、
//! 新会话插表）。任何「直接检查内部对象」的写法都抓不住这个缺口，只有走 UI 真正使用的
//! `peers()` 快照路径才有意义。
//!
//! # 为什么必须拔网而不是「断开再连」
//!
//! 触发条件是**静默看门狗**（发起方 ≥3 s 没听到对端任何包），所以闸门必须两个方向都关：
//! 只丢下行的话，发送侧的保活照样到达接收端、回执照样回来，看门狗永远不会触发。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use audiolink_audio::{NullPlayout, PlayoutSink, SyntheticCapture};
use audiolink_engine::{Engine, EngineConfig, SessionState};
use audiolink_types::NodeId;
use tokio::net::UdpSocket;

/// 双向闸门中继：闸门关着时**两个方向都丢** —— 这就是「拔网线」。
struct Gate {
    addr: SocketAddr,
    open: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl Gate {
    async fn start(upstream: SocketAddr) -> Self {
        let socket = UdpSocket::bind(r"127.0.0.1:0")
            .await
            .expect(r"中继绑定回环端口");
        let addr = socket.local_addr().expect(r"中继地址");
        let socket = Arc::new(socket);
        let open = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&open);
        let task = tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let mut client: Option<SocketAddr> = None;
            while let Ok((len, from)) = socket.recv_from(&mut buf).await {
                let client_to_server = from != upstream;
                let to = if client_to_server {
                    client = Some(from);
                    upstream
                } else {
                    match client {
                        Some(client) => client,
                        // 还不知道客户端是谁：丢掉（握手前的噪声）
                        None => continue,
                    }
                };
                if !flag.load(Ordering::SeqCst) {
                    continue;
                }
                let _ = socket.send_to(&buf[..len], to).await;
            }
        });
        Self { addr, open, task }
    }

    /// 拔网：此后两个方向的包一律丢弃。
    fn cut(&self) {
        self.open.store(false, Ordering::SeqCst);
    }

    /// 插回网线。
    fn restore(&self) {
        self.open.store(true, Ordering::SeqCst);
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// 有界等待：条件在预算内成立返回 `true`，否则 `false`。
///
/// 刻意**不在这里 panic**：条件不成立时把结论交给调用方的断言，失败信息里才带得上实测读数
/// （「reconnects 停在 0」比「10 s 超时」有用得多）。也不用长时间裸 sleep 赌时序。
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

/// 起一对引擎：真 QUIC 走中继，走完握手与开流。
async fn wire_up(
    dir: &Path,
) -> (
    Arc<Engine>,
    Arc<Engine>,
    Gate,
    tokio::task::JoinHandle<()>,
    NodeId,
) {
    let frame_ms = 20_u32;
    let mut send_config = EngineConfig::new(r"sender", dir.join(r"sender"));
    send_config.listen = r"127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));

    let mut recv_config = EngineConfig::new(r"receiver", dir.join(r"receiver"));
    recv_config.listen = r"127.0.0.1:0".parse().unwrap();
    recv_config.codec.frame_ms = frame_ms;
    recv_config.playout = Some(Arc::new(|| {
        Ok(Box::new(NullPlayout::new(60)) as Box<dyn PlayoutSink>)
    }));

    let receiver = Engine::start(recv_config).await.expect(r"接收引擎");
    let accept = receiver.spawn_accept_loop();

    let receiver_id = receiver.info().id;

    let gate = Gate::start(receiver.local_addr()).await;
    let sender = Engine::start(send_config).await.expect(r"发送引擎");

    sender
        .connect(gate.addr)
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
    sender.start_send(receiver_id).await.expect(r"开流");

    // 拔网之前必须先让发起方**听到过**对端：FR-27 的静默判据从「听到过」起算
    // （一次都没听到时 `silent_for_ms()` 恒为 0，免得刚建好的会话被判静默），
    // 所以少了这一等，`cut()` 之后的 3 s 阈值永远算不出来 —— 重连根本不会发生，
    // 而下面「状态仍是 Streaming、reconnects 0」会被误读成回执缺陷。
    let heard = wait_for(Duration::from_secs(5), || {
        sender
            .clock_probe_stats(receiver_id)
            .is_some_and(|stats| stats.received > 0)
    })
    .await;
    assert!(
        heard,
        r"发起方 5 s 内没收到对端的任何时钟探测：静默看门狗的前提不成立，这条用例失去意义"
    );

    (sender, receiver, gate, accept, receiver_id)
}

/// **审计 §2.6-④ 的回执**：拔网触发重连、插回重连成功后，`peers()` 必须能读出来。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconnect_ok_is_visible_through_peers() {
    let dir = tempfile::TempDir::new().expect(r"临时目录");
    let (sender, receiver, gate, accept, receiver_id) = wire_up(dir.path()).await;

    // ① 拔网。发起方（发送侧）在静默 3 s 后投递重连请求；闸门关着时每轮拨号都会超时，
    //    它在退避循环里反复重试 —— 这 5 s 是「链路真的断了」的必要条件，不是赌时序。
    gate.cut();
    tokio::time::sleep(Duration::from_secs(5)).await;

    // ② 插回网线：下一次拨号应当成功，重连完成。
    gate.restore();

    let recovered = wait_for(Duration::from_secs(15), || {
        sender
            .peers()
            .iter()
            .any(|peer| peer.id == receiver_id && peer.state == SessionState::Streaming)
    })
    .await;
    assert!(
        recovered,
        r"插回网线后 15 s 内没有恢复到 Streaming：重连本身没成功，这条测试失去意义"
    );

    // ③ 回执读数 —— 全部取自 UI 真正使用的那条路径。
    let status = sender
        .peers()
        .into_iter()
        .find(|peer| peer.id == receiver_id)
        .expect(r"发送侧 peers() 里应当有接收端");
    println!(
        "[reconnect-receipt] 重连后 peers() 读数：状态 {:?}，reconnects {}",
        status.state, status.reconnects
    );

    assert_eq!(
        status.state,
        SessionState::Streaming,
        r"重连成功后 peers() 里该 peer 的状态应当是 Streaming"
    );
    assert!(
        status.reconnects >= 1,
        "重连成功必须在 peers() 里有回执（实测 reconnects {}）—— 审计 §2.6-④：         ReconnectOk 被 apply 在已摘表的旧会话对象上，计数记在了 UI 读不到的地方",
        status.reconnects
    );

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();
}
