//! WASAPI **render 输出**：事件驱动共享模式 + 显式缓冲（ADR-005）
//!
//! 关键实现点：
//! - 启动前**预填静音**（`BufferFlags{silent:true}`）—— 否则 `Start` 之后会先欠载一轮，
//!   而「启动即出声」是 M1 的验收项之一；
//! - 写入前先看 `get_available_space_in_frames`：空间不足就等事件，**不忙等**；
//! - 水位（`buffered_frames`）就是延迟分解里「播放缓冲」一段的实测值；
//! - 缓冲被抽干（可用空间 = 整个缓冲）即记一次欠载 —— 欠载是统计项，不抛断流错误。

use std::time::{Duration, Instant};

use wasapi::{
    AudioClient, AudioRenderClient, BufferFlags, Direction, Handle, StreamMode, WasapiError,
};

use crate::error::AudioError;
use crate::format::{CHANNELS, DeviceFormat, convert_from_internal, require_unified_sample_rate};
use crate::sink::{PlayoutSink, PlayoutStats};

use super::{
    DeviceSelector, RenderDeviceInfo, map_wasapi, ms_to_hns, open_render_device, to_device_format,
};

/// 写入时等待空间的硬上限（超过即判欠载并让调用方决策）。
const WRITE_WAIT_LIMIT: Duration = Duration::from_millis(200);

/// 事件驱动 render 输出。
pub struct RenderSink {
    client: AudioClient,
    render: AudioRenderClient,
    event: Handle,
    device: RenderDeviceInfo,
    format: DeviceFormat,
    requested_buffer_ms: u32,
    effective_buffer_ms: f64,
    buffer_frames: u32,
    bytes_per_frame: usize,
    byte_scratch: Vec<u8>,
    stats: PlayoutStats,
    running: bool,
}

impl RenderSink {
    /// 打开（默认或指定）渲染端点：初始化 → 预填静音 → 启动。
    pub fn open(selector: &DeviceSelector, buffer_ms: u32) -> Result<Self, AudioError> {
        if buffer_ms == 0 {
            return Err(AudioError::invalid_config("播放缓冲时长必须 ≥ 1 ms"));
        }
        let (device, info) = open_render_device(selector)?;
        let mut client = device
            .get_iaudioclient()
            .map_err(|error| map_wasapi(error, "获取 IAudioClient"))?;
        let mix = client
            .get_mixformat()
            .map_err(|error| map_wasapi(error, "读取混音格式"))?;
        let format = to_device_format(&mix)?;
        require_unified_sample_rate(format.sample_rate)?;
        let bytes_per_frame = format.bytes_per_frame();
        if bytes_per_frame == 0 {
            return Err(AudioError::device_unavailable("混音格式的块对齐为 0"));
        }
        client
            .initialize_client(
                &mix,
                &Direction::Render,
                &StreamMode::EventsShared {
                    autoconvert: false,
                    buffer_duration_hns: ms_to_hns(buffer_ms),
                },
            )
            .map_err(|error| map_wasapi(error, "初始化 render 客户端"))?;
        let event = client
            .set_get_eventhandle()
            .map_err(|error| map_wasapi(error, "创建播放事件句柄"))?;
        let render = client
            .get_audiorenderclient()
            .map_err(|error| map_wasapi(error, "获取 IAudioRenderClient"))?;
        let buffer_frames = client
            .get_buffer_size()
            .map_err(|error| map_wasapi(error, "读取播放缓冲大小"))?;

        // 启动前填满静音：避免 Start 之后第一轮直接欠载
        let silence = vec![0u8; buffer_frames as usize * bytes_per_frame];
        render
            .write_to_device(
                buffer_frames as usize,
                &silence,
                Some(BufferFlags {
                    data_discontinuity: false,
                    silent: true,
                    timestamp_error: false,
                }),
            )
            .map_err(|error| map_wasapi(error, "预填静音"))?;
        client
            .start_stream()
            .map_err(|error| map_wasapi(error, "启动播放流"))?;

        let byte_scratch = vec![0u8; buffer_frames as usize * bytes_per_frame];
        let effective_buffer_ms =
            f64::from(buffer_frames) * 1_000.0 / f64::from(format.sample_rate);
        Ok(Self {
            client,
            render,
            event,
            device: info,
            format,
            requested_buffer_ms: buffer_ms,
            effective_buffer_ms,
            buffer_frames,
            bytes_per_frame,
            byte_scratch,
            stats: PlayoutStats::default(),
            running: true,
        })
    }

    /// 端点信息。
    pub fn device_info(&self) -> &RenderDeviceInfo {
        &self.device
    }

    /// 播放缓冲帧数（设备实际生效值）。
    pub fn buffer_frames(&self) -> u32 {
        self.buffer_frames
    }

    /// 设备周期（默认, 最小），单位 ms。
    pub fn device_period_ms(&self) -> (f64, f64) {
        (self.device.default_period_ms(), self.device.min_period_ms())
    }

    /// 当前可用空间（帧）。
    pub fn available_frames(&self) -> Result<u32, AudioError> {
        self.client
            .get_available_space_in_frames()
            .map_err(|error| map_wasapi(error, "读取播放可用空间"))
    }

    /// 等一次播放事件（供调用方在写入之间对齐节奏）。
    pub fn wait_for_space(&mut self, timeout: Duration) -> Result<(), AudioError> {
        let timeout_ms = timeout.as_millis().clamp(1, u32::MAX as u128) as u32;
        match self.event.wait_for_event(timeout_ms) {
            Ok(()) => Ok(()),
            Err(WasapiError::EventTimeout) => Ok(()), // 超时不是错误：调用方自行决定
            Err(error) => Err(map_wasapi(error, "等待播放事件")),
        }
    }
}

impl PlayoutSink for RenderSink {
    fn device_format(&self) -> DeviceFormat {
        self.format
    }

    fn requested_buffer_ms(&self) -> u32 {
        self.requested_buffer_ms
    }

    fn effective_buffer_ms(&self) -> u32 {
        self.effective_buffer_ms.round() as u32
    }

    fn buffered_frames(&mut self) -> u32 {
        match self.client.get_available_space_in_frames() {
            Ok(available) => self.buffer_frames.saturating_sub(available),
            Err(_) => 0,
        }
    }

    fn write(&mut self, samples: &[f32]) -> Result<(), AudioError> {
        if !self.running {
            return Ok(());
        }
        let frames = samples.len() / CHANNELS as usize;
        if frames == 0 {
            return Ok(());
        }
        let needed_bytes = frames * self.bytes_per_frame;
        if needed_bytes > self.byte_scratch.len() {
            return Err(AudioError::invalid_config_owned(format!(
                "一次写入 {frames} 帧超过播放缓冲 {} 帧",
                self.buffer_frames
            )));
        }

        // 等空间：空间不足（含缓冲被抽干）都靠事件推进，不忙等
        let deadline = Instant::now() + WRITE_WAIT_LIMIT;
        loop {
            let available = self.available_frames()?;
            if available == self.buffer_frames {
                // 整个缓冲都空着 → 已经欠载
                self.stats.underruns += 1;
            }
            if available as usize >= frames {
                break;
            }
            if Instant::now() >= deadline {
                self.stats.underruns += 1;
                self.stats.silent_padding_frames += frames as u64;
                return Err(AudioError::underrun_owned(format!(
                    "播放缓冲 {} ms 内没有腾出 {frames} 帧空间（可用 {available} 帧）",
                    WRITE_WAIT_LIMIT.as_millis()
                )));
            }
            let wait = self.device.default_period_ms() / 1_000.0;
            self.wait_for_space(Duration::from_secs_f64(wait))?;
        }

        let slice = match self.byte_scratch.get_mut(..needed_bytes) {
            Some(slice) => slice,
            None => return Err(AudioError::invalid_config("播放暂存缓冲不足")),
        };
        convert_from_internal(samples, self.format, slice)?;
        match self.render.write_to_device(frames, slice, None) {
            Ok(()) => {
                self.stats.writes += 1;
                self.stats.frames_written += frames as u64;
                Ok(())
            }
            Err(WasapiError::DataLengthMismatch { received, expected }) => {
                self.stats.write_errors += 1;
                Err(AudioError::stream_failed_owned(format!(
                    "写入长度不匹配：收到 {received} B，期望 {expected} B"
                )))
            }
            Err(error) => {
                self.stats.write_errors += 1;
                Err(map_wasapi(error, "写入播放数据"))
            }
        }
    }

    fn stats(&self) -> PlayoutStats {
        self.stats
    }

    fn stop(&mut self) {
        if self.running {
            self.running = false;
            let _ = self.client.stop_stream();
        }
    }

    fn backend_name(&self) -> &'static str {
        "wasapi-render"
    }
}

impl Drop for RenderSink {
    fn drop(&mut self) {
        self.stop();
    }
}
