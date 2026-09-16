#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn encoded_packets(count: usize) -> Vec<Vec<u8>> {
    let config = CodecConfig::m1_default();
    let mut encoder = OpusEncoder::new(config).unwrap();
    let mut phase = 0.0f64;
    let step = 440.0 / 48_000.0;
    let mut packet = vec![0u8; audiolink_types::DATAGRAM_MAX_PAYLOAD];
    (0..count)
        .map(|_| {
            let mut pcm = Vec::with_capacity(config.interleaved_frame());
            for _ in 0..config.frame_samples() {
                let value = (phase * std::f64::consts::TAU).sin() as f32 * 0.4;
                phase = (phase + step) % 1.0;
                pcm.extend_from_slice(&[value, value]);
            }
            let written = encoder.encode_into(&pcm, &mut packet).unwrap();
            packet[..written].to_vec()
        })
        .collect()
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|value| value * value).sum::<f32>() / samples.len() as f32).sqrt()
}

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

#[test]
fn packet_gap_emits_non_silent_concealment_before_recovery_frame() {
    let packets = encoded_packets(4);
    let mut receiver = AudioReceiver::new(CodecConfig::m1_default()).unwrap();
    let mut output = Vec::new();
    let first = receiver.receive(0, &packets[0], |frame| {
        output.push(frame);
        true
    });
    let recovered = receiver.receive(2, &packets[2], |frame| {
        output.push(frame);
        true
    });

    assert_eq!(
        first,
        ReceiveReport {
            expected: 1,
            ..ReceiveReport::default()
        }
    );
    assert_eq!(
        recovered,
        ReceiveReport {
            expected: 2,
            lost: 1,
            plc: 1,
            late_drops: 0,
        }
    );
    assert_eq!(
        output.iter().map(|frame| frame.seq).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert!(rms(&output[1].samples) > 0.02, "CELT 丢包不得退化为硬静音");
    assert!(output[2].samples.iter().all(|sample| sample.is_finite()));

    let before = output.len();
    let stale = receiver.receive(1, &packets[1], |frame| {
        output.push(frame);
        true
    });
    assert_eq!(stale.late_drops, 1);
    assert_eq!(stale.expected, 0);
    assert_eq!(output.len(), before, "迟到包不得再次扰动解码器或播放时间轴");
}

#[test]
fn decode_failure_uses_pcm_concealment_and_sequence_wrap_remains_contiguous() {
    let packets = encoded_packets(2);
    let mut receiver = AudioReceiver::new(CodecConfig::m1_default()).unwrap();
    let mut output = Vec::new();
    receiver.receive(u32::MAX, &packets[0], |frame| {
        output.push(frame);
        true
    });
    let failed = receiver.receive(0, &[], |frame| {
        output.push(frame);
        true
    });

    assert_eq!(failed.expected, 1);
    assert_eq!(failed.lost, 0);
    assert_eq!(failed.plc, 1);
    assert_eq!(
        output.iter().map(|frame| frame.seq).collect::<Vec<_>>(),
        [u32::MAX, 0]
    );
    assert!(rms(&output[1].samples) > 0.02);
}

#[test]
fn long_gap_is_bounded_and_does_not_fill_the_playback_queue_with_old_frames() {
    let packets = encoded_packets(2);
    let mut receiver = AudioReceiver::new(CodecConfig::m1_default()).unwrap();
    let mut output = Vec::new();
    receiver.receive(0, &packets[0], |frame| {
        output.push(frame);
        true
    });
    let report = receiver.receive(1_000, &packets[1], |frame| {
        output.push(frame);
        true
    });

    assert_eq!(report.expected, 1_000);
    assert_eq!(report.lost, 999);
    assert_eq!(report.plc, 6);
    assert_eq!(
        output.iter().map(|frame| frame.seq).collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5, 6, 1_000]
    );
}

#[test]
fn two_percent_deterministic_loss_has_no_hard_silence_or_sequence_hole() {
    const PACKETS: usize = 201;
    const DROPPED: [usize; 4] = [17, 63, 119, 157];
    let packets = encoded_packets(PACKETS);
    let mut receiver = AudioReceiver::new(CodecConfig::m1_default()).unwrap();
    let mut output = Vec::new();
    let mut total = ReceiveReport::default();
    for (seq, packet) in packets.iter().enumerate() {
        if DROPPED.contains(&seq) {
            continue;
        }
        let report = receiver.receive(seq as u32, packet, |frame| {
            output.push(frame);
            true
        });
        total.expected = total.expected.saturating_add(report.expected);
        total.lost = total.lost.saturating_add(report.lost);
        total.plc = total.plc.saturating_add(report.plc);
        total.late_drops = total.late_drops.saturating_add(report.late_drops);
    }

    assert_eq!(total.expected, PACKETS as u32);
    assert_eq!(total.lost, DROPPED.len() as u32);
    assert_eq!(total.plc, DROPPED.len() as u32);
    assert_eq!(total.late_drops, 0);
    assert_eq!(output.len(), PACKETS);
    assert!(
        output
            .iter()
            .enumerate()
            .all(|(seq, frame)| frame.seq == seq as u32)
    );
    for seq in DROPPED {
        assert!(
            rms(&output[seq].samples) > 0.02,
            "丢失序号 {seq} 的自建掩盖不应是硬静音"
        );
    }
}

#[test]
fn six_frame_burst_fades_instead_of_becoming_hard_silence() {
    let packets = encoded_packets(8);
    let mut receiver = AudioReceiver::new(CodecConfig::m1_default()).unwrap();
    let mut output = Vec::new();
    receiver.receive(0, &packets[0], |frame| {
        output.push(frame);
        true
    });
    let report = receiver.receive(7, &packets[7], |frame| {
        output.push(frame);
        true
    });

    assert_eq!(report.lost, 6);
    assert_eq!(report.plc, 6);
    assert_eq!(output.len(), 8);
    let levels = output[1..7]
        .iter()
        .map(|frame| rms(&frame.samples))
        .collect::<Vec<_>>();
    assert!(levels[0] > 0.1, "首个掩盖帧应保留主体能量：{levels:?}");
    assert!(levels.windows(2).all(|pair| pair[1] <= pair[0]));
    assert!(
        levels[5] > 0.001,
        "120 ms 内应淡出而不是提前硬切静音：{levels:?}"
    );
    let recovered = rms(&output[7].samples);
    assert!(recovered > 0.02, "恢复帧必须回到真实音频：rms={recovered}");
}
