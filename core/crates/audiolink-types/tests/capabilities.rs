//! §13 能力协商：位图语义与"必需位缺失"的判定（`docs/46-m4-capability-negotiation.md`）。
//!
//! 这里只测纯逻辑 —— 真正的握手协商在 `audiolink-engine` 的 `handshake::tests` 里走完整帧流程。

use audiolink_types::Capabilities;

#[test]
fn intersect_is_the_common_ground() {
    let both = Capabilities::CURRENT;
    assert_eq!(
        Capabilities::intersect(both, both),
        both,
        "相同能力 → 交集是自己"
    );

    let a = Capabilities::OPUS | Capabilities::CAPTURE | Capabilities::MIXER;
    let b = Capabilities::OPUS | Capabilities::PLAYOUT | Capabilities::GROUP_EPOCH;
    assert_eq!(
        Capabilities::intersect(a, b),
        Capabilities::OPUS,
        "只有 Opus 是共同的"
    );
    assert_eq!(
        Capabilities::intersect(a, 0),
        0,
        "对端什么都没声明 → 交集为空"
    );
}

#[test]
fn only_the_required_bits_block_a_session() {
    let both = Capabilities::CURRENT;
    assert_eq!(
        Capabilities::missing_required(both),
        0,
        "必需位都在 → 可以连接"
    );

    // 少一个**可选**位不算失败：这正是"降级"而不是"拒绝"的意义。
    let without_mixer = both & !Capabilities::MIXER;
    assert_eq!(Capabilities::missing_required(without_mixer), 0);
    let without_loopback = both & !Capabilities::SYSTEM_LOOPBACK;
    assert_eq!(Capabilities::missing_required(without_loopback), 0);

    // 少一个**必需**位（Opus）必须拒绝 —— 否则会"连上了却没声音"。
    let no_codec = Capabilities::CAPTURE | Capabilities::PLAYOUT;
    assert_eq!(
        Capabilities::missing_required(Capabilities::intersect(no_codec, both)),
        Capabilities::OPUS
    );
    assert_eq!(Capabilities::missing_required(0), Capabilities::OPUS);
}

#[test]
fn describe_is_stable_for_reports_and_ui() {
    assert_eq!(Capabilities::describe(0), "无");
    assert_eq!(
        Capabilities::describe(Capabilities::OPUS | Capabilities::MIXER),
        "Opus 编码、多路混音"
    );
    // 顺序固定：与 ALL_KNOWN 一致，报告与界面的文案才不会随机换位。
    assert_eq!(
        Capabilities::describe(Capabilities::GROUP_EPOCH | Capabilities::CAPTURE),
        "音频采集、同步组预约播放"
    );
    assert_eq!(Capabilities::describe(1 << 31), "未知能力");
}

#[test]
fn unknown_bits_are_reported_but_do_not_break_describe() {
    assert_eq!(Capabilities::unknown_bits(Capabilities::CURRENT), 0);
    assert_eq!(Capabilities::unknown_bits(1 << 31), 1 << 31);
    assert_eq!(
        Capabilities::unknown_bits(Capabilities::OPUS | (1 << 31)),
        1 << 31,
        "已知位不该被算进未知位"
    );
}

#[test]
fn current_declares_only_what_the_core_can_actually_do() {
    let known = Capabilities::ALL_KNOWN
        .iter()
        .fold(0u32, |acc, bit| acc | bit);
    assert_eq!(
        Capabilities::unknown_bits(Capabilities::CURRENT),
        0,
        "不能声明一个自己都不知道的位"
    );
    // 内录与麦克风要平台侧真的接上才算（Windows WASAPI loopback / Android AudioPlaybackCapture）——
    // 现在声明它们就是"声明了做不到的事"，将来接上平台能力时再打开。
    assert_eq!(Capabilities::CURRENT & Capabilities::SYSTEM_LOOPBACK, 0);
    assert_eq!(Capabilities::CURRENT & Capabilities::MICROPHONE, 0);
    assert_eq!(
        known & !Capabilities::CURRENT,
        Capabilities::SYSTEM_LOOPBACK | Capabilities::MICROPHONE
    );
    assert_eq!(
        Capabilities::CURRENT & Capabilities::REQUIRED,
        Capabilities::REQUIRED,
        "本端必须满足自己声明的必需能力"
    );
}
