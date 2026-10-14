//! 同身份**原位顶替**（对端进程被杀后立刻重连）时的会话世代号。
//!
//! # 场景
//!
//! 手机端被用户杀掉再立刻打开：旧进程没有发出 QUIC 关闭帧，PC 侧旧会话要等
//! idle_timeout（30 s）才判死；新进程用**同一身份**（identity_dir 持久化 → 同一 NodeId）
//! 立刻重连，`create_session` 把新会话**原位插进表里顶替旧表项** —— peer 不消失、
//! 没有断开事件、`reconnects` 也归新会话所有。外壳手里的「正在推流」阴影标记若只按
//! NodeId 记账，就会挂在新会话上永远不失效（旧推流其实已随旧任务死亡），自动推流再也被
//! 自己的记账挡住 —— 真机症状：PC 端显示着延迟（新会话在应答时钟探测），但推流永不恢复。
//!
//! 本文件钉住内核侧的判据：**世代号在顶替发生的那一刻就必须变**，且顶替后的新会话
//! 是一条完整可用的会话（握手走到 Streaming、能对它重新 `start_send`）。
//! 外壳（desktop `Cache::sending`）据此让阴影标记失效。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use audiolink_audio::{NullPlayout, SyntheticCapture};
use audiolink_engine::{Engine, EngineConfig, SessionState};

/// 有界等待（与 reconnect_receipt.rs 同一纪律：不在这里 panic，结论交给调用方的断言）。
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

/// 起一台引擎：监听 + 采集（SyntheticCapture）+ 播放（NullPlayout）全配齐，
/// 两个方向的能力都可用（本测试里「PC」要能对顶替后的新会话重新推流）。
async fn start_engine(name: &str, dir: &Path) -> Arc<Engine> {
    let frame_ms = 20_u32;
    let mut config = EngineConfig::new(name, dir);
    config.listen = r"127.0.0.1:0".parse().expect("回环地址");
    config.codec.frame_ms = frame_ms;
    config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));
    config.playout = Some(Arc::new(move || Ok(Box::new(NullPlayout::new(60)))));
    Engine::start(config).await.expect("引擎启动")
}

/// 顶替发生的那一刻世代号就必须变（不能等握手完成），顶替后的会话必须完整可用。
///
/// 「同一身份的第二实例直连」就是进程被杀重开的等价物：旧引擎（`phone_first`）在整个
/// 测试里**保持存活** —— 正如真机上旧连接在 idle_timeout 之前一直挂着，顶替必须在这
/// 种条件下发生，而不是等旧连接死掉之后。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_identity_reconnect_replaces_session_and_bumps_generation() {
    let dir = tempfile::TempDir::new().expect("临时目录");

    let pc = start_engine("pc", &dir.path().join("pc")).await;
    let accept = pc.spawn_accept_loop();

    // 手机端的两次「进程」：同一身份目录 → 同一 NodeId。
    let phone_dir = dir.path().join("phone");
    let phone_first = start_engine("phone", &phone_dir).await;
    let phone_id = phone_first.info().id;

    phone_first
        .connect(pc.local_addr())
        .await
        .expect("首次连接");
    assert!(
        wait_for(Duration::from_secs(10), || {
            pc.peers()
                .iter()
                .any(|peer| peer.id == phone_id && peer.state == SessionState::Streaming)
        })
        .await,
        "首次连接后 PC 侧应走到 Streaming"
    );
    pc.start_send(phone_id).await.expect("首次推流");
    let first_generation = pc
        .session_generation(phone_id)
        .expect("会话在表里就该有世代号");

    // 手机进程被杀、立刻重开：旧引擎不 shutdown（没有关闭帧，正如进程死亡），
    // 同一身份的新实例直接重连。
    let phone_second = start_engine("phone", &phone_dir).await;
    assert_eq!(
        phone_second.info().id,
        phone_id,
        "同一身份目录必须导出同一 NodeId（否则整条测试没有模拟到「同身份重连」）"
    );
    phone_second
        .connect(pc.local_addr())
        .await
        .expect("同身份重连");

    // ① 世代号必须在顶替那一刻就变 —— 这是外壳让「推流中」阴影标记失效的唯一判据。
    assert!(
        wait_for(Duration::from_secs(10), || {
            pc.session_generation(phone_id)
                .is_some_and(|generation| generation != first_generation)
        })
        .await,
        "同身份新连接顶替旧会话后，世代号必须变（旧推流已随旧会话死亡）"
    );

    // ② 表里仍然只有一条该对端的会话（原位顶替，不是并列两条）。
    assert_eq!(
        pc.peers().iter().filter(|peer| peer.id == phone_id).count(),
        1,
        "同身份重连必须是原位顶替，不能并列两条会话"
    );

    // ③ 新会话是完整可用的：握手走到 Streaming，且能对「它」重新推流
    //    （桌面端自动推流恢复依赖的正是这一步）。
    assert!(
        wait_for(Duration::from_secs(10), || {
            pc.peers()
                .iter()
                .any(|peer| peer.id == phone_id && peer.state == SessionState::Streaming)
        })
        .await,
        "顶替后的新会话必须能走到 Streaming"
    );
    pc.start_send(phone_id)
        .await
        .expect("顶替后的新会话必须能重新推流（旧推流已死，这一发不是 Busy）");

    // 收尾：旧「进程」此刻才退出 —— 它的会话任务早已收到 Shutdown，
    // 这里只是验证引擎关停不会因为顶替过的表项而出错。
    phone_first.shutdown().await;
    phone_second.shutdown().await;
    pc.shutdown().await;
    accept.abort();
}
