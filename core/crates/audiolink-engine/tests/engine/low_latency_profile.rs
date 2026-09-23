//! 低延迟档（10 ms Opus 帧）的端到端回归：**档位是「两端一起选」的，这条用例把这件事钉住**。
//!
//! # 为什么需要它
//!
//! 「低延迟档」在协议层**不是协商出来的**：发送端在 `OPEN_STREAM` 里给出的 `CodecPref` 只是建议，
//! 接收端的解码器是按**本地** `EngineConfig.codec` 建的帧长（engine `runtime.rs` 里那句
//! `let codec = inner.config.codec;`）。于是「10 ms 档」真正的风险不在编码器，而在**两端不同步**：
//! 一端 10 ms、另一端 20 ms 时，每个真实包对解码器来说都「不是它期望的那一帧」，
//! 会被当成 PLC 掩盖处理 —— 链路上看起来一切正常（会话 Streaming、包也在到），**只有听感是坏的**。
//!
//! 所以这条用例断言三件事，缺一不可：
//!
//! 1. **帧真的到达**：30 个 10 ms 帧（每帧 960 交错样本）被 sink 接走；
//! 2. **没有 decode 错误**：整段运行期间没有 `EngineEvent::Error`，且接收侧 `plc_count` 为 0
//!    —— 这正是「混档」的指纹（真实包被当 PLC 掩盖会推高它）；
//! 3. **帧长确实是 10 ms**：两端遥测里的 `codec.frame_ms` 都是 10，水位也按 10 ms 计。
//!
//! 三条一起成立，才说明「档位」从 `EngineConfig` 走到了真实链路上 —— 只测编码器帧长是测不到的
//! （那只能证明 `CodecConfig::m1_low_delay_tight()` 的 `frame_ms` 是 10，与本条覆盖的路径无关）。
//!
//! **不测延迟绝对值**：回环（同一台机器、合成源、NullPlayout）比真机 Wi-Fi 快得多，
//! 这里的数字没有验收意义；延迟的回归护栏在 `latency_budget.rs`。低延迟档相对标准档「低在哪里」
//! 由真机测量回答，不由这里回答。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use audiolink_audio::{
    AudioError, CodecConfig, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use tokio::sync::mpsc;

/// 低延迟档的帧长（ms）—— 独立写死，不从 `CodecConfig` 读。
///
/// 拿被测实现的常量当断言，常量一改断言就跟着走（`playout_watermark.rs` 记过这条教训）。
/// 契约写的就是 10 ms，这里就写 10。
const LOW_LATENCY_FRAME_MS: u32 = 10;
/// 10 ms @ 48 kHz 立体声 = 480 × 2（`CHANNELS` = 2）。
const LOW_LATENCY_INTERLEAVED: usize = 960;

/// 记录每一拍写出的样本；与 `pcm_delivery.rs` 同形（同一件事不写两套）。
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

/// 把档位**同时**装到两端（这正是「两端必须同档位」的操作形式），然后跑一条真链路。
async fn run_profile(frame_ms: u32, expected_samples: usize) -> Option<(u32, u32, Vec<Vec<f32>>)> {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let codec = if frame_ms == LOW_LATENCY_FRAME_MS {
        CodecConfig::m1_low_delay_tight()
    } else {
        CodecConfig::m1_default()
    };

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    // 用 builder 落档位：这条用例同时是 `EngineConfig::with_codec` 的第一个使用点。
    send_config = send_config.with_codec(codec);
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(codec.frame_ms, 440.0)?))
    }));

    let (samples_tx, mut samples_rx) = mpsc::unbounded_channel();
    let mut recv_config = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_config.listen = "127.0.0.1:0".parse().unwrap();
    recv_config = recv_config.with_codec(codec);
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
    // 发送侧同样订阅：两端都不该在整段运行里报错。
    let mut sender_events = sender.subscribe();
    let peer = receiver.info().id;
    let sender_id = sender.info().id;

    let outcome = tokio::time::timeout(Duration::from_secs(10), async {
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
        while audio_packets < 30 {
            let samples = samples_rx.recv().await.expect("播放回调");
            if samples.iter().any(|sample| sample.abs() > 0.01) {
                audio_packets += 1;
            }
            writes.push(samples);
        }

        // 遥测读回：接收端看**发送方**那一栏（它解码的就是这条流）。
        let recv_view = receiver.telemetry(sender_id).expect("会话仍在，遥测应在");
        // 发送端看**接收方**那一栏；两侧的 codec 快照都来自各自的本地配置。
        let send_view = sender.telemetry(peer).expect("会话仍在，遥测应在");
        (recv_view, send_view, writes)
    })
    .await;

    let (recv_view, send_view, writes) = outcome.expect("10 s 内必须连接、开流并收到 30 包音频");

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();

    // 整段运行不许有会话级错误：混档最典型的表现就是解码路径开始报错 / 走掩盖。
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

    assert_eq!(
        recv_view.plc_count, 0,
        "接收侧发生了 {} 次丢包隐藏 —— 真实包被当 PLC 掩盖正是「两端帧长不同」的指纹",
        recv_view.plc_count
    );
    for samples in &writes {
        assert_eq!(
            samples.len(),
            expected_samples,
            "{frame_ms} ms 档每一拍的样本数必须正好是 {expected_samples}"
        );
        assert!(samples.iter().all(|sample| sample.is_finite()));
    }

    Some((
        u32::from(recv_view.codec.frame_ms),
        u32::from(send_view.codec.frame_ms),
        writes,
    ))
}

/// 低延迟档：两端都 10 ms，真实 PCM 端到端到达且**没有走 PLC**。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn low_latency_profile_delivers_10ms_frames_without_concealment() {
    let (recv_frame_ms, send_frame_ms, writes) =
        run_profile(LOW_LATENCY_FRAME_MS, LOW_LATENCY_INTERLEAVED)
            .await
            .expect("低延迟档必须能跑完整条链路");

    assert_eq!(
        recv_frame_ms, LOW_LATENCY_FRAME_MS,
        "接收侧遥测的帧长必须是 10 ms —— 它同时是解码器建帧长的依据"
    );
    assert_eq!(
        send_frame_ms, LOW_LATENCY_FRAME_MS,
        "发送侧遥测的帧长必须是 10 ms"
    );

    let audio_samples: usize = writes
        .iter()
        .filter(|samples| samples.iter().any(|sample| sample.abs() > 0.01))
        .map(Vec::len)
        .sum();
    assert_eq!(
        audio_samples,
        30 * LOW_LATENCY_INTERLEAVED,
        "10 ms 档的样本吞吐对不上：每条真实包必须正好带 960 个交错样本"
    );
}

/// 标准档对照：同一个函数、同一套断言在 20 ms 下也必须绿。
///
/// **为什么要有这一条对照**：只用例低延迟档，就分不清「档位接线对了」与「断言本来就会过」——
/// 参数化的对照组是「这条用例真的在区分两个档位」的证据（`pcm_delivery.rs` 用的是同一个思路）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn standard_profile_still_uses_20ms_frames() {
    let (recv_frame_ms, send_frame_ms, _) = run_profile(20, 1920)
        .await
        .expect("标准档必须能跑完整条链路");
    assert_eq!(recv_frame_ms, 20);
    assert_eq!(send_frame_ms, 20);
}
