//! 桌面采集端点：只跨线程共享 ID 和信息，WASAPI 对象在采集线程内创建。

use std::sync::{Arc, Mutex};

use crate::error::CommandError;
use crate::view::CaptureDeviceView;

#[derive(Default)]
pub struct CaptureSelection {
    pub requested_id: Option<String>,
    pub opened: Option<CaptureDeviceView>,
}

pub type SharedCapture = Arc<Mutex<CaptureSelection>>;

pub async fn list_devices() -> Result<Vec<CaptureDeviceView>, CommandError> {
    // 枚举涉及 COM 与驱动，不占用 UI / Tokio 工作线程。
    tauri::async_runtime::spawn_blocking(enumerate_devices)
        .await
        .map_err(|error| {
            CommandError::bad_request("读取输出设备失败，请刷新重试", error.to_string())
        })?
}

#[cfg(windows)]
fn enumerate_devices() -> Result<Vec<CaptureDeviceView>, CommandError> {
    audiolink_audio::wasapi::list_render_devices()
        .map(|devices| devices.iter().map(device_view).collect())
        .map_err(|error| {
            CommandError::new(
                error.code(),
                "无法读取输出设备，请检查 Windows 声音设置后刷新",
                error.to_string(),
            )
        })
}

#[cfg(not(windows))]
fn enumerate_devices() -> Result<Vec<CaptureDeviceView>, CommandError> {
    Ok(Vec::new())
}

#[cfg(windows)]
fn device_view(info: &audiolink_audio::wasapi::RenderDeviceInfo) -> CaptureDeviceView {
    let unavailable_reason = if info.format.sample_rate != 48_000 {
        Some("请在 Windows 声音设置中将此设备改为 48000 Hz，然后刷新".into())
    } else if info.format.channels != 2 {
        Some("请在 Windows 声音设置中将此设备设为立体声，然后刷新".into())
    } else if info.format.sample_format != audiolink_audio::SampleFormat::F32 {
        Some("此设备的混音格式不支持当前采集模式，请选择其他输出设备".into())
    } else {
        None
    };
    CaptureDeviceView {
        id: info.id.clone(),
        name: info.name.clone(),
        is_default: info.is_default,
        is_virtual: info.looks_virtual(),
        sample_rate: info.format.sample_rate,
        channels: info.format.channels,
        unavailable_reason,
    }
}

/// 显式 ID 必须精确匹配；拔出的设备不允许退回默认端点（避免采错音源）。
pub fn validate_selection(
    devices: &[CaptureDeviceView],
    id: Option<&str>,
) -> Result<(), CommandError> {
    let device = devices
        .iter()
        .find(|device| match id {
            Some(id) => device.id == id,
            None => device.is_default,
        })
        .ok_or_else(|| {
            CommandError::new(
                audiolink_types::ErrorCode::CaptureLost,
                "所选输出设备不可用，请连接设备后刷新，或选择其他输出设备",
                format!(
                    "capture device unavailable: {}",
                    id.unwrap_or("system default")
                ),
            )
        })?;
    if let Some(reason) = &device.unavailable_reason {
        return Err(CommandError::new(
            audiolink_types::ErrorCode::CapUnsupported,
            reason,
            &device.id,
        ));
    }
    Ok(())
}

#[cfg(windows)]
pub fn factory(selection: SharedCapture) -> audiolink_engine::CaptureFactory {
    use audiolink_audio::wasapi::{DeviceSelector, LoopbackCapture};
    Arc::new(move || {
        let id = selection
            .lock()
            .map_err(|_| audiolink_audio::AudioError::stream_failed("采集设置不可读"))?
            .requested_id
            .clone();
        let selector = id
            .map(DeviceSelector::Id)
            .unwrap_or(DeviceSelector::Default);
        let capture = LoopbackCapture::open(&selector, 20)?;
        let info = capture.device_info();
        tracing::info!(device_name = %info.name, device_id = %info.id, format = %info.format_text, "loopback capture opened");
        selection
            .lock()
            .map_err(|_| audiolink_audio::AudioError::stream_failed("采集设置不可写"))?
            .opened = Some(device_view(info));
        Ok(Box::new(capture))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, is_default: bool) -> CaptureDeviceView {
        CaptureDeviceView {
            id: id.into(),
            name: "同名扬声器".into(),
            is_default,
            is_virtual: false,
            sample_rate: 48_000,
            channels: 2,
            unavailable_reason: None,
        }
    }

    #[test]
    fn missing_or_partial_ids_never_fall_back_to_default() {
        let devices = [device("endpoint-one", true), device("endpoint-two", false)];
        assert!(validate_selection(&devices, None).is_ok());
        assert!(validate_selection(&devices, Some("endpoint-two")).is_ok());
        for id in ["", "two", "同名扬声器", "disconnected"] {
            assert!(validate_selection(&devices, Some(id)).is_err());
        }
        assert!(validate_selection(&[], None).is_err());
    }

    #[test]
    fn unsupported_default_requires_explicit_selection() {
        let mut devices = [device("default", true), device("supported", false)];
        devices[0].unavailable_reason = Some("需要 48000 Hz".into());
        assert!(validate_selection(&devices, None).is_err());
        assert!(validate_selection(&devices, Some("default")).is_err());
        assert!(validate_selection(&devices, Some("supported")).is_ok());
    }

    /// 手工设备验收：不播放声音、不保存 PCM；CI 无声卡时不运行。
    #[cfg(windows)]
    #[test]
    #[ignore = "需要活动的 Windows 输出设备；显式运行以验证真实 WASAPI 选择"]
    #[allow(clippy::expect_used)]
    fn selected_endpoint_is_the_one_opened_by_wasapi() {
        let devices = enumerate_devices().expect("枚举活动端点");
        let supported: Vec<_> = devices
            .into_iter()
            .filter(|d| d.unavailable_reason.is_none())
            .collect();
        assert!(
            !supported.is_empty(),
            "需要至少一个 48 kHz / 2ch / f32 端点"
        );
        let state = SharedCapture::default();
        for device in supported {
            state.lock().expect("设置端点").requested_id = Some(device.id.clone());
            let make_capture = factory(Arc::clone(&state));
            let mut capture = make_capture().expect("打开选择的端点");
            let opened = state
                .lock()
                .expect("实际端点")
                .opened
                .clone()
                .expect("工厂报告端点");
            assert_eq!(opened.id, device.id);
            println!(
                "WASAPI selected={} opened={} name={}",
                device.id, opened.id, opened.name
            );
            capture.stop();
        }
        state.lock().expect("设置缺失端点").requested_id =
            Some("missing-capture-smoke-endpoint".into());
        assert!(
            factory(state)().is_err(),
            "不存在的端点必须失败，不能采默认设备"
        );
    }
}
