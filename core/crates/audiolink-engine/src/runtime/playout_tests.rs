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
fn idle_mixer_rebuilds_when_stream_frame_length_changes() {
    let format = MixFormat {
        sample_rate: 48_000,
        channels: 2,
    };
    let mut slot = None;
    let tight = mixer_slot_for_frame(&mut slot, format, 480).unwrap();
    {
        let mut mixer = tight.mixer.lock().unwrap();
        mixer.add_source(1).unwrap();
        mixer.push(1, &vec![0.25; 960]).unwrap();
        let mut output = Vec::new();
        mixer.mix_frame(&mut output);
        assert_eq!(output.len(), 960);
    }
    assert!(mixer_slot_for_frame(&mut slot, format, 960).is_err());
    tight.mixer.lock().unwrap().remove_source(1);

    let standard = mixer_slot_for_frame(&mut slot, format, 960).unwrap();
    assert!(!Arc::ptr_eq(&tight, &standard));
    {
        let mut mixer = standard.mixer.lock().unwrap();
        mixer.add_source(2).unwrap();
        mixer.push(2, &vec![0.25; 1_920]).unwrap();
        let mut output = Vec::new();
        mixer.mix_frame(&mut output);
        assert_eq!(output.len(), 1_920);
        mixer.remove_source(2);
    }

    let tight_again = mixer_slot_for_frame(&mut slot, format, 480).unwrap();
    assert!(!Arc::ptr_eq(&standard, &tight_again));
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
fn short_empty_gap_keeps_late_frame_detection() {
    let (tx, rx) = crossbeam_channel::bounded(4);
    let stats = telemetry();
    let mut pending = None;
    let mut expected = Some(7);
    let mut gap = PlayoutGapState::default();
    let now = Instant::now();

    for beat in 0..2 {
        assert!(matches!(
            take_due_frame(&rx, &mut pending, &mut expected, &stats),
            DueFrame::Missing
        ));
        let action = gap.on_missing(now + Duration::from_millis(beat * 20), true);
        assert_eq!(action.raise_depth, beat == 0);
        assert!(!action.resync_cursor);
    }

    tx.send(frame(7)).unwrap();
    tx.send(frame(9)).unwrap();
    assert!(matches!(
        take_due_frame(&rx, &mut pending, &mut expected, &stats),
        DueFrame::Ready(PlaybackFrame { seq: 9, .. })
    ));
    assert_eq!(stats.lock().unwrap().snapshot().late_drops, 1);
}

#[test]
fn capture_pause_resyncs_playout_cursor_without_repeated_depth_raises() {
    let (tx, rx) = crossbeam_channel::bounded(4);
    let stats = telemetry();
    let mut pending = None;
    let mut expected = Some(7);
    let mut gap = PlayoutGapState::default();
    let now = Instant::now();

    for beat in 0..=12 {
        assert!(matches!(
            take_due_frame(&rx, &mut pending, &mut expected, &stats),
            DueFrame::Missing
        ));
        let action = gap.on_missing(now + Duration::from_millis(beat * 10), true);
        assert_eq!(action.raise_depth, beat == 0);
        if action.resync_cursor {
            expected = None;
        }
    }

    assert_eq!(expected, None);
    tx.send(frame(7)).unwrap();
    assert!(matches!(
        take_due_frame(&rx, &mut pending, &mut expected, &stats),
        DueFrame::Ready(PlaybackFrame { seq: 7, .. })
    ));
    gap.on_ready();
    assert!(
        gap.on_missing(now + Duration::from_secs(1), true)
            .raise_depth
    );
    assert_eq!(stats.lock().unwrap().snapshot().late_drops, 0);
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

/// 帧长标志：10 ms 档的**所有**音频包类型都带 `FRAME_10MS`（bit2），其它档不带。
///
/// # 为什么按「调用点」逐条覆盖
///
/// 这条位的语义是「**该流**使用 10 ms 帧长」（见 `Flags::FRAME_10MS` 的文档），
/// 它描述的是流而不是单个包。发送侧有三个组包点：主包、冗余副本、NACK 重传 —— 漏掉任何一个，
/// 接收端的交叉校验就会对「本来正确的那一半包」报不一致，而重传包恰恰是丢包时唯一还在到的东西。
/// 所以这里对三个点各断言一次，而不是只测「辅助函数返回了什么」。
#[test]
fn ten_ms_profile_sets_frame_flag_on_every_audio_copy() {
    let tight = CodecConfig::m1_low_delay_tight();
    let standard = CodecConfig::m1_default();

    // 基础位：10 ms 档置位，其它档不置。
    assert!(stream_flags(&tight).contains(Flags::FRAME_10MS));
    assert!(!stream_flags(&standard).contains(Flags::FRAME_10MS));

    // 三个组包点：主包 / 冗余副本 / NACK 重传。
    // 主包与重传用基础位，副本再叠 `FEC_REDUNDANT`。
    let primary = with_copy(stream_flags(&tight), AudioCopy::Primary);
    let redundant = with_copy(stream_flags(&tight), AudioCopy::Redundant);
    let retransmit = stream_flags(&tight);

    for (name, flags) in [
        ("主包", primary),
        ("冗余副本", redundant),
        ("NACK 重传", retransmit),
    ] {
        assert!(
            flags.contains(Flags::FRAME_10MS),
            "{name}在 10 ms 档必须带 FRAME_10MS —— 漏掉它会让接收端的交叉校验对正确的包报错"
        );
    }

    // 冗余副本还要保住自己的位：叠加不能把 `FEC_REDUNDANT` 吃掉。
    assert!(redundant.contains(Flags::FEC_REDUNDANT));
    assert!(!primary.contains(Flags::FEC_REDUNDANT));

    // 标准档：三个点都不带 `FRAME_10MS`。
    for (name, flags) in [
        (
            "主包",
            with_copy(stream_flags(&standard), AudioCopy::Primary),
        ),
        (
            "冗余副本",
            with_copy(stream_flags(&standard), AudioCopy::Redundant),
        ),
        ("NACK 重传", stream_flags(&standard)),
    ] {
        assert!(
            !flags.contains(Flags::FRAME_10MS),
            "{name}在标准档不该带 FRAME_10MS"
        );
    }
}

/// 接收侧交叉校验：位与有效帧长自洽时不计数，不符时**只计数**（不断流）。
#[test]
fn frame_flag_mismatch_is_counted_but_never_fatal() {
    let codec = CodecConfig::m1_default();
    let mut tight = StreamRxState::new(codec, 10).unwrap();
    let mut standard = StreamRxState::new(codec, 20).unwrap();

    // 10 ms 有效帧长：带位 == 自洽；不带位 == 不符。
    assert!(tight.observe_frame_flag(Flags::FRAME_10MS));
    assert_eq!(tight.flag_mismatches, 0);
    assert!(!tight.observe_frame_flag(Flags::NONE));
    assert_eq!(tight.flag_mismatches, 1, "不符必须留痕");
    // 连来三个：计数累计，没有任何 panic / 提前返回。
    let _ = tight.observe_frame_flag(Flags::NONE);
    let _ = tight.observe_frame_flag(Flags::NONE);
    assert_eq!(tight.flag_mismatches, 3);

    // 20 ms 有效帧长：不带位 == 自洽；带位 == 不符。
    assert!(standard.observe_frame_flag(Flags::NONE));
    assert_eq!(standard.flag_mismatches, 0);
    assert!(!standard.observe_frame_flag(Flags::FRAME_10MS));
    assert_eq!(standard.flag_mismatches, 1);

    // 副本位不该干扰帧长判定（`contains` 是「全部包含」语义）。
    assert!(standard.observe_frame_flag(Flags::FEC_REDUNDANT));
    assert_eq!(standard.flag_mismatches, 1);
}

/// 协商选取：第一个 `Opus` 项的帧长说了算；非法 / 非 Opus 一律回退本地档。
#[test]
fn negotiated_frame_ms_picks_the_first_opus_pref_or_falls_back() {
    let local = 20u32;
    let opus = |frame_ms: u8| CodecPref::Opus {
        frame_ms,
        bitrate_kbps: 160,
        fec: false,
        vbr: true,
    };

    // 合法值逐个认。
    for wanted in [10u8, 20, 40, 60] {
        assert_eq!(
            negotiated_frame_ms(&[opus(wanted)], local),
            u32::from(wanted)
        );
    }
    // 取**第一个** `Opus` 项（prefs 按优先级降序）。
    assert_eq!(negotiated_frame_ms(&[opus(10), opus(20)], local), 10);
    // 非 Opus 项跳过，继续找后面的 Opus。
    assert_eq!(
        negotiated_frame_ms(&[CodecPref::Pcm16, opus(40)], local),
        40
    );
    // 没有 Opus 项 → 本地档。
    assert_eq!(negotiated_frame_ms(&[CodecPref::Pcm16], local), local);
    assert_eq!(negotiated_frame_ms(&[], local), local);
    // 非法帧长 → 本地档（**不**继续往后找：首选非法说明协商前提已坏）。
    assert_eq!(negotiated_frame_ms(&[opus(15)], local), local);
    assert_eq!(negotiated_frame_ms(&[opus(15), opus(10)], local), local);
    assert_eq!(negotiated_frame_ms(&[opus(0)], local), local);
}

/// 换档：只有帧长真的变了才重建，且派生量一起换（漏一个就是「半换档」）。
#[test]
fn retarget_rebuilds_every_frame_ms_derived_value() {
    let codec = CodecConfig::m1_default();
    let mut state = StreamRxState::new(codec, 20).unwrap();
    assert_eq!(state.frame_ms, 20);
    assert_eq!(state.frame_us, 20_000);
    assert_eq!(state.frame_period, Duration::from_millis(20));

    state.retarget(codec, 10).unwrap();
    assert_eq!(state.frame_ms, 10);
    assert_eq!(state.frame_us, 10_000, "自适应抖动的换算基准必须跟着换");
    assert_eq!(state.frame_period, Duration::from_millis(10));

    // 同帧长再调一次是幂等的（协商重复到达不该白重建解码器）。
    state.retarget(codec, 10).unwrap();
    assert_eq!(state.frame_ms, 10);

    // 10 → 20：反向同样要换全。
    state.retarget(codec, 20).unwrap();
    assert_eq!(state.frame_ms, 20);
    assert_eq!(state.frame_us, 20_000);
    assert_eq!(state.frame_period, Duration::from_millis(20));
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
    // 默认低延迟档也必须接住紧随下一主包到达的副本，不能靠事先升到 60 ms 才修复。
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

/// 直接驱动生产播放循环：按已写出的拍数注入帧，不依赖网络/线程间 sleep 的相对时序。
fn scripted_playout(
    after_write: impl Fn(usize, &Sender<PlaybackFrame>) + Send + Sync + 'static,
    writes: usize,
    local_gain: Arc<Mutex<GainState>>,
) -> (Vec<Vec<f32>>, StreamStats) {
    scripted_playout_at_depth(after_write, writes, local_gain, 1)
}

fn scripted_playout_at_depth(
    after_write: impl Fn(usize, &Sender<PlaybackFrame>) + Send + Sync + 'static,
    writes: usize,
    local_gain: Arc<Mutex<GainState>>,
    initial_depth: usize,
) -> (Vec<Vec<f32>>, StreamStats) {
    struct ScriptedSink {
        inner: audiolink_audio::NullPlayout,
        output: Arc<Mutex<Vec<Vec<f32>>>>,
        after_write: Arc<dyn Fn(usize) + Send + Sync>,
    }
    impl PlayoutSink for ScriptedSink {
        fn device_format(&self) -> audiolink_audio::DeviceFormat {
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
            let count = {
                let mut output = self.output.lock().unwrap();
                output.push(samples.to_vec());
                output.len()
            };
            (self.after_write)(count);
            Ok(())
        }
        fn stats(&self) -> audiolink_audio::PlayoutStats {
            self.inner.stats()
        }
        fn stop(&mut self) {
            self.inner.stop();
        }
        fn backend_name(&self) -> &'static str {
            "scripted-null"
        }
    }
    let config = CodecConfig::m1_default();
    let (tx, rx) = crossbeam_channel::bounded(32);
    for seq in 0..initial_depth as u32 {
        tx.send(PlaybackFrame {
            seq,
            samples: vec![0.4; config.interleaved_frame()],
        })
        .unwrap();
    }
    let stop = new_stop_flag();
    let script_stop = stop.clone();
    let script = Arc::new(move |count| {
        after_write(count, &tx);
        if count >= writes {
            script_stop.store(true, Ordering::Relaxed);
        }
    });
    let output = Arc::new(Mutex::new(Vec::new()));
    let factory = || -> Result<Box<dyn PlayoutSink>, AudioError> {
        Ok(Box::new(ScriptedSink {
            inner: audiolink_audio::NullPlayout::new(60),
            output: output.clone(),
            after_write: script.clone(),
        }))
    };
    let stats = telemetry();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let mixer = Arc::new(Mutex::new(PcmMixer::new(
        MixFormat {
            sample_rate: 48_000,
            channels: 2,
        },
        config.frame_samples(),
        8,
    )));
    playout_main(
        &factory,
        20,
        config.interleaved_frame(),
        stats.clone(),
        None,
        stop,
        Arc::new(AtomicUsize::new(initial_depth)),
        Arc::new(Mutex::new(PlayoutSync::default())),
        broadcast::channel(16).0,
        NodeId([0; 32]),
        Arc::new(Mutex::new(GainState::new(1_000))),
        local_gain,
        None,
        1,
        Arc::new(PlayoutMixSlot {
            mixer,
            frame_samples: config.frame_samples(),
            owner: AtomicU32::new(0),
        }),
        rx,
        ready_tx,
    );
    ready_rx.recv().unwrap().unwrap();
    let recorded = output.lock().unwrap().clone();
    let snapshot = stats.lock().unwrap().snapshot();
    (recorded, snapshot)
}

#[test]
fn larger_buffer_preserves_audio_during_repeated_eighty_ms_delivery_bursts() {
    let run = |depth| {
        scripted_playout_at_depth(
            move |count, tx| {
                if count % 4 == 0 {
                    for offset in 0..4 {
                        tx.send(PlaybackFrame {
                            seq: (depth + count - 4 + offset) as u32,
                            samples: vec![0.4; CodecConfig::m1_default().interleaved_frame()],
                        })
                        .unwrap();
                    }
                }
            },
            16,
            Arc::new(Mutex::new(GainState::new(1_000))),
            depth,
        )
    };
    let (_, shallow) = run(3);
    assert!(
        shallow.underruns > 0,
        "旧 60 ms 上限不能吸收 80 ms 成批到达"
    );
    let (output, protected) = run(6);
    assert_eq!(protected.underruns, 0);
    assert!(
        output
            .iter()
            .flatten()
            .all(|sample| (*sample - 0.4).abs() < 1e-6)
    );
}

#[test]
fn android_backlog_feedback_reduces_total_queue_without_output_underruns() {
    // 运行真实播放循环，模拟独立按 48 kHz 消费的 Android 输出链。
    // 到包由每拍的反馈读取驱动，不依赖 write 次数，因此停写一拍不会让声源也停止。
    #[derive(Default)]
    struct Model {
        ticks: u32,
        output_frames: u32,
        min_total: u32,
        first_total: u32,
        last_total: u32,
        device_underruns: u32,
    }
    struct AndroidSink {
        inner: audiolink_audio::NullPlayout,
        model: Arc<Mutex<Model>>,
        tx: Sender<PlaybackFrame>,
        stop: Arc<AtomicBool>,
    }
    impl PlayoutSink for AndroidSink {
        fn device_format(&self) -> audiolink_audio::DeviceFormat {
            self.inner.device_format()
        }
        fn requested_buffer_ms(&self) -> u32 {
            30
        }
        fn effective_buffer_ms(&self) -> u32 {
            80
        }
        fn buffered_frames(&mut self) -> u32 {
            self.model.lock().unwrap().output_frames
        }
        fn buffer_state(&mut self) -> Option<audiolink_audio::PlayoutBufferState> {
            let mut model = self.model.lock().unwrap();
            if model.output_frames < 960 {
                model.device_underruns += 1;
            }
            model.output_frames = model.output_frames.saturating_sub(960);
            self.tx
                .try_send(PlaybackFrame {
                    seq: 8 + model.ticks,
                    samples: vec![0.4; 1920],
                })
                .unwrap();
            model.ticks += 1;
            let total = self.tx.len() as u32 * 960 + model.output_frames;
            if model.ticks == 1 {
                model.first_total = total;
                model.min_total = total;
            }
            model.min_total = model.min_total.min(total);
            model.last_total = total;
            if model.ticks >= 300 {
                self.stop.store(true, Ordering::Relaxed);
            }
            Some(audiolink_audio::PlayoutBufferState {
                queued_frames: model.output_frames,
                target_frames: 1440,
            })
        }
        fn write(&mut self, samples: &[f32]) -> Result<(), AudioError> {
            // 常量声源不应在回收边界生成静音或尖峰。
            assert!(samples.iter().all(|value| (*value - 0.4).abs() < 1e-6));
            self.model.lock().unwrap().output_frames += (samples.len() / 2) as u32;
            self.inner.write(samples)
        }
        fn stats(&self) -> audiolink_audio::PlayoutStats {
            self.inner.stats()
        }
        fn stop(&mut self) {
            self.inner.stop();
        }
        fn backend_name(&self) -> &'static str {
            "simulated-android"
        }
    }
    let (tx, rx) = crossbeam_channel::bounded(32);
    for seq in 0..8 {
        tx.send(PlaybackFrame {
            seq,
            samples: vec![0.4; 1920],
        })
        .unwrap();
    }
    let model = Arc::new(Mutex::new(Model {
        output_frames: 3840,
        ..Model::default()
    }));
    let stop = new_stop_flag();
    let factory = || -> Result<Box<dyn PlayoutSink>, AudioError> {
        Ok(Box::new(AndroidSink {
            inner: audiolink_audio::NullPlayout::new(80),
            model: model.clone(),
            tx: tx.clone(),
            stop: stop.clone(),
        }))
    };
    let stats = telemetry();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let mixer = Arc::new(Mutex::new(PcmMixer::new(
        MixFormat {
            sample_rate: 48000,
            channels: 2,
        },
        960,
        8,
    )));
    playout_main(
        &factory,
        20,
        1920,
        stats.clone(),
        None,
        stop.clone(),
        Arc::new(AtomicUsize::new(1)),
        Arc::new(Mutex::new(PlayoutSync::default())),
        broadcast::channel(16).0,
        NodeId([0; 32]),
        Arc::new(Mutex::new(GainState::new(1000))),
        Arc::new(Mutex::new(GainState::new(1000))),
        None,
        1,
        Arc::new(PlayoutMixSlot {
            mixer,
            frame_samples: 960,
            owner: AtomicU32::new(0),
        }),
        rx,
        ready_tx,
    );
    ready_rx.recv().unwrap().unwrap();
    let model = model.lock().unwrap();
    eprintln!(
        "simulated Android total queue: {} -> {} ms; min {} ms; device underruns {}",
        model.first_total / 48,
        model.last_total / 48,
        model.min_total / 48,
        model.device_underruns
    );
    assert!(model.first_total >= 200 * 48);
    assert!(model.last_total <= 70 * 48, "水位不得只在队列间搬家");
    assert!(model.min_total >= 30 * 48, "不能抽干输出链");
    assert_eq!(model.device_underruns, 0);
    let stats = stats.lock().unwrap();
    assert_eq!(stats.snapshot().underruns, 0);
    assert_eq!(
        stats.snapshot().late_drops,
        0,
        "主动回收不得制造接收序号缺口"
    );
    assert!(stats.depth_drops() > 0);
}

#[test]
fn wifi_gap_and_refill_are_concealed_at_the_playback_deadline() {
    let config = CodecConfig::m1_default();
    let (output, stats) = scripted_playout(
        move |count, tx| {
            // 第一拍之后断流一拍；第二拍之后恢复，第三拍需 Hold 以补足两帧水位。
            if count == 2 || count == 3 {
                tx.send(PlaybackFrame {
                    seq: count as u32,
                    samples: vec![-0.4; config.interleaved_frame()],
                })
                .unwrap();
            }
        },
        4,
        Arc::new(Mutex::new(GainState::new(1_000))),
    );
    assert_eq!(stats.underruns, 1, "主动补水不能被当作新的网络欠载");
    assert!(
        rms(&output[1]) > 0.1,
        "后续包尚未到达时就必须掩盖，不能硬静音"
    );
    assert!(rms(&output[2]) > 0.1, "重缓冲等待也不能硬静音");
    assert!(
        (output[3][0] - output[2].last().unwrap()).abs() < 0.02,
        "恢复时必须从实际已播放的掩盖尾部交叉淡入"
    );
    assert_eq!(*output[3].last().unwrap(), -0.4, "缓冲补齐后继续播放真实帧");
}

#[test]
fn prolonged_wifi_gap_fades_out_instead_of_looping_old_audio() {
    let (output, _) = scripted_playout(|_, _| {}, 10, Arc::new(Mutex::new(GainState::new(1_000))));
    let levels: Vec<_> = output.iter().map(|samples| rms(samples)).collect();
    assert!(levels[1] > 0.1, "短断流必须先掩盖：{levels:?}");
    assert!(
        levels.windows(2).all(|pair| pair[1] <= pair[0]),
        "连续断流必须逐步淡出：{levels:?}"
    );
    assert!(
        levels[7..].iter().all(|level| *level < 1e-6),
        "120 ms 后不再重复旧音频：{levels:?}"
    );
}

#[test]
fn muting_during_wifi_gap_also_mutes_concealed_audio() {
    let gain = Arc::new(Mutex::new(GainState::new(1_000)));
    let script_gain = gain.clone();
    let (output, _) = scripted_playout(
        move |count, _| {
            if count == 2 {
                script_gain.lock().unwrap().set_target(0, 0, 20).unwrap();
            }
        },
        4,
        gain,
    );
    assert!(rms(&output[1]) > 0.1, "静音前应先有掩盖音频");
    assert!(
        output[2..]
            .iter()
            .flatten()
            .all(|sample| sample.abs() < 1e-6),
        "欠载期间音量控制也必须生效，不能重播静音前的音量"
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
