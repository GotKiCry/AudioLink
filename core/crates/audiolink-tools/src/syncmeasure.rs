//! 双机录音对齐（tools/sync-measure 的共用内核）。
//!
//! 路线图 M3 的验收条件是「组内偏差 ≤ ±10 ms（P95，双机同期录音 + 波形对齐）」；
//! 本模块就是那句话的可执行版本：从两段**同期录音**里找出同一串脉冲的位置差，
//! 输出偏差的均值 / P50 / P95(绝对值) / 极值，以及对「两台设备晶振速率差」的线性回归（ppm）。
//!
//! 四条纪律：
//!
//! 1. **纯逻辑**：只吃「已经解出来的采样」，WAV 解析在本模块、CLI 在 bin 里，这里不碰文件系统；
//! 2. **不做听感猜测**：偏差只由「脉冲到达时刻」定义。检测不到脉冲、脉冲数不足、采样率不一致
//!    一律**明确报错**，绝不退化成「0 偏差、验收通过」——测具撒谎比测不出更坏；
//! 3. **两级精度**：粗检测用「快攻击慢释放的包络穿越阈值」（找出脉冲在哪），
//!    精细对齐用「脉冲邻域的归一化互相关 + 抛物线插值」（把位置定到亚采样）。
//!    只靠阈值穿越会被上升沿斜率系统性带偏，只靠互相关又会在大范围里搜丢；
//! 4. **不做重采样**：两段录音采样率必须一致，否则直接报错。测具只该测设备偏差，
//!    不该把自己重采样器的误差也算进去（真实录音先统一到 48 kHz 再进来）。

use serde_json::{Value, json};

/// 脉冲检测参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetectConfig {
    /// 触发阈值 = 该路录音包络峰值 × ratio/1000（千分数，避免浮点参数不可复现）。
    pub threshold_ratio_x1000: u32,
    /// 包络「慢释放」时间常数（ms）：快攻击保证脉冲起点不被平滑掉。
    pub release_tau_ms: u32,
    /// 两个脉冲之间的最小间隔（ms）：间隔内的重复穿越忽略（脉冲内部的振荡不会二次触发）。
    pub min_gap_ms: u32,
    /// 精细对齐窗口长度（ms）。
    pub correlate_window_ms: u32,
    /// 精细对齐的最大搜索半径（ms）。
    pub max_shift_ms: u32,
}

impl Default for DetectConfig {
    fn default() -> Self {
        Self {
            threshold_ratio_x1000: 500,
            release_tau_ms: 4,
            min_gap_ms: 5,
            correlate_window_ms: 2,
            max_shift_ms: 3,
        }
    }
}

/// 验收门限（M3 口径：组内偏差 P95 ≤ ±10 ms）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    /// 偏差门限（ms）：|均值| 与 P95(绝对值) 都要落在里面。
    pub max_bias_ms: f64,
    /// 漂移门限（ppm）：两台设备晶振速率差。
    pub max_drift_ppm: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            max_bias_ms: 10.0,
            max_drift_ppm: 200.0,
        }
    }
}

// ============================ WAV 解析 ============================

/// WAV 解析失败的原因（每一条都必须能被人看懂并据此修输入）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WavError {
    /// 不是 RIFF/WAVE 容器。
    NotRiff,
    /// fmt 块本身不合法（太短、声道数 0、采样率 0）。
    BadFmt,
    /// 没有 fmt 块。
    MissingFmt,
    /// 没有 data 块。
    MissingData,
    /// 编码 / 位深不支持。
    UnsupportedCodec { format: u16, bits: u16 },
    /// 请求的声道号超出范围。
    ChannelOutOfRange { requested: usize, channels: u16 },
    /// 文件在块中途被截断。
    Truncated,
}

impl std::fmt::Display for WavError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRiff => write!(f, "not a RIFF/WAVE file"),
            Self::BadFmt => write!(f, "malformed fmt chunk"),
            Self::MissingFmt => write!(f, "missing fmt chunk"),
            Self::MissingData => write!(f, "missing data chunk"),
            Self::UnsupportedCodec { format, bits } => {
                write!(f, "unsupported codec: format={format} bits={bits}")
            }
            Self::ChannelOutOfRange {
                requested,
                channels,
            } => {
                write!(f, "channel {requested} out of range (file has {channels})")
            }
            Self::Truncated => write!(f, "truncated chunk"),
        }
    }
}

impl std::error::Error for WavError {}

/// fmt 块里我们真正在意的字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FmtChunk {
    /// 有效编码：1 = PCM 整数，3 = IEEE float（EXTENSIBLE 会被解到这里）。
    pub codec: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub bits: u16,
}

/// 解出来的单声道采样（录音文件里的一路）。
#[derive(Debug, Clone, PartialEq)]
pub struct WavData {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: usize,
    pub bits: u16,
    pub samples: Vec<f32>,
}

/// 解析 WAV 并取出指定声道（0 = 第一路）。
pub fn parse_wav(bytes: &[u8], channel: usize) -> Result<WavData, WavError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::NotRiff);
    }
    let mut pos = 12usize;
    let mut fmt: Option<FmtChunk> = None;
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        let body = pos + 8;
        let end = body.checked_add(size).ok_or(WavError::Truncated)?;
        if end > bytes.len() {
            // 只有 data 块允许「写到一半就断了」：按可用字节截到整帧，其它块一律判截断。
            if id == b"data" {
                data = Some(&bytes[body..]);
                break;
            }
            return Err(WavError::Truncated);
        }
        match id {
            b"fmt " => fmt = Some(parse_fmt(&bytes[body..end])?),
            b"data" => data = Some(&bytes[body..end]),
            _ => {}
        }
        // RIFF 块按偶数字节对齐（奇数长度要吃掉一个填充字节）
        pos = end + (size & 1);
    }
    let fmt = fmt.ok_or(WavError::MissingFmt)?;
    let data = data.ok_or(WavError::MissingData)?;
    if channel >= usize::from(fmt.channels) {
        return Err(WavError::ChannelOutOfRange {
            requested: channel,
            channels: fmt.channels,
        });
    }
    let samples = decode_mono(data, &fmt, channel)?;
    Ok(WavData {
        sample_rate: fmt.sample_rate,
        channels: fmt.channels,
        frames: samples.len(),
        bits: fmt.bits,
        samples,
    })
}

fn parse_fmt(body: &[u8]) -> Result<FmtChunk, WavError> {
    if body.len() < 16 {
        return Err(WavError::BadFmt);
    }
    let mut codec = u16::from_le_bytes([body[0], body[1]]);
    let channels = u16::from_le_bytes([body[2], body[3]]);
    let sample_rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
    let bits = u16::from_le_bytes([body[14], body[15]]);
    if codec == 0xFFFE {
        // WAVE_FORMAT_EXTENSIBLE：有效编码藏在 SubFormat GUID 的头两个字节
        if body.len() < 40 {
            return Err(WavError::BadFmt);
        }
        codec = u16::from_le_bytes([body[24], body[25]]);
    }
    if channels == 0 || sample_rate == 0 {
        return Err(WavError::BadFmt);
    }
    Ok(FmtChunk {
        codec,
        channels,
        sample_rate,
        bits,
    })
}

fn decode_mono(data: &[u8], fmt: &FmtChunk, channel: usize) -> Result<Vec<f32>, WavError> {
    let bytes_per_sample = usize::from(fmt.bits).div_ceil(8);
    if bytes_per_sample == 0 {
        return Err(WavError::UnsupportedCodec {
            format: fmt.codec,
            bits: fmt.bits,
        });
    }
    let frame_bytes = bytes_per_sample * usize::from(fmt.channels);
    if frame_bytes == 0 {
        return Err(WavError::BadFmt);
    }
    let frames = data.len() / frame_bytes;
    let mut out = Vec::with_capacity(frames);
    for frame in 0..frames {
        let base = frame * frame_bytes + channel * bytes_per_sample;
        let s = &data[base..base + bytes_per_sample];
        let value = match (fmt.codec, fmt.bits) {
            (1, 16) => f64::from(i16::from_le_bytes([s[0], s[1]])) / 32_768.0,
            (1, 24) => {
                let high = if s[2] & 0x80 != 0 { 0xFFu8 } else { 0x00u8 };
                f64::from(i32::from_le_bytes([s[0], s[1], s[2], high])) / 8_388_608.0
            }
            (1, 32) => f64::from(i32::from_le_bytes([s[0], s[1], s[2], s[3]])) / 2_147_483_648.0,
            (3, 32) => f64::from(f32::from_le_bytes([s[0], s[1], s[2], s[3]])),
            (3, 64) => f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]),
            _ => {
                return Err(WavError::UnsupportedCodec {
                    format: fmt.codec,
                    bits: fmt.bits,
                });
            }
        };
        out.push(value as f32);
    }
    Ok(out)
}

// ============================ 检测与对齐 ============================

/// 测量失败的原因。
#[derive(Debug, Clone, PartialEq)]
pub enum SyncError {
    /// 采样率为 0。
    ZeroSampleRate,
    /// 采样太少（连一个窗口都凑不齐）。
    TooShort,
    /// 录音是静音（没有可比的东西）。
    NoSignal,
    /// 某一侧完全没检测到脉冲。
    NoPulses {
        /// 哪一侧（"a" / "b"）。
        side: &'static str,
    },
    /// 脉冲太少（少于 2 个就算不出漂移）。
    NotEnoughPulses { found: usize },
    /// 两段录音采样率不一致。
    SampleRateMismatch { a: u32, b: u32 },
    /// WAV 解析失败。
    Wav(WavError),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroSampleRate => write!(f, "sample rate is 0"),
            Self::TooShort => write!(f, "recording is too short to analyse"),
            Self::NoSignal => write!(f, "recording is silent (no signal above the noise floor)"),
            Self::NoPulses { side } => write!(f, "no pulse detected in recording {side}"),
            Self::NotEnoughPulses { found } => {
                write!(f, "only {found} pulses detected, at least 2 are needed")
            }
            Self::SampleRateMismatch { a, b } => write!(
                f,
                "sample rates differ ({a} Hz vs {b} Hz); resample both to one rate first"
            ),
            Self::Wav(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SyncError {}

/// 一对脉冲的对齐结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PulsePair {
    /// A 侧脉冲时刻（秒，文件内）。
    pub a_secs: f64,
    /// B 侧脉冲时刻（秒，文件内）。
    pub b_secs: f64,
    /// 偏差（ms）：**正数 = B 晚于 A**。
    pub offset_ms: f64,
    /// 精细对齐的归一化互相关峰值（1.0 = 波形完全一致）。
    pub ncc: f64,
    /// 搜索范围内存在另一个几乎等高的相关峰：波形可能带周期性，偏差有落在隔壁周期的风险。
    pub ambiguous: bool,
}

/// 一次测量的完整结果。
#[derive(Debug, Clone, PartialEq)]
pub struct SyncReport {
    pub sample_rate: u32,
    pub a_frames: usize,
    pub b_frames: usize,
    pub a_pulses: usize,
    pub b_pulses: usize,
    pub pairs: Vec<PulsePair>,
    /// 偏差均值（ms）。
    pub mean_offset_ms: f64,
    /// 偏差中位数（ms）。
    pub p50_offset_ms: f64,
    /// |偏差| 的 P95（ms）。
    pub p95_abs_offset_ms: f64,
    /// |偏差| 的最大值（ms）。
    pub max_abs_offset_ms: f64,
    /// 漂移（ppm）：对「A 侧时刻 → 偏差」做线性回归的斜率。
    pub drift_ppm: f64,
    /// 判定结论。
    pub verdict: Verdict,
    /// 判定理由 / 警告（人类可读，逐条）。
    pub notes: Vec<String>,
}

/// 判定结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Within,
    Exceeded,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Within => "within",
            Self::Exceeded => "exceeded",
        }
    }
}

/// 包络：快攻击、慢释放的峰值跟随器（release 是每采样的衰减系数）。
fn envelope(samples: &[f32], release: f64) -> Vec<f64> {
    let mean = if samples.is_empty() {
        0.0
    } else {
        samples.iter().map(|s| f64::from(*s)).sum::<f64>() / samples.len() as f64
    };
    let mut out = Vec::with_capacity(samples.len());
    let mut env = 0.0f64;
    for s in samples {
        let x = (f64::from(*s) - mean).abs();
        env = if x > env { x } else { env * release };
        out.push(env);
    }
    out
}

/// 检测脉冲起始时刻（秒，文件内）。
pub fn detect_pulse_onsets(
    samples: &[f32],
    sample_rate: u32,
    cfg: DetectConfig,
) -> Result<Vec<f64>, SyncError> {
    if sample_rate == 0 {
        return Err(SyncError::ZeroSampleRate);
    }
    let fs = f64::from(sample_rate);
    let window = ((f64::from(cfg.correlate_window_ms) / 1000.0) * fs).round() as usize;
    if samples.len() < window.max(2) {
        return Err(SyncError::TooShort);
    }
    let tau = (f64::from(cfg.release_tau_ms) / 1000.0).max(1e-4);
    let release = (-1.0 / (tau * fs)).exp();
    let env = envelope(samples, release);
    let peak = env.iter().copied().fold(0.0f64, f64::max);
    if peak <= 1e-9 {
        return Err(SyncError::NoSignal);
    }
    let threshold = peak * (f64::from(cfg.threshold_ratio_x1000) / 1000.0);
    let min_gap_secs = f64::from(cfg.min_gap_ms) / 1000.0;

    let mut onsets: Vec<f64> = Vec::new();
    if env[0] >= threshold {
        onsets.push(0.0);
    }
    for i in 1..env.len() {
        let prev = env[i - 1];
        let cur = env[i];
        if prev < threshold && cur >= threshold {
            let denom = cur - prev;
            let frac = if denom > 0.0 {
                (threshold - prev) / denom
            } else {
                0.0
            };
            let pos = ((i - 1) as f64 + frac).max(0.0);
            let secs = pos / fs;
            let too_close = onsets
                .last()
                .is_some_and(|last| secs - *last < min_gap_secs);
            if !too_close {
                onsets.push(secs);
            }
        }
    }
    if onsets.is_empty() {
        return Err(SyncError::NoSignal);
    }
    Ok(onsets)
}

/// 归一化互相关（去均值），1.0 = 形状完全一致；任一侧无变化时返回 -1。
fn ncc(x: &[f32], y: &[f32]) -> f64 {
    let n = x.len().min(y.len());
    if n == 0 {
        return -1.0;
    }
    let mx = x[..n].iter().map(|v| f64::from(*v)).sum::<f64>() / n as f64;
    let my = y[..n].iter().map(|v| f64::from(*v)).sum::<f64>() / n as f64;
    let mut num = 0.0;
    let mut dx = 0.0;
    let mut dy = 0.0;
    for k in 0..n {
        let a = f64::from(x[k]) - mx;
        let b = f64::from(y[k]) - my;
        num += a * b;
        dx += a * a;
        dy += b * b;
    }
    if dx <= 0.0 || dy <= 0.0 {
        return -1.0;
    }
    num / (dx.sqrt() * dy.sqrt())
}

/// 抛物线插值：用 (ym1, y0, yp1) 求峰值相对 y0 的**亚采样**偏移。
fn parabolic_peak(ym1: f64, y0: f64, yp1: f64) -> f64 {
    let denom = ym1 - 2.0 * y0 + yp1;
    if denom.abs() < 1e-12 {
        0.0
    } else {
        0.5 * (ym1 - yp1) / denom
    }
}

/// 一对脉冲的波形对齐质量下限（归一化互相关）：低于它的配对直接丢弃。
const MIN_MATCH_NCC: f64 = 0.5;

/// 用互相关把一对脉冲的时刻差定到亚采样。
fn refine_pair(
    a: &[f32],
    b: &[f32],
    sample_rate: u32,
    a_secs: f64,
    b_secs: f64,
    cfg: DetectConfig,
) -> Option<PulsePair> {
    let fs = f64::from(sample_rate);
    let wanted = (((f64::from(cfg.correlate_window_ms) / 1000.0) * fs).round() as usize).max(8);
    let max_shift = ((f64::from(cfg.max_shift_ms) / 1000.0) * fs).round() as i64;
    let a_onset = (a_secs * fs).round() as i64;
    let b_onset = (b_secs * fs).round() as i64;
    // 模板从粗检测到的脉冲时刻开始，只往右取窗口：脉冲本身有 20 ms，起点右侧一定还有内容，
    // 而「往前留余量」在贴近文件开头的脉冲上会让对应的 B 窗口落到负索引、整对被丢掉。
    // 粗检测误差本来由下面的 lag 搜索补偿，不需要提前留余量。
    let a_begin = a_onset.max(0);
    let available = a.len() as i64 - a_begin;
    if available < 8 {
        return None;
    }
    let win = wanted.min(available as usize);
    let template = &a[a_begin as usize..(a_begin + win as i64) as usize];

    let mut best: Option<(i64, f64)> = None;
    let mut scores: Vec<(i64, f64)> = Vec::new();
    for lag in -max_shift..=max_shift {
        let b_start = b_onset + lag;
        if b_start < 0 || b_start + win as i64 > b.len() as i64 {
            continue;
        }
        let candidate = &b[b_start as usize..(b_start + win as i64) as usize];
        let score = ncc(template, candidate);
        scores.push((lag, score));
        if best.is_none_or(|(_, s)| score > s) {
            best = Some((lag, score));
        }
    }
    let (best_lag, best_score) = best?;
    // 对齐质量下限：NCC 低于这个值的「配对」不是同一段波形，宁可丢掉（报告里 pairs 会少于 pulses）
    // 也不能把一段不相干的信号当成偏差交出去 —— 测具撒谎比测不出更坏。
    if best_score < MIN_MATCH_NCC {
        return None;
    }
    // 亚采样：拿峰值左右两个 lag 的分数做抛物线
    let left = scores
        .iter()
        .find(|(l, _)| *l == best_lag - 1)
        .map(|(_, s)| *s);
    let right = scores
        .iter()
        .find(|(l, _)| *l == best_lag + 1)
        .map(|(_, s)| *s);
    let frac = match (left, right) {
        (Some(ym1), Some(yp1)) => parabolic_peak(ym1, best_score, yp1).clamp(-0.5, 0.5),
        _ => 0.0,
    };
    // 两个窗口内容匹配，等价于两个窗口起点在同一时间轴上对应，所以偏差就是起点差（加亚采样小数）
    // 歧义检查：搜索范围内若有别的局部极大值几乎和最佳峰一样高，说明波形带周期性
    // （单一频率的长正弦最典型）——偏差可能落在隔壁周期上，必须显式标出来而不是假装没看见。
    let mut ambiguous = false;
    for window in scores.windows(3) {
        let (prev, cur, next) = (&window[0], &window[1], &window[2]);
        if cur.1 > prev.1 && cur.1 > next.1 && cur.0 != best_lag && cur.1 >= best_score - 0.02 {
            ambiguous = true;
        }
    }
    let b_start_best = (b_onset + best_lag) as f64;
    let offset_secs = (b_start_best + frac - a_begin as f64) / fs;
    Some(PulsePair {
        a_secs,
        b_secs,
        offset_ms: offset_secs * 1000.0,
        ncc: best_score,
        ambiguous,
    })
}

fn percentile_nearest_rank(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (q * sorted.len() as f64).ceil() as isize - 1;
    let idx = rank.clamp(0, sorted.len() as isize - 1) as usize;
    sorted[idx]
}

/// 测量两段录音的偏差与漂移（这是工具的入口）。
pub fn measure(
    a: &WavData,
    b: &WavData,
    detect: DetectConfig,
    thresholds: Thresholds,
) -> Result<SyncReport, SyncError> {
    if a.sample_rate != b.sample_rate {
        return Err(SyncError::SampleRateMismatch {
            a: a.sample_rate,
            b: b.sample_rate,
        });
    }
    let onsets_a = detect_pulse_onsets(&a.samples, a.sample_rate, detect).map_err(|e| match e {
        SyncError::NoSignal => SyncError::NoPulses { side: "a" },
        other => other,
    })?;
    let onsets_b = detect_pulse_onsets(&b.samples, b.sample_rate, detect).map_err(|e| match e {
        SyncError::NoSignal => SyncError::NoPulses { side: "b" },
        other => other,
    })?;
    let count = onsets_a.len().min(onsets_b.len());
    if count < 2 {
        return Err(SyncError::NotEnoughPulses { found: count });
    }
    let mut notes: Vec<String> = Vec::new();
    if onsets_a.len() != onsets_b.len() {
        notes.push(format!(
            "pulse counts differ (a={}, b={}); paired the first {count} in each recording",
            onsets_a.len(),
            onsets_b.len()
        ));
    }

    let mut pairs = Vec::with_capacity(count);
    for i in 0..count {
        if let Some(pair) = refine_pair(
            &a.samples,
            &b.samples,
            a.sample_rate,
            onsets_a[i],
            onsets_b[i],
            detect,
        ) {
            pairs.push(pair);
        }
    }
    if pairs.len() < 2 {
        return Err(SyncError::NotEnoughPulses { found: pairs.len() });
    }
    let ambiguous = pairs.iter().filter(|p| p.ambiguous).count();
    if ambiguous > 0 {
        notes.push(format!(
            "{ambiguous} of {} pairs have a competing correlation peak; the signal may be periodic (use a broadband click rather than a steady tone)",
            pairs.len()
        ));
    }

    let offsets: Vec<f64> = pairs.iter().map(|p| p.offset_ms).collect();
    let mean = offsets.iter().sum::<f64>() / offsets.len() as f64;
    let mut sorted = offsets.clone();
    sorted.sort_by(f64::total_cmp);
    let p50 = percentile_nearest_rank(&sorted, 0.5);
    let mut abs_sorted: Vec<f64> = offsets.iter().map(|v| v.abs()).collect();
    abs_sorted.sort_by(f64::total_cmp);
    let p95_abs = percentile_nearest_rank(&abs_sorted, 0.95);
    let max_abs = abs_sorted.last().copied().unwrap_or(0.0);

    // 漂移：偏差随 A 侧时刻的线性回归斜率（秒/秒 → ppm）
    let xs: Vec<f64> = pairs.iter().map(|p| p.a_secs).collect();
    let x_mean = xs.iter().sum::<f64>() / xs.len() as f64;
    let y_mean = mean / 1000.0;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    for (x, y) in xs.iter().zip(offsets.iter()) {
        let dx = x - x_mean;
        sxx += dx * dx;
        sxy += dx * (y / 1000.0 - y_mean);
    }
    let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    let drift_ppm = slope * 1e6;

    let mut exceeded = false;
    if mean.abs() > thresholds.max_bias_ms {
        exceeded = true;
        notes.push(format!(
            "mean offset {:.3} ms exceeds ±{:.3} ms",
            mean, thresholds.max_bias_ms
        ));
    }
    if p95_abs > thresholds.max_bias_ms {
        exceeded = true;
        notes.push(format!(
            "P95 |offset| {:.3} ms exceeds ±{:.3} ms",
            p95_abs, thresholds.max_bias_ms
        ));
    }
    if drift_ppm.abs() > thresholds.max_drift_ppm {
        exceeded = true;
        notes.push(format!(
            "drift {:.1} ppm exceeds ±{:.1} ppm",
            drift_ppm, thresholds.max_drift_ppm
        ));
    }
    let verdict = if exceeded {
        Verdict::Exceeded
    } else {
        Verdict::Within
    };

    Ok(SyncReport {
        sample_rate: a.sample_rate,
        a_frames: a.frames,
        b_frames: b.frames,
        a_pulses: onsets_a.len(),
        b_pulses: onsets_b.len(),
        pairs,
        mean_offset_ms: mean,
        p50_offset_ms: p50,
        p95_abs_offset_ms: p95_abs,
        max_abs_offset_ms: max_abs,
        drift_ppm,
        verdict,
        notes,
    })
}

/// 报告 JSON（字段一律 snake_case，与 tools 的其它报告一致）。
pub fn report_json(
    report: &SyncReport,
    a_path: &str,
    b_path: &str,
    thresholds: Thresholds,
) -> Value {
    json!({
        "a": a_path,
        "b": b_path,
        "sample_rate": report.sample_rate,
        "a_frames": report.a_frames,
        "b_frames": report.b_frames,
        "a_pulses": report.a_pulses,
        "b_pulses": report.b_pulses,
        "pairs": report.pairs.len(),
        "offsets_ms": report.pairs.iter().map(|p| json!({
            "a_secs": p.a_secs,
            "b_secs": p.b_secs,
            "offset_ms": p.offset_ms,
            "ncc": p.ncc,
            "ambiguous": p.ambiguous,
        })).collect::<Vec<_>>(),
        "mean_offset_ms": report.mean_offset_ms,
        "p50_offset_ms": report.p50_offset_ms,
        "p95_abs_offset_ms": report.p95_abs_offset_ms,
        "max_abs_offset_ms": report.max_abs_offset_ms,
        "drift_ppm": report.drift_ppm,
        "thresholds": {
            "max_bias_ms": thresholds.max_bias_ms,
            "max_drift_ppm": thresholds.max_drift_ppm,
        },
        "verdict": report.verdict.as_str(),
        "notes": report.notes,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const FS: u32 = 48_000;

    /// 合成一段「宽带脉冲串」录音：每 gap_ms 一个噪声突发（0.4 ms 指数衰减，总长 pulse_ms），
    /// 第 i 个脉冲可带自己的偏移（ms，用于模拟两台设备的偏差与漂移）。
    ///
    /// 为什么不用正弦：单一频率的长正弦自相关有**周期歧义** —— lag 差一个周期时互相关一样高，
    /// 精细对齐会被骗到隔壁峰上（本轮实测偏差因此散开约 1 ms）。宽带瞬态的自相关只有一个尖峰，
    /// 现实里的 click、音乐里的瞬态也都是这个形状。
    fn pulses(total_ms: u32, gap_ms: u32, pulse_ms: u32, amp: f32, offsets_ms: &[f64]) -> Vec<f32> {
        let total = (FS as f64 * f64::from(total_ms) / 1000.0) as usize;
        let mut out = vec![0.0f32; total];
        let pulse_len = (FS as f64 * f64::from(pulse_ms) / 1000.0) as usize;
        let decay = (0.0004 * f64::from(FS)).max(1.0);
        for (i, off) in offsets_ms.iter().enumerate() {
            let mut state = 0x9E37_79B9u32 ^ (i as u32).wrapping_mul(2_654_435_761);
            let start_ms = i as f64 * f64::from(gap_ms) + off;
            let start = (FS as f64 * start_ms / 1000.0).round().max(0.0) as usize;
            for k in 0..pulse_len {
                let idx = start + k;
                if idx >= out.len() {
                    break;
                }
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = f64::from(state >> 9) / 4_194_304.0 - 1.0;
                let env = (-(k as f64) / decay).exp();
                out[idx] += (f64::from(amp) * noise * env) as f32;
            }
        }
        out
    }

    fn detect() -> DetectConfig {
        DetectConfig::default()
    }

    fn wrap(samples: &[f32]) -> WavData {
        WavData {
            sample_rate: FS,
            channels: 1,
            frames: samples.len(),
            bits: 16,
            samples: samples.to_vec(),
        }
    }

    /// 造一个真实的 16-bit PCM 单声道 WAV 字节流（含一个奇数长度的 LIST 块，验证跳过与填充字节）。
    fn wav_bytes_16(samples: &[f32]) -> Vec<u8> {
        let data_len = samples.len() * 2;
        let list_len = 5usize;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        let riff_size = 4 + (8 + 16) + (8 + list_len + 1) + (8 + data_len);
        out.extend_from_slice(&(riff_size as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&FS.to_le_bytes());
        out.extend_from_slice(&(FS * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"LIST");
        out.extend_from_slice(&(list_len as u32).to_le_bytes());
        out.extend_from_slice(b"INFO!");
        out.push(0); // 奇数长度块的填充字节
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        for s in samples {
            let v = (f64::from(*s) * 32_768.0)
                .round()
                .clamp(-32_768.0, 32_767.0) as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    #[test]
    fn parses_16bit_pcm_and_skips_other_chunks() {
        let samples: Vec<f32> = (0..10).map(|i| (i as f32 - 5.0) / 10.0).collect();
        let bytes = wav_bytes_16(&samples);
        let wav = parse_wav(&bytes, 0).unwrap();
        assert_eq!(wav.sample_rate, FS);
        assert_eq!(wav.channels, 1);
        assert_eq!(wav.frames, 10);
        for (got, want) in wav.samples.iter().zip(samples.iter()) {
            assert!((f64::from(*got) - f64::from(*want)).abs() < 1e-3);
        }
    }

    #[test]
    fn rejects_malformed_wavs() {
        assert_eq!(parse_wav(b"not a wav at all!", 0), Err(WavError::NotRiff));
        let mut only_header = Vec::new();
        only_header.extend_from_slice(b"RIFF");
        only_header.extend_from_slice(&4u32.to_le_bytes());
        only_header.extend_from_slice(b"WAVE");
        assert_eq!(parse_wav(&only_header, 0), Err(WavError::MissingFmt));

        let bytes = wav_bytes_16(&[0.1, 0.2, 0.3]);
        assert_eq!(
            parse_wav(&bytes, 1),
            Err(WavError::ChannelOutOfRange {
                requested: 1,
                channels: 1
            })
        );

        // data 块声明的长度超过真实字节：按可用字节截到整帧，绝不 panic
        let mut broken = bytes.clone();
        let n = broken.len();
        broken[n - 10..n - 6].copy_from_slice(&9999u32.to_le_bytes());
        let wav = parse_wav(&broken, 0).unwrap();
        assert_eq!(wav.frames, 3);
    }

    #[test]
    fn detects_pulse_onsets_at_known_positions() {
        let samples = pulses(2000, 500, 20, 0.8, &[0.0; 4]);
        let onsets = detect_pulse_onsets(&samples, FS, detect()).unwrap();
        assert_eq!(onsets.len(), 4);
        for (i, onset) in onsets.iter().enumerate() {
            let want = i as f64 * 0.5;
            assert!((onset - want).abs() < 5e-4, "onset {i}: {onset} vs {want}");
        }
    }

    #[test]
    fn silent_recording_must_error_not_report_zero() {
        let silence = vec![0.0f32; FS as usize];
        assert_eq!(
            detect_pulse_onsets(&silence, FS, detect()),
            Err(SyncError::NoSignal)
        );
        // 纯直流的录音：去掉直流后一点能量都没有 → 同样必须报 NoSignal（不能靠直流骗过检测）
        let dc = vec![0.5f32; FS as usize];
        assert_eq!(
            detect_pulse_onsets(&dc, FS, detect()),
            Err(SyncError::NoSignal)
        );
    }

    #[test]
    fn measures_the_injected_offset() {
        // B 比 A 晚 7.3 ms（含亚采样小数，考验互相关插值）
        let a = pulses(3000, 500, 20, 0.8, &[0.0; 5]);
        let b = pulses(3000, 500, 20, 0.8, &[7.3; 5]);
        let report = measure(&wrap(&a), &wrap(&b), detect(), Thresholds::default()).unwrap();
        assert_eq!(report.pairs.len(), 5);
        assert!(
            (report.mean_offset_ms - 7.3).abs() < 0.05,
            "mean = {}",
            report.mean_offset_ms
        );
        assert!((report.p95_abs_offset_ms - 7.3).abs() < 0.05);
        assert!(
            report.drift_ppm.abs() < 20.0,
            "drift = {}",
            report.drift_ppm
        );
        assert_eq!(report.verdict, Verdict::Within);
    }

    #[test]
    fn negative_offset_means_b_is_early() {
        // A 晚 4 ms ⟺ B 早 4 ms（偏差符号约定：正 = B 晚）
        let a = pulses(2000, 500, 20, 0.8, &[4.0; 4]);
        let b = pulses(2000, 500, 20, 0.8, &[0.0; 4]);
        let report = measure(&wrap(&a), &wrap(&b), detect(), Thresholds::default()).unwrap();
        assert!(
            (report.mean_offset_ms + 4.0).abs() < 0.05,
            "mean = {}",
            report.mean_offset_ms
        );
    }

    #[test]
    fn detects_clock_drift_in_ppm() {
        // B 的脉冲间隔比 A 慢 200 ppm（偏差随时刻线性累积）
        let mut b_offsets = [0.0f64; 6];
        for (i, slot) in b_offsets.iter_mut().enumerate() {
            *slot = 500.0 * i as f64 * 200e-6;
        }
        let a = pulses(4000, 500, 20, 0.8, &[0.0; 6]);
        let b = pulses(4000, 500, 20, 0.8, &b_offsets);
        let report = measure(&wrap(&a), &wrap(&b), detect(), Thresholds::default()).unwrap();
        assert!(
            (report.drift_ppm - 200.0).abs() < 8.0,
            "drift = {}",
            report.drift_ppm
        );
        assert_eq!(report.verdict, Verdict::Within);
    }

    #[test]
    fn verdict_exceeds_when_bias_is_too_large() {
        let a = pulses(2000, 500, 20, 0.8, &[0.0; 4]);
        let b = pulses(2000, 500, 20, 0.8, &[25.0; 4]);
        let report = measure(&wrap(&a), &wrap(&b), detect(), Thresholds::default()).unwrap();
        assert_eq!(report.verdict, Verdict::Exceeded);
        assert!(report.notes.iter().any(|n| n.contains("mean offset")));
    }

    #[test]
    fn mismatched_sample_rates_are_rejected() {
        let a = wrap(&pulses(1000, 500, 20, 0.8, &[0.0; 2]));
        let mut b = wrap(&pulses(1000, 500, 20, 0.8, &[0.0; 2]));
        b.sample_rate = 44_100;
        assert_eq!(
            measure(&a, &b, detect(), Thresholds::default()),
            Err(SyncError::SampleRateMismatch {
                a: 48_000,
                b: 44_100
            })
        );
    }

    #[test]
    fn one_pulse_is_not_enough() {
        let a = wrap(&pulses(1000, 500, 20, 0.8, &[0.0; 1]));
        let b = wrap(&pulses(1000, 500, 20, 0.8, &[1.0; 1]));
        assert_eq!(
            measure(&a, &b, detect(), Thresholds::default()),
            Err(SyncError::NotEnoughPulses { found: 1 })
        );
    }

    #[test]
    fn quiet_recording_reports_which_side_failed() {
        let a = wrap(&pulses(1000, 500, 20, 0.8, &[0.0; 2]));
        let silent = wrap(&vec![0.0f32; FS as usize]);
        assert_eq!(
            measure(&a, &silent, detect(), Thresholds::default()),
            Err(SyncError::NoPulses { side: "b" })
        );
    }

    #[test]
    fn unrelated_recordings_error_instead_of_a_fake_offset() {
        // A 是脉冲串，B 是「同一长度、完全不相干」的伪随机噪声。
        // 允许两种结局：B 侧检测不到脉冲（NoPulses），或者配对全部因对齐质量太低被丢弃
        // （NotEnoughPulses）—— 但**绝不能**返回一个看起来正常的偏差。
        let a = pulses(2000, 500, 20, 0.8, &[0.0; 4]);
        let mut state = 0x1234_5678u32;
        let noise: Vec<f32> = (0..a.len())
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 9) as f32 / 8_388_608.0) * 2.0 - 1.0
            })
            .collect();
        let outcome = measure(&wrap(&a), &wrap(&noise), detect(), Thresholds::default());
        assert!(
            matches!(
                outcome,
                Err(SyncError::NoPulses { .. }) | Err(SyncError::NotEnoughPulses { .. })
            ),
            "unrelated recordings must not produce a report: {outcome:?}"
        );
    }

    #[test]
    fn report_json_is_self_consistent() {
        let a = pulses(2000, 500, 20, 0.8, &[0.0; 4]);
        let b = pulses(2000, 500, 20, 0.8, &[3.0; 4]);
        let report = measure(&wrap(&a), &wrap(&b), detect(), Thresholds::default()).unwrap();
        let value = report_json(&report, "a.wav", "b.wav", Thresholds::default());
        assert_eq!(value["pairs"], json!(4));
        assert_eq!(value["offsets_ms"].as_array().unwrap().len(), 4);
        assert_eq!(value["verdict"], json!("within"));
        assert_eq!(value["thresholds"]["max_bias_ms"], json!(10.0));
        let mean = value["mean_offset_ms"].as_f64().unwrap();
        assert!((mean - 3.0).abs() < 0.05, "mean = {mean}");
    }

    #[test]
    fn envelope_release_keeps_pulse_start_sharp() {
        let mut samples = vec![0.0f32; FS as usize];
        for (i, s) in samples.iter_mut().enumerate().take(480) {
            *s = if i < 240 { 0.9 } else { 0.0 };
        }
        let env = envelope(&samples, (-1.0f64 / (0.004 * f64::from(FS))).exp());
        let at_start = env[10];
        let peak = env.iter().copied().fold(0.0f64, f64::max);
        let after = env[(FS / 2) as usize];
        assert!(at_start > 0.5);
        // 慢释放不等于不释放：半个文件之后包络必须落到峰值的 1% 以下。
        // （不会落到 0 —— 去直流后残留的是 0.5% 量级的底噪，远低于触发阈值的一半。）
        assert!(
            after < 0.01 * peak,
            "envelope did not decay: {after} vs {peak}"
        );
    }
}
