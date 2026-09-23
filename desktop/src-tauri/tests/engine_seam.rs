//! 接缝假设的**运行时**验证：两个真实 `Engine` 经 127.0.0.1 真 QUIC 握手。
//!
//! ## 这个测试在验什么（以及不验什么）
//! `EngineBridge` 依赖引擎的几条行为约定，它们一旦变化，外壳会以很难查的方式坏掉
//! （卡片不出现、状态映射错位、开流失败被当成成功）：
//! 1. `connect()` **成功即握手完成**（无认证：没有 PIN，也没有挑战应答），返回对端身份，
//!    且对端已经进入 `peers()`、`addr` 与请求一致 —— 外壳的去重与卡片渲染靠这条；
//! 2. 会话状态随即进入 `Streaming` —— 外壳的状态映射靠这条（它因此必须自己区分
//!    「会话已建立」与「正在推流」）；
//! 3. 响应方侧也把发起方登记进 `peers()` —— 接收端卡片靠这条；
//! 4. `start_send` 之后确实有音频在流（遥测出现非零码率）。
//!
//! **不验**：Tauri 侧的事件翻译与前端渲染（那需要 AppHandle + WebView，属"未验证"清单）。
//!
//! ## 为什么不可能发出声音
//! 两端都用**合成实现**：A 端 `SyntheticCapture`（合成样本，不打开声卡），
//! B 端 `NullPlayout`（只消费样本，不渲染）。真实 WASAPI 工厂一行都没跑到。

// 测试里断言失败就该炸：显式放行 panic 系列 lint（工作区默认 deny）。
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use audiolink_audio::{CaptureSource, NullPlayout, PlayoutSink, SyntheticCapture};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::ErrorCode;
use tokio::sync::broadcast;

/// 各自的落盘目录（身份材料），用进程 + 时间戳隔离，测完删掉。
fn temp_dir(tag: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "audiolink-seam-{tag}-{}-{nanos}",
        std::process::id()
    ))
}

/// 合成采集（无声卡）。
fn synthetic_capture(
    fail: Arc<AtomicBool>,
    opened: Arc<AtomicBool>,
) -> audiolink_engine::CaptureFactory {
    Arc::new(move || {
        if fail.swap(false, Ordering::SeqCst) {
            return Err(audiolink_audio::AudioError::device_unavailable(
                "selected endpoint was unplugged",
            ));
        }
        opened.store(true, Ordering::SeqCst);
        SyntheticCapture::new(20, 440.0).map(|capture| Box::new(capture) as Box<dyn CaptureSource>)
    })
}

/// 空播放（只消费，不渲染）。
fn null_playout() -> audiolink_engine::PlayoutFactory {
    Arc::new(|| Ok(Box::new(NullPlayout::new(40)) as Box<dyn PlayoutSink>))
}

/// 等到一条满足条件的事件；超时返回 `None`（**不** panic，让调用方给更好的失败信息）。
async fn next_matching(
    events: &mut broadcast::Receiver<EngineEvent>,
    timeout: Duration,
    mut accept: impl FnMut(&EngineEvent) -> bool,
) -> Option<EngineEvent> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(event)) if accept(&event) => return Some(event),
            Ok(Ok(_)) => continue,     // 不相关的事件：跳过
            Ok(Err(_)) => return None, // 通道关闭 / 积压
            Err(_) => return None,     // 超时
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connect_then_stream_flow_matches_bridge_assumptions() {
    // 总超时：任何一步卡住都要失败，而不是让 CI 挂在那里。
    let outcome = tokio::time::timeout(Duration::from_secs(60), async {
        let dir_a = temp_dir("a");
        let dir_b = temp_dir("b");

        // A = 发起端（推流方向），B = 接收端（播放方向）
        let mut config_a = EngineConfig::new("node-a", &dir_a);
        config_a.listen = "127.0.0.1:0".parse().expect("listen a");
        let fail_capture = Arc::new(AtomicBool::new(true));
        let capture_opened = Arc::new(AtomicBool::new(false));
        config_a.capture = Some(synthetic_capture(
            Arc::clone(&fail_capture),
            Arc::clone(&capture_opened),
        ));

        let mut config_b = EngineConfig::new("node-b", &dir_b);
        config_b.listen = "127.0.0.1:0".parse().expect("listen b");
        config_b.playout = Some(null_playout());

        let engine_a = Engine::start(config_a).await.expect("start a");
        let engine_b = Engine::start(config_b).await.expect("start b");
        let _accept_a = engine_a.spawn_accept_loop();
        let _accept_b = engine_b.spawn_accept_loop();

        // 必须**先订阅**：事件是广播，订阅前的不会回放（接收端的遥测读数靠它）。
        let mut events_b = engine_b.subscribe();

        let addr_b = engine_b.local_addr();

        // ---- 1) 连接即成功：没有 PIN，也没有挑战应答 ----
        let peer_id = engine_a.connect(addr_b).await.expect("连接应当直接成功");

        // ---- 2) 会话已经登记：外壳这时候就该把卡片画出来 ----
        let peers = engine_a.peers();
        assert_eq!(peers.len(), 1, "连接成功的对端必须在 peers() 里");
        let peer = peers.first().expect("peer").clone();
        assert_eq!(peer.id, peer_id, "返回的身份与会话表里那条必须是同一个");
        assert_eq!(peer.addr, addr_b, "addr 必须与请求一致（外壳靠它去重）");
        assert_eq!(
            peer.state,
            SessionState::Streaming,
            "引擎在握手完成后置 Streaming —— 外壳因此必须自己区分'会话已建立'与'正在推流'"
        );

        // ---- 3) 接收端侧也记下了发起方 ----
        // 注意两侧的 id 是**各自的视角**：A 记录的是 B 的指纹，B 记录的是 A 的指纹。
        let inbound = engine_b.peers();
        assert_eq!(inbound.len(), 1, "响应方也要把入站会话登记进 peers()");
        assert_eq!(
            inbound.first().map(|status| status.id),
            Some(engine_a.info().id)
        );

        // ---- 4) 开流：合成采集 → Opus → QUIC → 空播放，应当真的流动起来 ----
        // 驱动在采集线程里拒绝打开时，command 必须返回错误，不能把“已入队”当作成功。
        let error = engine_a
            .start_send(peer_id)
            .await
            .expect_err("采集设备打开失败必须回传");
        assert_eq!(error.code(), ErrorCode::CaptureLost);
        assert!(error.context().contains("unplugged"));
        assert!(!capture_opened.load(Ordering::SeqCst));

        // 失败后同一连接可以重试；成功返回时采集工厂已经运行。
        engine_a.start_send(peer_id).await.expect("start_send");
        assert!(capture_opened.load(Ordering::SeqCst));
        let duplicate = engine_a
            .start_send(peer_id)
            .await
            .expect_err("拒绝重复开流，保留已有采集线程");
        assert_eq!(duplicate.code(), ErrorCode::Busy);
        let stream_id = engine_a
            .telemetry(peer_id)
            .map(|s| s.stream_id)
            .unwrap_or(0);
        assert_eq!(stream_id, 1, "发送侧 M1 的 stream_id 固定为 1");

        let flowing = next_matching(
            &mut events_b,
            Duration::from_secs(20),
            |event| matches!(event, EngineEvent::Telemetry(stats) if stats.bitrate_bps > 0),
        )
        .await
        .expect("接收端应当在 20 s 内收到非零码率的遥测");
        match flowing {
            EngineEvent::Telemetry(stats) => {
                assert!(stats.bitrate_bps > 0);
                assert_eq!(stats.stream_id, 1);
            }
            other => panic!("期望 Telemetry，实际 {other:?}"),
        }

        engine_a.stop_send(peer_id).await.expect("stop_send");
        engine_a.shutdown().await;
        engine_b.shutdown().await;

        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    })
    .await;

    assert!(outcome.is_ok(), "整个流程超时（60 s）");
}
