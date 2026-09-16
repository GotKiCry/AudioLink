//! 真实 QUIC/Opus 管线：播放线程短暂停顿不能变成余下整段音频的固定延迟。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use audiolink_audio::{
    AudioError, DeviceFormat, NullPlayout, PlayoutSink, PlayoutStats, SyntheticCapture,
};
use audiolink_engine::{Engine, EngineConfig, EngineEvent, MeasurementTap, SessionState};
use tokio::sync::mpsc;

struct PausingSink {
    inner: NullPlayout,
    writes: usize,
    events: mpsc::UnboundedSender<usize>,
}

impl PlayoutSink for PausingSink {
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
        self.writes += 1;
        if self.writes == 30 {
            // 模拟 OS 抢占/设备提交卡顿。输入端继续按采样时钟送包。
            std::thread::sleep(Duration::from_millis(250));
        }
        self.inner.write(samples)?;
        let _ = self.events.send(self.writes);
        Ok(())
    }
    fn stats(&self) -> PlayoutStats {
        self.inner.stats()
    }
    fn stop(&mut self) {
        self.inner.stop();
    }
    fn backend_name(&self) -> &'static str {
        "pausing-null"
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn receiver_pause_does_not_leave_permanent_playout_delay() {
    let dir = tempfile::tempdir().unwrap();
    let tap = Arc::new(MeasurementTap::default());
    let mut send_cfg = EngineConfig::new("sender", dir.path().join("sender"));
    send_cfg.listen = "127.0.0.1:0".parse().unwrap();
    send_cfg.capture = Some(Arc::new(|| Ok(Box::new(SyntheticCapture::new(20, 440.0)?))));
    send_cfg.measurement = Some(Arc::clone(&tap));

    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let mut recv_cfg = EngineConfig::new("receiver", dir.path().join("receiver"));
    recv_cfg.listen = "127.0.0.1:0".parse().unwrap();
    recv_cfg.measurement = Some(Arc::clone(&tap));
    recv_cfg.playout = Some(Arc::new(move || {
        Ok(Box::new(PausingSink {
            inner: NullPlayout::new(60),
            writes: 0,
            events: events_tx.clone(),
        }))
    }));
    let sender = Engine::start(send_cfg).await.unwrap();
    let receiver = Engine::start(recv_cfg).await.unwrap();
    let accept = receiver.spawn_accept_loop();
    let mut events = receiver.subscribe();
    let peer = receiver.info().id;
    let measured = tokio::time::timeout(Duration::from_secs(10), async {
        assert!(sender.connect(receiver.local_addr()).await.is_err());
        let pin = loop {
            if let EngineEvent::DisplayPin { pin, .. } = events.recv().await.unwrap() {
                break pin;
            }
        };
        sender.submit_pin(peer, &pin).await.unwrap();
        while !sender
            .peers()
            .iter()
            .any(|p| p.id == peer && p.state == SessionState::Streaming)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        sender.start_send(peer).await.unwrap();
        while events_rx.recv().await.unwrap() < 30 {}
        tap.reset();
        while events_rx.recv().await.unwrap() < 100 {}
        tap.summary().expect("停顿后应继续播放真实音频")
    })
    .await;

    sender.shutdown().await;
    receiver.shutdown().await;
    accept.await.unwrap();
    let summary = measured.expect("10 s 内应完成配对和停顿后恢复");
    eprintln!("停顿后恢复样本：{summary:?}");
    assert!(
        summary.count >= 30,
        "恢复后必须有足够的真实音频配对样本：{summary:?}"
    );
    assert!(
        summary.p95 < 100_000,
        "250 ms 停顿不能留下永久延迟：{summary:?}"
    );
}
