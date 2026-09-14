//! Windows 平台实现：WASAPI loopback 采集与 render 输出（ADR-005）
//!
//! # 本机实测基线（2026-09-14；四个活动渲染端点：Realtek / 网易虚拟音频设备 / NVIDIA HDMI / AudioRelay 虚拟）
//!
//! | 事实 | 实测值 | 对设计的影响 |
//! |---|---|---|
//! | 设备周期 `get_device_period` | 默认 **10 ms**、最小 **3 ms** | 事件驱动下**读出粒度 = 周期**，不是缓冲大小 |
//! | 请求缓冲 1–20 ms | `GetBufferSize` 一律 **1056 帧 = 22 ms** | 共享模式拿不到 10–20 ms 的**缓冲**；能压的是读出口粒度（10 ms），M1 风险清单据此更新 |
//! | 请求缓冲 ≥ 30 ms | 按请求值生效（30/50/100 ms 精确） | 需要更大缓冲时可信 |
//! | 每包帧数 | 480 帧（= 1 个周期） | 采集侧固有粒度 10 ms |
//! | 混音格式 | 四个端点均 **48000 Hz / 2ch / f32** | 「零重采样」成立的前提（若是 44.1 kHz 必须让用户改设备设置，见 [`crate::format::require_unified_sample_rate`]） |
//! | `autoconvert` | 44.1 kHz 且 `autoconvert=false` → `0x88890008`；`true` 会做 SRC | **本项目禁用** autoconvert：静默重采样会让 M1「零重采样」验收失去意义 |
//! | NVIDIA HDMI 端点 | 2 s 内 **0 包**（端点未播放） | loopback 只在端点播放时交付数据 → 默认输出若是 HDMI 且无播放，要诊断提示 |
//! | 独占模式 | Realtek 上 `EventsExclusive` 报 `0x88890008` | 独占路径不可用，v1 不依赖它 |
//!
//! # 线程与 COM
//!
//! WASAPI 对象活在 COM 里：**每个触碰它们的线程都要先进 MTA**（[`init_thread_mta`]）。
//! 采集线程与工具线程是不同线程，因此两边都要调用（重复调用是幂等的）。

pub mod capture;
pub mod render;

pub use capture::LoopbackCapture;
pub use render::RenderSink;

use wasapi::{Device, DeviceEnumerator, DeviceState, Direction, SampleType, WaveFormat};

use crate::error::AudioError;
use crate::format::{DeviceFormat, SampleFormat};

/// 设备选择器。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DeviceSelector {
    /// 默认渲染端点（`eConsole` 角色）。
    #[default]
    Default,
    /// 实例 ID：完整 `{0.0.0.00000000}.{guid}`、`{guid}` 或它们的唯一子串。
    Id(String),
    /// 友好名（精确匹配）。
    Name(String),
}

impl DeviceSelector {
    /// 从命令行文本解析：`default`（或空）→ 默认端点；`id:xxx` / `name:xxx` 显式指定；
    /// 其它文本先当 ID 子串匹配，匹配不到再当名字。
    pub fn parse(text: &str) -> Self {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("default") {
            return Self::Default;
        }
        if let Some(rest) = trimmed.strip_prefix("id:") {
            return Self::Id(rest.trim().to_string());
        }
        if let Some(rest) = trimmed.strip_prefix("name:") {
            return Self::Name(rest.trim().to_string());
        }
        Self::Id(trimmed.to_string())
    }

    /// 人类可读描述。
    pub fn describe(&self) -> String {
        match self {
            Self::Default => "默认渲染端点".to_string(),
            Self::Id(id) => format!("端点 ID≈{id}"),
            Self::Name(name) => format!("端点名={name}"),
        }
    }
}

/// 渲染端点信息（报告与诊断用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderDeviceInfo {
    /// 友好名（用户可见）。
    pub name: String,
    /// 唯一实例 ID（`{0.0.0.00000000}.{guid}`）。
    pub id: String,
    /// 是否为默认渲染端点。
    pub is_default: bool,
    /// 混音格式（共享模式下必须原样请求）。
    pub format: DeviceFormat,
    /// 混音格式的人类可读描述。
    pub format_text: String,
    /// 设备周期（默认, 最小），单位 100 ns。
    pub period_hns: (i64, i64),
}

impl RenderDeviceInfo {
    /// 默认周期（ms）。
    pub fn default_period_ms(&self) -> f64 {
        self.period_hns.0 as f64 / 10_000.0
    }

    /// 最小周期（ms）。
    pub fn min_period_ms(&self) -> f64 {
        self.period_hns.1 as f64 / 10_000.0
    }

    /// `{guid}` 形式短 ID（多设备同名时用来区分）。
    pub fn short_id(&self) -> String {
        match self.id.rfind('{') {
            Some(index) => self.id.get(index..).unwrap_or(&self.id).to_string(),
            None => self.id.clone(),
        }
    }

    /// 是否疑似虚拟声卡（ADR-005 的本机警示：默认输出若是虚拟设备会采到空流）。
    pub fn looks_virtual(&self) -> bool {
        let lower = self.name.to_lowercase();
        lower.contains("virtual") || self.name.contains("虚拟")
    }

    /// 一行摘要。
    pub fn summary(&self) -> String {
        format!(
            "{}{} [{}] {}（周期 默认 {:.1} ms / 最小 {:.1} ms）",
            self.name,
            if self.is_default { "（默认）" } else { "" },
            self.short_id(),
            self.format_text,
            self.default_period_ms(),
            self.min_period_ms()
        )
    }
}

/// 在所有触碰 WASAPI 对象的线程上调用一次（COM MTA；重复调用幂等）。
pub fn init_thread_mta() -> Result<(), AudioError> {
    let result = wasapi::initialize_mta();
    if result.is_ok() {
        Ok(())
    } else {
        Err(AudioError::stream_failed_owned(format!(
            "COM 初始化失败（initialize_mta）：HRESULT 0x{:08X}",
            result.0
        )))
    }
}

/// 枚举活动渲染端点；默认端点排在最前。
pub fn list_render_devices() -> Result<Vec<RenderDeviceInfo>, AudioError> {
    init_thread_mta()?;
    let enumerator =
        DeviceEnumerator::new().map_err(|error| map_wasapi(error, "创建设备枚举器"))?;
    let default_id = enumerator
        .get_default_device(&Direction::Render)
        .ok()
        .and_then(|device| device.get_id().ok());
    let collection = enumerator
        .get_device_collection(&Direction::Render)
        .map_err(|error| map_wasapi(error, "枚举渲染端点"))?;
    let count = collection
        .get_nbr_devices()
        .map_err(|error| map_wasapi(error, "取渲染端点数量"))?;
    let mut devices = Vec::with_capacity(count as usize);
    for index in 0..count {
        let device = match collection.get_device_at_index(index) {
            Ok(device) => device,
            Err(_) => continue,
        };
        match describe_device(&device, default_id.as_deref()) {
            Ok(info) => devices.push(info),
            Err(_) => continue, // 非活动/格式不可读的端点直接跳过（报告里由数量差异体现）
        }
    }
    devices.sort_by_key(|info| !info.is_default);
    Ok(devices)
}

/// 按选择器打开渲染端点。
pub(crate) fn open_render_device(
    selector: &DeviceSelector,
) -> Result<(Device, RenderDeviceInfo), AudioError> {
    init_thread_mta()?;
    let enumerator =
        DeviceEnumerator::new().map_err(|error| map_wasapi(error, "创建设备枚举器"))?;
    let default_id = enumerator
        .get_default_device(&Direction::Render)
        .ok()
        .and_then(|device| device.get_id().ok());

    let device = match selector {
        DeviceSelector::Default => enumerator
            .get_default_device(&Direction::Render)
            .map_err(|error| map_wasapi(error, "取默认渲染端点"))?,
        DeviceSelector::Id(needle) => {
            let collection = enumerator
                .get_device_collection(&Direction::Render)
                .map_err(|error| map_wasapi(error, "枚举渲染端点"))?;
            let count = collection
                .get_nbr_devices()
                .map_err(|error| map_wasapi(error, "取渲染端点数量"))?;
            let mut found: Option<Device> = None;
            for index in 0..count {
                let candidate = match collection.get_device_at_index(index) {
                    Ok(device) => device,
                    Err(_) => continue,
                };
                if let Ok(id) = candidate.get_id()
                    && (id == *needle
                        || id.ends_with(needle.as_str())
                        || id.contains(needle.as_str()))
                {
                    found = Some(candidate);
                    break;
                }
            }
            found.ok_or_else(|| {
                AudioError::device_unavailable_owned(format!(
                    "没有渲染端点的实例 ID 匹配 {needle:?}"
                ))
            })?
        }
        DeviceSelector::Name(name) => {
            let collection = enumerator
                .get_device_collection(&Direction::Render)
                .map_err(|error| map_wasapi(error, "枚举渲染端点"))?;
            let count = collection
                .get_nbr_devices()
                .map_err(|error| map_wasapi(error, "取渲染端点数量"))?;
            let mut found: Option<Device> = None;
            for index in 0..count {
                let candidate = match collection.get_device_at_index(index) {
                    Ok(device) => device,
                    Err(_) => continue,
                };
                if let Ok(friendly) = candidate.get_friendlyname()
                    && friendly == *name
                {
                    found = Some(candidate);
                    break;
                }
            }
            found.ok_or_else(|| {
                AudioError::device_unavailable_owned(format!("没有渲染端点的名字等于 {name:?}"))
            })?
        }
    };

    let info = describe_device(&device, default_id.as_deref())?;
    Ok((device, info))
}

/// 读取设备信息（要求端点处于 Active）。
fn describe_device(
    device: &Device,
    default_id: Option<&str>,
) -> Result<RenderDeviceInfo, AudioError> {
    let state = device
        .get_state()
        .map_err(|error| map_wasapi(error, "读取端点状态"))?;
    if state != DeviceState::Active {
        return Err(AudioError::device_unavailable_owned(format!(
            "端点不是 Active（实际 {state}）"
        )));
    }
    let name = device
        .get_friendlyname()
        .map_err(|error| map_wasapi(error, "读取端点友好名"))?;
    let id = device
        .get_id()
        .map_err(|error| map_wasapi(error, "读取端点实例 ID"))?;
    let client = device
        .get_iaudioclient()
        .map_err(|error| map_wasapi(error, "获取 IAudioClient"))?;
    let mix = client
        .get_mixformat()
        .map_err(|error| map_wasapi(error, "读取混音格式"))?;
    let period_hns = client
        .get_device_period()
        .map_err(|error| map_wasapi(error, "读取设备周期"))?;
    let is_default = default_id.map(|value| value == id).unwrap_or(false);
    Ok(RenderDeviceInfo {
        name,
        id,
        is_default,
        format: to_device_format(&mix)?,
        format_text: describe_format(&mix),
        period_hns,
    })
}

/// `WaveFormat` → 内核的 [`DeviceFormat`]。
pub(crate) fn to_device_format(wave: &WaveFormat) -> Result<DeviceFormat, AudioError> {
    let sample_type = wave
        .get_subformat()
        .map_err(|error| map_wasapi(error, "读取子格式"))?;
    let store_bits = wave.get_bitspersample();
    let valid_bits = wave.get_validbitspersample();
    let sample_format = match (sample_type, store_bits) {
        (SampleType::Float, 32) => SampleFormat::F32,
        (SampleType::Int, 8) => SampleFormat::U8,
        (SampleType::Int, 16) => SampleFormat::I16,
        (SampleType::Int, 24) => SampleFormat::I24,
        (SampleType::Int, 32) => SampleFormat::I32,
        (kind, bits) => {
            return Err(AudioError::device_unavailable_owned(format!(
                "不支持的样本格式：{kind:?} / {bits} bit（存储位宽）"
            )));
        }
    };
    Ok(DeviceFormat {
        sample_rate: wave.get_samplespersec(),
        channels: wave.get_nchannels(),
        sample_format,
        valid_bits: if valid_bits == 0 {
            store_bits
        } else {
            valid_bits
        },
    })
}

/// 人类可读的 `WaveFormat` 描述（报告用；与 `docs/04-tech-stack.md` 的写法一致）。
pub(crate) fn describe_format(wave: &WaveFormat) -> String {
    let kind = match wave.get_subformat() {
        Ok(SampleType::Float) => "float",
        Ok(SampleType::Int) => "int",
        Err(_) => "unknown",
    };
    format!(
        "{} Hz / {} ch / {}-bit store (valid {}) / {}",
        wave.get_samplespersec(),
        wave.get_nchannels(),
        wave.get_bitspersample(),
        wave.get_validbitspersample(),
        kind
    )
}

/// 把 `WasapiError` 映射到内核错误（保留原始信息，便于诊断）。
pub(crate) fn map_wasapi(error: wasapi::WasapiError, what: &str) -> AudioError {
    use wasapi::WasapiError;
    match error {
        WasapiError::DeviceNotFound(name) => {
            AudioError::device_unavailable_owned(format!("找不到端点：{name}"))
        }
        WasapiError::UnsupportedFormat => AudioError::device_unavailable_owned(format!(
            "{what}：设备不接受该格式（共享模式必须使用设备混音格式，且本项目不使用 autoconvert）"
        )),
        WasapiError::LoopbackWithExclusiveMode => {
            AudioError::invalid_config("loopback 采集不能用独占模式")
        }
        other => AudioError::stream_failed_owned(format!("{what}：{other}")),
    }
}

/// 缓冲时长（ms）→ 100 ns 单位（WASAPI 的 `hns` 口径）。
pub(crate) const fn ms_to_hns(ms: u32) -> i64 {
    ms as i64 * 10_000
}
