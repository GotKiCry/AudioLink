//! M3 验收「同步」的回环版本：**两个接收端播放同一帧的时刻差**（真实 QUIC）。
//!
//! M3 的验收线是「组内偏差 ≤ ±10 ms（P95）」，验收方法写的是「双机同期录音 + 波形对齐」——
//! 那需要两台设备。但这条指标在**同一进程**里就能量：两个接收端共用同一个单调时钟，
//! 只要记下各自 sink 写出每一帧的时刻，同一序号（第 k 次写出）的时刻差就是它们的播放偏差。
//!
//! 这样做的价值不是替代真机（真机还有扬声器输出延迟差异，进程内量不到），而是：
//! **把「组内排播有没有退化」变成每次 CI 都会检查的事**。真机验收回答「能不能用」，
//! 这条护栏回答「有没有变坏」。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use audiolink_types::{ErrorCode, NodeId};
use tokio::task::JoinHandle;

/// 一次写出：时刻 + 是否静音补帧。
#[derive(Clone, Copy)]
struct Stamp {
    at: Instant,
    /// 全是零样本 = 排播补的静音（不是真实音频）。
    silence: bool,
}

/// 每次写出都**盖一个时间戳**的播放端（同一进程、同一单调时钟，所以两端可比）。
struct StampingSink {
    inner: NullPlayout,
    stamps: Arc<Mutex<Vec<Stamp>>>,
}

impl PlayoutSink for StampingSink {
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
        if let Ok(mut list) = self.stamps.lock() {
            list.push(Stamp {
                at: Instant::now(),
                silence: samples.iter().all(|sample| sample.abs() < 0.01),
            });
        }
        Ok(())
    }

    fn stats(&self) -> PlayoutStats {
        self.inner.stats()
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn backend_name(&self) -> &'static str {
        "stamping"
    }
}

async fn start_receiver(
    dir: &Path,
    index: usize,
    stamps: Arc<Mutex<Vec<Stamp>>>,
    frame_ms: u32,
) -> (Arc<Engine>, JoinHandle<()>) {
    let mut config = EngineConfig::new(
        format!("receiver-{index}"),
        dir.join(format!("receiver-{index}")),
    );
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.codec.frame_ms = frame_ms;
    config.playout = Some(Arc::new(move || {
        Ok(Box::new(StampingSink {
            inner: NullPlayout::new(60),
            stamps: Arc::clone(&stamps),
        }) as Box<dyn PlayoutSink>)
    }));
    let engine = Engine::start(config).await.expect("接收引擎");
    let accept = engine.spawn_accept_loop();
    (engine, accept)
}

async fn connect_and_pair(sender: &Arc<Engine>, receiver: &Arc<Engine>) -> NodeId {
    let mut events = receiver.subscribe();
    let receiver_id = receiver.info().id;
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
    receiver_id
}

async fn wait_streaming(sender: &Arc<Engine>, id: NodeId) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !sender
        .peers()
        .iter()
        .any(|peer| peer.id == id && peer.state == SessionState::Streaming)
    {
        assert!(
            Instant::now() < deadline,
            "会话没有在 20 s 内进入 streaming"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// 允许整数帧偏移后、使 P50 最小的那一组：返回 (偏移帧数, 样本数, P50, P95)。
///
/// 为什么需要它：直接比「第 k 次写出」会把**两端起播时刻的差**整帧算成偏差。
/// 两个数字回答的是两个不同问题 ——
/// * `absolute`（偏移 0）：两台设备**听到同一内容**的时刻差，也就是 M3 验收线关心的那个量；
/// * `aligned`（最佳偏移）：把起播差扣掉之后**播放抖动**有多大，反映排播稳不稳。
fn best_alignment(a: &[Instant], b: &[Instant]) -> (i32, usize, f64, f64) {
    let skip = 50;
    let mut best: Option<(i32, usize, f64, f64)> = None;
    for shift in -3_i32..=3 {
        let mut diffs: Vec<f64> = Vec::new();
        for (i, stamp) in a.iter().enumerate().skip(skip) {
            let j = i as i32 + shift;
            if j < skip as i32 || j as usize >= b.len() {
                continue;
            }
            let other = b[j as usize];
            let delta = if *stamp >= other {
                *stamp - other
            } else {
                other - *stamp
            };
            diffs.push(delta.as_secs_f64() * 1000.0);
        }
        if diffs.len() < 20 {
            continue;
        }
        diffs.sort_by(|x, y| x.partial_cmp(y).unwrap());
        let p50 = diffs[(diffs.len() - 1) / 2];
        let p95 = diffs[((diffs.len() - 1) as f64 * 0.95).round() as usize];
        if best.is_none_or(|(_, _, best_p50, _)| p50 < best_p50) {
            best = Some((shift, diffs.len(), p50, p95));
        }
    }
    best.expect("至少应当有一组对齐方式可用")
}

/// 把广播缓冲里累积的「首次排播」事件扫出来：`(target_local_us, wait_us)`。
///
/// 为什么需要：`PlayoutScheduled` 只在**首次进入排播**时报一次，它就是「这一端到底有没有走排播」的证据。
/// 若失败样本里两端一个有、一个没有，那就说明有一端是在排播生效**之前**就开始播了 —— 它走的是本地游标，
/// 起拍时刻由「队列攒够帧」决定，与对端的 epoch 网格无关（见 docs/49 §5.5）。
fn drain_scheduled(events: &mut tokio::sync::broadcast::Receiver<EngineEvent>) -> Vec<(i64, u64)> {
    let mut out = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let EngineEvent::PlayoutScheduled {
            target_local_us,
            wait_us,
            ..
        } = event
        {
            out.push((target_local_us, wait_us));
        }
    }
    out
}
/// 同一「第 k 次写出」在两端的时刻差（毫秒），返回 (样本数, P50, P95)。
fn deviation_ms(a: &[Instant], b: &[Instant]) -> (usize, f64, f64) {
    // 丢掉前 1 s：攒帧与首帧那一秒本来就不该进稳态统计。
    let skip = 50;
    let n = a.len().min(b.len());
    assert!(n > skip + 20, "采样点太少（{n} 个），这条护栏没有意义");
    let mut diffs: Vec<f64> = (skip..n)
        .map(|i| {
            let delta = if a[i] >= b[i] {
                a[i] - b[i]
            } else {
                b[i] - a[i]
            };
            delta.as_secs_f64() * 1000.0
        })
        .collect();
    diffs.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let pick = |pct: f64| diffs[((diffs.len() - 1) as f64 * pct).round() as usize];
    (n - skip, pick(0.50), pick(0.95))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_receivers_play_the_same_frame_within_ten_milliseconds() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;
    let lead_ms = 120_u32;

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));
    let sender = Engine::start(send_config).await.expect("发送引擎");

    let stamps_a = Arc::new(Mutex::new(Vec::new()));
    let stamps_b = Arc::new(Mutex::new(Vec::new()));
    let (receiver_a, accept_a) =
        start_receiver(dir.path(), 0, Arc::clone(&stamps_a), frame_ms).await;
    let (receiver_b, accept_b) =
        start_receiver(dir.path(), 1, Arc::clone(&stamps_b), frame_ms).await;

    let id_a = connect_and_pair(&sender, &receiver_a).await;
    let id_b = connect_and_pair(&sender, &receiver_b).await;
    wait_streaming(&sender, id_a).await;
    wait_streaming(&sender, id_b).await;

    // §7：建组 → 成员按**同一个** epoch 排播（这正是「组内同步」要解决的问题）。
    // 组号本身不用断言：组有没有真的生效，由下面的**偏差数字**体现（这比断言一个 ID 有力）。
    let _group_id = sender
        .create_group(&[id_a, id_b], lead_ms)
        .await
        .expect("建组");

    sender
        .start_send_many(&[id_a, id_b])
        .await
        .expect("两台一起开流");

    let mut events_a = receiver_a.subscribe();
    let mut events_b = receiver_b.subscribe();

    tokio::time::sleep(Duration::from_secs(6)).await;

    let a_all = stamps_a.lock().unwrap().clone();
    let b_all = stamps_b.lock().unwrap().clone();
    // 诊断（第 58 轮）：两端**首次写出真实音频**的位置。若两侧不同（例如 1 vs 0），说明有一端
    // 起拍时先补了一拍静音 —— 那会让它的整条时间轴后移一帧，正是「k = 0 就差 20 ms」的形状。
    let first_audio_a = a_all.iter().position(|stamp| !stamp.silence);
    let first_audio_b = b_all.iter().position(|stamp| !stamp.silence);
    let scheduled_a = drain_scheduled(&mut events_a);
    let scheduled_b = drain_scheduled(&mut events_b);
    println!("[group-sync][diag] 排播事件 A={scheduled_a:?} · B={scheduled_b:?}");
    let a: Vec<Instant> = a_all.iter().map(|stamp| stamp.at).collect();
    let b: Vec<Instant> = b_all.iter().map(|stamp| stamp.at).collect();
    let (samples, absolute_p50, absolute_p95) = deviation_ms(&a, &b);
    let (shift, aligned_samples, aligned_p50, aligned_p95) = best_alignment(&a, &b);
    // 诊断（2026-09-17，第 57 轮）：区分「起拍就差一帧」与「中途某拍开始差一帧」。
    //
    // 这条测试比的是**第 k 次写出的下标** —— 只要一端多写/少写一拍（补静音），索引就整体错位，
    // 看起来就像「整段差一帧」。下面的打印把首次偏离的位置与两端写入次数钉出来：
    // k 很小 = 起拍差异；k 靠中间 = 运行中某一拍多写了一次。
    let first_jump = {
        let n = a.len().min(b.len());
        let mut jump: Option<(usize, f64)> = None;
        for k in 0..n {
            let delta = if a[k] >= b[k] {
                a[k] - b[k]
            } else {
                b[k] - a[k]
            };
            let ms = delta.as_secs_f64() * 1000.0;
            if ms > 10.0 {
                jump = Some((k, ms));
                break;
            }
        }
        jump
    };
    println!(
        "[group-sync][diag] A 写入 {} 次（首个音频 #{:?}）· B 写入 {} 次（首个音频 #{:?}）· 首次偏离>10ms：{:?}",
        a.len(),
        first_audio_a,
        b.len(),
        first_audio_b,
        first_jump
    );
    println!(
        "[group-sync] 绝对偏差（含起播差）：样本 {samples} · P50 {absolute_p50:.2} ms · P95 {absolute_p95:.2} ms"
    );
    println!(
        "[group-sync] 扣掉起播差后（偏移 {shift} 帧）：样本 {aligned_samples} · P50 {aligned_p50:.2} ms · P95 {aligned_p95:.2} ms"
    );
    // 断言现在直接盯 M3 的验收线（absolute，含起播差）。
    //
    // 曾经它只能断言扣掉起播差之后的 `aligned`：当时起播会偶发差一整帧（5 次里 2 次，19.7 ms），
    // 一条会随机变红的断言只会被忽略。2026-09-17 前后修了两轮：第一轮修判定精度与拍点相位；
    // 第二轮（第三层，见 `docs/49-m3-group-start-phase.md`）才发现真正的问题是**参考系** ——
    // 拍点锚在全局帧网格、target 又取整到同一网格，两端时钟估计只差 ε 就会落到相邻两格，
    // 于是结果只有「0 或整整一帧」两种。改成「拍点相位由该帧 target 算出」
    // （`EpochSchedule::phase_us` + runtime 的 `playout_start_anchor`）之后，绝对偏差实测
    // P50 0.00 ms / P95 0.02 ms。所以护栏按验收线立：**这才是它该盯的东西**。
    assert!(
        absolute_p95 <= 10.0,
        "组内偏差 P95 = {absolute_p95:.2} ms 超过验收线 10 ms（两端播放同一帧的时刻差，含起播差）"
    );
    // 附带一条更严的自检：扣掉起播差后仍应当远小于一帧 —— 它若变大，说明排播本身开始抖了。
    assert!(
        aligned_p95 <= 5.0,
        "扣掉起播差后仍有 {aligned_p95:.2} ms（偏移 {shift} 帧）—— 排播抖动不该这么大"
    );

    accept_a.abort();
    accept_b.abort();
    sender.shutdown().await;
    receiver_a.shutdown().await;
    receiver_b.shutdown().await;
}
