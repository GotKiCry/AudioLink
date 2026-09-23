//! M1 延迟账本里**最大的一段**：对端播放环水位（接收侧待播队列深度）。
//!
//! `docs/12-m1-device-acceptance.md` §3 的账本把这一段量成 **[实测] 80 ms**（对端 1 Hz
//! `STREAM_STATS` 的 `buffer_level_us` P50）；§8.2 更把它记成 P0：水位以 ≈0.5%/s（≈5500 ppm，
//! 远超晶振 ±50 ppm）稳定上涨，涨到队列容量上限后开始「迟到就丢」→ 听感是周期性卡顿。
//! 那个上涨的根因（`receive_audio` 把解码样本数再乘一次声道数，每包多出 20 ms 静音供给）
//! 已在 `docs/12` §9 修掉，修复后本机 60 s 的水位稳定在 **40 ms**。
//!
//! 但**没有任何自动化在守它** —— 水位的两端是延迟与稳定性：压水位会欠载，放任堆积则周期性丢帧。
//! 这条护栏只做一件事：在真实 QUIC + 真实 Opus 的回环链路上，把「水位**不随时间堆积**」钉住。
//!
//! 两条纪律：
//! 1. **不 import 被测实现的常量**（`DEFAULT_TARGET_FRAMES` / `PLAYBACK_QUEUE_FRAMES` /
//!    `MIN|MAX_TARGET_FRAMES`）。上限用**独立算式**写死：20 ms 帧 × 4 帧 = 80 ms —— 正好是真机
//!    账本里的那个 80 ms。拿共享常量当断言，常量一改断言就跟着走（第 107 轮的教训）。
//! 2. 判据是**前后两半的对照**（后半段不得比前半段高出一帧以上），而不是一个绝对值 ——
//!    堆积是**趋势**，单点快照抓不住它（真机那次就是从 20 ms 一路爬到 260 ms）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use audiolink_audio::{NullPlayout, PlayoutSink, SyntheticCapture};
use audiolink_engine::{Engine, EngineConfig, SessionState};

/// 中位数（就地排序后取中点；样本数是几十个，不值得上分位库）。
fn median(values: &mut [u32]) -> u32 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[values.len() / 2]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn playout_watermark_does_not_back_up_over_a_short_run() {
    const FRAME_MS: u32 = 20;
    /// 观测 20 s：1 Hz 遥测能给出约 20 个水位样本，足够看趋势。
    const RUN: Duration = Duration::from_secs(20);
    /// 水位上限（µs）：真机账本里这一段就是 80 ms（`docs/12` §3）。
    /// **独立算式**：20 ms 帧 × 4 帧 = 80 ms（不 import 任何被测常量）。
    const MAX_WATERMARK_US: u32 = 80_000;
    /// 「后半段比前半段高出这么多就算堆积」：一帧（20 ms）的余量，够吸收采样噪声。
    const BACKLOG_TOLERANCE_US: u32 = FRAME_MS * 1_000;

    let dir = tempfile::TempDir::new().expect("临时目录");
    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = FRAME_MS;
    send_config.capture = Some(Arc::new(|| {
        Ok(Box::new(SyntheticCapture::new(FRAME_MS, 440.0)?))
    }));
    let sender = Engine::start(send_config).await.expect("发送引擎");

    let mut recv_config = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_config.listen = "127.0.0.1:0".parse().unwrap();
    recv_config.codec.frame_ms = FRAME_MS;
    recv_config.playout = Some(Arc::new(|| {
        Ok(Box::new(NullPlayout::new(60)) as Box<dyn PlayoutSink>)
    }));
    let receiver = Engine::start(recv_config).await.expect("接收引擎");
    let accept = receiver.spawn_accept_loop();
    let receiver_id = receiver.info().id;
    let sender_id = sender.info().id;

    let outcome = tokio::time::timeout(Duration::from_secs(90), async {
        sender
            .connect(receiver.local_addr())
            .await
            .expect("连接应当直接成功（无认证）");
        while !sender
            .peers()
            .iter()
            .any(|peer| peer.id == receiver_id && peer.state == SessionState::Streaming)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        sender.start_send(receiver_id).await.expect("开流");

        // 丢掉起播阶段（攒帧、抖动缓冲填满前的水位不代表稳态）。
        tokio::time::sleep(Duration::from_secs(3)).await;

        // 丢弃 / 欠载计数是**累计值**：先记基线，最后取增量 —— 否则握手期的抖动也会算进来。
        let base = receiver.telemetry(sender_id).expect("会话仍在，遥测应在");
        let deadline = Instant::now() + RUN;
        let mut samples: Vec<u32> = Vec::new();
        while Instant::now() < deadline {
            // 接收端**自己视角**的待播队列水位（`depth_frames × 帧长`），就是账本里那一段。
            if let Some(stats) = receiver.telemetry(sender_id) {
                samples.push(stats.buffer_level_us);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let last = receiver.telemetry(sender_id).expect("会话仍在，遥测应在");
        let late_delta = last.late_drops.saturating_sub(base.late_drops);
        let underrun_delta = last.underruns.saturating_sub(base.underruns);
        (samples, late_delta, underrun_delta)
    })
    .await;

    let (samples, late_delta, underrun_delta) =
        outcome.expect("90 s 内必须完成连接、开流与 20 s 水位采样");

    sender.shutdown().await;
    accept.abort();
    receiver.shutdown().await;

    assert!(
        samples.len() >= 20,
        "水位样本只有 {} 个：1 Hz 遥测在 20 s 里应当给出约 20 个（遥测没更新？）",
        samples.len()
    );
    let half = samples.len() / 2;
    let mut early_half = samples[..half].to_vec();
    let mut late_half = samples[half..].to_vec();
    let early = median(&mut early_half);
    let late = median(&mut late_half);
    let peak = samples.iter().copied().max().unwrap_or(0);
    let planned_frames = u32::try_from(RUN.as_millis() / u128::from(FRAME_MS)).unwrap_or(0);
    println!(
        "[watermark] 对端播放环水位（本机回环 · {FRAME_MS} ms 帧 · 20 s）：前半段 P50 {early} µs · 后半段 P50 {late} µs · 峰值 {peak} µs；窗口内增量：迟到丢弃 {late_delta} 拍 · 供给欠载 {underrun_delta} 拍（计划 {planned_frames} 帧，1% 上限）"
    );

    // ① 稳态水位不得越过账本里那一段（80 ms）。
    assert!(
        early <= MAX_WATERMARK_US && late <= MAX_WATERMARK_US,
        "水位 P50 越过 80 ms（前半 {early} µs / 后半 {late} µs）—— 那已经吃掉 M1 预算的大半"
    );
    // ② 真正的回归判据：**不得随时间堆积**（真机那次是 20 → 260 ms 的单向爬升）。
    assert!(
        late <= early.saturating_add(BACKLOG_TOLERANCE_US),
        "水位在后半段比前半段高出一帧以上（{early} µs → {late} µs，峰值 {peak} µs）—— 正在堆积：\
         这正是 docs/12 §8.2 那个 P0 的形状（收多于播，最终顶到队列上限后周期性丢帧）"
    );
    // ③ 堆积的另一面：窗口内不该出现**成片**丢弃。
    //
    // 为什么不是「绝对零」：这条测试跑在 50 条 engine 测试并发 + 常态 soak 的负载下，回环也会
    // 因为 CPU 抢占而偶尔补一拍静音（本轮实测：全量并发时红过一次，单独跑与再次全量都绿）。
    // 绝对零容忍会把它变成噪声源（与 group_sync* 那类假红同族）；1% 与仓库既有口径一致
    // （弱网 soak 档就是「掩盖帧 ≤ 总帧数的 1%」）。**水位上涨不归这一条管** —— 那是 ①② 的职责。
    assert!(
        late_delta * 100 <= planned_frames,
        "窗口内迟到丢弃 {late_delta} 拍，超过计划帧数 {planned_frames} 的 1% —— 不是负载抖动，是成片丢弃"
    );
    assert!(
        underrun_delta * 100 <= planned_frames,
        "窗口内供给欠载 {underrun_delta} 拍，超过计划帧数 {planned_frames} 的 1% —— 不是负载抖动，是成片静音"
    );
}
