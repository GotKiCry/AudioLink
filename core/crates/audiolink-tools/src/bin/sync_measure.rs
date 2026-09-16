//! sync-measure：双机录音对齐工具（路线图 M3 交付物 5）。
//!
//! 用途：让两台接收端同时播放同一路音频（同步组），各自录下自己的输出，再把两段录音喂给本工具 ——
//! 它给出「组内偏差」的均值 / P50 / P95 / 极值，以及对两台设备晶振速率差的线性回归（ppm）。
//! 这就是 M3 验收表里「组内偏差 ≤ ±10 ms（P95）」的测具。
//!
//! 用法（方括号内为可选）：
//!
//!     sync-measure a.wav b.wav [--channel 0] [--threshold 0.5] [--min-gap-ms 5]
//!                  [--max-bias-ms 10] [--max-drift-ppm 200] [--json PATH] [--quiet]
//!
//! 参数：
//!
//! - a.wav / b.wav   两段同期录音（WAV，PCM 16/24/32 或 float 32/64；采样率必须一致）
//! - --channel       取第几路声道（默认 0）
//! - --threshold     脉冲触发阈值（占该路包络峰值的比例，默认 0.5）
//! - --min-gap-ms    两个脉冲的最小间隔（默认 5 ms）
//! - --max-bias-ms   偏差门限（默认 10 ms，M3 口径）
//! - --max-drift-ppm 漂移门限（默认 200 ppm）
//! - --json          报告路径（默认 target/evidence/sync/sync-measure-<unix 秒>.json）
//! - --quiet         只输出报告路径
//! - --self-test     不读文件：合成一对「已知偏差 + 已知漂移」的录音，验证测具本身量得准
//!
//! 退出码：0 达标 / 1 超标 / 2 用法错误 / 3 输入或检测失败（解析不了、检测不到脉冲、采样率不一致）。
//!
//! 约定：偏差为正 = B 比 A 晚；漂移为正 = B 的时钟比 A 快。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use audiolink_tools::syncmeasure::{
    DetectConfig, SyncError, Thresholds, Verdict, WavData, measure, parse_wav, report_json,
};

const USAGE: &str = "usage: sync-measure <a.wav> <b.wav> [--channel N] [--threshold R] \
[--min-gap-ms N] [--max-bias-ms F] [--max-drift-ppm F] [--json PATH] [--quiet] [--self-test [--write DIR]]";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(&args));
}

/// 解析参数并执行（返回退出码，方便单测直接调用）。
pub fn run(args: &[String]) -> i32 {
    let mut paths: Vec<String> = Vec::new();
    let mut detect = DetectConfig::default();
    let mut thresholds = Thresholds::default();
    let mut channel = 0usize;
    let mut json_path: Option<PathBuf> = None;
    let mut dump_dir: Option<PathBuf> = None;
    let mut self_test_requested = false;
    let mut quiet = false;

    let mut index = 0usize;
    while index < args.len() {
        let arg = args[index].clone();
        let mut take_value = |name: &str| -> Result<String, String> {
            let value = args.get(index + 1).cloned();
            match value {
                Some(v) => {
                    index += 1;
                    Ok(v)
                }
                None => Err(format!("{name} needs a value\n{USAGE}")),
            }
        };
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                return 0;
            }
            "--channel" => {
                match take_value("--channel").and_then(|v| parse_uint(&v, "--channel")) {
                    Ok(v) => channel = v as usize,
                    Err(e) => return usage_error(&e),
                }
            }
            "--threshold" => match take_value("--threshold").and_then(|v| parse_ratio(&v)) {
                Ok(v) => detect.threshold_ratio_x1000 = v,
                Err(e) => return usage_error(&e),
            },
            "--min-gap-ms" => {
                match take_value("--min-gap-ms").and_then(|v| parse_uint(&v, "--min-gap-ms")) {
                    Ok(v) => detect.min_gap_ms = v as u32,
                    Err(e) => return usage_error(&e),
                }
            }
            "--max-bias-ms" => {
                match take_value("--max-bias-ms").and_then(|v| parse_float(&v, "--max-bias-ms")) {
                    Ok(v) => thresholds.max_bias_ms = v,
                    Err(e) => return usage_error(&e),
                }
            }
            "--max-drift-ppm" => match take_value("--max-drift-ppm")
                .and_then(|v| parse_float(&v, "--max-drift-ppm"))
            {
                Ok(v) => thresholds.max_drift_ppm = v,
                Err(e) => return usage_error(&e),
            },
            "--json" => match take_value("--json") {
                Ok(v) => json_path = Some(PathBuf::from(v)),
                Err(e) => return usage_error(&e),
            },
            "--quiet" => quiet = true,
            "--self-test" => self_test_requested = true,
            "--write" => match take_value("--write") {
                Ok(v) => dump_dir = Some(PathBuf::from(v)),
                Err(e) => return usage_error(&e),
            },
            other if other.starts_with("--") => {
                return usage_error(&format!("unknown flag: {other}"));
            }
            other => paths.push(other.to_string()),
        }
        index += 1;
    }

    // --self-test 与 --write 的先后顺序不影响结果：解析完所有参数再决定
    if self_test_requested {
        return self_test(dump_dir);
    }

    if paths.len() != 2 {
        return usage_error(&format!(
            "expected exactly two recordings, got {}",
            paths.len()
        ));
    }
    analyse(
        &paths[0], &paths[1], channel, detect, thresholds, json_path, quiet,
    )
}

#[allow(clippy::too_many_arguments)]
fn analyse(
    a_path: &str,
    b_path: &str,
    channel: usize,
    detect: DetectConfig,
    thresholds: Thresholds,
    json_path: Option<PathBuf>,
    quiet: bool,
) -> i32 {
    let bytes_a = match fs::read(a_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {a_path}: {e}");
            return 3;
        }
    };
    let bytes_b = match fs::read(b_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {b_path}: {e}");
            return 3;
        }
    };
    let wav_a = match parse_wav(&bytes_a, channel) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{a_path}: {e}");
            return 3;
        }
    };
    let wav_b = match parse_wav(&bytes_b, channel) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{b_path}: {e}");
            return 3;
        }
    };

    let report = match measure(&wav_a, &wav_b, detect, thresholds) {
        Ok(r) => r,
        Err(e) => {
            let hint = match e {
                SyncError::SampleRateMismatch { .. } => "（两段录音请先统一采样率）",
                SyncError::NoPulses { .. } => "（该段录音里没有检测到脉冲，检查阈值与增益）",
                SyncError::NotEnoughPulses { .. } => "（脉冲太少，至少需要 2 个）",
                _ => "",
            };
            eprintln!("measurement failed: {e}{hint}");
            return 3;
        }
    };

    let json = report_json(&report, a_path, b_path, thresholds);
    let out_path = json_path.unwrap_or_else(default_report_path);
    if let Some(parent) = out_path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        eprintln!("cannot create {}: {e}", parent.display());
        return 3;
    }
    let text = match serde_json::to_string_pretty(&json) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot serialise report: {e}");
            return 3;
        }
    };
    if let Err(e) = fs::write(&out_path, text) {
        eprintln!("cannot write {}: {e}", out_path.display());
        return 3;
    }

    if !quiet {
        println!(
            "样本率 {} Hz，脉冲 a={} / b={}，配对 {} 对",
            report.sample_rate,
            report.a_pulses,
            report.b_pulses,
            report.pairs.len()
        );
        println!(
            "偏差 均值 {:+.3} ms / P50 {:+.3} ms / P95(|·|) {:.3} ms / 极值 {:.3} ms",
            report.mean_offset_ms,
            report.p50_offset_ms,
            report.p95_abs_offset_ms,
            report.max_abs_offset_ms
        );
        println!("漂移 {:+.1} ppm（正 = B 比 A 快）", report.drift_ppm);
        println!(
            "判定 {}（门限 ±{:.3} ms / ±{:.1} ppm）",
            report.verdict.as_str(),
            thresholds.max_bias_ms,
            thresholds.max_drift_ppm
        );
        for note in &report.notes {
            println!("  注：{note}");
        }
    }
    println!("报告：{}", out_path.display());

    match report.verdict {
        Verdict::Within => 0,
        Verdict::Exceeded => 1,
    }
}

/// 写一个 16-bit PCM 单声道 WAV（自检用；把合成出来的录音落成真实文件）。
fn write_wav_16(path: &Path, samples: &[f32], sample_rate: u32) -> std::io::Result<()> {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in samples {
        let v = (f64::from(*s) * 32_768.0)
            .round()
            .clamp(-32_768.0, 32_767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    fs::write(path, out)
}

/// 合成一段「宽带脉冲串」录音（自检用；与单测夹具同口径，但独立实现：测具不依赖被测量实现的内部）。
///
/// 用噪声突发而不是正弦：单一频率的长正弦互相关有周期歧义，会把亚采样对齐骗到隔壁周期上。
fn synth_pulses(
    sample_rate: u32,
    count: usize,
    gap_ms: f64,
    amp: f32,
    offsets_ms: &[f64],
) -> Vec<f32> {
    let fs = f64::from(sample_rate);
    let total_ms = count as f64 * gap_ms + 1000.0;
    let mut out = vec![0.0f32; (fs * total_ms / 1000.0) as usize];
    let pulse_len = (fs * 0.020) as usize;
    let decay = (0.0004 * fs).max(1.0);
    for (i, off) in offsets_ms.iter().enumerate().take(count) {
        let mut state = 0x9E37_79B9u32 ^ (i as u32).wrapping_mul(2_654_435_761);
        let start = (fs * (i as f64 * gap_ms + off) / 1000.0).round().max(0.0) as usize;
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

/// 自检：合成一对**已知真值**的录音，验证测具本身量得准。
///
/// 真值 = 4.7 ms 固定偏差 + 120 ppm 漂移（偏差随时刻线性累积，正是两台设备晶振速率不等的样子）。
/// 判据：量出的均值与真值差 < 0.1 ms、漂移差 < 10 ppm，否则退出码 1。
/// 只有它能过，这些数字才配拿去量真机 —— 量具本身也得先被量一遍。
fn self_test(dump_dir: Option<PathBuf>) -> i32 {
    const FS: u32 = 48_000;
    const TRUTH_OFFSET_MS: f64 = 4.7;
    const TRUTH_DRIFT_PPM: f64 = 120.0;
    const COUNT: usize = 6;
    const GAP_MS: f64 = 500.0;

    let mut b_offsets = [0.0f64; COUNT];
    for (i, slot) in b_offsets.iter_mut().enumerate() {
        *slot = TRUTH_OFFSET_MS + GAP_MS * i as f64 * TRUTH_DRIFT_PPM * 1e-6;
    }
    let a_samples = synth_pulses(FS, COUNT, GAP_MS, 0.8, &[0.0; COUNT]);
    let b_samples = synth_pulses(FS, COUNT, GAP_MS, 0.8, &b_offsets);
    // 给了 --write DIR 就先落成真实 WAV 文件再读回来：把「文件 → 解析 → 测量」整条路径真的走一遍
    let (a, b) = match dump_dir {
        Some(dir) => {
            if let Err(e) = fs::create_dir_all(&dir) {
                eprintln!("cannot create {}: {e}", dir.display());
                return 1;
            }
            let a_path = dir.join("self-test-a.wav");
            let b_path = dir.join("self-test-b.wav");
            let written = write_wav_16(&a_path, &a_samples, FS)
                .and_then(|()| write_wav_16(&b_path, &b_samples, FS));
            if let Err(e) = written {
                eprintln!("cannot write self-test recordings: {e}");
                return 1;
            }
            println!("已写出 {}", a_path.display());
            println!("已写出 {}", b_path.display());
            let read_back = |path: &Path| -> Result<WavData, String> {
                let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
                parse_wav(&bytes, 0).map_err(|e| format!("{}: {e}", path.display()))
            };
            match (read_back(&a_path), read_back(&b_path)) {
                (Ok(a), Ok(b)) => (a, b),
                (Err(e), _) | (_, Err(e)) => {
                    eprintln!("self-test failed to read the recordings back: {e}");
                    return 1;
                }
            }
        }
        None => (
            WavData {
                sample_rate: FS,
                channels: 1,
                frames: a_samples.len(),
                bits: 32,
                samples: a_samples,
            },
            WavData {
                sample_rate: FS,
                channels: 1,
                frames: b_samples.len(),
                bits: 32,
                samples: b_samples,
            },
        ),
    };
    let report = match measure(&a, &b, DetectConfig::default(), Thresholds::default()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("self-test failed: {e}");
            return 1;
        }
    };
    // 逐对比真值：第 i 个脉冲的真值偏移 = 固定偏差 + 漂移 × 该脉冲时刻。
    // 不能拿「偏差均值」去比固定偏差 —— 有漂移时均值天然等于「固定偏差 + 漂移 × 平均时刻」，
    // 那样比出来的不是测具误差，而是判据自己算错了。序号从 a_secs 反推（少几对也不会错位）。
    let mut worst_offset_error = 0.0f64;
    for pair in &report.pairs {
        let index = (pair.a_secs / (GAP_MS / 1000.0)).round();
        let truth_ms = TRUTH_OFFSET_MS + GAP_MS * index * TRUTH_DRIFT_PPM * 1e-6;
        let error = (pair.offset_ms - truth_ms).abs();
        if error > worst_offset_error {
            worst_offset_error = error;
        }
    }
    let drift_error = (report.drift_ppm - TRUTH_DRIFT_PPM).abs();
    println!(
        "自检：配对 {} 对（a={} / b={} 个脉冲）",
        report.pairs.len(),
        report.a_pulses,
        report.b_pulses
    );
    println!("  真值 偏差 {TRUTH_OFFSET_MS:+.3} ms / 漂移 {TRUTH_DRIFT_PPM:+.1} ppm");
    println!(
        "  量出 偏差 {:+.3} ms / 漂移 {:+.1} ppm（P95 {:.3} ms）",
        report.mean_offset_ms, report.drift_ppm, report.p95_abs_offset_ms
    );
    println!(
        "  误差 单对最大 {worst_offset_error:.3} ms（判据 < 0.05）/ 漂移 {drift_error:.1} ppm（判据 < 10）"
    );
    if worst_offset_error < 0.05 && drift_error < 10.0 {
        println!("自检通过");
        0
    } else {
        eprintln!("self-test failed: measurement error too large");
        1
    }
}

fn default_report_path() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Path::new("target")
        .join("evidence")
        .join("sync")
        .join(format!("sync-measure-{stamp}.json"))
}

fn usage_error(message: &str) -> i32 {
    eprintln!("{message}\n{USAGE}");
    2
}

fn parse_uint(value: &str, name: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{name} expects a non-negative integer, got {value}"))
}

fn parse_ratio(value: &str) -> Result<u32, String> {
    let parsed = value
        .parse::<f64>()
        .map_err(|_| format!("--threshold expects a number, got {value}"))?;
    if !(0.01..=0.99).contains(&parsed) {
        return Err(format!(
            "--threshold must be between 0.01 and 0.99, got {value}"
        ));
    }
    Ok((parsed * 1000.0).round() as u32)
}

fn parse_float(value: &str, name: &str) -> Result<f64, String> {
    let parsed = value
        .parse::<f64>()
        .map_err(|_| format!("{name} expects a number, got {value}"))?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return Err(format!("{name} must be a positive number, got {value}"));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const FS: u32 = 48_000;

    fn pulses(total_ms: u32, gap_ms: u32, offsets_ms: &[f64]) -> Vec<f32> {
        let total = (FS as f64 * f64::from(total_ms) / 1000.0) as usize;
        let mut out = vec![0.0f32; total];
        let pulse_len = (FS as f64 * 0.020) as usize;
        for (i, off) in offsets_ms.iter().enumerate() {
            let start_ms = i as f64 * f64::from(gap_ms) + off;
            let start = (FS as f64 * start_ms / 1000.0).round().max(0.0) as usize;
            for k in 0..pulse_len {
                let idx = start + k;
                if idx >= out.len() {
                    break;
                }
                let t = k as f64 / FS as f64;
                let fade = (k as f64 / (0.0002 * FS as f64)).min(1.0);
                out[idx] += (0.8 * fade * (2.0 * std::f64::consts::PI * 1000.0 * t).sin()) as f32;
            }
        }
        out
    }

    fn write_wav(path: &Path, samples: &[f32]) {
        let mut out = Vec::new();
        let data_len = samples.len() * 2;
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&FS.to_le_bytes());
        out.extend_from_slice(&(FS * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        for s in samples {
            let v = (f64::from(*s) * 32_768.0)
                .round()
                .clamp(-32_768.0, 32_767.0) as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
        fs::write(path, out).unwrap();
    }

    /// 造一对临时录音，返回 (a 路径, b 路径, 临时目录句柄)。
    fn fixture(offset_ms: f64) -> (PathBuf, PathBuf, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.wav");
        let b = dir.path().join("b.wav");
        write_wav(&a, &pulses(3000, 500, &[0.0; 5]));
        write_wav(&b, &pulses(3000, 500, &[offset_ms; 5]));
        (a, b, dir)
    }

    #[test]
    fn within_tolerance_exits_zero_and_writes_report() {
        let (a, b, dir) = fixture(4.0);
        let json = dir.path().join("out.json");
        let args = vec![
            a.to_string_lossy().to_string(),
            b.to_string_lossy().to_string(),
            "--json".to_string(),
            json.to_string_lossy().to_string(),
            "--quiet".to_string(),
        ];
        assert_eq!(run(&args), 0);
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(value["verdict"], "within");
        assert!((value["mean_offset_ms"].as_f64().unwrap() - 4.0).abs() < 0.05);
    }

    #[test]
    fn exceeding_tolerance_exits_one() {
        let (a, b, dir) = fixture(25.0);
        let json = dir.path().join("out.json");
        let args = vec![
            a.to_string_lossy().to_string(),
            b.to_string_lossy().to_string(),
            "--json".to_string(),
            json.to_string_lossy().to_string(),
            "--quiet".to_string(),
        ];
        assert_eq!(run(&args), 1);
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(value["verdict"], "exceeded");
    }

    #[test]
    fn usage_errors_exit_two() {
        assert_eq!(run(&[]), 2);
        assert_eq!(run(&["only-one.wav".to_string()]), 2);
        assert_eq!(
            run(&[
                "a.wav".to_string(),
                "b.wav".to_string(),
                "--nope".to_string()
            ]),
            2
        );
        assert_eq!(
            run(&[
                "a.wav".to_string(),
                "b.wav".to_string(),
                "--threshold".to_string(),
                "1.5".to_string()
            ]),
            2
        );
    }

    #[test]
    fn missing_or_silent_input_exits_three() {
        let missing = vec![
            "no-such-file-a.wav".to_string(),
            "no-such-file-b.wav".to_string(),
            "--quiet".to_string(),
        ];
        assert_eq!(run(&missing), 3);

        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.wav");
        let b = dir.path().join("b.wav");
        write_wav(&a, &vec![0.0f32; FS as usize]);
        write_wav(&b, &vec![0.0f32; FS as usize]);
        let silent = vec![
            a.to_string_lossy().to_string(),
            b.to_string_lossy().to_string(),
            "--quiet".to_string(),
        ];
        assert_eq!(run(&silent), 3);
    }
}
