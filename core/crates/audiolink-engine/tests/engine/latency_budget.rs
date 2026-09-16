//! M1 的核心指标：**端到端延迟**（「帧封口 → sink 写出」）在 CI 里也被测。
//!
//! 为什么需要这条护栏：`P50 ≤ 110 ms / P95 ≤ 150 ms` 是 M1 的验收线，但它此前只有**人工**测量
//! （`tools/link-loop` 跑一次、看数字）。人工测量不会在回归时报警 —— 某次改动让延迟翻倍，
//! 只有下次有人想起来跑一遍才会发现。这里把同一套测量（`MeasurementTap`）搬进集成测试。
//!
//! **阈值不是验收线**：回环（同一台机器、合成源、NullPlayout）比真机 Wi-Fi 快得多，所以这里的数字
//! 只能当**回归护栏**用 —— 它回答的是「延迟有没有突然变坏」，不是「有没有达到验收标准」。
//! 阈值按实测留了很宽的余量：宁可漏报一次轻微退化，也不要让 CI 因为机器抖动随机变红。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use audiolink_audio::{NullPlayout, PlayoutSink, SyntheticCapture};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, MeasurementTap, SessionState};
use audiolink_types::ErrorCode;

/// 回环下的回归上限（µs）。
///
/// **实测基线（2026-09-17，本机，且 8 h 长跑同时在跑）**：P50 = 40 615 µs、P95 = 41 007 µs、
/// P99 = 41 132 µs —— 约等于「两帧」（20 ms 编码 + 20 ms 抖动缓冲）。
/// 上限取到实测的 2–3.7 倍：够宽，不至于在共享 runner 上随机变红；够紧，能抓住「延迟翻倍」这类退化。
const P50_LIMIT_US: u64 = 80_000;
const P95_LIMIT_US: u64 = 150_000;

/// 采样窗口：够长才能覆盖攒帧、首帧与稳态三段。
const MEASURE_WINDOW: Duration = Duration::from_secs(6);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn loopback_end_to_end_latency_stays_within_budget() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;

    // 同一个 tap 装到两端：发送侧记「帧封口」，接收侧记「sink 写出」，按 seq 配对。
    let tap = Arc::new(MeasurementTap::new(16_384));

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    send_config.measurement = Some(Arc::clone(&tap));
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));

    let mut recv_config = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_config.listen = "127.0.0.1:0".parse().unwrap();
    recv_config.codec.frame_ms = frame_ms;
    recv_config.measurement = Some(Arc::clone(&tap));
    recv_config.playout = Some(Arc::new(move || {
        Ok(Box::new(NullPlayout::new(60)) as Box<dyn PlayoutSink>)
    }));

    let receiver = Engine::start(recv_config).await.expect("接收引擎");
    let _accept = receiver.spawn_accept_loop();
    let mut events = receiver.subscribe();
    let receiver_id = receiver.info().id;

    let sender = Engine::start(send_config).await.expect("发送引擎");

    // 首次连接必须先走 PIN 配对（§5）。
    let error = sender.connect(receiver.local_addr()).await.unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotPaired, "首次连接必须先要 PIN");
    let pin = loop {
        if let EngineEvent::DisplayPin { pin, .. } = events.recv().await.unwrap() {
            break pin;
        }
    };
    sender
        .submit_pin(receiver_id, &pin)
        .await
        .expect("PIN 配对");

    let deadline = Instant::now() + Duration::from_secs(15);
    while !sender
        .peers()
        .iter()
        .any(|peer| peer.id == receiver_id && peer.state == SessionState::Streaming)
    {
        assert!(Instant::now() < deadline, "握手没有在 15 s 内完成");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    sender.start_send(receiver_id).await.expect("开流");

    tokio::time::sleep(MEASURE_WINDOW).await;

    let summary = tap.summary().expect("采样窗口内必须配对上一些帧");
    let (orphan_sealed, orphan_played) = tap.orphan_counts();
    println!(
        "[latency] 配对={} 封口未播={orphan_sealed} 播放未封口={orphan_played} P50={} µs P95={} µs P99={} µs",
        tap.matched(),
        summary.p50,
        summary.p95,
        summary.p99,
    );

    assert!(
        tap.matched() >= 50,
        "配对的帧太少（{}），这条护栏没有意义",
        tap.matched()
    );
    assert!(
        u64::from(summary.p50) <= P50_LIMIT_US,
        "回环 P50 退化到 {} µs（上限 {P50_LIMIT_US} µs）",
        summary.p50
    );
    assert!(
        u64::from(summary.p95) <= P95_LIMIT_US,
        "回环 P95 退化到 {} µs（上限 {P95_LIMIT_US} µs）",
        summary.p95
    );
    assert!(
        summary.p50 <= summary.p95,
        "P50 {} µs 不应大于 P95 {} µs",
        summary.p50,
        summary.p95
    );
}
