//! FR-28 端到端回归：**真实双 Engine + 真实 QUIC**。接收侧的播放 sink 被打成「坏了」之后，
//! 引擎必须在「链路还在给音频、设备却 ≥ 2 s 一帧都没接走」时**自己把播放器换掉**并恢复出声。
//!
//! # 这条测试要证明的三件事（缺一件就等于没证）
//!
//! 1. **坏的那一代一帧都没被算作输出**（`accepted` 里 generation=0 的条数为 0）——
//!    否则「重建后有声」只是原本就有声；
//! 2. **坏的那一代期间引擎一直在喂数据**（`attempts` 里 generation=0 的条数远大于 0）——
//!    链路是好的，问题在设备。少了这条，断流也会被算成「sink 故障」；
//! 3. **重建之后真的接走了非静音音频**，且 `2002 SINK_REBUILD` 被报出一次。
//!
//! 两种故障形态各测一遍：**写失败**（`write` 返回 Err）与**假成功**（`write` 返回 Ok 却不接数据）。
//! 后者是「一次异常永久静音」的现实形态 —— 引擎提交成功、设备不出声，只有 sink 自己的账本
//! （`PlayoutStats::frames_written`）能戳穿它。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::ErrorCode;
use tokio::sync::broadcast;

/// 注入的故障形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    /// 正常：真的接走数据并记账。
    Healthy,
    /// 写失败（设备掉线 / 缓冲抽干）：`write` 返回 `Err`。
    WriteError,
    /// 「假成功」：`write` 返回 `Ok`，却一帧都不接走（账本不增长）。
    SilentAccept,
}

/// 引擎**试图写**给设备的每一帧：(第几代 sink, 样本 RMS)。
type Attempt = (usize, f32);
/// 设备**真的接走**的每一帧：(第几代 sink, 样本 RMS)。
type Accepted = (usize, f32);

#[derive(Default)]
struct SinkLog {
    attempts: Vec<Attempt>,
    accepted: Vec<Accepted>,
}

struct FlakySink {
    /// 第几代（0 = 起播那一代，1 起 = 重建后的）。
    generation: usize,
    failure: Failure,
    inner: NullPlayout,
    log: Arc<Mutex<SinkLog>>,
}

impl FlakySink {
    fn new(generation: usize, failure: Failure, log: Arc<Mutex<SinkLog>>) -> Self {
        Self {
            generation,
            failure,
            inner: NullPlayout::new(60),
            log,
        }
    }
}

impl PlayoutSink for FlakySink {
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
        let rms = rms(samples);
        if let Ok(mut log) = self.log.lock() {
            log.attempts.push((self.generation, rms));
        }
        match self.failure {
            Failure::Healthy => {
                self.inner.write(samples)?;
                if let Ok(mut log) = self.log.lock() {
                    log.accepted.push((self.generation, rms));
                }
                Ok(())
            }
            // 坏的两代都**不碰** inner：账本因此不增长 —— 这正是看门狗唯一的判据。
            Failure::WriteError => Err(AudioError::stream_failed_owned(
                "注入：播放设备写失败".to_owned(),
            )),
            Failure::SilentAccept => Ok(()),
        }
    }

    fn stats(&self) -> PlayoutStats {
        self.inner.stats()
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn backend_name(&self) -> &'static str {
        "flaky-null"
    }
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|value| value * value).sum::<f32>() / samples.len() as f32).sqrt()
}

/// 起一对真实引擎并把流拉起来；`first_generation` 是**起播那一代** sink 的形态（之后一律正常）。
struct Rig {
    _dir: tempfile::TempDir,
    sender: Arc<Engine>,
    receiver: Arc<Engine>,
    accept: tokio::task::JoinHandle<()>,
    events: broadcast::Receiver<EngineEvent>,
    generations: Arc<AtomicUsize>,
    log: Arc<Mutex<SinkLog>>,
}

async fn wire_up(first_generation: Failure) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let mut send_cfg = EngineConfig::new("sender", dir.path().join("sender"));
    send_cfg.listen = "127.0.0.1:0".parse().unwrap();
    send_cfg.capture = Some(Arc::new(|| Ok(Box::new(SyntheticCapture::new(20, 440.0)?))));

    let log = Arc::new(Mutex::new(SinkLog::default()));
    let generations = Arc::new(AtomicUsize::new(0));
    let mut recv_cfg = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_cfg.listen = "127.0.0.1:0".parse().unwrap();
    recv_cfg.playout = Some({
        let log = Arc::clone(&log);
        let generations = Arc::clone(&generations);
        Arc::new(move || {
            let generation = generations.fetch_add(1, Ordering::SeqCst);
            // 只有起播那一代会坏：重建之后设备就“修好了”，否则看不出重建有没有用。
            let failure = if generation == 0 {
                first_generation
            } else {
                Failure::Healthy
            };
            Ok(
                Box::new(FlakySink::new(generation, failure, Arc::clone(&log)))
                    as Box<dyn PlayoutSink>,
            )
        })
    });

    let sender = Engine::start(send_cfg).await.unwrap();
    let receiver = Engine::start(recv_cfg).await.unwrap();
    let accept = receiver.spawn_accept_loop();
    let events = receiver.subscribe();
    let peer = receiver.info().id;

    sender
        .connect(receiver.local_addr())
        .await
        .expect("连接应当直接成功（无认证）");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while !sender
        .peers()
        .iter()
        .any(|status| status.id == peer && status.state == SessionState::Streaming)
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "等 Streaming 超时（接收侧的播放管线没建起来）"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    sender.start_send(peer).await.unwrap();

    Rig {
        _dir: dir,
        sender,
        receiver,
        accept,
        events,
        generations,
        log,
    }
}

impl Rig {
    async fn shutdown(self) {
        eprintln!("[rig] shutdown 开始");
        self.sender.shutdown().await;
        eprintln!("[rig] 发送侧已停");
        self.receiver.shutdown().await;
        eprintln!("[rig] 接收侧已停");
        self.accept.await.unwrap();
    }

    fn accepted_non_silent(&self, after_generation: usize) -> Option<Accepted> {
        self.log
            .lock()
            .unwrap()
            .accepted
            .iter()
            .copied()
            .find(|(generation, rms)| *generation >= after_generation && *rms > 0.01)
    }

    fn count_attempts_of_generation(&self, generation: usize) -> usize {
        self.log
            .lock()
            .unwrap()
            .attempts
            .iter()
            .filter(|(found, _)| *found == generation)
            .count()
    }

    fn count_accepted_of_generation(&self, generation: usize) -> usize {
        self.log
            .lock()
            .unwrap()
            .accepted
            .iter()
            .filter(|(found, _)| *found == generation)
            .count()
    }

    /// 轮询到「重建后接走了非静音音频」为止；期间把 2002 事件收下来。
    async fn wait_for_audible_after_rebuild(
        &mut self,
        timeout: Duration,
    ) -> (Vec<String>, Option<Accepted>) {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut rebuilds: Vec<String> = Vec::new();
        loop {
            while let Ok(event) = self.events.try_recv() {
                if let EngineEvent::Error { code, context } = event
                    && code == ErrorCode::SinkRebuild.as_u16()
                {
                    rebuilds.push(context);
                }
            }
            if let Some(found) = self.accepted_non_silent(1) {
                return (rebuilds, Some(found));
            }
            if tokio::time::Instant::now() >= deadline {
                return (rebuilds, None);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_write_failure_triggers_a_rebuild_and_the_audio_comes_back() {
    let mut rig = wire_up(Failure::WriteError).await;
    let (rebuilds, audible) = rig
        .wait_for_audible_after_rebuild(Duration::from_secs(20))
        .await;

    let attempts_of_first = rig.count_attempts_of_generation(0);
    let accepted_of_first = rig.count_accepted_of_generation(0);
    let generations = rig.generations.load(Ordering::SeqCst);
    println!(
        "[注入=写失败] 坏代被喂 {} 帧 / 被接走 {} 帧；工厂调用 {} 次；2002 {} 条：{:?}",
        attempts_of_first,
        accepted_of_first,
        generations,
        rebuilds.len(),
        rebuilds
    );
    rig.shutdown().await;

    // 「重建前一段时间无输出」：坏的那一代引擎一直在喂数据，但一帧都没被接走。
    assert!(
        attempts_of_first > 50,
        "坏 sink 期间引擎必须一直在喂数据（链路正常），实际只有 {attempts_of_first} 次"
    );
    assert_eq!(accepted_of_first, 0, "写失败的 sink 一帧都不该被算作输出");
    // 「真的重建了」：向工厂要了第二台设备，并报出 2002。
    assert_eq!(generations, 2, "必须重建一次（= 第二次向工厂要设备）");
    assert_eq!(
        rebuilds.len(),
        1,
        "2002 SINK_REBUILD 必须被报出一次：{rebuilds:?}"
    );
    assert!(
        rebuilds[0].contains("没有接走任何一帧音频"),
        "重建理由要写清是「无输出但链路正常」：{}",
        rebuilds[0]
    );
    // 「重建后有非静音输出」：不是没报错，是真的接走了真实音频。
    let (generation, rms) = audible.expect("重建后必须真的接走非静音音频");
    assert_eq!(generation, 1, "非静音输出必须来自重建后的那一代");
    assert!(rms > 0.01, "接走的必须是真实音频（不是补静音）：rms={rms}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sink_that_accepts_but_never_accounts_frames_is_rebuilt_too() {
    // 「假成功」：write 返回 Ok、设备不出声 —— 只有 sink 自己的账本能戳穿它。
    // 现实里的「一次异常永久静音」正是这个形态（提交成功、永远没声音）。
    let mut rig = wire_up(Failure::SilentAccept).await;
    let (rebuilds, audible) = rig
        .wait_for_audible_after_rebuild(Duration::from_secs(20))
        .await;

    let accepted_of_first = rig.count_accepted_of_generation(0);
    let generations = rig.generations.load(Ordering::SeqCst);
    println!(
        "[注入=假成功] 坏代被接走 {} 帧；工厂调用 {} 次；2002 {} 条：{:?}",
        accepted_of_first,
        generations,
        rebuilds.len(),
        rebuilds
    );
    rig.shutdown().await;

    assert_eq!(accepted_of_first, 0, "假成功的 sink 也不该有任何输出证据");
    assert_eq!(generations, 2, "假成功同样必须触发重建");
    assert_eq!(
        rebuilds.len(),
        1,
        "2002 SINK_REBUILD 必须被报出：{rebuilds:?}"
    );
    let (generation, rms) = audible.expect("重建后必须真的出声");
    assert_eq!(generation, 1);
    assert!(rms > 0.01, "重建后接走的必须是真实音频：rms={rms}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_healthy_playout_never_rebuilds_the_sink() {
    // 不误伤的对照：干净链路上跑 6 s（= 3 个判定窗口），一台设备都不该被换掉。
    let mut rig = wire_up(Failure::Healthy).await;
    tokio::time::sleep(Duration::from_secs(6)).await;

    let generations = rig.generations.load(Ordering::SeqCst);
    let accepted = rig.count_accepted_of_generation(0);
    let mut rebuilds = 0_usize;
    while let Ok(event) = rig.events.try_recv() {
        if let EngineEvent::Error { code, .. } = event
            && code == ErrorCode::SinkRebuild.as_u16()
        {
            rebuilds += 1;
        }
    }
    println!(
        "[对照=健康] 6 s 内接走 {accepted} 帧真实音频；工厂调用 {generations} 次；2002 {rebuilds} 条"
    );
    rig.shutdown().await;

    assert!(
        accepted > 100,
        "健康链路上必须一直在输出真实音频：{accepted} 帧"
    );
    assert_eq!(rebuilds, 0, "健康链路不得重建");
    assert_eq!(generations, 1, "工厂只该被调用一次");
}

#[test]
fn silence_helper_agrees_with_the_audio_threshold() {
    // 断言用到的「非静音」阈值（0.01 RMS）与合成源的实际电平分开锚一下：
    // 440 Hz / 0.4 幅度正弦的 RMS ≈ 0.28，补静音则是 0.0 —— 两者差两个数量级。
    let silence = vec![0.0_f32; 960];
    assert!(rms(&silence) < 0.01);
    let tone: Vec<f32> = (0..960)
        .map(|index| (index as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.4)
        .collect();
    assert!(rms(&tone) > 0.2, "正弦 RMS 应当远高于判定阈值");
}
