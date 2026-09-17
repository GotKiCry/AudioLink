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

fn deliver_reorder_batch(
    receiver: &mut AudioReceiver,
    batch: ReorderBatch,
    output: &mut Vec<PlaybackFrame>,
) -> ReceiveReport {
    let mut total = ReceiveReport {
        late_drops: batch.late_drops,
        ..ReceiveReport::default()
    };
    for EncodedAudioPacket { seq, payload, .. } in batch.ready {
        let report = receiver.receive(seq, &payload, |frame| {
            output.push(frame);
            true
        });
        total.expected = total.expected.saturating_add(report.expected);
        total.lost = total.lost.saturating_add(report.lost);
        total.plc = total.plc.saturating_add(report.plc);
        total.late_drops = total.late_drops.saturating_add(report.late_drops);
    }
    total
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
fn depth_downgrade_records_active_drops_not_late_drops() {
    // 第 85 轮：降档那一拍（PlayoutDepthAction::DropOldest）是**控制器的主动策略选择**，
    // 不是「帧到得太晚」。它必须记 depth_drops，且不得再污染 late_drops ——
    // 否则严格档会把一个每次降档必现的设计代价判成质量违规。
    let (tx, rx) = crossbeam_channel::bounded(4);
    let stats = telemetry();
    let mut pending = None;
    let mut expected = Some(0);
    tx.send(frame(0)).unwrap();
    tx.send(frame(1)).unwrap();

    assert!(
        drop_oldest_due_frames(&rx, &mut pending, &mut expected, &stats, 1),
        "队列没断，应当正常走完丢弃"
    );
    assert_eq!(expected, Some(1), "丢一拍必须推进播放游标");

    let aggregator = stats.lock().unwrap();
    assert_eq!(aggregator.depth_drops(), 1, "降档主动丢帧记 depth_drops");
    assert_eq!(
        aggregator.snapshot().late_drops,
        0,
        "主动丢帧不得再记 late_drops（late_drops 的零容忍留给真迟到）"
    );
}

#[test]
fn truly_late_frame_still_records_late_drops() {
    // 反向必须成立：序号已经落后于播放游标的帧是真迟到（网络 / 调度把它拖过了播放拍），
    // 即使它出现在降档那一拍的丢弃循环里，也仍然记 late_drops —— 既有语义一字未改。
    let (tx, rx) = crossbeam_channel::bounded(4);
    let stats = telemetry();
    let mut pending = None;
    let mut expected = Some(7);
    tx.send(frame(6)).unwrap();

    assert!(drop_oldest_due_frames(
        &rx,
        &mut pending,
        &mut expected,
        &stats,
        1
    ));

    let aggregator = stats.lock().unwrap();
    assert_eq!(
        aggregator.snapshot().late_drops,
        1,
        "真迟到仍记 late_drops：这是网络 / 调度质量问题，严格档零容忍不变"
    );
    assert_eq!(
        aggregator.depth_drops(),
        1,
        "同一拍里控制器请求的这次丢弃另记 depth_drops"
    );
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

#[test]
fn bounded_reordering_repairs_packet_order_before_opus_decode() {
    let packets = encoded_packets(3);
    let now = Instant::now();
    let mut reorder = PacketReorderBuffer::new(Duration::from_millis(20));
    reorder.set_target_frames(3);
    let mut receiver = AudioReceiver::new(CodecConfig::m1_default()).unwrap();
    let mut output = Vec::new();
    let mut total = ReceiveReport::default();

    for (seq, at) in [(0usize, 0u64), (2, 20), (1, 25)] {
        let report = deliver_reorder_batch(
            &mut receiver,
            reorder.push(
                seq as u32,
                packets[seq].clone(),
                now + Duration::from_millis(at),
            ),
            &mut output,
        );
        total.expected = total.expected.saturating_add(report.expected);
        total.lost = total.lost.saturating_add(report.lost);
        total.plc = total.plc.saturating_add(report.plc);
        total.late_drops = total.late_drops.saturating_add(report.late_drops);
    }

    assert_eq!(total.lost, 0);
    assert_eq!(total.plc, 0);
    assert_eq!(total.late_drops, 0);
    assert_eq!(
        output.iter().map(|frame| frame.seq).collect::<Vec<_>>(),
        [0, 1, 2]
    );
}

#[test]
fn redundant_copy_recovers_a_missing_primary_without_duplicate_decode() {
    let packets = encoded_packets(4);
    let now = Instant::now();
    let mut reorder = PacketReorderBuffer::new(Duration::from_millis(20));
    reorder.set_target_frames(3);
    let mut receiver = AudioReceiver::new(CodecConfig::m1_default()).unwrap();
    let mut output = Vec::new();
    let mut total = ReceiveReport::default();

    // 主包 1 丢失；它的延迟副本在主包 2 之后到达。其他冗余副本必须静默去重。
    for (seq, at) in [(0usize, 0u64), (0, 20), (2, 40), (1, 41), (3, 60), (2, 61)] {
        let report = deliver_reorder_batch(
            &mut receiver,
            reorder.push(
                seq as u32,
                packets[seq].clone(),
                now + Duration::from_millis(at),
            ),
            &mut output,
        );
        total.expected = total.expected.saturating_add(report.expected);
        total.lost = total.lost.saturating_add(report.lost);
        total.plc = total.plc.saturating_add(report.plc);
        total.late_drops = total.late_drops.saturating_add(report.late_drops);
    }

    assert_eq!(total.expected, 4);
    assert_eq!(total.lost, 0);
    assert_eq!(total.plc, 0);
    assert_eq!(total.late_drops, 0);
    assert_eq!(
        output.iter().map(|frame| frame.seq).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
}

#[test]
fn reorder_deadline_falls_back_to_plc_and_rejects_the_late_packet() {
    let packets = encoded_packets(3);
    let now = Instant::now();
    let mut reorder = PacketReorderBuffer::new(Duration::from_millis(20));
    reorder.set_target_frames(3);
    let mut receiver = AudioReceiver::new(CodecConfig::m1_default()).unwrap();
    let mut output = Vec::new();

    deliver_reorder_batch(
        &mut receiver,
        reorder.push(0, packets[0].clone(), now),
        &mut output,
    );
    assert!(
        reorder
            .push(2, packets[2].clone(), now + Duration::from_millis(20))
            .ready
            .is_empty()
    );
    let expired = deliver_reorder_batch(
        &mut receiver,
        reorder.flush_expired(now + Duration::from_millis(40)),
        &mut output,
    );
    let late = reorder.push(1, packets[1].clone(), now + Duration::from_millis(41));

    assert_eq!(expired.lost, 1);
    assert_eq!(expired.plc, 1);
    assert_eq!(late.late_drops, 1);
    assert_eq!(
        output.iter().map(|frame| frame.seq).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert!(rms(&output[1].samples) > 0.02);
}
// ---------------------------------------------------------------------------
// §7 预约播放（epoch 驱动排播）
// ---------------------------------------------------------------------------

#[test]
fn epoch_schedule_maps_sequence_numbers_to_sample_indices() {
    let sync = PlayoutSync {
        schedule: None,
        offset_us: 0,
        frame_samples: 960,
        base: Some((50, 4_800_000)),
    };
    assert_eq!(sync.sample_index_of(50), Some(4_800_000));
    assert_eq!(sync.sample_index_of(51), Some(4_800_960));
    // 早于基准（重传 / 重复副本）也要给得出答案，而不是 panic 或 None 逃逸
    assert_eq!(sync.sample_index_of(49), Some(4_799_040));
    // 没有基准就没有答案：调用方据此继续走本地游标排播
    assert_eq!(PlayoutSync::default().sample_index_of(1), None);
}

#[test]
fn peek_frame_does_not_consume_the_frame() {
    let (tx, rx) = crossbeam_channel::bounded(2);
    let mut pending = None;
    tx.send(frame(3)).unwrap();
    assert_eq!(peek_frame(&rx, &mut pending).map(|f| f.seq), Some(3));
    // 第二次 peek 还是同一帧（它已经躺在 pending 里），并且队列没有被二次消费
    assert_eq!(peek_frame(&rx, &mut pending).map(|f| f.seq), Some(3));
    assert!(rx.try_recv().is_err());
}

#[test]
fn epoch_schedule_gates_playout_by_the_target_time() {
    let sync: PlayoutSyncHandle = Arc::new(Mutex::new(PlayoutSync {
        schedule: None,
        offset_us: 0,
        frame_samples: 960,
        base: Some((0, 0)),
    }));

    // 没有组基准 → 不做判定（M1 / M2 的本地游标排播原样保留）
    assert!(schedule_action(&sync, Some(&frame(0))).is_none());

    let now = i64::try_from(now_monotonic_us()).unwrap();
    // 基准取「现在 + 10 s」以保证是正数：协议里 epoch_local_us 是 u64，表示不了过去
    let anchor = u64::try_from(now).unwrap().saturating_add(10_000_000);
    // 用 offset 制造「过去 / 现在」：epoch_local_us 是 u64，表示不了过去，
    // 而 local(epoch) = epoch_local_us − offset，于是把 offset 调大就把目标拉到过去。
    let set_target = |epoch_local_us: u64, offset_us: i64| {
        let mut state = sync.lock().unwrap();
        state.offset_us = offset_us;
        state.schedule = Some(EpochSchedule::new(11, epoch_local_us, 0));
    };

    // 目标在未来 5 s → 等待（并且明确告诉调用方还要等多久）
    set_target(anchor + 5_000_000, 0);
    match schedule_action(&sync, Some(&frame(0))) {
        Some((PlayoutAction::Wait { wait_us }, 0)) => {
            assert!(wait_us > 4_000_000, "wait = {wait_us}");
        }
        other => panic!("expected Wait, got {other:?}"),
    }

    // 正好到点（对端比本机快 10 s，把 epoch 拉到「现在」）→ 照常播
    set_target(anchor, 10_000_000);
    assert!(matches!(
        schedule_action(&sync, Some(&frame(0))),
        Some((PlayoutAction::Play, 0))
    ));

    // 已经过期 5 s → 丢弃（宁可丢一帧，也不延迟出声破坏组同步）
    set_target(anchor, 15_000_000);
    match schedule_action(&sync, Some(&frame(0))) {
        Some((PlayoutAction::Drop { late_us }, _)) => {
            assert!(late_us > 4_000_000, "late = {late_us}");
        }
        other => panic!("expected Drop, got {other:?}"),
    }

    // 手上没有帧就不做判定
    assert!(schedule_action(&sync, None).is_none());
}
#[test]
fn hub_clock_hands_out_one_timeline_to_every_session() {
    // 共享采集的核心承诺：全组拿到的是同一套编号（这也是「同一根时间轴」的全部含义）。
    // M4 起 next 多了一个「本端单调时刻」参数：未对齐时它只参与共同起点的判定。
    let mut clock = HubClock::default();
    assert_eq!(clock.next(960, 1_000_000), Some((0, 0)));
    assert_eq!(clock.next(960, 1_010_000), Some((1, 960)));
    assert_eq!(clock.next(960, 1_020_000), Some((2, 1_920)));

    // 序号回绕仍按 u32 语义（接收侧本来就按回绕判丢包）
    let mut wrapped = HubClock {
        seq: u32::MAX,
        sample_index: u32::MAX - 100,
        start_at_us: None,
    };
    assert_eq!(
        wrapped.next(960, 1_000_000),
        Some((u32::MAX, u32::MAX - 100))
    );
    assert_eq!(wrapped.next(960, 1_010_000), Some((0, 859)));
}

/// M4 共同基准：接收端广播的 epoch 一到，编号 0 点钉到它，**到点之前一帧都不发** ——
/// 否则那帧的编号在接收端没有可比性（它属于 epoch 之前的时间）。
#[test]
fn hub_clock_aligns_sample_index_to_a_common_start() {
    let mut clock = HubClock::default();
    assert_eq!(clock.next(960, 1_000_000), Some((0, 0)));
    assert_eq!(clock.next(960, 1_010_000), Some((1, 960)));

    clock.align_epoch(2_000_000);
    assert_eq!(clock.next(960, 1_999_999), None, "共同起点之前不该发帧");

    assert_eq!(
        clock.next(960, 2_000_000),
        Some((2, 0)),
        "到点后编号从 0 起算"
    );
    assert_eq!(clock.next(960, 2_020_000), Some((3, 960)));
}

/// 接收端换基准（重新对齐）：编号再次归零，seq 不受影响 —— 它只管丢包检测。
#[test]
fn realigning_moves_the_zero_point_without_touching_seq() {
    let mut clock = HubClock::default();
    assert_eq!(clock.next(480, 100), Some((0, 0)));
    clock.align_epoch(500);
    assert_eq!(clock.next(480, 500), Some((1, 0)));
    clock.align_epoch(900);
    assert_eq!(clock.next(480, 900), Some((2, 0)));
    assert_eq!(clock.next(480, 1_400), Some((3, 480)));
}
