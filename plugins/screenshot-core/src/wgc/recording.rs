use super::*;
use crate::model::ScreenRect;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::{
    core::BOOL,
    Win32::{
        Foundation::{HMODULE, LPARAM, RECT},
        Graphics::{
            Direct3D::*,
            Direct3D11::*,
            Dxgi::Common::*,
            Gdi::{
                EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
            },
        },
        System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
    },
};

struct MonitorEnumeration {
    selection: ScreenRect,
    regions: Vec<MonitorRegion>,
    failed: bool,
}

struct MonitorRegion {
    handle: HMONITOR,
    bounds: ScreenRect,
    selected: ScreenRect,
    ready: bool,
}

unsafe extern "system" fn enumerate_monitor(
    handle: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let enumeration = &mut *(data.0 as *mut MonitorEnumeration);
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !GetMonitorInfoW(handle, &mut info).as_bool() {
        enumeration.failed = true;
        return false.into();
    }
    let bounds = ScreenRect {
        left: info.rcMonitor.left,
        top: info.rcMonitor.top,
        right: info.rcMonitor.right,
        bottom: info.rcMonitor.bottom,
    };
    let selected = intersect(bounds, enumeration.selection);
    if let Some(selected) = selected {
        enumeration.regions.push(MonitorRegion {
            handle,
            bounds,
            selected,
            ready: false,
        });
    }
    true.into()
}

fn intersect(left: ScreenRect, right: ScreenRect) -> Option<ScreenRect> {
    let rect = ScreenRect {
        left: left.left.max(right.left),
        top: left.top.max(right.top),
        right: left.right.min(right.right),
        bottom: left.bottom.min(right.bottom),
    };
    (rect.right > rect.left && rect.bottom > rect.top).then_some(rect)
}

fn enumerate_regions(selection: ScreenRect) -> Result<Vec<MonitorRegion>, String> {
    let mut enumeration = MonitorEnumeration {
        selection,
        regions: Vec::new(),
        failed: false,
    };
    let result = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(enumerate_monitor),
            LPARAM(&mut enumeration as *mut MonitorEnumeration as isize),
        )
    };
    if enumeration.failed {
        return Err("无法读取显示器边界".into());
    }
    if !result.as_bool() {
        return Err(format!("无法枚举显示器：{}", std::io::Error::last_os_error()));
    }
    if enumeration.regions.is_empty() {
        return Err("录制区域未覆盖任何显示器".into());
    }
    Ok(enumeration.regions)
}

struct MonitorCapture {
    capture: ScreenCapture,
    region: MonitorRegion,
}

impl MonitorCapture {
    unsafe fn start(
        mut region: MonitorRegion,
        device: &ID3D11Device,
        with_cursor: bool,
    ) -> Result<Self, String> {
        let mut capture = ScreenCapture::default();
        screen_capture_create(&mut capture, true);
        if !capture.runtime_initialized || capture.item_interop.is_none() {
            screen_capture_release(&mut capture);
            return Err("无法初始化 Windows 屏幕采集运行时".into());
        }

        let local_rect = RECT {
            left: region.selected.left - region.bounds.left,
            top: region.selected.top - region.bounds.top,
            right: region.selected.right - region.bounds.left,
            bottom: region.selected.bottom - region.bounds.top,
        };
        if !screen_capture_create_for_monitor(
            &mut capture,
            device,
            region.handle,
            Some(&local_rect),
        ) {
            screen_capture_release(&mut capture);
            return Err("无法创建显示器录制会话".into());
        }
        if !screen_capture_start(&mut capture, with_cursor, false, false) {
            screen_capture_release(&mut capture);
            return Err("无法启动显示器录制会话".into());
        }
        region.ready = false;
        Ok(Self { capture, region })
    }

    fn poll(
        &mut self,
        context: &ID3D11DeviceContext,
        target: &ID3D11Texture2D,
        recording_rect: ScreenRect,
    ) -> Result<bool, String> {
        let mut frame = ScreenCaptureFrame::default();
        if !unsafe { screen_capture_get_frame(&mut self.capture, &mut frame) } {
            return Ok(false);
        }
        let result = (|| {
            let texture = frame
                .texture
                .as_ref()
                .ok_or_else(|| "Windows 屏幕采集返回了空画面".to_string())?;
            let expected = RECT {
                left: self.region.selected.left - self.region.bounds.left,
                top: self.region.selected.top - self.region.bounds.top,
                right: self.region.selected.right - self.region.bounds.left,
                bottom: self.region.selected.bottom - self.region.bounds.top,
            };
            if frame.rect.left != expected.left
                || frame.rect.top != expected.top
                || frame.rect.right != expected.right
                || frame.rect.bottom != expected.bottom
            {
                return Err("录制期间显示器尺寸发生变化，请重新开始录制".into());
            }
            let source = D3D11_BOX {
                left: frame.rect.left as u32,
                top: frame.rect.top as u32,
                front: 0,
                right: frame.rect.right as u32,
                bottom: frame.rect.bottom as u32,
                back: 1,
            };
            let destination_x = (self.region.selected.left - recording_rect.left) as u32;
            let destination_y = (self.region.selected.top - recording_rect.top) as u32;
            unsafe {
                context.CopySubresourceRegion(
                    target,
                    0,
                    destination_x,
                    destination_y,
                    0,
                    texture,
                    0,
                    Some(&source),
                );
            }
            self.region.ready = true;
            Ok(true)
        })();
        unsafe { screen_capture_release_frame(&mut self.capture, &mut frame) };
        result
    }
}

impl Drop for MonitorCapture {
    fn drop(&mut self) {
        unsafe { screen_capture_release(&mut self.capture) };
    }
}

pub struct RecordingCapture {
    selection: ScreenRect,
    source_width: u32,
    source_height: u32,
    output_width: u32,
    output_height: u32,
    context: ID3D11DeviceContext,
    composed_texture: ID3D11Texture2D,
    preview_staging: ID3D11Texture2D,
    monitors: Vec<MonitorCapture>,
    encoder: Encoder,
    finished: bool,
}

impl RecordingCapture {
    pub fn create(
        selection: ScreenRect,
        source_width: u32,
        source_height: u32,
        max_output_width: u32,
        max_output_height: u32,
        fps: u32,
        with_cursor: bool,
        path: &Path,
        bitrate_mbps: u32,
    ) -> Result<Self, String> {
        if !unsafe { screen_capture_is_supported() } {
            return Err("屏幕录制需要 Windows 10 1903 或更新版本".into());
        }
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(|error| format!("初始化 Direct3D 屏幕采集设备失败：{error}"))?;
        let device = device.ok_or_else(|| "Direct3D 未返回屏幕采集设备".to_string())?;
        let context = context.ok_or_else(|| "Direct3D 未返回屏幕采集上下文".to_string())?;

        let padded_width = source_width
            .checked_add(source_width % 2)
            .ok_or_else(|| "录制区域宽度过大".to_string())?;
        let padded_height = source_height
            .checked_add(source_height % 2)
            .ok_or_else(|| "录制区域高度过大".to_string())?;
        let texture_desc = D3D11_TEXTURE2D_DESC {
            Width: padded_width,
            Height: padded_height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            ..Default::default()
        };
        let mut composed_texture = None;
        unsafe {
            device.CreateTexture2D(&texture_desc, None, Some(&mut composed_texture))
        }
        .map_err(|error| format!("创建 GPU 录制画布失败：{error}"))?;
        let composed_texture =
            composed_texture.ok_or_else(|| "Direct3D 未返回录制画布".to_string())?;
        let mut render_target = None;
        unsafe {
            device.CreateRenderTargetView(&composed_texture, None, Some(&mut render_target))
        }
        .map_err(|error| format!("初始化 GPU 录制画布失败：{error}"))?;
        unsafe { context.ClearRenderTargetView(render_target.as_ref().unwrap(), &[0.0; 4]) };
        drop(render_target);

        let regions = enumerate_regions(selection)?;
        let mut monitors = Vec::with_capacity(regions.len());
        for region in regions {
            monitors.push(unsafe { MonitorCapture::start(region, &device, with_cursor) }?);
        }

        let mut encoder = Encoder::default();
        unsafe { encoder_init(&mut encoder) };
        if let Some(error) = encoder.last_error.as_ref() {
            return Err(format!("初始化 Windows 视频编码器失败：{error}"));
        }
        let mut config = Config::default();
        config.hardware_encoder = true.into();
        config.capture_audio = false.into();
        config.fragmented_output = false.into();
        config.video_codec = CONFIG_VIDEO_H264;
        config.video_profile = CONFIG_VIDEO_HIGH;
        config.video_max_width = max_output_width;
        config.video_max_height = max_output_height;
        config.video_max_framerate = fps;
        config.video_bitrate = bitrate_mbps.saturating_mul(1000);
        let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut encoder_config = EncoderConfig {
            width: source_width,
            height: source_height,
            framerate_num: fps,
            framerate_den: 1,
            audio_format: std::ptr::null_mut(),
            config: &mut config,
        };
        if !unsafe {
            encoder_start(
                &mut encoder,
                &device,
                &path_wide,
                &encoder_config,
            )
        }
        .as_bool()
        {
            let hardware_error = encoder
                .last_error
                .take()
                .map(|error| error.to_string())
                .unwrap_or_else(|| "硬件编码器启动失败".into());
            let _ = std::fs::remove_file(path);
            config.hardware_encoder = false.into();
            encoder_config.config = &mut config;
            if !unsafe {
                encoder_start(
                    &mut encoder,
                    &device,
                    &path_wide,
                    &encoder_config,
                )
            }
            .as_bool()
            {
                let software_error = encoder
                    .last_error
                    .take()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "软件编码器启动失败".into());
                return Err(format!(
                    "无法启动 H.264 编码器（硬件：{hardware_error}；软件：{software_error}）"
                ));
            }
        }
        let output_width = encoder.output_width;
        let output_height = encoder.output_height;
        let mut output_desc = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            encoder
                .resize
                .output_texture
                .as_ref()
                .ok_or_else(|| "视频编码器未创建输出画面".to_string())?
                .GetDesc(&mut output_desc);
        }
        output_desc.Usage = D3D11_USAGE_STAGING;
        output_desc.BindFlags = 0;
        output_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        output_desc.MiscFlags = 0;
        let mut preview_staging = None;
        unsafe { device.CreateTexture2D(&output_desc, None, Some(&mut preview_staging)) }
            .map_err(|error| format!("创建录制预览缓冲区失败：{error}"))?;
        let preview_staging =
            preview_staging.ok_or_else(|| "Direct3D 未返回录制预览缓冲区".to_string())?;

        Ok(Self {
            selection,
            source_width,
            source_height,
            output_width,
            output_height,
            context,
            composed_texture,
            preview_staging,
            monitors,
            encoder,
            finished: false,
        })
    }

    pub fn output_dimensions(&self) -> (u32, u32) {
        (self.output_width, self.output_height)
    }

    pub fn ready(&self) -> bool {
        self.monitors.iter().all(|monitor| monitor.region.ready)
    }

    pub fn poll(&mut self) -> Result<bool, String> {
        let mut updated = false;
        for monitor in &mut self.monitors {
            updated |= monitor.poll(&self.context, &self.composed_texture, self.selection)?;
        }
        Ok(updated)
    }

    pub fn encode_frame(&mut self, time: u64, frequency: u64) -> Result<bool, String> {
        let full_rect = RECT {
            left: 0,
            top: 0,
            right: self.source_width as i32,
            bottom: self.source_height as i32,
        };
        let encoded = unsafe {
            encoder_new_frame(
                &mut self.encoder,
                &self.composed_texture,
                full_rect,
                time,
                frequency,
            )
        }
        .as_bool();
        if let Some(error) = self.encoder.last_error.as_ref() {
            return Err(format!("编码录制画面失败：{error}"));
        }
        Ok(encoded)
    }

    pub fn preview_pixels(&mut self) -> Result<Vec<u8>, String> {
        let output = self
            .encoder
            .resize
            .output_texture
            .as_ref()
            .ok_or_else(|| "录制预览画面尚未准备好".to_string())?;
        unsafe {
            self.context.CopyResource(&self.preview_staging, output);
        }
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(&self.preview_staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        }
        .map_err(|error| format!("读取录制预览画面失败：{error}"))?;
        let result = (|| {
            if mapped.pData.is_null() || mapped.RowPitch < self.output_width.saturating_mul(4) {
                return Err("录制预览缓冲区格式无效".into());
            }
            let mut rgba = vec![
                0u8;
                (self.output_width as usize)
                    .checked_mul(self.output_height as usize)
                    .and_then(|pixels| pixels.checked_mul(4))
                    .ok_or_else(|| "录制预览尺寸过大".to_string())?
            ];
            for y in 0..self.output_height as usize {
                let source = unsafe {
                    std::slice::from_raw_parts(
                        (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                        self.output_width as usize * 4,
                    )
                };
                let target = &mut rgba[y * self.output_width as usize * 4
                    ..(y + 1) * self.output_width as usize * 4];
                for (source, target) in source
                    .chunks_exact(4)
                    .zip(target.chunks_exact_mut(4))
                {
                    target.copy_from_slice(&[source[2], source[1], source[0], 255]);
                }
            }
            Ok(rgba)
        })();
        unsafe { self.context.Unmap(&self.preview_staging, 0) };
        result
    }

    pub fn finish(&mut self) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        self.monitors.clear();
        unsafe { encoder_stop(&mut self.encoder) };
        self.finished = true;
        match self.encoder.last_error.take() {
            Some(error) => Err(format!("完成 MP4 文件失败：{error}")),
            None => Ok(()),
        }
    }
}

impl Drop for RecordingCapture {
    fn drop(&mut self) {
        if !self.finished {
            self.monitors.clear();
            unsafe { encoder_stop(&mut self.encoder) };
            self.finished = true;
        }
    }
}

pub fn qpc_frequency() -> Result<u64, String> {
    let mut frequency = 0i64;
    unsafe { QueryPerformanceFrequency(&mut frequency) }
        .map_err(|error| format!("读取高精度计时频率失败：{error}"))?;
    u64::try_from(frequency)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "高精度计时频率无效".into())
}

pub fn qpc_now() -> Result<u64, String> {
    let mut counter = 0i64;
    unsafe { QueryPerformanceCounter(&mut counter) }
        .map_err(|error| format!("读取高精度计时失败：{error}"))?;
    u64::try_from(counter).map_err(|_| "高精度计时值无效".into())
}
