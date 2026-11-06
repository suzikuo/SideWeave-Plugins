use super::*;

fn current_color(state: &UiState) -> [u8; 4] {
    state
        .settings
        .lock()
        .map(|settings| settings.annotation_rgba())
        .unwrap_or([255, 58, 72, 255])
}

fn text_size(state: &UiState) -> u32 {
    state
        .settings
        .lock()
        .map(|settings| settings.text_size)
        .unwrap_or(18)
}

pub(super) fn begin_capture(state: &mut UiState, scrolling: bool) {
    if state.capture.is_some() {
        return;
    }
    let hdr_mode = state
        .settings
        .lock()
        .map(|settings| settings.hdr_capture_mode)
        .unwrap_or_default();
    let Some(screen) = ScreenCapture::grab_virtual_desktop(hdr_mode) else {
        if state.recording_requested_fps.is_some() {
            state.recording.fail(
                "\u{65e0}\u{6cd5}\u{8bfb}\u{53d6}\u{865a}\u{62df}\u{684c}\u{9762}\u{56fe}\u{50cf}".into(),
            );
            state.recording_requested_fps = None;
        }
        set_error(state, "无法读取虚拟桌面图像".into());
        return;
    };
    let target_window = unsafe { GetForegroundWindow() };
    let screen_rect = ScreenRect {
        left: screen.x,
        top: screen.y,
        right: screen.x + screen.width,
        bottom: screen.y + screen.height,
    };
    let initial_color = current_color(state);
    let initial_text_size = text_size(state);
    let capture = CaptureSession::new(screen_rect, initial_color, initial_text_size, scrolling);
    state.capture = Some(capture);
    state.scroll_target = target_window;
    state.last_hover_query = None;
    state.screen = Some(screen);
    unsafe {
        let class_name = wide("AgileTilesScreenshotCoreV2");
        let instance = GetModuleHandleW(null());
        state.overlay = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_POPUP,
            screen_rect.left,
            screen_rect.top,
            screen_rect.width() as i32,
            screen_rect.height() as i32,
            null_mut(),
            null_mut(),
            instance,
            state as *mut UiState as *const c_void,
        );
        state.render = state
            .screen
            .as_ref()
            .and_then(|screen| create_overlay_render(state.overlay, screen));
        ShowWindow(state.overlay, SW_SHOW);
        SetForegroundWindow(state.overlay);
        SetFocus(state.overlay);
    }
    update_status(state, true);
}

fn set_recording_capture_style(hwnd: HWND, recording: bool) {
    if hwnd.is_null() {
        return;
    }
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let updated = if recording {
            current | WS_EX_LAYERED
        } else {
            current & !WS_EX_LAYERED
        };
        if current == updated {
            return;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, updated as isize);
        if recording {
            SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
        }
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
        );
    }
}

pub(super) fn restore_recording_selection(state: &mut UiState) {
    state.recording_overlay = false;
    destroy_scroll_masks(state);
    destroy_scroll_border(state);
    set_recording_capture_style(state.overlay, false);
    if !state.overlay.is_null() {
        unsafe {
            ShowWindow(state.overlay, SW_SHOW);
            SetForegroundWindow(state.overlay);
            SetFocus(state.overlay);
        }
    }
    invalidate(state);
}

pub(super) fn start_recording_selection(state: &mut UiState) {
    if state.capture.is_some() {
        set_error(state, "请先完成或取消当前截图".into());
        return;
    }
    let fps = super::configured_recording_fps(&state.settings);
    if let Err(error) = state.recording.request_selection(fps) {
        set_error(state, error);
        return;
    }
    state.recording_requested_fps = Some(fps);
    begin_capture(state, false);
}

pub(super) fn start_recording_selected_region(state: &mut UiState) {
    state.recording_auto_open_pending = false;
    let Some(fps) = state.recording_requested_fps.take() else {
        return;
    };
    let Some(selection) = state
        .capture
        .as_ref()
        .and_then(|capture| capture.selection)
    else {
        return;
    };
    let Some(screen_bounds) = state.screen.as_ref().map(ScreenCapture::bounds) else {
        state.recording.fail("无法读取录制屏幕范围".into());
        return;
    };
    let (max_height, bitrate_mbps) = configured_recording_options(&state.settings);
    set_recording_capture_style(state.overlay, true);
    if let Err(error) = state.recording.start_capture(
        selection,
        fps,
        state.recording_show_cursor,
        max_height,
        bitrate_mbps,
    ) {
        state.recording.fail(error.clone());
        set_error(state, error);
        cancel(state);
        return;
    }
    state.recording_auto_open_pending = true;
    state.recording_overlay = true;
    if let Some(capture) = state.capture.as_mut() {
        capture.close_menu();
        capture.toolbar_hover = None;
        capture.secondary_hover = None;
        capture.menu_hover = None;
    }
    create_scroll_masks(state, screen_bounds, selection);
    create_scroll_border(state, selection);
    invalidate(state);
    if let Err(error) = recording_ui::create_control_toolbar(state, selection, screen_bounds) {
        state.recording_auto_open_pending = false;
        state.recording.cancel();
        restore_recording_selection(state);
        set_error(state, error);
    } else {
        unsafe { ShowWindow(state.overlay, SW_HIDE) };
    }
}

pub(super) fn start_recording_from_toolbar(state: &mut UiState, fps: u32) {
    state.recording_auto_open_pending = false;
    let Some(selection) = state
        .capture
        .as_ref()
        .and_then(|capture| capture.selection)
    else {
        return;
    };
    let Some(screen_bounds) = state.screen.as_ref().map(ScreenCapture::bounds) else {
        set_error(state, "无法读取录制屏幕范围".into());
        return;
    };
    let (max_height, bitrate_mbps) = configured_recording_options(&state.settings);
    if let Err(error) = state.recording.request_selection(fps) {
        set_error(state, error);
        return;
    }
    set_recording_capture_style(state.overlay, true);
    state.recording_overlay = true;
    if let Some(capture) = state.capture.as_mut() {
        capture.close_menu();
        capture.toolbar_hover = None;
        capture.secondary_hover = None;
        capture.menu_hover = None;
    }
    create_scroll_masks(state, screen_bounds, selection);
    create_scroll_border(state, selection);
    invalidate(state);

    if let Err(error) = state.recording.start_capture(
        selection,
        fps,
        state.recording_show_cursor,
        max_height,
        bitrate_mbps,
    ) {
        state.recording.fail(error.clone());
        restore_recording_selection(state);
        set_error(state, error);
        return;
    }
    state.recording_auto_open_pending = true;
    if let Err(error) = recording_ui::create_control_toolbar(state, selection, screen_bounds) {
        state.recording_auto_open_pending = false;
        state.recording.cancel();
        restore_recording_selection(state);
        set_error(state, error);
    } else {
        unsafe { ShowWindow(state.overlay, SW_HIDE) };
    }
}

pub(super) fn open_recording_editor(state: &mut UiState) {
    if state.recording.preview_frame(0).is_none() {
        return;
    }
    state.recording_auto_open_pending = false;
    state.recording_overlay = false;
    recording_ui::destroy_control_toolbar(state);
    editor::open_recording_editor(state);
}

pub(super) fn export_recording_from_editor(
    state: &mut UiState,
    format: AnimationFormat,
) {
    recording_ui::export_recording(state, format);
}

pub(super) fn stop_recording(state: &mut UiState) {
    if let Err(error) = state.recording.request_stop() {
        set_error(state, error);
    }
}

pub(super) fn cancel(state: &mut UiState) {
    state.recording_auto_open_pending = false;
    ocr_ui::close_session_transients(state);
    state.recording_overlay = false;
    recording_ui::destroy_control_toolbar(state);
    if state.recording_requested_fps.take().is_some() {
        state.recording.cancel();
    }
    if state.editor.take().is_some() {
        stop_scroll_input(state);
        if !state.overlay.is_null() {
            unsafe {
                KillTimer(state.overlay, TIMER_SCROLL);
                KillTimer(state.overlay, editor::TIMER_RECORDING_PREVIEW);
                KillTimer(state.overlay, editor::TIMER_RECORDING_EXPORT_STATUS);
                ShowWindow(state.overlay, SW_HIDE);
                DestroyWindow(state.overlay);
            }
            state.overlay = null_mut();
        }
        state.render = None;
        state.last_hover_query = None;
        state.capture = None;
        state.screen = None;
        update_status(state, false);
        return;
    }
    stop_scroll_input(state);
    if !state.overlay.is_null() {
        unsafe {
            KillTimer(state.overlay, TIMER_SCROLL);
            KillTimer(state.overlay, editor::TIMER_RECORDING_PREVIEW);
            KillTimer(state.overlay, editor::TIMER_RECORDING_EXPORT_STATUS);
            ShowWindow(state.overlay, SW_HIDE);
            DestroyWindow(state.overlay);
        }
        state.overlay = null_mut();
    }
    state.render = None;
    state.last_hover_query = None;
    if !state.scroll_control.is_null() {
        unsafe {
            DestroyWindow(state.scroll_control);
        }
        state.scroll_control = null_mut();
    }
    destroy_scroll_preview(state);
    destroy_scroll_masks(state);
    destroy_scroll_border(state);
    state.capture = None;
    state.screen = None;
    update_status(state, false);
}

pub(super) fn request_cancel(state: &UiState) {
    let _ = state.command_tx.send(Command::Cancel);
    if !state.hwnd.is_null() {
        unsafe {
            PostMessageW(state.hwnd, WAKE, 0, 0);
        }
    }
}

pub(super) fn trigger_capture_or_stop_recording(state: &mut UiState) {
    match state.recording.snapshot().phase.as_str() {
        "recording" => stop_recording(state),
        "selecting" | "processing" | "exporting" | "cancelling" => {}
        "preview" => {
            // The completed recording preview keeps the old capture session
            // alive, which would make begin_capture return without opening a
            // new screenshot. A hotkey should start a fresh capture here.
            state.recording.cancel();
            state.recording_requested_fps = None;
            cancel(state);
            begin_capture(state, false);
        }
        _ => begin_capture(state, false),
    }
}
