//! §8 自适应码率的**降级分支**端到端回归：真实双 Engine + 真实 QUIC + 可开关的高比例丢包注入。
//!
//! # 为什么需要这条测试
//!
//! `adaptive` 模块的 8 项单测只喂**假样本**给状态机；`adaptive_bitrate` 那条端到端测试只覆盖
//! 「从 §8 下限恢复升级」。降级分支在本地一直**没有端到端证据**，那条测试的文件头也自认了原因：
//! 回环上 §8.1 的双发 + NACK 会把人造丢包修得一个洞不剩，接收侧上报的丢包率因此常是 0，
//! 而「修好了就不该降码率」是**正确行为** —— 于是降级在本地不可复现。
//!
//! # 这条测试怎么绕开它
//!
//! 注入方式不是「按比例随机丢」，而是**开关式的高比例丢**：开关打开时，
//! 发送端 → 接收端的包每 4 个只放行 1 个（≈75% 丢包）。反方向（接收端 → 发送端）**照常转发** ——
//! 这一点是必须的：自适应读的是**对端** 1 Hz 的 `STREAM_STATS`，那条上报走的就是反方向。
//!
//! **不能用「整体断流」**：如果接收端一个音频包都收不到，它就**算不出丢包**
//! （丢包率来自「收到新序号时发现的洞」，没有新帧就没有洞）—— 自适应看到的仍然是 0，
//! 降级依旧不发生。这是本轮实测出来的一个反直觉点，写在这里免得下一个人再踩。
//!
//! 于是：打开 ≈75% 丢包 → 接收侧丢包率必然 ≫ 1%（远超 §8 的 1% 阈值与 3 s 持续窗口）→ 先降一级；
//! 关掉注入 → 连续 10 s 无丢包 + 每 5 s 最多一级 → 升回去。断言「**先降、丢包缓解后升回**」。
//!
//! # 观测途径
//!
//! 只看 `EngineEvent::CodecAdapted`：它记的是**目标码率**（自适应这个部件的动作）。
//! 接收侧实测链路码率不能当动作看 —— 它含冗余双发 ×2 与重传/突发噪声（`docs/22` §13 同款口径）。
//!
//! 这条测试**不依赖机器负载**：触发条件是「丢包 > 1% 持续 3 s」，而注入后丢包率是几十个百分点，
//! 与 CPU 争用无关（对照：`docs/22` §11.6 里那条对负载敏感的静音判据）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use audiolink_audio::{NullPlayout, PlayoutSink, SyntheticCapture};
use audiolink_engine::adaptive::MAX_BITRATE_BPS;
use audiolink_engine::{Engine, EngineConfig, EngineEvent, SessionState};
use tokio::net::UdpSocket;

/// 可开关的高比例丢包中继：客户端（发送端）连它，它把字节原样转给服务端（接收端）。
///
/// 注入打开时，**客户端 → 服务端**方向每 `keep_every` 个包只放行 1 个；反方向全程放行。
struct LossyRelay {
    addr: SocketAddr,
    dropped: Arc<AtomicU64>,
    forwarded: Arc<AtomicU64>,
    injecting: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl LossyRelay {
    async fn start(upstream: SocketAddr, keep_every: u64) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("中继绑定回环端口");
        let addr = socket.local_addr().expect("中继地址");
        let socket = Arc::new(socket);
        let dropped = Arc::new(AtomicU64::new(0));
        let forwarded = Arc::new(AtomicU64::new(0));
        let injecting = Arc::new(AtomicBool::new(false));
        let (dropped_counter, forwarded_counter, switch) =
            (dropped.clone(), forwarded.clone(), injecting.clone());

        let task = tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let mut client: Option<SocketAddr> = None;
            let mut seen = 0_u64;
            while let Ok((len, from)) = socket.recv_from(&mut buf).await {
                let client_to_server = from != upstream;
                let to = if client_to_server {
                    client = Some(from);
                    upstream
                } else {
                    match client {
                        Some(client) => client,
                        None => continue, // 还不知道客户端是谁：握手前的噪声，丢掉
                    }
                };

                if client_to_server && switch.load(Ordering::Relaxed) {
                    seen += 1;
                    if !seen.is_multiple_of(keep_every.max(1)) {
                        dropped_counter.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                }
                forwarded_counter.fetch_add(1, Ordering::Relaxed);
                let _ = socket.send_to(&buf[..len], to).await;
            }
        });

        Self {
            addr,
            dropped,
            forwarded,
            injecting,
            task,
        }
    }

    /// 打开 / 关闭注入。
    fn set_injection(&self, on: bool) {
        self.injecting.store(on, Ordering::Relaxed);
    }

    fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn forwarded(&self) -> u64 {
        self.forwarded.load(Ordering::Relaxed)
    }
}

impl Drop for LossyRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// 目标码率的变更时间线（`from_bps, to_bps, reason`）。
type Timeline = Arc<Mutex<Vec<(i32, i32, String)>>>;

fn snapshot(timeline: &Timeline) -> Vec<(i32, i32, String)> {
    timeline.lock().map(|slot| slot.clone()).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn injected_loss_downgrades_the_target_and_clearing_it_recovers() {
    let dir = tempfile::TempDir::new().expect("临时目录");
    let frame_ms = 20_u32;

    let mut send_config = EngineConfig::new("sender", dir.path().join("sender"));
    send_config.listen = "127.0.0.1:0".parse().unwrap();
    send_config.codec.frame_ms = frame_ms;
    // 从 §8 的**上限**起步：上限之上没有「恢复」可走，于是第一次变更必然是降级。
    send_config.codec.bitrate_bps = MAX_BITRATE_BPS;
    send_config.capture = Some(Arc::new(move || {
        Ok(Box::new(SyntheticCapture::new(frame_ms, 440.0)?))
    }));

    let mut recv_config = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_config.listen = "127.0.0.1:0".parse().unwrap();
    recv_config.codec.frame_ms = frame_ms;
    recv_config.playout = Some(Arc::new(move || {
        Ok(Box::new(NullPlayout::new(60)) as Box<dyn PlayoutSink>)
    }));

    let receiver = Engine::start(recv_config).await.expect("接收引擎");
    let accept = receiver.spawn_accept_loop();
    let receiver_id = receiver.info().id;

    // 每 4 个包放行 1 个 ≈ 75% 丢包（双向都在，只有发送端 → 接收端那半边被丢）。
    let relay = LossyRelay::start(receiver.local_addr(), 4).await;

    let sender = Engine::start(send_config).await.expect("发送引擎");
    let sender_id = sender.info().id;
    let mut sender_events = sender.subscribe();

    let timeline: Timeline = Arc::new(Mutex::new(Vec::new()));
    let collector = {
        let sink = timeline.clone();
        tokio::spawn(async move {
            loop {
                match sender_events.recv().await {
                    Ok(EngineEvent::CodecAdapted {
                        from_bps,
                        to_bps,
                        reason,
                    }) => {
                        if let Ok(mut slot) = sink.lock() {
                            slot.push((from_bps, to_bps, reason));
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        })
    };

    let outcome = tokio::time::timeout(Duration::from_secs(90), async {
        sender
            .connect(relay.addr)
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

        // 先跑几秒健康链路：接收端的 STREAM_STATS 要到发送端，自适应才看得见对端。
        tokio::time::sleep(Duration::from_secs(3)).await;
        // 只钉「注入之前没有降级」：从上限起步，健康链路上连「恢复」都没得升（上限之上无处可去），
        // 而**降级**只可能来自真实的丢包样本 —— 这条断言因此在满载机器上也成立。
        assert!(
            snapshot(&timeline).iter().all(|(from, to, _)| to > from),
            "注入之前不该出现降级：{:?}",
            snapshot(&timeline)
        );

        // §8 的降级窗口是「丢包 > 1% 持续 3 s」：注入 9 s 够它连降两级。
        relay.set_injection(true);
        let mut peak_loss = 0_u16;
        let sampling = tokio::time::Instant::now() + Duration::from_secs(9);
        while tokio::time::Instant::now() < sampling {
            if let Some(stats) = receiver.telemetry(sender_id) {
                peak_loss = peak_loss.max(stats.loss_pct_x100);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let during = snapshot(&timeline);

        // 缓解：恢复全量转发，等恢复窗口（连续 10 s 无丢包 + 每 5 s 最多升一级）。
        relay.set_injection(false);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while tokio::time::Instant::now() < deadline {
            if snapshot(&timeline).iter().any(|(from, to, _)| to > from) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let cleared_loss = receiver
            .telemetry(sender_id)
            .map_or(0, |stats| stats.loss_pct_x100);
        (peak_loss, during, cleared_loss)
    })
    .await;

    let (peak_loss, during, cleared_loss) = outcome.expect("90 s 内必须完成连接、注入、降级与恢复");
    let timeline = snapshot(&timeline);

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.abort();
    collector.abort();

    println!(
        "注入期间接收侧峰值丢包 {peak_loss}（×100，缓解后 {cleared_loss}）· 中继丢 {} / 转 {}；目标码率时间线 {timeline:?}",
        relay.dropped(),
        relay.forwarded(),
    );

    // ① 触发条件真的成立：接收侧丢包率远超 1%（不是「双发修好了所以 0」）。
    assert!(
        peak_loss > 100,
        "注入期间接收侧丢包率必须 > 1%：实测 {peak_loss}（×100）"
    );

    // ② 先降：注入后的**第一次降级**必然是「§8 上限 −20%」。
    let (down_from, down_to, down_reason) = during
        .iter()
        .find(|(from, to, _)| to < from)
        .cloned()
        .expect("注入 9 s 后必须出现降级事件");
    assert_eq!(down_from, MAX_BITRATE_BPS, "起点是 §8 的码率上限");
    assert_eq!(down_to, MAX_BITRATE_BPS * 4 / 5, "降级步长 −20%");
    assert!(down_to < down_from, "降级：目标必须变小");
    assert!(
        down_reason.contains("丢包"),
        "原因必须写明丢包触发：{down_reason}"
    );

    // ③ 丢包缓解后升回，且**在降级之后**。
    let down_pos = timeline
        .iter()
        .position(|(from, to, _)| to < from)
        .expect("至少一次降级");
    let up_pos = timeline
        .iter()
        .position(|(from, to, _)| to > from)
        .expect("恢复转发后必须出现恢复事件");
    assert!(up_pos > down_pos, "顺序必须是先降后升：{timeline:?}");

    let last_down = timeline[..up_pos]
        .iter()
        .rfind(|(from, to, _)| to < from)
        .cloned()
        .expect("恢复之前必有降级");
    let (up_from, up_to, up_reason) = timeline[up_pos].clone();
    assert_eq!(up_from, last_down.1, "恢复必须从降级后的目标起步");
    assert_eq!(up_to, up_from * 11 / 10, "恢复步长 +10%");
    assert!(
        up_reason.contains("恢复") || up_reason.contains("无丢包"),
        "原因必须写明恢复：{up_reason}"
    );
}
