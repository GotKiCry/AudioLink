//! WASAPI **loopback 采集**：在渲染端点上"听"该端点的输出（ADR-005）
//!
//! 关键实现点：
//! - 请求格式 = 设备**混音格式原样**（`autoconvert: false`）→ 不做静默重采样；
//! - 事件驱动共享模式（`StreamMode::EventsShared`）→ 靠事件句柄唤醒，不轮询；
//! - 每次唤醒把**所有已就绪的包**读干净（事件驱动下通常 1 包 = 1 个设备周期 = 10 ms）；
//! - 读缓冲与转换缓冲都在 `open()` 时一次分配好，`read()` 稳态零分配。

use std::time::{Duration, Instant};

use wasapi::{AudioCaptureClient, AudioClient, Direction, Handle, StreamMode, WasapiError};

use crate::error::AudioError;
use crate::format::{CHANNELS, DeviceFormat, convert_to_internal, require_unified_sample_rate};
use crate::source::{CaptureSource, CaptureStats, CapturedPacket};

use super::{
    DeviceSelector, RenderDeviceInfo, map_wasapi, ms_to_hns, open_render_device, to_device_format,
};

/// 渲染端点上的 loopback 采集源。
pub struct LoopbackCapture {
    client: AudioClient,
    capture: AudioCaptureClient,
    event: Handle,
    device: RenderDeviceInfo,
    format: DeviceFormat,
    requested_buffer_ms: u32,
    effective_buffer_ms: f64,
    buffer_frames: u32,
    bytes_per_frame: usize,
    byte_scratch: Vec<u8>,
    f32_scratch: Vec<f32>,
    stats: CaptureStats,
    last_read_at: Option<Instant>,
    running: bool,
}

impl LoopbackCapture {
    /// 打开默认（或指定）渲染端点的 loopback 采集并启动流。
    ///
    /// `buffer_ms` 是**请求**的缓冲时长；共享模式下音频引擎会把小于一个周期的请求抬到
    /// 1056 帧（22 ms，实测），实际生效值见 [`CaptureSource::effective_buffer_ms`]。
    pub fn open(selector: &DeviceSelector, buffer_ms: u32) -> Result<Self, AudioError> {
        if buffer_ms == 0 {
            return Err(AudioError::invalid_config("采集缓冲时长必须 ≥ 1 ms"));
        }
        let (device, info) = open_render_device(selector)?;
        let mut client = device
            .get_iaudioclient()
            .map_err(|error| map_wasapi(error, "获取 IAudioClient"))?;
        let mix = client
            .get_mixformat()
            .map_err(|error| map_wasapi(error, "读取混音格式"))?;
        let format = to_device_format(&mix)?;
        // 「零重采样」的前置条件：设备混音格式必须是 48 kHz（否则给出可操作提示，不静默 SRC）
        require_unified_sample_rate(format.sample_rate)?;
        let bytes_per_frame = format.bytes_per_frame();
        if bytes_per_frame == 0 {
            return Err(AudioError::device_unavailable("混音格式的块对齐为 0"));
        }

        client
            .initialize_client(
                &mix,
                &Direction::Capture,
                &StreamMode::EventsShared {
                    autoconvert: false,
                    buffer_duration_hns: ms_to_hns(buffer_ms),
                },
            )
            .map_err(|error| map_wasapi(error, "初始化 loopback 采集客户端"))?;
        let event = client
            .set_get_eventhandle()
            .map_err(|error| map_wasapi(error, "创建采集事件句柄"))?;
        let capture = client
            .get_audiocaptureclient()
            .map_err(|error| map_wasapi(error, "获取 IAudioCaptureClient"))?;
        let buffer_frames = client
            .get_buffer_size()
            .map_err(|error| map_wasapi(error, "读取采集缓冲大小"))?;
        let effective_buffer_ms =
            f64::from(buffer_frames) * 1_000.0 / f64::from(format.sample_rate);
        client
            .start_stream()
            .map_err(|error| map_wasapi(error, "启动采集流"))?;

        let byte_scratch = vec![0u8; buffer_frames as usize * bytes_per_frame];
        let f32_scratch = vec![0.0f32; buffer_frames as usize * CHANNELS as usize];
        Ok(Self {
            client,
            capture,
            event,
            device: info,
            format,
            requested_buffer_ms: buffer_ms,
            effective_buffer_ms,
            buffer_frames,
            bytes_per_frame,
            byte_scratch,
            f32_scratch,
            stats: CaptureStats::default(),
            last_read_at: None,
            running: true,
        })
    }

    /// 端点信息。
    pub fn device_info(&self) -> &RenderDeviceInfo {
        &self.device
    }

    /// 采集缓冲帧数（设备实际生效值）。
    pub fn buffer_frames(&self) -> u32 {
        self.buffer_frames
    }

    /// 设备周期（默认, 最小），单位 ms。
    pub fn device_period_ms(&self) -> (f64, f64) {
        (self.device.default_period_ms(), self.device.min_period_ms())
    }
}

impl CaptureSource for LoopbackCapture {
    fn device_format(&self) -> DeviceFormat {
        self.format
    }

    fn requested_buffer_ms(&self) -> u32 {
        self.requested_buffer_ms
    }

    fn effective_buffer_ms(&self) -> u32 {
        self.effective_buffer_ms.round() as u32
    }

    fn read(
        &mut self,
        dst: &mut Vec<f32>,
        timeout: Duration,
    ) -> Result<Option<CapturedPacket>, AudioError> {
        if !self.running {
            return Ok(None);
        }
        dst.clear();

        // 1) 等事件（设备周期级唤醒）
        let timeout_ms = timeout.as_millis().clamp(1, u32::MAX as u128) as u32;
        match self.event.wait_for_event(timeout_ms) {
            Ok(()) => {}
            Err(WasapiError::EventTimeout) => {
                self.stats.read_timeouts += 1;
                return Ok(None);
            }
            Err(error) => return Err(map_wasapi(error, "等待采集事件")),
        }

        // 2) 把就绪的包读干净
        let mut total_frames = 0usize;
        let mut first_timestamp: Option<i64> = None;
        let mut silent = false;
        let mut discontinuity = false;
        let mut timestamp_error = false;

        loop {
            let pending = self
                .capture
                .get_next_packet_size()
                .map_err(|error| map_wasapi(error, "查询待读包大小"))?;
            let frames = match pending {
                Some(frames) if frames > 0 => frames as usize,
                _ => break,
            };
            if frames > self.buffer_frames as usize {
                // 设备不会给超过缓冲的包；出现即视为异常，计数并停止本轮
                self.stats.discontinuities += 1;
                break;
            }
            let bytes = frames * self.bytes_per_frame;
            let slice = match self.byte_scratch.get_mut(..bytes) {
                Some(slice) => slice,
                None => break,
            };
            let (frames_read, info) = self
                .capture
                .read_from_device(slice)
                .map_err(|error| map_wasapi(error, "读取采集数据"))?;
            if frames_read == 0 {
                break;
            }
            let frames_read = frames_read as usize;
            if first_timestamp.is_none() && !info.flags.timestamp_error {
                first_timestamp = Some(info.timestamp as i64);
            }
            silent |= info.flags.silent;
            discontinuity |= info.flags.data_discontinuity;
            timestamp_error |= info.flags.timestamp_error;

            if info.flags.silent {
                // 设备声明静音：不信任缓冲内容，直接补零
                dst.extend(std::iter::repeat_n(0.0f32, frames_read * CHANNELS as usize));
            } else {
                let written = convert_to_internal(slice, self.format, &mut self.f32_scratch)?;
                let keep = written.min(frames_read * CHANNELS as usize);
                dst.extend_from_slice(&self.f32_scratch[..keep]);
            }
            total_frames += frames_read;
        }

        let now = Instant::now();
        if total_frames == 0 {
            self.stats.empty_wakeups += 1;
            return Ok(None);
        }

        // 3) 统计（节奏间隙 = 两次读出之间的墙钟间隔超过 2 个设备周期）
        let period = Duration::from_secs_f64(self.device.default_period_ms() / 1_000.0);
        if let Some(previous) = self.last_read_at
            && now.duration_since(previous) > period * 2
        {
            self.stats.gaps += 1;
        }
        self.last_read_at = Some(now);

        self.stats.packets += 1;
        self.stats.frames += total_frames as u64;
        if silent {
            self.stats.silent_packets += 1;
        }
        if discontinuity {
            self.stats.discontinuities += 1;
        }
        if timestamp_error {
            first_timestamp = None;
        }

        Ok(Some(CapturedPacket {
            frames: total_frames,
            device_timestamp_100ns: first_timestamp,
            read_at: now,
            silent,
            discontinuity,
        }))
    }

    fn stats(&self) -> CaptureStats {
        self.stats
    }

    fn stop(&mut self) {
        if self.running {
            self.running = false;
            let _ = self.client.stop_stream();
        }
    }

    fn backend_name(&self) -> &'static str {
        "wasapi-loopback"
    }
}

impl Drop for LoopbackCapture {
    fn drop(&mut self) {
        self.stop();
    }
}
