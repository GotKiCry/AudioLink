//! 同一端口连续启停，含双向音频和取消停止等待；不访问实际声卡。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig};
use audiolink_ffi::{
    EngineStartConfig, PcmFeed, PcmPull, engine_start, engine_stop, local_status, peers, start_send,
};
use audiolink_types::ErrorCode;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tokio::sync::oneshot;

#[derive(Default)]
struct Counts {
    writes: AtomicUsize,
    reads: AtomicUsize,
    feed_dropped: AtomicBool,
    pull_dropped: AtomicBool,
}

struct Feed(Arc<Counts>);
impl PcmFeed for Feed {
    fn feed_pcm(&self, _samples: Vec<f32>, frames: i32) -> i32 {
        self.0.writes.fetch_add(1, Ordering::Relaxed);
        frames
    }
}
impl Drop for Feed {
    fn drop(&mut self) {
        self.0.feed_dropped.store(true, Ordering::SeqCst);
    }
}
struct Pull(Arc<Counts>);
impl PcmPull for Pull {
    fn read_pcm(&self, max_samples: i32) -> Vec<f32> {
        std::thread::sleep(Duration::from_millis(20));
        self.0.reads.fetch_add(1, Ordering::Relaxed);
        vec![0.125; max_samples as usize]
    }
}
impl Drop for Pull {
    fn drop(&mut self) {
        self.0.pull_dropped.store(true, Ordering::SeqCst);
    }
}

struct Sink {
    inner: NullPlayout,
    writes: Arc<AtomicUsize>,
}
impl PlayoutSink for Sink {
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
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn stats(&self) -> PlayoutStats {
        self.inner.stats()
    }
    fn stop(&mut self) {
        self.inner.stop();
    }
    fn backend_name(&self) -> &'static str {
        "restart-test"
    }
}

async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("状态应在 5 s 内收敛");
}

async fn remote(dir: &std::path::Path, name: &str, writes: Arc<AtomicUsize>) -> Arc<Engine> {
    let mut config = EngineConfig::new(name, dir.join(name));
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.capture = Some(Arc::new(|| Ok(Box::new(SyntheticCapture::new(20, 440.0)?))));
    config.playout = Some(Arc::new(move || {
        Ok(Box::new(Sink {
            inner: NullPlayout::new(60),
            writes: Arc::clone(&writes),
        }))
    }));
    Engine::start(config).await.unwrap()
}

/// 让原生对端与 FFI 引擎建立连接（**连上即通**：没有 PIN 交互、没有等待输码的中间态）。
async fn pair(remote: &Arc<Engine>, addr: std::net::SocketAddr) {
    remote.connect(addr).await.expect("连接 FFI 引擎");
    until(|| !peers().unwrap().is_empty()).await;
}

struct BlockedFeed {
    entered: Mutex<Option<oneshot::Sender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
    dropped: Arc<AtomicBool>,
}
impl PcmFeed for BlockedFeed {
    fn feed_pcm(&self, _samples: Vec<f32>, frames: i32) -> i32 {
        if let Some(entered) = self.entered.lock().unwrap().take() {
            let _ = entered.send(());
            let _ = self.release.lock().unwrap().recv();
        }
        frames
    }
}
impl Drop for BlockedFeed {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ffi_restarts_on_one_port_and_finishes_cancelled_stop() {
    tokio::time::timeout(Duration::from_secs(40), check_restarts())
        .await
        .unwrap();
}

async fn check_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let port = UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = EngineStartConfig {
        node_name: "phone".into(),
        data_dir: dir.path().join("phone").to_string_lossy().into(),
        listen_port: port,
        // 这条用例不关心平台能力位；显式 0 = 与加这个字段之前逐位一致。
        capabilities: 0,
        low_latency: false,
    };
    let bind_addr = SocketAddr::from(([0, 0, 0, 0], port));
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    // 冷启动也须串行：同配置双方成功，不同配置只报 BUSY，不出现端口绑定竞态。
    let (first, same) = tokio::join!(
        engine_start(config.clone(), None, None),
        engine_start(config.clone(), None, None)
    );
    assert_eq!(first.unwrap(), same.unwrap());
    engine_stop().await.unwrap();
    let (first, conflict) = tokio::join!(
        engine_start(config.clone(), None, None),
        engine_start(
            EngineStartConfig {
                node_name: "conflict".into(),
                ..config.clone()
            },
            None,
            None
        )
    );
    assert!(first.is_ok());
    assert_eq!(conflict.unwrap_err().code(), ErrorCode::Busy.as_u16());
    engine_stop().await.unwrap();
    // 三轮，每轮覆盖空闲、已连接、接收中、发送中，共 12 次同端口重启。
    for round in 0..3 {
        for phase in 0..4 {
            let counts = Arc::new(Counts::default());
            let local = engine_start(
                config.clone(),
                Some(Box::new(Feed(counts.clone()))),
                Some(Box::new(Pull(counts.clone()))),
            )
            .await
            .unwrap();
            let (same, conflict) = tokio::join!(
                engine_start(config.clone(), None, None),
                engine_start(
                    EngineStartConfig {
                        node_name: "conflict".into(),
                        ..config.clone()
                    },
                    None,
                    None
                )
            );
            assert_eq!(same.unwrap(), local);
            assert_eq!(conflict.unwrap_err().code(), ErrorCode::Busy.as_u16());
            let writes = Arc::new(AtomicUsize::new(0));
            let remote = remote(
                dir.path(),
                &format!("remote-{round}-{phase}"),
                writes.clone(),
            )
            .await;
            if phase > 0 {
                pair(&remote, addr).await;
                if phase == 2 {
                    let id = audiolink_types::NodeId::from_hex(&local.id_hex).unwrap();
                    remote.start_send(id).await.unwrap();
                    until(|| counts.writes.load(Ordering::Relaxed) >= 3).await;
                } else if phase == 3 {
                    start_send().await.unwrap();
                    until(|| writes.load(Ordering::Relaxed) >= 3).await;
                }
            }
            let (first, second) = tokio::join!(engine_stop(), engine_stop());
            first.unwrap();
            second.unwrap();
            assert!(local_status().is_err());
            assert!(counts.feed_dropped.load(Ordering::SeqCst));
            assert!(counts.pull_dropped.load(Ordering::SeqCst));
            drop(UdpSocket::bind(bind_addr).expect("停止返回后立即由 OS 重绑同一端口"));
            remote.shutdown().await;
        }
    }

    // 把真实播放回调停在中途：取消等待 engineStop 的协程，随后 start 仍必须等旧回调退出。
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let dropped = Arc::new(AtomicBool::new(false));
    let local = engine_start(
        config.clone(),
        Some(Box::new(BlockedFeed {
            entered: Mutex::new(Some(entered_tx)),
            release: Mutex::new(release_rx),
            dropped: dropped.clone(),
        })),
        None,
    )
    .await
    .unwrap();
    let remote = remote(dir.path(), "blocked", Arc::new(AtomicUsize::new(0))).await;
    pair(&remote, addr).await;
    remote
        .start_send(audiolink_types::NodeId::from_hex(&local.id_hex).unwrap())
        .await
        .unwrap();
    entered_rx.await.unwrap();
    let stop = tokio::spawn(engine_stop());
    until(|| local_status().is_err()).await;
    assert!(!stop.is_finished());
    stop.abort();
    let _ = stop.await;
    let mut restart = tokio::spawn(engine_start(config.clone(), None, None));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut restart)
            .await
            .is_err()
    );
    assert!(!dropped.load(Ordering::SeqCst));
    release_tx.send(()).unwrap();
    restart.await.unwrap().unwrap();
    assert!(
        dropped.load(Ordering::SeqCst),
        "新启动不能抢在旧回调退出之前"
    );
    assert!(peers().unwrap().is_empty());
    engine_stop().await.unwrap();
    drop(UdpSocket::bind(bind_addr).unwrap());
    remote.shutdown().await;
}
