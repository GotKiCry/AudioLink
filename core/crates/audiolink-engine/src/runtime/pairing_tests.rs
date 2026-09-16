//! 配对快照与广播背压/时钟/会话替换的回归；通过真实 Handshake/PinGate 推进状态。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::payload::PairSubmitPayload;

async fn engine(dir: &std::path::Path, name: &str) -> Arc<Engine> {
    let mut config = EngineConfig::new(name, dir.join(name));
    config.listen = "127.0.0.1:0".parse().unwrap();
    Engine::start(config).await.unwrap()
}

fn begin(
    receiver: &Engine,
    sender: &Engine,
    now: Instant,
) -> (Arc<PeerSession>, Handshake, mpsc::Receiver<SessionCommand>) {
    let (session, commands) = create_session(
        &receiver.inner,
        sender.info().id,
        sender.local_addr(),
        false,
    );
    let mut handshake = Handshake::new(Role::Responder, receiver.info(), sender.info().id, false);
    let mut initiator = Handshake::new(Role::Initiator, sender.info(), receiver.info().id, false);
    let hello = initiator.start().send.remove(0).request;
    let step = handshake.on_control(
        &hello,
        &receiver.inner.identity,
        sender.inner.identity.cert_der(),
        now,
    );
    session.update_pairing(&handshake, &step.event);
    assert!(matches!(step.event, HandshakeEvent::DisplayPin { .. }));
    (session, handshake, commands)
}

#[tokio::test]
async fn pin_survives_lost_broadcast_and_expires_without_another_event() {
    let dir = tempfile::tempdir().unwrap();
    let receiver = engine(dir.path(), "receiver").await;
    let sender = engine(dir.path(), "sender").await;
    let mut events = receiver.subscribe();
    let now = Instant::now();
    let (_session, handshake, _commands) = begin(&receiver, &sender, now);
    let pin = handshake.displayed_pin().unwrap().to_string();
    receiver
        .inner
        .events
        .send(EngineEvent::DisplayPin {
            from: sender.info().id,
            name: "sender".into(),
            pin: pin.clone(),
            remaining_attempts: 5,
        })
        .unwrap();
    // 定向挤掉一次性的 DisplayPin，确认真的是广播 Lagged，而非只模拟丢事件。
    for _ in 0..512 {
        receiver
            .inner
            .events
            .send(EngineEvent::Telemetry(Box::default()))
            .unwrap();
    }
    assert!(matches!(
        events.try_recv(),
        Err(broadcast::error::TryRecvError::Lagged(_))
    ));
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, EngineEvent::DisplayPin { .. }));
    }
    assert_eq!(receiver.displayed_pin().as_deref(), Some(pin.as_str()));
    let mut late = receiver.subscribe();
    assert!(matches!(
        late.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    assert_eq!(
        receiver.displayed_pin_at(now + PIN_TTL - Duration::from_nanos(1)),
        Some(pin)
    );
    assert!(receiver.displayed_pin_at(now + PIN_TTL).is_none());
    receiver.shutdown().await;
    sender.shutdown().await;
}

#[tokio::test]
async fn wrong_pin_keeps_original_deadline_and_lockout_clears_it() {
    let dir = tempfile::tempdir().unwrap();
    let receiver = engine(dir.path(), "receiver").await;
    let sender = engine(dir.path(), "sender").await;
    let now = Instant::now();
    let (session, mut handshake, _commands) = begin(&receiver, &sender, now);
    let pin = receiver.displayed_pin().unwrap();
    let wrong = if pin == "000000" { "000001" } else { "000000" };
    for attempt in 1..=5 {
        let step = handshake.on_control(
            &ControlRequest::PairSubmit(PairSubmitPayload { pin: wrong.into() }),
            &receiver.inner.identity,
            sender.inner.identity.cert_der(),
            now + Duration::from_secs(30),
        );
        session.update_pairing(&handshake, &step.event);
        if attempt < 5 {
            assert_eq!(receiver.displayed_pin().as_deref(), Some(pin.as_str()));
            assert!(
                receiver.displayed_pin_at(now + PIN_TTL).is_none(),
                "输错 PIN 不应续期"
            );
        } else {
            assert!(matches!(step.event, HandshakeEvent::Rejected { .. }));
            assert!(receiver.displayed_pin().is_none(), "锁定立即隐藏 PIN");
        }
    }
    receiver.shutdown().await;
    sender.shutdown().await;
}

#[tokio::test]
async fn disconnect_and_old_session_cleanup_preserve_other_pairings() {
    let dir = tempfile::tempdir().unwrap();
    let receiver = engine(dir.path(), "receiver").await;
    let first = engine(dir.path(), "first").await;
    let second = engine(dir.path(), "second").await;
    let now = Instant::now();
    let (old, _, mut old_commands) = begin(&receiver, &first, now);
    let (other, other_handshake, _other_commands) =
        begin(&receiver, &second, now + Duration::from_secs(1));
    let (current, handshake, _commands) = begin(&receiver, &first, now + Duration::from_secs(2));
    let pin = handshake.displayed_pin().unwrap().to_string();
    assert!(matches!(
        old_commands.recv().await,
        Some(SessionCommand::Shutdown)
    ));
    let mut events = receiver.subscribe();
    drop_session(&receiver.inner, &old, "old connection closed");
    assert!(matches!(
        events.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    assert_eq!(receiver.peers().len(), 2);
    assert_eq!(receiver.displayed_pin().as_deref(), Some(pin.as_str()));
    drop_session(&receiver.inner, &current, "current connection closed");
    assert_eq!(
        receiver.displayed_pin().as_deref(),
        other_handshake.displayed_pin()
    );
    drop_session(&receiver.inner, &other, "other connection closed");
    assert!(receiver.displayed_pin().is_none());
    receiver.shutdown().await;
    first.shutdown().await;
    second.shutdown().await;
}
