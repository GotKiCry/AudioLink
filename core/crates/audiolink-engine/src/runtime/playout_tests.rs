#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn frame(seq: u32) -> PlaybackFrame {
    PlaybackFrame {
        seq,
        samples: vec![seq as f32],
    }
}

fn telemetry() -> Arc<Mutex<TelemetryAggregator>> {
    Arc::new(Mutex::new(TelemetryAggregator::new(
        1,
        CodecConfig::m1_default().telemetry(),
    )))
}

#[test]
fn missing_tick_advances_cursor_and_late_frame_is_never_played() {
    let (tx, rx) = crossbeam_channel::bounded(4);
    let stats = telemetry();
    let mut pending = None;
    let mut expected = Some(7);
    tx.send(frame(8)).unwrap();
    tx.send(frame(7)).unwrap();
    tx.send(frame(9)).unwrap();

    assert!(matches!(
        take_due_frame(&rx, &mut pending, &mut expected, &stats),
        DueFrame::Missing
    ));
    assert_eq!(expected, Some(8));
    assert!(matches!(
        take_due_frame(&rx, &mut pending, &mut expected, &stats),
        DueFrame::Ready(PlaybackFrame { seq: 8, .. })
    ));
    assert!(matches!(
        take_due_frame(&rx, &mut pending, &mut expected, &stats),
        DueFrame::Ready(PlaybackFrame { seq: 9, .. })
    ));
    assert_eq!(expected, Some(10));
    assert_eq!(stats.lock().unwrap().snapshot().late_drops, 1);
}

#[test]
fn sequence_cursor_and_stale_detection_cross_u32_wrap() {
    let (tx, rx) = crossbeam_channel::bounded(4);
    let stats = telemetry();
    let mut pending = None;
    let mut expected = Some(u32::MAX);
    tx.send(frame(u32::MAX)).unwrap();
    tx.send(frame(0)).unwrap();
    tx.send(frame(u32::MAX)).unwrap();
    tx.send(frame(1)).unwrap();

    for wanted in [u32::MAX, 0, 1] {
        match take_due_frame(&rx, &mut pending, &mut expected, &stats) {
            DueFrame::Ready(frame) => assert_eq!(frame.seq, wanted),
            DueFrame::Missing | DueFrame::Disconnected => panic!("应播放序号 {wanted}"),
        }
    }
    assert_eq!(expected, Some(2));
    assert_eq!(stats.lock().unwrap().snapshot().late_drops, 1);
}
