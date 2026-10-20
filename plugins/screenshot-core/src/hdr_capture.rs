use crate::model::{HdrCaptureMode, ScreenRect};
use half::f16;
use std::sync::OnceLock;
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_FORMAT_B8G8R8A8_UNORM,
    DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_MODE_ROTATION, DXGI_MODE_ROTATION_IDENTITY,
    DXGI_MODE_ROTATION_ROTATE180, DXGI_MODE_ROTATION_ROTATE270, DXGI_MODE_ROTATION_ROTATE90,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1, IDXGIOutput, IDXGIOutput5, IDXGIOutput6,
    IDXGIOutputDuplication, IDXGIResource, DXGI_OUTDUPL_FRAME_INFO,
};

/// Captures all attached outputs using DXGI's high precision format when the
/// driver offers it. The returned bytes stay in BGRA order to match the
/// existing crop and overlay paths.
pub(super) fn capture_virtual_desktop(
    bounds: ScreenRect,
    hdr_mode: HdrCaptureMode,
) -> Option<Vec<u8>> {
    if hdr_mode == HdrCaptureMode::ForceOff {
        return None;
    }
    let width = usize::try_from(bounds.width()).ok()?;
    let height = usize::try_from(bounds.height()).ok()?;
    let byte_count = width.checked_mul(height)?.checked_mul(4)?;
    let mut pixels = vec![0u8; byte_count];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = 255;
    }

    let captured = unsafe {
        let factory = CreateDXGIFactory1::<IDXGIFactory1>().ok()?;
        let mut captured_outputs = 0usize;
        let mut adapter_index = 0;
        loop {
            let Ok(adapter1) = factory.EnumAdapters1(adapter_index) else {
                break;
            };
            adapter_index += 1;
            let adapter = adapter1.cast::<IDXGIAdapter>().ok()?;
            let mut outputs = Vec::new();

            let mut output_index = 0;
            loop {
                let Ok(output) = adapter.EnumOutputs(output_index) else {
                    break;
                };
                output_index += 1;
                let Some(output_bounds) = attached_output_bounds(&output).ok()? else {
                    continue;
                };
                if output_bounds.clipped_to(bounds).is_none() {
                    continue;
                }
                outputs.push((output, output_bounds));
            }
            if outputs.is_empty() {
                continue;
            }

            let (device, context) = create_device(&adapter)?;
            for (output, output_bounds) in outputs {
                capture_output(
                    &output,
                    &device,
                    &context,
                    output_bounds,
                    bounds,
                    &mut pixels,
                    width,
                    hdr_mode,
                )?;
                captured_outputs += 1;
            }
        }
        captured_outputs > 0
    };

    captured.then_some(pixels)
}

unsafe fn create_device(adapter: &IDXGIAdapter) -> Option<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device = None;
    let mut context = None;
    D3D11CreateDevice(
        adapter,
        D3D_DRIVER_TYPE_UNKNOWN,
        HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        None,
        D3D11_SDK_VERSION,
        Some(&mut device),
        None,
        Some(&mut context),
    )
    .ok()?;
    Some((device?, context?))
}

unsafe fn attached_output_bounds(
    output: &IDXGIOutput,
) -> windows::core::Result<Option<ScreenRect>> {
    let desc = output.GetDesc()?;
    if !desc.AttachedToDesktop.as_bool() {
        return Ok(None);
    }
    let rect = desc.DesktopCoordinates;
    let bounds = ScreenRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    };
    Ok((bounds.right > bounds.left && bounds.bottom > bounds.top).then_some(bounds))
}

unsafe fn capture_output(
    output: &IDXGIOutput,
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    output_bounds: ScreenRect,
    desktop_bounds: ScreenRect,
    desktop_pixels: &mut [u8],
    desktop_width: usize,
    hdr_mode: HdrCaptureMode,
) -> Option<()> {
    let use_hdr_capture = match hdr_mode {
        HdrCaptureMode::Auto => {
            let output6 = output.cast::<IDXGIOutput6>().ok()?;
            output6.GetDesc1().ok()?.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020
        }
        HdrCaptureMode::ForceOn => true,
        HdrCaptureMode::ForceOff => false,
    };
    if !use_hdr_capture {
        return copy_sdr_output_with_gdi(
            output_bounds,
            desktop_bounds,
            desktop_pixels,
            desktop_width,
        );
    }

    let output5 = output.cast::<IDXGIOutput5>().ok()?;
    let formats = [DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_B8G8R8A8_UNORM];
    let duplication = output5.DuplicateOutput1(device, 0, &formats).ok()?;

    let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut desktop_resource: Option<IDXGIResource> = None;
    duplication
        .AcquireNextFrame(500, &mut frame_info, &mut desktop_resource)
        .ok()?;
    let _frame = AcquiredFrame(&duplication);
    let desktop_resource = desktop_resource?;
    let source_texture = desktop_resource.cast::<ID3D11Texture2D>().ok()?;

    let mut source_desc = D3D11_TEXTURE2D_DESC::default();
    source_texture.GetDesc(&mut source_desc);
    let rotation = duplication.GetDesc().Rotation;
    if source_desc.Format == DXGI_FORMAT_B8G8R8A8_UNORM {
        drop(_frame);
        return copy_sdr_output_with_gdi(
            output_bounds,
            desktop_bounds,
            desktop_pixels,
            desktop_width,
        );
    }
    if source_desc.Format != DXGI_FORMAT_R16G16B16A16_FLOAT {
        return None;
    }
    if !dimensions_match_rotation(
        output_bounds.width() as usize,
        output_bounds.height() as usize,
        source_desc.Width as usize,
        source_desc.Height as usize,
        rotation,
    ) {
        return None;
    }

    let staging_desc = D3D11_TEXTURE2D_DESC {
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
        ..source_desc
    };
    let mut staging = None;
    device
        .CreateTexture2D(&staging_desc, None, Some(&mut staging))
        .ok()?;
    let staging = staging?;
    let source_resource = source_texture.cast::<ID3D11Resource>().ok()?;
    let staging_resource = staging.cast::<ID3D11Resource>().ok()?;
    context.CopyResource(&staging_resource, &source_resource);

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    context
        .Map(&staging_resource, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        .ok()?;
    let _mapped = MappedTexture {
        context,
        resource: &staging_resource,
    };
    if mapped.pData.is_null() {
        return None;
    }

    let bytes_per_pixel = 8usize;
    let min_row_pitch = (source_desc.Width as usize).checked_mul(bytes_per_pixel)?;
    let row_pitch = mapped.RowPitch as usize;
    if row_pitch < min_row_pitch {
        return None;
    }
    let mapped_len = row_pitch.checked_mul(source_desc.Height as usize)?;
    let source_bytes = std::slice::from_raw_parts(mapped.pData.cast::<u8>(), mapped_len);
    copy_output_to_desktop(
        source_bytes,
        row_pitch,
        source_desc.Width as usize,
        source_desc.Height as usize,
        bytes_per_pixel,
        rotation,
        output_bounds,
        desktop_bounds,
        desktop_pixels,
        desktop_width,
    )?;
    Some(())
}

fn copy_sdr_output_with_gdi(
    output_bounds: ScreenRect,
    desktop_bounds: ScreenRect,
    desktop_pixels: &mut [u8],
    desktop_width: usize,
) -> Option<()> {
    let clipped = output_bounds.clipped_to(desktop_bounds)?;
    let rgba = super::capture::ScreenCapture::grab_region(clipped)?;
    let width = clipped.width();
    let height = clipped.height();
    if rgba.len() != width.checked_mul(height)?.checked_mul(4)? {
        return None;
    }

    let destination_x = (clipped.left - desktop_bounds.left) as usize;
    let destination_y = (clipped.top - desktop_bounds.top) as usize;
    for y in 0..height {
        for x in 0..width {
            let source = (y * width + x) * 4;
            let destination = ((destination_y + y) * desktop_width + destination_x + x) * 4;
            desktop_pixels[destination] = rgba[source + 2];
            desktop_pixels[destination + 1] = rgba[source + 1];
            desktop_pixels[destination + 2] = rgba[source];
            desktop_pixels[destination + 3] = 255;
        }
    }
    Some(())
}

fn dimensions_match_rotation(
    output_width: usize,
    output_height: usize,
    source_width: usize,
    source_height: usize,
    rotation: DXGI_MODE_ROTATION,
) -> bool {
    match rotation {
        DXGI_MODE_ROTATION_IDENTITY | DXGI_MODE_ROTATION_ROTATE180 => {
            source_width == output_width && source_height == output_height
        }
        DXGI_MODE_ROTATION_ROTATE90 | DXGI_MODE_ROTATION_ROTATE270 => {
            source_width == output_height && source_height == output_width
        }
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn copy_output_to_desktop(
    source: &[u8],
    row_pitch: usize,
    source_width: usize,
    source_height: usize,
    bytes_per_pixel: usize,
    rotation: DXGI_MODE_ROTATION,
    output_bounds: ScreenRect,
    desktop_bounds: ScreenRect,
    desktop_pixels: &mut [u8],
    desktop_width: usize,
) -> Option<()> {
    let clipped = output_bounds.clipped_to(desktop_bounds)?;
    let destination_x = (clipped.left - desktop_bounds.left) as usize;
    let destination_y = (clipped.top - desktop_bounds.top) as usize;
    let source_x_offset = (clipped.left - output_bounds.left) as usize;
    let source_y_offset = (clipped.top - output_bounds.top) as usize;
    let fp16_to_srgb = fp16_to_srgb_lut();
    let mut has_visible_pixel = false;

    for y in 0..clipped.height() {
        for x in 0..clipped.width() {
            let desktop_x = destination_x + x;
            let desktop_y = destination_y + y;
            let local_x = source_x_offset + x;
            let local_y = source_y_offset + y;
            let (source_x, source_y) = match rotation {
                DXGI_MODE_ROTATION_IDENTITY => (local_x, local_y),
                DXGI_MODE_ROTATION_ROTATE90 => (source_width - 1 - local_y, local_x),
                DXGI_MODE_ROTATION_ROTATE180 => {
                    (source_width - 1 - local_x, source_height - 1 - local_y)
                }
                DXGI_MODE_ROTATION_ROTATE270 => (local_y, source_height - 1 - local_x),
                _ => return None,
            };
            if source_x >= source_width || source_y >= source_height {
                return None;
            }
            let src = source_y
                .checked_mul(row_pitch)?
                .checked_add(source_x.checked_mul(bytes_per_pixel)?)?;
            let dst = desktop_y
                .checked_mul(desktop_width)?
                .checked_add(desktop_x)?
                .checked_mul(4)?;
            if src.checked_add(bytes_per_pixel)? > source.len()
                || dst.checked_add(4)? > desktop_pixels.len()
            {
                return None;
            }
            let red = u16::from_le_bytes([source[src], source[src + 1]]);
            let green = u16::from_le_bytes([source[src + 2], source[src + 3]]);
            let blue = u16::from_le_bytes([source[src + 4], source[src + 5]]);
            let blue = fp16_to_srgb[blue as usize];
            let green = fp16_to_srgb[green as usize];
            let red = fp16_to_srgb[red as usize];
            has_visible_pixel |= red > 2 || green > 2 || blue > 2;
            desktop_pixels[dst] = blue;
            desktop_pixels[dst + 1] = green;
            desktop_pixels[dst + 2] = red;
            desktop_pixels[dst + 3] = 255;
        }
    }
    // Treat an empty FP16 frame as a capture failure so the caller retries
    // the complete desktop through the established GDI path.
    has_visible_pixel.then_some(())
}

fn fp16_to_srgb_lut() -> &'static [u8; 65_536] {
    static LUT: OnceLock<[u8; 65_536]> = OnceLock::new();
    LUT.get_or_init(|| {
        std::array::from_fn(|bits| linear_to_srgb8(f16::from_bits(bits as u16).to_f32()))
    })
}

fn linear_to_srgb8(linear: f32) -> u8 {
    let linear = if linear.is_finite() {
        linear.clamp(0.0, 1.0)
    } else if linear.is_sign_positive() {
        1.0
    } else {
        0.0
    };
    let srgb = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0 + 0.5) as u8
}

struct AcquiredFrame<'a>(&'a IDXGIOutputDuplication);

impl Drop for AcquiredFrame<'_> {
    fn drop(&mut self) {
        unsafe {
            let _ = self.0.ReleaseFrame();
        }
    }
}

struct MappedTexture<'a> {
    context: &'a ID3D11DeviceContext,
    resource: &'a ID3D11Resource,
}

impl Drop for MappedTexture<'_> {
    fn drop(&mut self) {
        unsafe {
            self.context.Unmap(self.resource, 0);
        }
    }
}
