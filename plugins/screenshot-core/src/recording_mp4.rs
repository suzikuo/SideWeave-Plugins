use super::RecordedFrame;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::PCWSTR;
use windows::Win32::Media::MediaFoundation::{
    MFCreateAttributes, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
    MFCreateSinkWriterFromURL, MFCreateSourceReaderFromURL, MFShutdown, MFStartup,
    IMFAttributes, IMFByteStream, IMFSourceReader, IMFSinkWriter, MF_MT_AVG_BITRATE,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
    MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_ENDOFSTREAM, MFSTARTUP_FULL,
    MFMediaType_Video, MFVideoFormat_H264, MFVideoFormat_RGB32, MFVideoInterlace_Progressive,
    MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

pub(super) fn encode_mp4(
    path: &Path,
    frames: &[RecordedFrame],
    width: u32,
    height: u32,
    source_fps: u32,
    output_fps: u32,
    bitrate_mbps: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if frames.is_empty() || width == 0 || height == 0 || source_fps == 0 || output_fps == 0 {
        return Err("MP4 录制参数无效".into());
    }
    // H.264 encoders commonly require even dimensions. Pad only at the right
    // and bottom edges so odd-sized selections remain intact in the UI.
    let encoded_width = width.checked_add(width % 2).ok_or_else(|| "MP4 帧尺寸过大".to_string())?;
    let encoded_height = height.checked_add(height % 2).ok_or_else(|| "MP4 帧尺寸过大".to_string())?;
    let source_buffer_length = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "MP4 帧尺寸过大".to_string())?;
    let buffer_length = encoded_width
        .checked_mul(encoded_height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "MP4 帧尺寸过大".to_string())?;
    let com_result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    com_result
        .ok()
        .map_err(|error| format!("初始化媒体线程失败：{error}"))?;
    let _com_apartment = ComApartment;
    unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
        .map_err(|error| format!("初始化 Windows 视频编码器失败：{error}"))?;
    let _media_foundation = MediaFoundation;

    let output_url: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let writer = unsafe {
        MFCreateSinkWriterFromURL(
            PCWSTR(output_url.as_ptr()),
            None::<&IMFByteStream>,
            None::<&IMFAttributes>,
        )
    }
    .map_err(|error| format!("创建 MP4 文件失败：{error}"))?;

    let output_type = unsafe { MFCreateMediaType() }
        .map_err(|error| format!("创建 H.264 媒体类型失败：{error}"))?;
    unsafe { output_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) }
        .map_err(|error| format!("设置 MP4 视频类型失败：{error}"))?;
    unsafe { output_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264) }
        .map_err(|error| format!("设置 H.264 编码失败：{error}"))?;
    unsafe {
        output_type.SetUINT32(
            &MF_MT_AVG_BITRATE,
            bitrate_to_bps(bitrate_mbps),
        )
    }
        .map_err(|error| format!("设置 MP4 码率失败：{error}"))?;
    unsafe { output_type.SetUINT32(
            &MF_MT_INTERLACE_MODE,
            MFVideoInterlace_Progressive.0 as u32,
        ) }
        .map_err(|error| format!("设置 MP4 帧模式失败：{error}"))?;
    unsafe { output_type.SetUINT64(&MF_MT_FRAME_SIZE, pack_ratio(encoded_width, encoded_height)) }
        .map_err(|error| format!("设置 MP4 画面尺寸失败：{error}"))?;
    unsafe { output_type.SetUINT64(&MF_MT_FRAME_RATE, pack_ratio(output_fps, 1)) }
        .map_err(|error| format!("设置 MP4 帧率失败：{error}"))?;
    unsafe { output_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_ratio(1, 1)) }
        .map_err(|error| format!("设置 MP4 像素比例失败：{error}"))?;

    let stream = unsafe { writer.AddStream(&output_type) }
        .map_err(|error| format!("创建 MP4 视频流失败：{error}"))?;
    let input_type = unsafe { MFCreateMediaType() }
        .map_err(|error| format!("创建屏幕帧输入格式失败：{error}"))?;
    unsafe { input_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) }
        .map_err(|error| format!("设置屏幕帧类型失败：{error}"))?;
    unsafe { input_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32) }
        .map_err(|error| format!("设置屏幕帧像素格式失败：{error}"))?;
    unsafe { input_type.SetUINT32(&MF_MT_DEFAULT_STRIDE, encoded_width * 4) }
        .map_err(|error| format!("设置屏幕帧行距失败：{error}"))?;
    unsafe { input_type.SetUINT32(
            &MF_MT_INTERLACE_MODE,
            MFVideoInterlace_Progressive.0 as u32,
        ) }
        .map_err(|error| format!("设置屏幕帧模式失败：{error}"))?;
    unsafe { input_type.SetUINT64(&MF_MT_FRAME_SIZE, pack_ratio(encoded_width, encoded_height)) }
        .map_err(|error| format!("设置屏幕帧尺寸失败：{error}"))?;
    unsafe { input_type.SetUINT64(&MF_MT_FRAME_RATE, pack_ratio(output_fps, 1)) }
        .map_err(|error| format!("设置屏幕帧率失败：{error}"))?;
    unsafe { input_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_ratio(1, 1)) }
        .map_err(|error| format!("设置屏幕像素比例失败：{error}"))?;
    unsafe { writer.SetInputMediaType(stream, &input_type, None::<&IMFAttributes>) }
        .map_err(|error| format!("初始化 MP4 视频流失败：{error}"))?;
    unsafe { writer.BeginWriting() }.map_err(|error| format!("启动 MP4 编码失败：{error}"))?;

    for (frame_index, timestamp_ms, duration_ms) in
        super::frame_schedule(frames, source_fps, output_fps)
    {
        if cancel.load(Ordering::Acquire) {
            let _ = unsafe { writer.Finalize() };
            return Err("导出已取消".into());
        }
        let frame = &frames[frame_index];
        if frame.pixels.len() != source_buffer_length as usize {
            let _ = unsafe { writer.Finalize() };
            return Err("MP4 帧数据长度无效".into());
        }

        let buffer = unsafe { MFCreateMemoryBuffer(buffer_length) }
            .map_err(|error| format!("分配 MP4 帧缓冲区失败：{error}"))?;
        let mut destination = std::ptr::null_mut();
        unsafe { buffer.Lock(&mut destination, None, None) }
            .map_err(|error| format!("锁定 MP4 帧缓冲区失败：{error}"))?;
        if destination.is_null() {
            let _ = unsafe { buffer.Unlock() };
            return Err("MP4 帧缓冲区为空".into());
        }
        let target = unsafe { std::slice::from_raw_parts_mut(destination, buffer_length as usize) };
        for y in 0..encoded_height {
            let source_y = y.min(height - 1);
            for x in 0..encoded_width {
                let source_x = x.min(width - 1);
                let source_offset = ((source_y * width + source_x) * 4) as usize;
                let target_offset = ((y * encoded_width + x) * 4) as usize;
                target[target_offset] = frame.pixels[source_offset + 2];
                target[target_offset + 1] = frame.pixels[source_offset + 1];
                target[target_offset + 2] = frame.pixels[source_offset];
                target[target_offset + 3] = 0;
            }
        }
        unsafe { buffer.Unlock() }
            .map_err(|error| format!("释放 MP4 帧缓冲区失败：{error}"))?;
        unsafe { buffer.SetCurrentLength(buffer_length) }
            .map_err(|error| format!("设置 MP4 帧长度失败：{error}"))?;

        let sample = unsafe { MFCreateSample() }
            .map_err(|error| format!("创建 MP4 视频样本失败：{error}"))?;
        unsafe { sample.AddBuffer(&buffer) }
            .map_err(|error| format!("写入 MP4 帧缓冲区失败：{error}"))?;
        let timestamp = milliseconds_to_hns(timestamp_ms);
        let duration = milliseconds_to_hns(u64::from(duration_ms));
        unsafe { sample.SetSampleTime(timestamp) }
            .map_err(|error| format!("设置 MP4 帧时间失败：{error}"))?;
        unsafe { sample.SetSampleDuration(duration) }
            .map_err(|error| format!("设置 MP4 帧时长失败：{error}"))?;
        unsafe { writer.WriteSample(stream, &sample) }
            .map_err(|error| format!("编码 MP4 视频帧失败：{error}"))?;
    }

    unsafe { writer.Finalize() }.map_err(|error| format!("完成 MP4 文件失败：{error}"))?;
    Ok(())
}

fn pack_ratio(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

fn bitrate_to_bps(bitrate_mbps: u32) -> u32 {
    bitrate_mbps.saturating_mul(1_000_000)
}

fn milliseconds_to_hns(milliseconds: u64) -> i64 {
    milliseconds.saturating_mul(10_000).min(i64::MAX as u64) as i64
}

struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct MediaFoundation;

impl Drop for MediaFoundation {
    fn drop(&mut self) {
        let _ = unsafe { MFShutdown() };
    }
}

/// Decode a finalized MP4 one frame at a time. The callback keeps export memory
/// bounded even when a user converts a long recording to GIF or animated WebP.
pub(super) fn visit_mp4_frames(
    path: &Path,
    width: u32,
    height: u32,
    source_fps: u32,
    output_fps: u32,
    cancel: &AtomicBool,
    mut visit: impl FnMut(u64, u64, &[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let mut reader = Mp4FrameReader::open(path, width, height)?;
    let source_fps = u64::from(source_fps.max(1));
    let output_fps = u64::from(output_fps.max(1));
    let final_frame_duration = 1000u64.saturating_add(source_fps - 1) / source_fps;
    let mut output_index = 0u64;
    let mut current = reader.read_frame()?;
    while let Some((timestamp_ms, pixels)) = current {
        if cancel.load(Ordering::Acquire) {
            return Err("导出已取消".into());
        }
        let next = reader.read_frame()?;
        let source_end_ms = next
            .as_ref()
            .map(|(next_timestamp, _)| *next_timestamp)
            .unwrap_or_else(|| timestamp_ms.saturating_add(final_frame_duration.max(1)));
        loop {
            let output_timestamp_ms = output_index.saturating_mul(1000) / output_fps;
            if output_timestamp_ms >= source_end_ms {
                break;
            }
            let next_output_ms = output_index.saturating_add(1).saturating_mul(1000) / output_fps;
            let duration_ms = next_output_ms.saturating_sub(output_timestamp_ms).max(1);
            visit(output_timestamp_ms, duration_ms, &pixels)?;
            output_index = output_index.saturating_add(1);
        }
        current = next;
    }
    Ok(())
}

struct Mp4FrameReader {
    reader: Option<IMFSourceReader>,
    width: u32,
    height: u32,
    encoded_width: u32,
    encoded_height: u32,
    _media_foundation: MediaFoundation,
    _com_apartment: ComApartment,
}

impl Mp4FrameReader {
    fn open(path: &Path, width: u32, height: u32) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("MP4 视频尺寸无效".into());
        }
        let encoded_width = width.checked_add(width % 2).ok_or_else(|| "MP4 尺寸过大".to_string())?;
        let encoded_height = height.checked_add(height % 2).ok_or_else(|| "MP4 尺寸过大".to_string())?;
        let com_result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        com_result
            .ok()
            .map_err(|error| format!("初始化视频解码线程失败：{error}"))?;
        let com_apartment = ComApartment;
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
            .map_err(|error| format!("初始化 Windows 视频解码器失败：{error}"))?;
        let media_foundation = MediaFoundation;
        let mut attributes = None;
        unsafe { MFCreateAttributes(&mut attributes, 1) }
            .map_err(|error| format!("创建视频解码属性失败：{error}"))?;
        let attributes = attributes.ok_or("创建视频解码属性失败")?;
        unsafe { attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1) }
            .map_err(|error| format!("启用视频帧转换失败：{error}"))?;
        let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let reader = unsafe { MFCreateSourceReaderFromURL(PCWSTR(wide_path.as_ptr()), &attributes) }
            .map_err(|error| format!("打开临时 MP4 失败：{error}"))?;
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let media_type = unsafe { MFCreateMediaType() }
            .map_err(|error| format!("创建 MP4 解码格式失败：{error}"))?;
        unsafe { media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) }
            .map_err(|error| format!("设置 MP4 视频类型失败：{error}"))?;
        unsafe { media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32) }
            .map_err(|error| format!("设置 MP4 解码像素格式失败：{error}"))?;
        unsafe { media_type.SetUINT32(&MF_MT_DEFAULT_STRIDE, encoded_width * 4) }
            .map_err(|error| format!("设置 MP4 解码行距失败：{error}"))?;
        unsafe { media_type.SetUINT64(&MF_MT_FRAME_SIZE, pack_ratio(encoded_width, encoded_height)) }
            .map_err(|error| format!("设置 MP4 解码尺寸失败：{error}"))?;
        unsafe { media_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32) }
            .map_err(|error| format!("设置 MP4 解码帧模式失败：{error}"))?;
        unsafe { reader.SetCurrentMediaType(stream, None, &media_type) }
            .map_err(|error| format!("初始化 MP4 视频解码失败：{error}"))?;

        Ok(Self {
            reader: Some(reader),
            width,
            height,
            encoded_width,
            encoded_height,
            _media_foundation: media_foundation,
            _com_apartment: com_apartment,
        })
    }

    fn read_frame(&mut self) -> Result<Option<(u64, Vec<u8>)>, String> {
        let reader = self.reader.as_ref().ok_or("MP4 解码器已关闭")?;
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        loop {
            let mut flags = 0u32;
            let mut timestamp = 0i64;
            let mut sample = None;
            unsafe {
                reader.ReadSample(
                    stream,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut timestamp),
                    Some(&mut sample),
                )
            }
            .map_err(|error| format!("读取 MP4 视频帧失败：{error}"))?;
            if flags & (MF_SOURCE_READERF_ENDOFSTREAM.0 as u32) != 0 {
                return Ok(None);
            }
            let Some(sample) = sample else {
                continue;
            };
            let buffer = unsafe { sample.ConvertToContiguousBuffer() }
                .map_err(|error| format!("读取 MP4 帧缓冲区失败：{error}"))?;
            let mut source = std::ptr::null_mut();
            let mut current_length = 0u32;
            unsafe { buffer.Lock(&mut source, None, Some(&mut current_length)) }
                .map_err(|error| format!("锁定 MP4 解码帧失败：{error}"))?;
            let required_length = self.encoded_width as usize * self.encoded_height as usize * 4;
            if source.is_null() || (current_length as usize) < required_length {
                let _ = unsafe { buffer.Unlock() };
                return Err("MP4 解码帧尺寸无效".into());
            }
            let bgra = unsafe {
                std::slice::from_raw_parts(source, current_length as usize)
            };
            let mut rgba = vec![0u8; self.width as usize * self.height as usize * 4];
            for y in 0..self.height {
                for x in 0..self.width {
                    let source_offset = ((y * self.encoded_width + x) * 4) as usize;
                    let target_offset = ((y * self.width + x) * 4) as usize;
                    rgba[target_offset] = bgra[source_offset + 2];
                    rgba[target_offset + 1] = bgra[source_offset + 1];
                    rgba[target_offset + 2] = bgra[source_offset];
                    rgba[target_offset + 3] = 255;
                }
            }
            unsafe { buffer.Unlock() }
                .map_err(|error| format!("释放 MP4 解码帧失败：{error}"))?;
            return Ok(Some(((timestamp.max(0) / 10_000) as u64, rgba)));
        }
    }
}

impl Drop for Mp4FrameReader {
    fn drop(&mut self) {
        self.reader.take();
    }
}

/// A live MP4 sink used during capture. Frames are encoded and written as they
/// arrive, so recording duration does not grow the raw-frame memory buffer.
pub(super) struct StreamingMp4Encoder {
    writer: Option<IMFSinkWriter>,
    stream: u32,
    source_width: u32,
    source_height: u32,
    encoded_width: u32,
    encoded_height: u32,
    fps: u32,
    buffer_length: u32,
    finalized: bool,
    _media_foundation: MediaFoundation,
    _com_apartment: ComApartment,
}

impl StreamingMp4Encoder {
    pub(super) fn create(
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        bitrate_mbps: u32,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 || fps == 0 {
            return Err("MP4 录制参数无效".into());
        }
        let encoded_width = width.checked_add(width % 2).ok_or_else(|| "MP4 帧尺寸过大".to_string())?;
        let encoded_height = height.checked_add(height % 2).ok_or_else(|| "MP4 帧尺寸过大".to_string())?;
        let buffer_length = encoded_width
            .checked_mul(encoded_height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "MP4 帧尺寸过大".to_string())?;

        let com_result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        com_result
            .ok()
            .map_err(|error| format!("初始化媒体线程失败：{error}"))?;
        let com_apartment = ComApartment;
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
            .map_err(|error| format!("初始化 Windows 视频编码器失败：{error}"))?;
        let media_foundation = MediaFoundation;

        let output_url: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let writer = unsafe {
            MFCreateSinkWriterFromURL(
                PCWSTR(output_url.as_ptr()),
                None::<&IMFByteStream>,
                None::<&IMFAttributes>,
            )
        }
        .map_err(|error| format!("创建 MP4 临时文件失败：{error}"))?;

        let output_type = unsafe { MFCreateMediaType() }
            .map_err(|error| format!("创建 H.264 媒体类型失败：{error}"))?;
        unsafe { output_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) }
            .map_err(|error| format!("设置 MP4 视频类型失败：{error}"))?;
        unsafe { output_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264) }
            .map_err(|error| format!("设置 H.264 编码失败：{error}"))?;
        unsafe {
            output_type.SetUINT32(
                &MF_MT_AVG_BITRATE,
                bitrate_to_bps(bitrate_mbps),
            )
        }
            .map_err(|error| format!("设置 MP4 码率失败：{error}"))?;
        unsafe {
            output_type.SetUINT32(
                &MF_MT_INTERLACE_MODE,
                MFVideoInterlace_Progressive.0 as u32,
            )
        }
        .map_err(|error| format!("设置 MP4 帧模式失败：{error}"))?;
        unsafe { output_type.SetUINT64(&MF_MT_FRAME_SIZE, pack_ratio(encoded_width, encoded_height)) }
            .map_err(|error| format!("设置 MP4 画面尺寸失败：{error}"))?;
        unsafe { output_type.SetUINT64(&MF_MT_FRAME_RATE, pack_ratio(fps, 1)) }
            .map_err(|error| format!("设置 MP4 帧率失败：{error}"))?;
        unsafe { output_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_ratio(1, 1)) }
            .map_err(|error| format!("设置 MP4 像素比例失败：{error}"))?;

        let stream = unsafe { writer.AddStream(&output_type) }
            .map_err(|error| format!("创建 MP4 视频流失败：{error}"))?;
        let input_type = unsafe { MFCreateMediaType() }
            .map_err(|error| format!("创建屏幕帧输入格式失败：{error}"))?;
        unsafe { input_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) }
            .map_err(|error| format!("设置屏幕帧类型失败：{error}"))?;
        unsafe { input_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32) }
            .map_err(|error| format!("设置屏幕帧像素格式失败：{error}"))?;
        unsafe { input_type.SetUINT32(&MF_MT_DEFAULT_STRIDE, encoded_width * 4) }
            .map_err(|error| format!("设置屏幕帧行距失败：{error}"))?;
        unsafe {
            input_type.SetUINT32(
                &MF_MT_INTERLACE_MODE,
                MFVideoInterlace_Progressive.0 as u32,
            )
        }
        .map_err(|error| format!("设置屏幕帧模式失败：{error}"))?;
        unsafe { input_type.SetUINT64(&MF_MT_FRAME_SIZE, pack_ratio(encoded_width, encoded_height)) }
            .map_err(|error| format!("设置屏幕帧尺寸失败：{error}"))?;
        unsafe { input_type.SetUINT64(&MF_MT_FRAME_RATE, pack_ratio(fps, 1)) }
            .map_err(|error| format!("设置屏幕帧率失败：{error}"))?;
        unsafe { input_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_ratio(1, 1)) }
            .map_err(|error| format!("设置屏幕像素比例失败：{error}"))?;
        unsafe { writer.SetInputMediaType(stream, &input_type, None::<&IMFAttributes>) }
            .map_err(|error| format!("初始化 MP4 视频流失败：{error}"))?;
        unsafe { writer.BeginWriting() }.map_err(|error| format!("启动 MP4 编码失败：{error}"))?;

        Ok(Self {
            writer: Some(writer),
            stream,
            source_width: width,
            source_height: height,
            encoded_width,
            encoded_height,
            fps,
            buffer_length,
            finalized: false,
            _media_foundation: media_foundation,
            _com_apartment: com_apartment,
        })
    }

    pub(super) fn write_frame(&mut self, timestamp_ms: u64, pixels: &[u8]) -> Result<(), String> {
        let expected_length = self.source_width as usize * self.source_height as usize * 4;
        if pixels.len() != expected_length {
            return Err("MP4 帧数据长度无效".into());
        }
        let buffer = unsafe { MFCreateMemoryBuffer(self.buffer_length) }
            .map_err(|error| format!("分配 MP4 帧缓冲区失败：{error}"))?;
        let mut destination = std::ptr::null_mut();
        unsafe { buffer.Lock(&mut destination, None, None) }
            .map_err(|error| format!("锁定 MP4 帧缓冲区失败：{error}"))?;
        if destination.is_null() {
            let _ = unsafe { buffer.Unlock() };
            return Err("MP4 帧缓冲区为空".into());
        }
        let target = unsafe {
            std::slice::from_raw_parts_mut(destination, self.buffer_length as usize)
        };
        for y in 0..self.encoded_height {
            let source_y = y.min(self.source_height - 1);
            for x in 0..self.encoded_width {
                let source_x = x.min(self.source_width - 1);
                let source_offset = ((source_y * self.source_width + source_x) * 4) as usize;
                let target_offset = ((y * self.encoded_width + x) * 4) as usize;
                target[target_offset] = pixels[source_offset + 2];
                target[target_offset + 1] = pixels[source_offset + 1];
                target[target_offset + 2] = pixels[source_offset];
                target[target_offset + 3] = 0;
            }
        }
        unsafe { buffer.Unlock() }
            .map_err(|error| format!("释放 MP4 帧缓冲区失败：{error}"))?;
        unsafe { buffer.SetCurrentLength(self.buffer_length) }
            .map_err(|error| format!("设置 MP4 帧长度失败：{error}"))?;

        let sample = unsafe { MFCreateSample() }
            .map_err(|error| format!("创建 MP4 视频样本失败：{error}"))?;
        unsafe { sample.AddBuffer(&buffer) }
            .map_err(|error| format!("写入 MP4 帧缓冲区失败：{error}"))?;
        let timestamp = milliseconds_to_hns(timestamp_ms);
        let duration = i64::from((1000 / self.fps).max(1)) * 10_000;
        unsafe { sample.SetSampleTime(timestamp) }
            .map_err(|error| format!("设置 MP4 帧时间失败：{error}"))?;
        unsafe { sample.SetSampleDuration(duration) }
            .map_err(|error| format!("设置 MP4 帧时长失败：{error}"))?;
        let writer = self.writer.as_ref().ok_or("MP4 编码器已关闭")?;
        unsafe { writer.WriteSample(self.stream, &sample) }
            .map_err(|error| format!("编码 MP4 视频帧失败：{error}"))
    }

    pub(super) fn finish(&mut self) -> Result<(), String> {
        if self.finalized {
            return Ok(());
        }
        self.finalized = true;
        let Some(writer) = self.writer.take() else {
            return Ok(());
        };
        unsafe { writer.Finalize() }.map_err(|error| format!("完成 MP4 文件失败：{error}"))
    }
}

impl Drop for StreamingMp4Encoder {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
