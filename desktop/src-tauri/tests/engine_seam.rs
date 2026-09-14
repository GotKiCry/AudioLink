//! 接缝假设的**运行时**验证：两个真实 `Engine` 经 127.0.0.1 真 QUIC 握手 + 真 PIN 配对。
//!
//! ## 这个测试在验什么（以及不验什么）
//! `EngineBridge` 依赖引擎的几条行为约定，它们一旦变化，外壳会以很难查的方式坏掉
//! （卡片不出现、PIN 输入框不弹出、配对结果永远显示"已提交"）：
//! 1. `connect()` 在需要配对时返回 `1002 NOT_PAIRED`，**但对端已经进入 `peers()`** 且 `addr` 与请求一致
//!    —— 外壳的去重与卡片渲染靠这条；
//! 2. 同一时刻发起端收到 `PinNeeded`、接收端收到 `DisplayPin{pin}` —— 前端两种配对界面靠这条；
//! 3. `submit_pin(peer, pin)` 之后，结果以 `PairCompleted{ok:true}` 事件回来（不是立即返回）
//!    —— 外壳的"等结果"逻辑靠这条；
//! 4. 配对成功后 `trusted == true`、会话状态进入 `Streaming` —— 外壳的状态映射靠这条；
//! 5. `start_send` 之后确实有音频在流（遥测出现非零码率）。
//!
//! **不验**：Tauri 侧的事件翻译与前端渲染（那需要 AppHandle + WebView，属"未验证"清单）。
//!
//! ## 为什么不可能发出声音
//! 两端都用**合成实现**：A 端 `SyntheticCapture`（合成样本，不打开声卡），
//! B 端 `NullPlayout`（只消费样本，不渲染）。真实 WASAPI 工厂一行都没跑到。

// 测试里断言失败就该炸：显式放行 panic 系列 lint（工作区默认 deny）。
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use audiolink_audio::{CaptureSource, NullPlayout, PlayoutSink, SyntheticCapture};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::{ErrorCode, NodeId};
use tokio::sync::broadcast;

/// 各自的落盘目录（身份材料 / 信任库），用进程 + 时间戳隔离，测完删掉。
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
fn synthetic_capture() -> audiolink_engine::CaptureFactory {
    Arc::new(|| {
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
async fn pairing_and_streaming_flow_matches_bridge_assumptions() {
    // 总超时：任何一步卡住都要失败，而不是让 CI 挂在那里。
    let outcome = tokio::time::timeout(Duration::from_secs(60), async {
        let dir_a = temp_dir("a");
        let dir_b = temp_dir("b");

        // A = 发起端（推流方向），B = 接收端（播放方向）
        let mut config_a = EngineConfig::new("node-a", &dir_a);
        config_a.listen = "127.0.0.1:0".parse().expect("listen a");
        config_a.capture = Some(synthetic_capture());

        let mut config_b = EngineConfig::new("node-b", &dir_b);
        config_b.listen = "127.0.0.1:0".parse().expect("listen b");
        config_b.playout = Some(null_playout());

        let engine_a = Engine::start(config_a).await.expect("start a");
        let engine_b = Engine::start(config_b).await.expect("start b");
        let _accept_a = engine_a.spawn_accept_loop();
        let _accept_b = engine_b.spawn_accept_loop();

        // 必须**先订阅**：事件是广播，订阅前的不会回放。
        let mut events_a = engine_a.subscribe();
        let mut events_b = engine_b.subscribe();

        let addr_b = engine_b.local_addr();

        // ---- 1) 首次连接必然要求配对，且错误码是 1002 ----
        let error = engine_a
            .connect(addr_b)
            .await
            .expect_err("首次连接应当返回 NotPaired");
        assert_eq!(
            error.code(),
            ErrorCode::NotPaired,
            "需要的是 1002，实际 {error:?}"
        );

        // ---- 2) 会话已经登记：外壳这时候就该把卡片画出来 ----
        let peers = engine_a.peers();
        assert_eq!(peers.len(), 1, "配对中的对端也必须在 peers() 里");
        let peer = peers.first().expect("peer").clone();
        assert_eq!(peer.addr, addr_b, "addr 必须与请求一致（外壳靠它去重）");
        assert!(!peer.trusted, "还没配对，不能是已受信");
        assert_eq!(peer.state, SessionState::Handshaking);
        let peer_id: NodeId = peer.id;

        // ---- 3) 两个方向各自的配对信号 ----
        let needed = next_matching(&mut events_a, Duration::from_secs(10), |event| {
            matches!(event, EngineEvent::PinNeeded { .. })
        })
        .await
        .expect("发起端应当收到 PinNeeded");
        match needed {
            EngineEvent::PinNeeded { id, .. } => assert_eq!(id, peer_id),
            other => panic!("期望 PinNeeded，实际 {other:?}"),
        }

        let pin = match next_matching(&mut events_b, Duration::from_secs(10), |event| {
            matches!(event, EngineEvent::DisplayPin { .. })
        })
        .await
        .expect("接收端应当收到 DisplayPin")
        {
            EngineEvent::DisplayPin { pin, .. } => pin,
            other => panic!("期望 DisplayPin，实际 {other:?}"),
        };
        assert_eq!(pin.len(), 6, "PIN 必须是 6 位");
        assert!(pin.chars().all(|c| c.is_ascii_digit()), "PIN 必须是数字");

        // ---- 4) 提交 PIN：结果走 PairCompleted 事件（外壳正是这么等的）----
        engine_a
            .submit_pin(peer_id, &pin)
            .await
            .expect("submit_pin 应当被接受");

        let completed = next_matching(&mut events_a, Duration::from_secs(15), |event| {
            matches!(event, EngineEvent::PairCompleted { .. })
        })
        .await
        .expect("应当收到 PairCompleted");
        match completed {
            EngineEvent::PairCompleted { id, ok, reason } => {
                assert_eq!(id, peer_id);
                assert!(ok, "配对应当成功，reason={reason}");
            }
            other => panic!("期望 PairCompleted，实际 {other:?}"),
        }

        // ---- 5) 配对后的状态：外壳据此把"等待配对"换成"开始推流"----
        let after = engine_a.peers();
        let after_peer = after.first().expect("peer after pairing");
        assert!(after_peer.trusted, "配对成功后必须受信");
        assert_eq!(
            after_peer.state,
            SessionState::Streaming,
            "引擎在握手完成后置 Streaming —— 外壳因此必须自己区分'会话已建立'与'正在推流'"
        );
        // B 侧也应记下 A（白名单落盘）
        assert!(
            engine_b.peers().first().map(|p| p.trusted).unwrap_or(false),
            "接收端也应把发起端写入信任库"
        );

        // ---- 6) 开流：合成采集 → Opus → QUIC → 空播放，应当真的流动起来 ----
        engine_a.start_send(peer_id).await.expect("start_send");
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
