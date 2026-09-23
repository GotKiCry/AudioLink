//! 帧长联动：流的 Opus 帧长由**发送端档位**决定，接收端在开流期协商并跟随。
//!
//! # 这条用例钉住的是「混档」那个坑
//!
//! 在此之前，接收端的解码器是按**本地** `EngineConfig.codec` 建的帧长：一端 10 ms、另一端
//! 20 ms 时，每个真实包对解码器来说都「不是它期望的那一帧」，会被当成 PLC 掩盖处理 ——
//! 链路上看起来一切正常（会话 Streaming、包也在到、没有任何错误），**只有听感是坏的**。
//! 所以这条用例的判据不是「有没有报错」，而是：
//!
//! 1. **帧真的到达**：接收端 sink 接走 30 个真实帧，每拍样本数按**发送端**的帧长；
//! 2. **没有走掩盖**：`plc_count == 0` —— 这正是「混档」的指纹（真实包被当 PLC 掩盖会推高它）；
//! 3. **协商结果可读**：接收端遥测的 `frame_ms` 是对端的帧长，且 `Engine::negotiated_frame_ms`
//!    读到的也是对端的帧长。
//!
//! 反向用例（发送 20 / 接收 10）同样必须对齐 —— 只测一个方向的话，「跟随」和「碰巧本地就对」
//! 分不出来。同档对照（10/10、20/20）保证没有回归。
//!
//! 最后一条是**非法帧长**（15 ms）：协商必须回退本地档并且**不 panic**。
//!
//! 与 low_latency_profile.rs 的分工：那边测「两端同档时档位真的生效」，这边测
//! 「两端**不同档**时接收端会不会跟上」。两条一起成立，帧长联动才算完整。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use audiolink_audio::{
    AudioError, CodecConfig, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use tokio::sync::mpsc;

/// 标准档帧长（ms）—— 独立写死，不从 `CodecConfig` 读（常量一改断言就跟着走的教训见
/// playout_watermark.rs）。
const STD_FRAME_MS: u32 = 20;
/// 低延迟档帧长（ms）。
const TIGHT_FRAME_MS: u32 = 10;
/// 15 ms：协议合法集合 {10,20,40,60} 之外的取值，用来验协商回退。
const ILLEGAL_FRAME_MS: u32 = 15;

/// 记录每一拍写出的样本；与 pcm_delivery.rs / low_latency_profile.rs 同形。
struct RecordingSink {
    inner: NullPlayout,
    samples: mpsc::UnboundedSender<Vec<f32>>,
}

impl PlayoutSink for RecordingSink {
    fn device_format(&self) -> DeviceFormat {
        self.inner.device_format()
    }

    fn requested_buffer_ms(&self) -> u32 {
        self.inner.requested_buffer_ms()
    }

    fn effective_buffer_ms(&self) -> u32 {
        self.inner.effective_buffer_ms()
    }

    fn buffered_frames(&mut self) -> u32 {
        self.inner.buffered_frames()
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), AudioError> {
        self.inner.write(samples)?;
        let _ = self.samples.send(samples.to_vec());
        Ok(())
    }

    fn stats(&self) -> PlayoutStats {
        self.inner.stats()
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn backend_name(&self) -> &'static str {
        "recording-null"
    }
}

/// 一条真实链路跑完之后的读数。
struct Outcome {
    /// 接收端 `Engine::negotiated_frame_ms` 读到的协商帧长。
    negotiated: Option<u8>,
    /// 两端跑完整段之后收到的会话级错误（空 = 整段运行没有会话级错误）。
    errors: Vec<String>,
}

/// 按帧长取对应档位预设。
fn codec_for(frame_ms: u32) -> CodecConfig {
    if frame_ms == TIGHT_FRAME_MS {
        CodecConfig::m1_low_delay_tight()
    } else {
        CodecConfig::m1_default()
    }
}

/// 用「发送端档位 + 接收端档位」跑一条真链路，返回接收侧的读数。
///
/// `expected_samples` 按**发送端**的帧长算：协商生效时每拍就该是那么多样本。
async fn run_link(
    send_frame_ms: u32,
    recv_frame_ms: u32,
    expected_samples: usize,
    expected_packets: usize,
) -> Outcome {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let send_codec = codec_for(send_frame_ms);
    let recv_codec = codec_for(recv_frame_ms);

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config = send_config.with_codec(send_codec);
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(send_codec.frame_ms, 440.0)?))
    }));

    let (samples_tx, mut samples_rx) = mpsc::unbounded_channel();
    let mut recv_config = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_config.listen = "127.0.0.1:0".parse().unwrap();
    // 接收端就装**异档**的 codec：协商正确时它必须在开流后改用发送端的帧长。
    recv_config = recv_config.with_codec(recv_codec);
    recv_config.playout = Some(Arc::new(move || {
        Ok(Box::new(RecordingSink {
            inner: NullPlayout::new(60),
            samples: samples_tx.clone(),
        }))
    }));

    let sender = Engine::start(send_config).await.expect("发送引擎");
    let receiver = Engine::start(recv_config).await.expect("接收引擎");
    let accept = receiver.spawn_accept_loop();
    let mut events = receiver.subscribe();
    let mut sender_events = sender.subscribe();
    let peer = receiver.info().id;
    let sender_id = sender.info().id;

    let outcome = tokio::time::timeout(Duration::from_secs(15), async {
        sender
            .connect(receiver.local_addr())
            .await
            .expect("连接应当直接成功（无认证）");
        while !sender
            .peers()
            .iter()
            .any(|p| p.id == peer && p.state == SessionState::Streaming)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        sender.start_send(peer).await.expect("开流");

        // 等实际输出而不是固定 sleep：CI 的调度速度不该决定这条用例的成败。
        let mut writes = Vec::new();
        let mut audio_packets = 0;
        while audio_packets < expected_packets {
            let samples = samples_rx.recv().await.expect("播放回调");
            if samples.iter().any(|sample| sample.abs() > 0.01) {
                audio_packets += 1;
            }
            writes.push(samples);
        }

        let recv_view = receiver.telemetry(sender_id).expect("会话仍在，遥测应在");
        let negotiated = receiver.negotiated_frame_ms(sender_id);
        (recv_view, negotiated, writes)
    })
    .await;

    let (recv_view, negotiated, writes) =
        outcome.unwrap_or_else(|_| panic!("15 s 内必须连接、开流并收到 {expected_packets} 包音频"));

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();

    // 整段运行不许有会话级错误。混档本身不一定报错，所以这条只是**额外**护栏；
    // 真正的判据是下面三条。
    let mut errors = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let EngineEvent::Error { code, context } = event {
            errors.push(format!("#{code} {context}"));
        }
    }
    while let Ok(event) = sender_events.try_recv() {
        if let EngineEvent::Error { code, context } = event {
            errors.push(format!("#{code} {context}"));
        }
    }
    assert!(errors.is_empty(), "链路报错了：{errors:?}");

    // 三条核心判据在这里统一断言：帧到达、没走掩盖、协商结果可读。
    assert_eq!(
        recv_view.plc_count, 0,
        "接收侧发生了 {} 次丢包隐藏 —— 真实包被当 PLC 掩盖正是「两端帧长不同」的指纹",
        recv_view.plc_count
    );
    for samples in &writes {
        assert_eq!(
            samples.len(),
            expected_samples,
            "发送端 {send_frame_ms} ms / 接收端 {recv_frame_ms} ms：每一拍的样本数必须是 {expected_samples}"
        );
        assert!(samples.iter().all(|sample| sample.is_finite()));
    }
    assert_eq!(
        u32::from(recv_view.codec.frame_ms),
        send_frame_ms,
        "接收端遥测的帧长必须跟随**发送端**（{send_frame_ms} ms），而不是本地档（{recv_frame_ms} ms）"
    );
    assert_eq!(
        negotiated,
        Some(u8::try_from(send_frame_ms).unwrap()),
        "接收端的 negotiated_frame_ms 必须是发送端的帧长"
    );

    Outcome { negotiated, errors }
}

/// 发送端 10 ms + 接收端 20 ms（**异档**）：接收端按 10 ms 解码。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sender_10ms_makes_receiver_follow_to_10ms() {
    // 10 ms @ 48 kHz 立体声 = 480 × 2 = 960。
    let outcome = run_link(TIGHT_FRAME_MS, STD_FRAME_MS, 960, 30).await;
    assert!(
        outcome.errors.is_empty(),
        "链路报错了：{:?}",
        outcome.errors
    );
    assert_eq!(
        outcome.negotiated,
        Some(10),
        "接收端必须跟随发送端的 10 ms 档"
    );
}

/// 反向：发送端 20 ms + 接收端 10 ms（**异档**）：接收端同样对齐到 20 ms。
///
/// **为什么反向也要测**：只测 10→20 一个方向的话，「接收端真的跟随了」与「断言本来就会过」
/// 分不出来 —— 两个方向一起才排得干净。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sender_20ms_makes_receiver_follow_to_20ms() {
    // 20 ms @ 48 kHz 立体声 = 960 × 2 = 1920。
    let outcome = run_link(STD_FRAME_MS, TIGHT_FRAME_MS, 1920, 30).await;
    assert!(
        outcome.errors.is_empty(),
        "链路报错了：{:?}",
        outcome.errors
    );
    assert_eq!(
        outcome.negotiated,
        Some(20),
        "接收端必须跟随发送端的 20 ms 档（反向同样要跟上）"
    );
}

/// 同档对照 10/10：协商结果与本地档一致，不回归。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_tight_profile_is_unchanged() {
    let outcome = run_link(TIGHT_FRAME_MS, TIGHT_FRAME_MS, 960, 30).await;
    assert!(
        outcome.errors.is_empty(),
        "链路报错了：{:?}",
        outcome.errors
    );
    assert_eq!(outcome.negotiated, Some(10));
}

/// 同档对照 20/20：标准档行为与加入帧长联动之前完全一致。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_standard_profile_is_unchanged() {
    let outcome = run_link(STD_FRAME_MS, STD_FRAME_MS, 1920, 30).await;
    assert!(
        outcome.errors.is_empty(),
        "链路报错了：{:?}",
        outcome.errors
    );
    assert_eq!(outcome.negotiated, Some(20));
}

/// 非法帧长（15 ms）：协商**回退本地档**，链路照跑，不 panic。
///
/// 这条覆盖的是「对端比我们新 / 载荷损坏」那一类：帧长不在 {10,20,40,60} 里时，
/// 引擎必须用本地档继续出声，而不是拒绝开流（拒绝 = 用户完全没声音）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn illegal_frame_ms_falls_back_to_local_profile() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    let send_codec = CodecConfig::m1_default();
    send_config = send_config.with_codec(send_codec);
    // 发送端在 OPEN_STREAM 里**声称** 15 ms，而它实际仍按 20 ms 编码 ——
    // 这正是「对端比我们新 / 载荷被改坏」时的最坏输入。
    send_config.test_advertised_frame_ms = Some(u8::try_from(ILLEGAL_FRAME_MS).unwrap());
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(send_codec.frame_ms, 440.0)?))
    }));

    let (samples_tx, mut samples_rx) = mpsc::unbounded_channel();
    let mut recv_config = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_config.listen = "127.0.0.1:0".parse().unwrap();
    recv_config = recv_config.with_codec(CodecConfig::m1_default());
    recv_config.playout = Some(Arc::new(move || {
        Ok(Box::new(RecordingSink {
            inner: NullPlayout::new(60),
            samples: samples_tx.clone(),
        }))
    }));

    let sender = Engine::start(send_config).await.expect("发送引擎");
    let receiver = Engine::start(recv_config).await.expect("接收引擎");
    let accept = receiver.spawn_accept_loop();
    let peer = receiver.info().id;
    let sender_id = sender.info().id;

    let outcome = tokio::time::timeout(Duration::from_secs(15), async {
        sender
            .connect(receiver.local_addr())
            .await
            .expect("连接应当直接成功（无认证）");
        while !sender
            .peers()
            .iter()
            .any(|p| p.id == peer && p.state == SessionState::Streaming)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // 发送端带着非法帧长的 codec_prefs 开流：接收端必须回退本地档（20 ms）而不是 panic。
        sender.start_send(peer).await.expect("开流");

        let mut audio_packets = 0;
        while audio_packets < 30 {
            let samples = samples_rx.recv().await.expect("播放回调");
            if samples.iter().any(|sample| sample.abs() > 0.01) {
                audio_packets += 1;
            }
        }

        receiver.negotiated_frame_ms(sender_id)
    })
    .await
    .expect("15 s 内必须连接、开流并收到 30 包音频");

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();

    assert_eq!(
        outcome,
        Some(20),
        "非法帧长必须回退到本地档（20 ms），而不是照单全收或拒绝开流"
    );
}
