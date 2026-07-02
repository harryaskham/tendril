use std::collections::HashSet;
use std::io::Cursor;

use image::{DynamicImage, ImageBuffer, ImageFormat as RasterImageFormat, Rgba};
use x11rb::CURRENT_TIME;
use x11rb::atom_manager;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::image::{Image, PixelLayout};
use x11rb::protocol::randr;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt as _, EventMask, InputFocus,
    MapState, StackMode, VisualClass, Visualid, Window,
};
use x11rb::protocol::xtest;
use x11rb::protocol::xtest::ConnectionExt as _;

use crate::clipboard::{ClipboardSelection, serve_x11_clipboard_in_background};
use crate::error::TendrilError;
use crate::input::reliability_delay;
use crate::model::{Bounds, FocusSnapshot, InputAction, ModifierKey, MouseButton, ScaleFactor};
use crate::platform::{
    AdapterContext, AdapterOperation, Capability, CapabilityErrorReason, CaptureTargetKind,
    InputOutcome, InputRequest, PlatformAdapterError, PlatformKind, TargetDescriptor,
    TargetInventory,
};

const KEY_PRESS_EVENT: u8 = 2;
const KEY_RELEASE_EVENT: u8 = 3;
const BUTTON_PRESS_EVENT: u8 = 4;
const BUTTON_RELEASE_EVENT: u8 = 5;
const MOTION_NOTIFY_EVENT: u8 = 6;
const X11_WHEEL_UP_BUTTON: u8 = 4;
const X11_WHEEL_DOWN_BUTTON: u8 = 5;
const SOLID_BLACK_CHANNEL_THRESHOLD: u8 = 2;
const DRAG_BUTTON_HOLD_DELAY_MS: u64 = 120;
const DOUBLE_CLICK_INTERVAL_MS: u64 = 80;
const DRAG_MOTION_STEP_PX: u64 = 16;
const DRAG_MIN_STEPS: u64 = 6;
const DRAG_MAX_STEPS: u64 = 96;
const X11_UNICODE_PASTE_SERVE_MS: u64 = 1_500;
const X11_TEMPORARY_KEYSYM_SETTLE_MS: u64 = 200;

const XK_BACK_SPACE: u32 = 0xff08;
const XK_TAB: u32 = 0xff09;
const XK_RETURN: u32 = 0xff0d;
const XK_ESCAPE: u32 = 0xff1b;
const XK_HOME: u32 = 0xff50;
const XK_LEFT: u32 = 0xff51;
const XK_UP: u32 = 0xff52;
const XK_RIGHT: u32 = 0xff53;
const XK_DOWN: u32 = 0xff54;
const XK_PAGE_UP: u32 = 0xff55;
const XK_PAGE_DOWN: u32 = 0xff56;
const XK_END: u32 = 0xff57;
const XK_INSERT: u32 = 0xff63;
const XK_DELETE: u32 = 0xffff;
const XK_SHIFT_L: u32 = 0xffe1;
const XK_CONTROL_L: u32 = 0xffe3;
const XK_ALT_L: u32 = 0xffe9;
const XK_SUPER_L: u32 = 0xffeb;

atom_manager! {
    pub Atoms: AtomsCookie {
        UTF8_STRING,
        _NET_ACTIVE_WINDOW,
        _NET_CLIENT_LIST,
        _NET_CLIENT_LIST_STACKING,
        _NET_WM_NAME,
        _NET_WM_PID,
        WM_CLASS,
        WM_NAME,
    }
}

pub(crate) fn discover_targets(
    context: &AdapterContext,
) -> Result<TargetInventory, PlatformAdapterError> {
    let connection = X11Connection::connect(context, AdapterOperation::TargetDiscovery)?;
    let mut targets = discover_displays(context, &connection)?;
    targets.extend(discover_windows(context, &connection)?);
    Ok(TargetInventory { targets })
}

pub(crate) fn capture_target(
    context: &AdapterContext,
    target: CaptureTargetKind,
    target_id: &str,
) -> Result<Vec<u8>, PlatformAdapterError> {
    let connection = X11Connection::connect(context, AdapterOperation::Capture)?;
    match target {
        CaptureTargetKind::Window => capture_window(context, &connection, target_id),
        CaptureTargetKind::Display => capture_display(context, &connection, target_id),
    }
}

pub(crate) fn execute_input(
    platform: PlatformKind,
    x11_display: Option<&str>,
    request: &InputRequest,
) -> Result<InputOutcome, TendrilError> {
    let context = AdapterContext::linux(crate::platform::DesktopSession::X11, None)
        .with_x11_display(x11_display.map(str::to_owned));
    let connection = X11Connection::connect(&context, AdapterOperation::InputControl)
        .map_err(TendrilError::from)?;

    ensure_xtest_available(&connection, platform)?;

    let mut run_state = prepare_x11_input_state(&connection, request)?;
    let keyboard_map = KeyboardMap::load(&connection).map_err(|error| {
        input_execution_error(
            "keyboard_mapping_failed",
            format!("failed to load the X11 keyboard mapping: {error}"),
            None,
            None,
        )
    })?;
    let mut held_modifiers = HashSet::new();

    if let Some(text) = &request.text {
        let dispatch_notes = type_text(
            &connection,
            &keyboard_map,
            request,
            text,
            &held_modifiers,
            Some(0),
            Some("text"),
        )?;
        run_state.notes.extend(dispatch_notes);
        flush_x11_input(&connection, Some(0), Some("text"), "text events")?;
        return Ok(run_state.finish(&connection, request, 1));
    }

    let dispatch_notes =
        dispatch_x11_actions(&connection, &keyboard_map, request, &mut held_modifiers)?;
    run_state.notes.extend(dispatch_notes);
    Ok(run_state.finish(&connection, request, request.actions.len()))
}

#[derive(Debug, Clone)]
struct X11RestoreState {
    window: Window,
    pointer_root: Option<(i32, i32)>,
}

#[derive(Debug, Clone)]
struct X11WindowBounds {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone)]
struct X11OccludingWindow {
    id: String,
    name: String,
    bounds: X11WindowBounds,
}

#[derive(Debug, Clone)]
enum X11WindowCaptureFallbackReason {
    SolidBlackWindowDrawable,
    OverlappingWindows(Vec<X11OccludingWindow>),
}

impl X11WindowCaptureFallbackReason {
    fn summary(&self) -> String {
        match self {
            Self::SolidBlackWindowDrawable => "solid black window-drawable image".to_owned(),
            Self::OverlappingWindows(windows) => format!(
                "{} overlapping X11 window(s) above the target in the stacking order: {}",
                windows.len(),
                summarize_occluding_windows(windows)
            ),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct X11RestoreOutcome {
    focus_restored: bool,
    pointer_restored: bool,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct X11InputRunState {
    focus_required: bool,
    focus_transferred: bool,
    previous_focus: Option<FocusSnapshot>,
    restore_state: Option<X11RestoreState>,
    restore_error: Option<String>,
    notes: Vec<String>,
}

impl X11InputRunState {
    fn finish(
        mut self,
        connection: &X11Connection,
        request: &InputRequest,
        action_count: usize,
    ) -> InputOutcome {
        let X11RestoreOutcome {
            focus_restored,
            pointer_restored,
            error,
        } = restore_x11_state_if_requested(connection, self.restore_state.as_ref());
        if let Some(error) = error {
            self.notes
                .push(format!("Focus restoration after input failed: {error}."));
            self.restore_error = Some(error);
        }

        InputOutcome {
            action_count,
            focus_required: self.focus_required,
            focus_transferred: self.focus_transferred,
            focused_target: self.focus_transferred.then(|| request.target_id.clone()),
            previous_focus: self.previous_focus,
            focus_restored,
            pointer_restored,
            restore_error: self.restore_error,
            notes: self.notes,
        }
    }
}

fn prepare_x11_input_state(
    connection: &X11Connection,
    request: &InputRequest,
) -> Result<X11InputRunState, TendrilError> {
    let mut notes = Vec::new();
    let mut restore_error = None;
    let restore_state = capture_requested_restore_state(
        connection,
        request.restore_focus,
        &mut notes,
        &mut restore_error,
    );
    let previous_focus = restore_state
        .as_ref()
        .map(|state| focus_snapshot_for_window(connection, state.window));

    let keyboard_input = request.text.is_some() || request.actions.iter().any(action_is_keyboard);
    let window_target = matches!(request.target, CaptureTargetKind::Window);
    let focus_required = keyboard_input || window_target;
    let focus_transferred = if window_target {
        prepare_window_focus(connection, request, &mut notes)?
    } else if keyboard_input {
        notes.push(
            "Display-scoped keyboard input uses the currently focused control; place focus explicitly if a different app should receive text or key taps."
                .to_owned(),
        );
        false
    } else {
        false
    };

    Ok(X11InputRunState {
        focus_required,
        focus_transferred,
        previous_focus,
        restore_state,
        restore_error,
        notes,
    })
}

fn capture_requested_restore_state(
    connection: &X11Connection,
    restore_focus: bool,
    notes: &mut Vec<String>,
    restore_error: &mut Option<String>,
) -> Option<X11RestoreState> {
    if !restore_focus {
        notes.push(
            "Focus restoration disabled for this run; focus may remain on the target.".to_owned(),
        );
        return None;
    }

    match capture_x11_restore_state(connection) {
        Ok(Some(state)) => {
            notes.push(format!(
                "Captured previous X11 focus {} and pointer position for post-run restoration.",
                format_window_id(state.window)
            ));
            Some(state)
        }
        Ok(None) => {
            let message = "X11 did not report a restorable active window before input".to_owned();
            notes.push(format!(
                "Focus restoration requested but skipped: {message}."
            ));
            *restore_error = Some(message);
            None
        }
        Err(error) => {
            notes.push(format!(
                "Focus restoration requested but pre-run snapshot failed: {error}."
            ));
            *restore_error = Some(error);
            None
        }
    }
}

fn prepare_window_focus(
    connection: &X11Connection,
    request: &InputRequest,
    notes: &mut Vec<String>,
) -> Result<bool, TendrilError> {
    let window = parse_window_id(&request.target_id)
        .map_err(|message| input_execution_error("invalid_target", message, None, Some("focus")))?;
    activate_window(connection, window).map_err(|error| {
        input_execution_error(
            "focus_failed",
            format!("failed to focus target window before input delivery: {error}"),
            None,
            Some("focus"),
        )
    })?;
    notes.push(
        "Activated the target X11 window before input delivery so mouse gestures and keyboard events are delivered to the requested window instead of the previously focused or stacked target."
            .to_owned(),
    );
    std::thread::sleep(reliability_delay());
    Ok(true)
}

fn dispatch_x11_actions(
    connection: &X11Connection,
    keyboard_map: &KeyboardMap,
    request: &InputRequest,
    held_modifiers: &mut HashSet<ModifierKey>,
) -> Result<Vec<String>, TendrilError> {
    let mut notes = Vec::new();
    for (action_index, action) in request.actions.iter().enumerate() {
        let label = action_label(action);
        dispatch_action(
            connection,
            keyboard_map,
            request,
            action,
            action_index,
            &label,
            held_modifiers,
            &mut notes,
        )?;
        if !matches!(action, InputAction::Wait { .. }) {
            flush_x11_input(connection, Some(action_index), Some(&label), "input events")?;
            std::thread::sleep(reliability_delay());
        }
    }
    Ok(notes)
}

fn flush_x11_input(
    connection: &X11Connection,
    action_index: Option<usize>,
    label: Option<&str>,
    event_kind: &str,
) -> Result<(), TendrilError> {
    connection.conn.flush().map_err(|error| {
        input_execution_error(
            "dispatch_failed",
            format!("failed to flush X11 {event_kind}: {error}"),
            action_index,
            label,
        )
    })
}

fn capture_x11_restore_state(
    connection: &X11Connection,
) -> Result<Option<X11RestoreState>, String> {
    let window = match active_x11_window(connection)? {
        Some(window) => Some(window),
        None => input_focus_window(connection)?,
    };
    let Some(window) = window else {
        return Ok(None);
    };
    let pointer_root = connection
        .conn
        .query_pointer(connection.screen.root)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| (i32::from(reply.root_x), i32::from(reply.root_y)));
    Ok(Some(X11RestoreState {
        window,
        pointer_root,
    }))
}

fn restore_x11_state_if_requested(
    connection: &X11Connection,
    restore_state: Option<&X11RestoreState>,
) -> X11RestoreOutcome {
    let Some(state) = restore_state else {
        return X11RestoreOutcome::default();
    };

    let mut outcome = X11RestoreOutcome::default();
    let mut errors = Vec::new();
    match activate_window(connection, state.window) {
        Ok(()) => outcome.focus_restored = true,
        Err(error) => errors.push(format!("failed to restore X11 focus: {error}")),
    }

    if let Some((x, y)) = state.pointer_root {
        match move_pointer(connection, x, y, None, Some("restore_pointer")) {
            Ok(()) => {
                if let Err(error) = connection.conn.flush() {
                    errors.push(format!(
                        "failed to flush restored X11 pointer position: {error}"
                    ));
                } else {
                    outcome.pointer_restored = true;
                }
            }
            Err(error) => errors.push(format!("failed to restore X11 pointer position: {error}")),
        }
    }

    if !errors.is_empty() {
        outcome.error = Some(errors.join("; "));
    }
    outcome
}

fn active_x11_window(connection: &X11Connection) -> Result<Option<Window>, String> {
    let reply = connection
        .conn
        .get_property(
            false,
            connection.screen.root,
            connection.atoms._NET_ACTIVE_WINDOW,
            AtomEnum::WINDOW,
            0,
            1,
        )
        .map_err(|error| format!("failed to query _NET_ACTIVE_WINDOW: {error}"))?
        .reply()
        .map_err(|error| format!("failed to read _NET_ACTIVE_WINDOW: {error}"))?;
    Ok(reply
        .value32()
        .and_then(|mut values| values.next())
        .filter(|window| *window != 0))
}

fn input_focus_window(connection: &X11Connection) -> Result<Option<Window>, String> {
    let reply = connection
        .conn
        .get_input_focus()
        .map_err(|error| format!("failed to query X11 input focus: {error}"))?
        .reply()
        .map_err(|error| format!("failed to read X11 input focus: {error}"))?;
    Ok(
        (reply.focus != 0 && reply.focus != 1 && reply.focus != connection.screen.root)
            .then_some(reply.focus),
    )
}

fn focus_snapshot_for_window(connection: &X11Connection, window: Window) -> FocusSnapshot {
    FocusSnapshot {
        id: format_window_id(window),
        kind: "window".to_owned(),
        name: connection
            .text_property(window, connection.atoms._NET_WM_NAME)
            .or_else(|| connection.text_property(window, connection.atoms.WM_NAME)),
    }
}

fn discover_displays(
    context: &AdapterContext,
    connection: &X11Connection,
) -> Result<Vec<TargetDescriptor>, PlatformAdapterError> {
    if connection
        .conn
        .extension_information(randr::X11_EXTENSION_NAME)
        .map_err(|error| adapter_failure(context, AdapterOperation::TargetDiscovery, error))?
        .is_none()
    {
        return Ok(vec![fallback_display(connection)]);
    }

    let monitors = connection
        .conn
        .randr_get_monitors(connection.screen.root, true)
        .map_err(|error| adapter_failure(context, AdapterOperation::TargetDiscovery, error))?
        .reply()
        .map_err(|error| adapter_failure(context, AdapterOperation::TargetDiscovery, error))?;

    let mut displays = Vec::new();
    for (index, monitor) in monitors.monitors.into_iter().enumerate() {
        if monitor.width == 0 || monitor.height == 0 {
            continue;
        }
        let id = connection
            .atom_name(monitor.name)
            .unwrap_or_else(|| format!("{}", index + 1));
        displays.push(TargetDescriptor {
            id: id.clone(),
            title: None,
            kind: CaptureTargetKind::Display,
            name: id,
            bounds: Bounds {
                x: i32::from(monitor.x),
                y: i32::from(monitor.y),
                width: u32::from(monitor.width),
                height: u32::from(monitor.height),
            },
            scale_factor: ScaleFactor::identity(),
            capture_supported: true,
            input_supported: true,
            app_name: None,
            process_id: None,
            diagnostics: Vec::new(),
        });
    }

    if displays.is_empty() {
        displays.push(fallback_display(connection));
    }

    Ok(displays)
}

fn fallback_display(connection: &X11Connection) -> TargetDescriptor {
    TargetDescriptor {
        id: format!("screen-{}", connection.screen_num + 1),
        title: None,
        kind: CaptureTargetKind::Display,
        name: format!("screen-{}", connection.screen_num + 1),
        bounds: Bounds {
            x: 0,
            y: 0,
            width: u32::from(connection.screen.width_in_pixels),
            height: u32::from(connection.screen.height_in_pixels),
        },
        scale_factor: ScaleFactor::identity(),
        capture_supported: true,
        input_supported: true,
        app_name: None,
        process_id: None,
        diagnostics: Vec::new(),
    }
}

fn discover_windows(
    context: &AdapterContext,
    connection: &X11Connection,
) -> Result<Vec<TargetDescriptor>, PlatformAdapterError> {
    let mut windows = connection.window_list(context)?;
    if windows.is_empty() {
        windows = connection
            .conn
            .query_tree(connection.screen.root)
            .map_err(|error| adapter_failure(context, AdapterOperation::TargetDiscovery, error))?
            .reply()
            .map(|reply| reply.children)
            .map_err(|error| adapter_failure(context, AdapterOperation::TargetDiscovery, error))?;
    }

    let mut targets = Vec::new();
    for window in windows {
        if let Some(descriptor) = discover_window(connection, window) {
            targets.push(descriptor);
        }
    }
    Ok(targets)
}

fn discover_window(connection: &X11Connection, window: Window) -> Option<TargetDescriptor> {
    let attributes = connection
        .conn
        .get_window_attributes(window)
        .ok()?
        .reply()
        .ok()?;
    if attributes.map_state != MapState::VIEWABLE {
        return None;
    }

    let geometry = connection.conn.get_geometry(window).ok()?.reply().ok()?;
    if geometry.width == 0 || geometry.height == 0 {
        return None;
    }

    let translated = connection
        .conn
        .translate_coordinates(window, connection.screen.root, 0, 0)
        .ok()?
        .reply()
        .ok()?;

    let title = connection
        .text_property(window, connection.atoms._NET_WM_NAME)
        .or_else(|| connection.text_property(window, connection.atoms.WM_NAME))
        .filter(|value| !value.is_empty());
    let app_name = connection.class_name(window);
    let process_id = connection.cardinal_property(window, connection.atoms._NET_WM_PID);
    if crate::discovery::is_filtered_system_window(app_name.as_deref(), title.as_deref()) {
        return None;
    }
    let name = app_name
        .clone()
        .or_else(|| title.clone())
        .unwrap_or_else(|| format_window_id(window));

    Some(TargetDescriptor {
        id: format_window_id(window),
        title,
        kind: CaptureTargetKind::Window,
        name,
        bounds: Bounds {
            x: i32::from(translated.dst_x),
            y: i32::from(translated.dst_y),
            width: u32::from(geometry.width),
            height: u32::from(geometry.height),
        },
        scale_factor: ScaleFactor::identity(),
        capture_supported: true,
        input_supported: true,
        app_name,
        process_id,
        diagnostics: Vec::new(),
    })
}

fn capture_window(
    context: &AdapterContext,
    connection: &X11Connection,
    target_id: &str,
) -> Result<Vec<u8>, PlatformAdapterError> {
    let window = parse_window_id(target_id).map_err(|message| {
        PlatformAdapterError::adapter_failure(AdapterOperation::Capture, context.platform, message)
    })?;
    let geometry = connection
        .conn
        .get_geometry(window)
        .map_err(|error| adapter_failure(context, AdapterOperation::Capture, error))?
        .reply()
        .map_err(|error| adapter_failure(context, AdapterOperation::Capture, error))?;
    if geometry.width == 0 || geometry.height == 0 {
        return Err(PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            context.platform,
            format!("window `{target_id}` has zero-sized bounds"),
        ));
    }

    let occluding_windows = occluding_windows_above_target(context, connection, window)?;
    let direct_capture = capture_region(
        context,
        connection,
        window,
        0,
        0,
        geometry.width,
        geometry.height,
        Some(connection.screen.root_visual),
    )?;

    let direct_is_black = capture_png_is_solid_black(&direct_capture).map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            context.platform,
            format!(
                "failed to validate X11 window capture for `{target_id}` before returning it: {error}"
            ),
        )
    })?;
    if direct_is_black {
        return capture_window_via_root_crop_after_raise(
            context,
            connection,
            target_id,
            window,
            &geometry,
            &X11WindowCaptureFallbackReason::SolidBlackWindowDrawable,
        );
    }

    if !occluding_windows.is_empty() {
        return capture_window_via_root_crop_after_raise(
            context,
            connection,
            target_id,
            window,
            &geometry,
            &X11WindowCaptureFallbackReason::OverlappingWindows(occluding_windows),
        );
    }

    Ok(direct_capture)
}

fn capture_window_via_root_crop_after_raise(
    context: &AdapterContext,
    connection: &X11Connection,
    target_id: &str,
    window: Window,
    geometry: &x11rb::protocol::xproto::GetGeometryReply,
    reason: &X11WindowCaptureFallbackReason,
) -> Result<Vec<u8>, PlatformAdapterError> {
    let reason_summary = reason.summary();
    let restore_state = capture_x11_restore_state(connection).map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            context.platform,
            format!(
                "X11 window capture for `{target_id}` detected {reason_summary}. Tendril would need to temporarily raise the target and capture a root-window crop, but it could not snapshot the current focus for restoration: {error}"
            ),
        )
    })?;

    activate_window(connection, window).map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            context.platform,
            format!(
                "X11 window capture for `{target_id}` detected {reason_summary}. Tendril could not raise it for an unoccluded root-window crop fallback: {error}"
            ),
        )
    })?;
    std::thread::sleep(reliability_delay());

    let fallback_result = capture_raised_window_from_root_crop(
        context,
        connection,
        target_id,
        window,
        geometry.width,
        geometry.height,
        &reason_summary,
    );
    let restore_outcome = restore_x11_state_if_requested(connection, restore_state.as_ref());

    match (fallback_result, restore_outcome.error) {
        (Ok(image_bytes), None) => Ok(image_bytes),
        (Ok(_), Some(restore_error)) => Err(PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            context.platform,
            format!(
                "X11 window capture for `{target_id}` recovered from {reason_summary} by raising and root-cropping the target, but focus restoration failed afterwards: {restore_error}"
            ),
        )),
        (Err(error), None) => Err(error),
        (Err(error), Some(restore_error)) => Err(PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            context.platform,
            format!(
                "{error}; additionally, focus restoration failed after the fallback attempt: {restore_error}"
            ),
        )),
    }
}

fn capture_raised_window_from_root_crop(
    context: &AdapterContext,
    connection: &X11Connection,
    target_id: &str,
    window: Window,
    width: u16,
    height: u16,
    reason_summary: &str,
) -> Result<Vec<u8>, PlatformAdapterError> {
    let translated = connection
        .conn
        .translate_coordinates(window, connection.screen.root, 0, 0)
        .map_err(|error| adapter_failure(context, AdapterOperation::Capture, error))?
        .reply()
        .map_err(|error| adapter_failure(context, AdapterOperation::Capture, error))?;

    let image_bytes = capture_region(
        context,
        connection,
        connection.screen.root,
        translated.dst_x,
        translated.dst_y,
        width,
        height,
        Some(connection.screen.root_visual),
    )?;

    let fallback_is_black = capture_png_is_solid_black(&image_bytes).map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            context.platform,
            format!(
                "failed to validate X11 root-crop fallback capture for `{target_id}` before returning it: {error}"
            ),
        )
    })?;
    if fallback_is_black {
        return Err(PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            context.platform,
            format!(
                "X11 window capture for `{target_id}` detected {reason_summary}, then the raise-plus-root-crop fallback was also solid black. The target may be minimized, outside the visible display, still occluded by an override-redirect window, or otherwise unavailable to XGetImage; capture a visible display target or make the window visible before retrying."
            ),
        ));
    }

    Ok(image_bytes)
}

fn capture_png_is_solid_black(image_bytes: &[u8]) -> Result<bool, String> {
    let decoded = image::load_from_memory(image_bytes)
        .map_err(|error| format!("captured PNG could not be decoded: {error}"))?;
    let rgba = decoded.to_rgba8();
    if rgba.width() == 0 || rgba.height() == 0 {
        return Ok(false);
    }

    Ok(rgba.pixels().all(|pixel| {
        pixel[0] <= SOLID_BLACK_CHANNEL_THRESHOLD
            && pixel[1] <= SOLID_BLACK_CHANNEL_THRESHOLD
            && pixel[2] <= SOLID_BLACK_CHANNEL_THRESHOLD
    }))
}

fn occluding_windows_above_target(
    context: &AdapterContext,
    connection: &X11Connection,
    target: Window,
) -> Result<Vec<X11OccludingWindow>, PlatformAdapterError> {
    let Some(target_bounds) = viewable_window_bounds(connection, target).map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            context.platform,
            format!("failed to inspect X11 target bounds for occlusion detection: {error}"),
        )
    })?
    else {
        return Ok(Vec::new());
    };

    let stacking = stacking_windows(context, connection)?;
    let Some(target_index) = stacking.iter().position(|window| *window == target) else {
        return Ok(Vec::new());
    };

    let mut occluding_windows = Vec::new();
    for candidate in stacking.iter().skip(target_index + 1).copied() {
        if candidate == target {
            continue;
        }
        let Some(bounds) = viewable_window_bounds(connection, candidate).map_err(|error| {
            PlatformAdapterError::adapter_failure(
                AdapterOperation::Capture,
                context.platform,
                format!(
                    "failed to inspect X11 stacking candidate {} for occlusion detection: {error}",
                    format_window_id(candidate)
                ),
            )
        })?
        else {
            continue;
        };
        if windows_overlap(&target_bounds, &bounds) {
            occluding_windows.push(X11OccludingWindow {
                id: format_window_id(candidate),
                name: window_display_name(connection, candidate),
                bounds,
            });
        }
    }

    Ok(occluding_windows)
}

fn stacking_windows(
    context: &AdapterContext,
    connection: &X11Connection,
) -> Result<Vec<Window>, PlatformAdapterError> {
    let windows = connection.window_list(context)?;
    if !windows.is_empty() {
        return Ok(windows);
    }

    connection
        .conn
        .query_tree(connection.screen.root)
        .map_err(|error| adapter_failure(context, AdapterOperation::Capture, error))?
        .reply()
        .map(|reply| reply.children)
        .map_err(|error| adapter_failure(context, AdapterOperation::Capture, error))
}

fn viewable_window_bounds(
    connection: &X11Connection,
    window: Window,
) -> Result<Option<X11WindowBounds>, String> {
    let attributes = connection
        .conn
        .get_window_attributes(window)
        .map_err(|error| format!("failed to query window attributes: {error}"))?
        .reply()
        .map_err(|error| format!("failed to read window attributes: {error}"))?;
    if attributes.map_state != MapState::VIEWABLE {
        return Ok(None);
    }

    let geometry = connection
        .conn
        .get_geometry(window)
        .map_err(|error| format!("failed to query window geometry: {error}"))?
        .reply()
        .map_err(|error| format!("failed to read window geometry: {error}"))?;
    if geometry.width == 0 || geometry.height == 0 {
        return Ok(None);
    }

    let translated = connection
        .conn
        .translate_coordinates(window, connection.screen.root, 0, 0)
        .map_err(|error| format!("failed to translate window coordinates: {error}"))?
        .reply()
        .map_err(|error| format!("failed to read translated window coordinates: {error}"))?;

    Ok(Some(X11WindowBounds {
        x: i32::from(translated.dst_x),
        y: i32::from(translated.dst_y),
        width: u32::from(geometry.width),
        height: u32::from(geometry.height),
    }))
}

fn windows_overlap(left: &X11WindowBounds, right: &X11WindowBounds) -> bool {
    let left_right = left.x.saturating_add(u32_to_i32_saturating(left.width));
    let left_bottom = left.y.saturating_add(u32_to_i32_saturating(left.height));
    let right_right = right.x.saturating_add(u32_to_i32_saturating(right.width));
    let right_bottom = right.y.saturating_add(u32_to_i32_saturating(right.height));

    left.x < right_right && right.x < left_right && left.y < right_bottom && right.y < left_bottom
}

fn u32_to_i32_saturating(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn window_display_name(connection: &X11Connection, window: Window) -> String {
    connection
        .text_property(window, connection.atoms._NET_WM_NAME)
        .or_else(|| connection.text_property(window, connection.atoms.WM_NAME))
        .or_else(|| connection.class_name(window))
        .unwrap_or_else(|| format_window_id(window))
}

fn summarize_occluding_windows(windows: &[X11OccludingWindow]) -> String {
    windows
        .iter()
        .take(3)
        .map(|window| {
            format!(
                "{} `{}` at {},{} {}x{}",
                window.id,
                window.name,
                window.bounds.x,
                window.bounds.y,
                window.bounds.width,
                window.bounds.height
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn capture_display(
    context: &AdapterContext,
    connection: &X11Connection,
    target_id: &str,
) -> Result<Vec<u8>, PlatformAdapterError> {
    let mut displays = discover_displays(context, connection)?;
    // Sort displays the same way sort_inventory does so numeric index lookup
    // matches the sequential IDs assigned by discovery.
    displays.sort_by(|left, right| {
        let left_key = (
            left.bounds.y,
            left.bounds.x,
            left.name.to_ascii_lowercase(),
            left.id.to_ascii_lowercase(),
        );
        let right_key = (
            right.bounds.y,
            right.bounds.x,
            right.name.to_ascii_lowercase(),
            right.id.to_ascii_lowercase(),
        );
        left_key.cmp(&right_key)
    });
    // target_id is a 1-based numeric index assigned by sort_inventory.
    let index: usize = target_id.parse().map_err(|_| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            context.platform,
            format!("display `{target_id}` is not a valid numeric display index"),
        )
    })?;
    let display = displays.get(index.saturating_sub(1)).ok_or_else(|| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            context.platform,
            format!("display `{target_id}` was not found during capture"),
        )
    })?;

    let x = clamp_i16(display.bounds.x).map_err(|message| {
        PlatformAdapterError::adapter_failure(AdapterOperation::Capture, context.platform, message)
    })?;
    let y = clamp_i16(display.bounds.y).map_err(|message| {
        PlatformAdapterError::adapter_failure(AdapterOperation::Capture, context.platform, message)
    })?;
    let width = clamp_u16(display.bounds.width).map_err(|message| {
        PlatformAdapterError::adapter_failure(AdapterOperation::Capture, context.platform, message)
    })?;
    let height = clamp_u16(display.bounds.height).map_err(|message| {
        PlatformAdapterError::adapter_failure(AdapterOperation::Capture, context.platform, message)
    })?;

    capture_region(
        context,
        connection,
        connection.screen.root,
        x,
        y,
        width,
        height,
        Some(connection.screen.root_visual),
    )
}

#[allow(clippy::too_many_arguments)]
fn capture_region(
    context: &AdapterContext,
    connection: &X11Connection,
    drawable: Window,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    fallback_visual: Option<Visualid>,
) -> Result<Vec<u8>, PlatformAdapterError> {
    let (image, visual) = Image::get(&connection.conn, drawable, x, y, width, height)
        .map_err(|error| adapter_failure(context, AdapterOperation::Capture, error))?;
    let visual = if visual == 0 {
        fallback_visual
    } else {
        Some(visual)
    };
    encode_png(&image, connection, visual).map_err(|error| {
        PlatformAdapterError::adapter_failure(AdapterOperation::Capture, context.platform, error)
    })
}

fn encode_png(
    image: &Image<'_>,
    connection: &X11Connection,
    visual: Option<Visualid>,
) -> Result<Vec<u8>, String> {
    let visual_id = visual.unwrap_or(connection.screen.root_visual);
    let visual = lookup_visual(connection.conn.setup(), visual_id)
        .ok_or_else(|| format!("failed to resolve X11 visual 0x{visual_id:x} for capture"))?;
    if !matches!(
        visual.class,
        VisualClass::TRUE_COLOR | VisualClass::DIRECT_COLOR
    ) {
        return Err(format!(
            "capture requires a TrueColor/DirectColor visual, got {:?}",
            visual.class
        ));
    }
    let pixel_layout = PixelLayout::from_visual_type(visual)
        .map_err(|error| format!("failed to derive pixel layout from X11 visual: {error}"))?;

    let mut rgba = ImageBuffer::new(u32::from(image.width()), u32::from(image.height()));
    for y in 0..image.height() {
        for x in 0..image.width() {
            let (red, green, blue) = pixel_layout.decode(image.get_pixel(x, y));
            rgba.put_pixel(
                u32::from(x),
                u32::from(y),
                Rgba([
                    u8::try_from(red >> 8).unwrap_or(0),
                    u8::try_from(green >> 8).unwrap_or(0),
                    u8::try_from(blue >> 8).unwrap_or(0),
                    255,
                ]),
            );
        }
    }

    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(rgba)
        .write_to(&mut Cursor::new(&mut bytes), RasterImageFormat::Png)
        .map_err(|error| format!("failed to encode captured X11 image as png: {error}"))?;
    Ok(bytes)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn dispatch_action(
    connection: &X11Connection,
    keyboard_map: &KeyboardMap,
    request: &InputRequest,
    action: &InputAction,
    action_index: usize,
    label: &str,
    held_modifiers: &mut HashSet<ModifierKey>,
    notes: &mut Vec<String>,
) -> Result<(), TendrilError> {
    match action {
        InputAction::KeyTap { key } => {
            let keysym = key_name_to_keysym(key).ok_or_else(|| {
                input_execution_error(
                    "unsupported_key",
                    format!("key `{key}` is not supported by the X11 backend"),
                    Some(action_index),
                    Some(label),
                )
            })?;
            let stroke = keyboard_map.stroke_for_keysym(keysym).ok_or_else(|| {
                input_execution_error(
                    "key_not_mapped",
                    format!("key `{key}` is not available in the active X11 keyboard map"),
                    Some(action_index),
                    Some(label),
                )
            })?;
            send_key_stroke(
                connection,
                keyboard_map,
                stroke,
                held_modifiers,
                Some(action_index),
                Some(label),
            )
        }
        InputAction::Hold { modifier } => {
            let stroke = keyboard_map.modifier_stroke(*modifier).ok_or_else(|| {
                input_execution_error(
                    "modifier_not_mapped",
                    format!(
                        "modifier `{modifier:?}` is not available in the active X11 keyboard map"
                    ),
                    Some(action_index),
                    Some(label),
                )
            })?;
            fake_key_event(
                connection,
                KEY_PRESS_EVENT,
                stroke.keycode,
                Some(action_index),
                Some(label),
            )?;
            held_modifiers.insert(*modifier);
            Ok(())
        }
        InputAction::Release { modifier } => {
            let stroke = keyboard_map.modifier_stroke(*modifier).ok_or_else(|| {
                input_execution_error(
                    "modifier_not_mapped",
                    format!(
                        "modifier `{modifier:?}` is not available in the active X11 keyboard map"
                    ),
                    Some(action_index),
                    Some(label),
                )
            })?;
            fake_key_event(
                connection,
                KEY_RELEASE_EVENT,
                stroke.keycode,
                Some(action_index),
                Some(label),
            )?;
            held_modifiers.remove(modifier);
            Ok(())
        }
        InputAction::Send { text } => {
            let dispatch_notes = type_text(
                connection,
                keyboard_map,
                request,
                text,
                held_modifiers,
                Some(action_index),
                Some(label),
            )?;
            notes.extend(dispatch_notes);
            Ok(())
        }
        InputAction::Wait { duration_ms } => {
            std::thread::sleep(std::time::Duration::from_millis(*duration_ms));
            Ok(())
        }
        InputAction::Click { button, x, y } => click_button(
            connection,
            request,
            *button,
            *x,
            *y,
            Some(action_index),
            Some(label),
        ),
        InputAction::PointerMove { x, y } => {
            pointer_move(connection, request, *x, *y, Some(action_index), Some(label))
        }
        InputAction::DoubleClick { x, y } => {
            double_click_button(connection, request, *x, *y, Some(action_index), Some(label))
        }
        InputAction::Drag { x0, y0, x1, y1 } => drag_mouse(
            connection,
            request,
            *x0,
            *y0,
            *x1,
            *y1,
            Some(action_index),
            Some(label),
        ),
        InputAction::Scroll { x, y, dy } => scroll_wheel(
            connection,
            request,
            *x,
            *y,
            *dy,
            Some(action_index),
            Some(label),
        ),
        InputAction::ElementClick { .. } => Err(input_execution_error(
            "unresolved_element_click_action",
            "click(<element-id>) should be resolved to target coordinates before reaching the X11 input adapter".to_owned(),
            Some(action_index),
            Some(label),
        )),
    }
}

fn pointer_move(
    connection: &X11Connection,
    request: &InputRequest,
    x: i32,
    y: i32,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    let (absolute_x, absolute_y) = absolute_point(request, x, y);
    move_pointer(connection, absolute_x, absolute_y, action_index, action)?;
    flush_x11_input(connection, action_index, action, "pointer move")?;
    std::thread::sleep(reliability_delay());
    Ok(())
}

fn scroll_wheel(
    connection: &X11Connection,
    request: &InputRequest,
    x: i32,
    y: i32,
    dy: i32,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    let (absolute_x, absolute_y) = absolute_point(request, x, y);
    move_pointer(connection, absolute_x, absolute_y, action_index, action)?;
    flush_x11_input(connection, action_index, action, "scroll pointer move")?;
    std::thread::sleep(reliability_delay());

    let button = if dy > 0 {
        X11_WHEEL_DOWN_BUTTON
    } else {
        X11_WHEEL_UP_BUTTON
    };

    for _ in 0..dy.unsigned_abs() {
        fake_button_event(connection, BUTTON_PRESS_EVENT, button, action_index, action)?;
        flush_x11_input(connection, action_index, action, "scroll button press")?;
        std::thread::sleep(reliability_delay());
        fake_button_event(
            connection,
            BUTTON_RELEASE_EVENT,
            button,
            action_index,
            action,
        )?;
        flush_x11_input(connection, action_index, action, "scroll button release")?;
        std::thread::sleep(reliability_delay());
    }

    Ok(())
}

fn click_button(
    connection: &X11Connection,
    request: &InputRequest,
    button: MouseButton,
    x: i32,
    y: i32,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    let (absolute_x, absolute_y) = absolute_point(request, x, y);
    move_pointer(connection, absolute_x, absolute_y, action_index, action)?;
    flush_x11_input(connection, action_index, action, "click pointer move")?;
    std::thread::sleep(reliability_delay());

    click_button_at_current_pointer(connection, button, action_index, action, "click")
}

fn double_click_button(
    connection: &X11Connection,
    request: &InputRequest,
    x: i32,
    y: i32,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    let (absolute_x, absolute_y) = absolute_point(request, x, y);
    move_pointer(connection, absolute_x, absolute_y, action_index, action)?;
    flush_x11_input(
        connection,
        action_index,
        action,
        "double-click pointer move",
    )?;
    std::thread::sleep(reliability_delay());

    click_button_at_current_pointer(
        connection,
        MouseButton::Left,
        action_index,
        action,
        "double-click first click",
    )?;
    std::thread::sleep(std::time::Duration::from_millis(DOUBLE_CLICK_INTERVAL_MS));
    click_button_at_current_pointer(
        connection,
        MouseButton::Left,
        action_index,
        action,
        "double-click second click",
    )
}

fn click_button_at_current_pointer(
    connection: &X11Connection,
    button: MouseButton,
    action_index: Option<usize>,
    action: Option<&str>,
    context: &str,
) -> Result<(), TendrilError> {
    let button = mouse_button_number(button);
    fake_button_event(connection, BUTTON_PRESS_EVENT, button, action_index, action)?;
    flush_x11_input(
        connection,
        action_index,
        action,
        &format!("{context} button press"),
    )?;
    std::thread::sleep(reliability_delay());

    fake_button_event(
        connection,
        BUTTON_RELEASE_EVENT,
        button,
        action_index,
        action,
    )?;
    flush_x11_input(
        connection,
        action_index,
        action,
        &format!("{context} button release"),
    )
}

#[allow(clippy::too_many_arguments)]
fn drag_mouse(
    connection: &X11Connection,
    request: &InputRequest,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    let (start_x, start_y) = absolute_point(request, x0, y0);
    let (end_x, end_y) = absolute_point(request, x1, y1);
    move_pointer(connection, start_x, start_y, action_index, action)?;
    flush_x11_input(connection, action_index, action, "drag start pointer move")?;
    std::thread::sleep(reliability_delay());

    fake_button_event(connection, BUTTON_PRESS_EVENT, 1, action_index, action)?;
    flush_x11_input(connection, action_index, action, "drag button press")?;
    std::thread::sleep(std::time::Duration::from_millis(DRAG_BUTTON_HOLD_DELAY_MS));

    for (x, y) in drag_motion_points(start_x, start_y, end_x, end_y) {
        move_pointer(connection, x, y, action_index, action)?;
        flush_x11_input(connection, action_index, action, "drag motion")?;
        std::thread::sleep(reliability_delay());
    }

    fake_button_event(connection, BUTTON_RELEASE_EVENT, 1, action_index, action)?;
    flush_x11_input(connection, action_index, action, "drag button release")
}

fn drag_motion_points(start_x: i32, start_y: i32, end_x: i32, end_y: i32) -> Vec<(i32, i32)> {
    let dx = i64::from(end_x) - i64::from(start_x);
    let dy = i64::from(end_y) - i64::from(start_y);
    let distance = dx.unsigned_abs().max(dy.unsigned_abs());
    let steps = ((distance / DRAG_MOTION_STEP_PX) + 1).clamp(DRAG_MIN_STEPS, DRAG_MAX_STEPS);
    (1..=steps)
        .map(|step| {
            let step = i64::try_from(step).unwrap_or(1);
            let steps = i64::try_from(steps).unwrap_or(1);
            let x = i64::from(start_x) + (dx * step / steps);
            let y = i64::from(start_y) + (dy * step / steps);
            (
                i32::try_from(x).unwrap_or(if x < 0 { i32::MIN } else { i32::MAX }),
                i32::try_from(y).unwrap_or(if y < 0 { i32::MIN } else { i32::MAX }),
            )
        })
        .collect()
}

fn absolute_point(request: &InputRequest, x: i32, y: i32) -> (i32, i32) {
    match request.target {
        CaptureTargetKind::Window | CaptureTargetKind::Display => (
            request.bounds.x.saturating_add(x),
            request.bounds.y.saturating_add(y),
        ),
    }
}

fn move_pointer(
    connection: &X11Connection,
    x: i32,
    y: i32,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    let root_x = clamp_i16(x).map_err(|message| {
        input_execution_error("coordinate_out_of_range", message, action_index, action)
    })?;
    let root_y = clamp_i16(y).map_err(|message| {
        input_execution_error("coordinate_out_of_range", message, action_index, action)
    })?;
    connection
        .conn
        .xtest_fake_input(
            MOTION_NOTIFY_EVENT,
            0,
            CURRENT_TIME,
            connection.screen.root,
            root_x,
            root_y,
            0,
        )
        .map_err(|error| {
            input_execution_error(
                "dispatch_failed",
                format!("failed to move the pointer: {error}"),
                action_index,
                action,
            )
        })?;
    Ok(())
}

fn fake_button_event(
    connection: &X11Connection,
    event_type: u8,
    button: u8,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    connection
        .conn
        .xtest_fake_input(
            event_type,
            button,
            CURRENT_TIME,
            connection.screen.root,
            0,
            0,
            0,
        )
        .map_err(|error| {
            input_execution_error(
                "dispatch_failed",
                format!("failed to dispatch the X11 button event: {error}"),
                action_index,
                action,
            )
        })?;
    Ok(())
}

fn fake_key_event(
    connection: &X11Connection,
    event_type: u8,
    keycode: u8,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    connection
        .conn
        .xtest_fake_input(
            event_type,
            keycode,
            CURRENT_TIME,
            connection.screen.root,
            0,
            0,
            0,
        )
        .map_err(|error| {
            input_execution_error(
                "dispatch_failed",
                format!("failed to dispatch the X11 key event: {error}"),
                action_index,
                action,
            )
        })?;
    Ok(())
}

fn type_text(
    connection: &X11Connection,
    keyboard_map: &KeyboardMap,
    request: &InputRequest,
    text: &str,
    held_modifiers: &HashSet<ModifierKey>,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<Vec<String>, TendrilError> {
    if let Some(character) = first_unmapped_text_character(keyboard_map, text) {
        if !held_modifiers.is_empty() {
            return Err(input_execution_error(
                "character_not_mapped",
                format!("character `{character}` is not available in the active X11 keyboard map and clipboard paste fallback cannot run while modifiers are held"),
                action_index,
                action,
            )
            .with_detail_entry("fallback", serde_json::json!("x11_clipboard_paste"))
            .with_detail_entry("fallback_available", serde_json::json!(false))
            .with_detail_entry(
                "suggested_action",
                serde_json::json!("Release held modifiers before send(...) when entering non-ASCII text on X11, or paste with an explicit clipboard workflow."),
            ));
        }
        if is_x11_temporary_keysym_unicode_target(request) {
            return type_text_via_temporary_x11_keysyms(
                connection,
                keyboard_map,
                text,
                character,
                held_modifiers,
                action_index,
                action,
            );
        }
        return paste_text_via_x11_selection(
            connection,
            keyboard_map,
            request,
            text,
            character,
            action_index,
            action,
        );
    }

    for character in text.chars() {
        let stroke = keyboard_map.stroke_for_char(character).ok_or_else(|| {
            input_execution_error(
                "character_not_mapped",
                format!("character `{character}` is not available in the active X11 keyboard map"),
                action_index,
                action,
            )
        })?;
        send_key_stroke(
            connection,
            keyboard_map,
            stroke,
            held_modifiers,
            action_index,
            action,
        )?;
    }
    Ok(Vec::new())
}

fn first_unmapped_text_character(keyboard_map: &KeyboardMap, text: &str) -> Option<char> {
    text.chars()
        .find(|character| keyboard_map.stroke_for_char(*character).is_none())
}

fn type_text_via_temporary_x11_keysyms(
    connection: &X11Connection,
    keyboard_map: &KeyboardMap,
    text: &str,
    unmapped_character: char,
    held_modifiers: &HashSet<ModifierKey>,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<Vec<String>, TendrilError> {
    let mut remapped_count = 0usize;
    for character in text.chars() {
        if let Some(stroke) = keyboard_map.stroke_for_char(character) {
            send_key_stroke(
                connection,
                keyboard_map,
                stroke,
                held_modifiers,
                action_index,
                action,
            )?;
        } else {
            send_temporary_x11_keysym_character(
                connection,
                keyboard_map,
                character,
                held_modifiers,
                action_index,
                action,
            )?;
            remapped_count += 1;
        }
    }

    Ok(vec![format!(
        "X11 send(...) temporarily mapped a keycode for {remapped_count} Unicode character(s) because character `{unmapped_character}` is not present in the active keyboard map; this preserves rich-editor caret state without replacing existing content."
    )])
}

fn send_temporary_x11_keysym_character(
    connection: &X11Connection,
    keyboard_map: &KeyboardMap,
    character: char,
    held_modifiers: &HashSet<ModifierKey>,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    let keycode = keyboard_map.temporary_unicode_keycode();
    let original = keyboard_map.keysyms_for_keycode(keycode).ok_or_else(|| {
        input_execution_error(
            "unicode_keycode_not_available",
            "no X11 keycode is available for temporary Unicode keysym mapping",
            action_index,
            action,
        )
    })?;
    let mut replacement = vec![0; usize::from(keyboard_map.keysyms_per_keycode)];
    if let Some(first) = replacement.first_mut() {
        *first = keysym_for_char(character);
    }

    change_single_key_mapping(
        connection,
        keycode,
        keyboard_map.keysyms_per_keycode,
        &replacement,
        action_index,
        action,
    )?;
    std::thread::sleep(std::time::Duration::from_millis(
        X11_TEMPORARY_KEYSYM_SETTLE_MS,
    ));
    let dispatch_result = send_key_stroke(
        connection,
        keyboard_map,
        KeyStroke {
            keycode,
            shift: false,
        },
        held_modifiers,
        action_index,
        action,
    )
    .and_then(|()| {
        // X11 KeyPress events carry a keycode, and clients resolve that
        // keycode against the keyboard map when they process the event. Flush
        // the synthetic key event and leave the temporary map installed briefly
        // before restoring it, otherwise slower consumers such as Firefox's
        // contenteditable rich editor can observe the restored map and drop the
        // first unmapped Unicode segment.
        flush_x11_input(
            connection,
            action_index,
            action,
            "temporary Unicode key event",
        )?;
        std::thread::sleep(std::time::Duration::from_millis(
            X11_TEMPORARY_KEYSYM_SETTLE_MS,
        ));
        Ok(())
    });

    let restore_result = change_single_key_mapping(
        connection,
        keycode,
        keyboard_map.keysyms_per_keycode,
        &original,
        action_index,
        action,
    );
    dispatch_result?;
    restore_result?;
    std::thread::sleep(reliability_delay());
    Ok(())
}

fn change_single_key_mapping(
    connection: &X11Connection,
    keycode: u8,
    keysyms_per_keycode: u8,
    keysyms: &[u32],
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    connection
        .conn
        .change_keyboard_mapping(1, keycode, keysyms_per_keycode, keysyms)
        .map_err(|error| {
            input_execution_error(
                "unicode_keycode_mapping_failed",
                format!("failed to temporarily change the X11 keyboard mapping: {error}"),
                action_index,
                action,
            )
        })?;
    flush_x11_input(
        connection,
        action_index,
        action,
        "temporary Unicode key mapping",
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct X11UnicodePasteAttempt {
    selection: ClipboardSelection,
    shortcut: X11PasteShortcut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum X11PasteShortcut {
    CtrlV,
    ShiftInsert,
    CtrlShiftV,
}

impl X11PasteShortcut {
    fn label(self) -> &'static str {
        match self {
            Self::CtrlV => "Ctrl+V",
            Self::ShiftInsert => "Shift+Insert",
            Self::CtrlShiftV => "Ctrl+Shift+V",
        }
    }
}

fn paste_text_via_x11_selection(
    connection: &X11Connection,
    keyboard_map: &KeyboardMap,
    request: &InputRequest,
    text: &str,
    unmapped_character: char,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<Vec<String>, TendrilError> {
    let attempts = x11_unicode_paste_attempts(request);
    let mut failed_attempts = Vec::new();
    let mut last_error = None;

    for attempt in attempts {
        match try_x11_unicode_paste_attempt(
            connection,
            keyboard_map,
            text,
            attempt,
            action_index,
            action,
        ) {
            Ok(Some((served_requests, mut notes))) => {
                let fallback = if is_terminal_paste_attempt(attempt) {
                    "terminal-compatible selection paste fallback"
                } else {
                    "transient CLIPBOARD paste fallback"
                };
                notes.insert(
                    0,
                    format!(
                        "X11 send(...) used a {fallback} because character `{unmapped_character}` is not present in the active keyboard map; Tendril temporarily owned the {} selection and dispatched {}.",
                        clipboard_selection_name(attempt.selection).to_uppercase(),
                        attempt.shortcut.label()
                    ),
                );
                notes.push(format!(
                    "X11 Unicode paste fallback served {served_requests} selection request(s) for {} bytes of UTF-8 text via {} + {}.",
                    text.len(),
                    clipboard_selection_name(attempt.selection),
                    attempt.shortcut.label()
                ));
                return Ok(notes);
            }
            Ok(None) => failed_attempts.push(serde_json::json!({
                "selection": clipboard_selection_name(attempt.selection),
                "shortcut": attempt.shortcut.label(),
                "served_requests": 0,
            })),
            Err(error) => {
                failed_attempts.push(serde_json::json!({
                    "selection": clipboard_selection_name(attempt.selection),
                    "shortcut": attempt.shortcut.label(),
                    "error": error.to_string(),
                }));
                last_error = Some(error);
            }
        }
    }

    let terminal_target = is_x11_terminal_target(request);
    let mut error = input_execution_error(
        "clipboard_paste_unserved",
        if terminal_target {
            "X11 Unicode paste fallback tried terminal-compatible paste shortcuts, but no application requested Tendril's selection text"
        } else {
            "X11 clipboard paste fallback dispatched Ctrl+V but no application requested Tendril's clipboard text"
        },
        action_index,
        action,
    )
    .with_detail_entry(
        "fallback",
        serde_json::json!(if terminal_target {
            "x11_terminal_selection_paste"
        } else {
            "x11_clipboard_paste"
        }),
    )
    .with_detail_entry("text_len", serde_json::json!(text.len()))
    .with_detail_entry("attempts", serde_json::json!(failed_attempts))
    .with_detail_entry(
        "suggested_action",
        serde_json::json!(if terminal_target {
            "Capture the terminal to verify focus is at a shell prompt, then retry send(...). Tendril should use PRIMARY+Shift+Insert for XTerm-like targets and CLIPBOARD terminal paste chords for other X11 terminals."
        } else {
            "Capture the target to verify focus is in an editable control, then retry send(...) or use tendril clipboard set plus an explicit paste shortcut."
        }),
    );
    if let Some(last_error) = last_error {
        error = error.with_detail_entry("last_error", serde_json::json!(last_error.to_string()));
    }
    Err(error)
}

fn try_x11_unicode_paste_attempt(
    connection: &X11Connection,
    keyboard_map: &KeyboardMap,
    text: &str,
    attempt: X11UnicodePasteAttempt,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<Option<(usize, Vec<String>)>, TendrilError> {
    let owner = serve_x11_clipboard_in_background(
        attempt.selection,
        text,
        std::time::Duration::from_millis(X11_UNICODE_PASTE_SERVE_MS),
    )
    .map_err(|error| {
        error
            .with_detail_entry("fallback", serde_json::json!("x11_unicode_paste"))
            .with_detail_entry(
                "selection",
                serde_json::json!(clipboard_selection_name(attempt.selection)),
            )
            .with_detail_entry("shortcut", serde_json::json!(attempt.shortcut.label()))
            .with_detail_entry("action", serde_json::json!(action))
    })?;

    let paste_result = send_x11_paste_shortcut(
        connection,
        keyboard_map,
        attempt.shortcut,
        action_index,
        action,
    );
    let serve_result = owner.join();
    if let Err(error) = paste_result {
        let _ = serve_result;
        return Err(error
            .with_detail_entry(
                "selection",
                serde_json::json!(clipboard_selection_name(attempt.selection)),
            )
            .with_detail_entry("shortcut", serde_json::json!(attempt.shortcut.label())));
    }
    let (served_requests, notes) = serve_result.map_err(|error| {
        error
            .with_detail_entry("fallback", serde_json::json!("x11_unicode_paste"))
            .with_detail_entry(
                "selection",
                serde_json::json!(clipboard_selection_name(attempt.selection)),
            )
            .with_detail_entry("shortcut", serde_json::json!(attempt.shortcut.label()))
            .with_detail_entry("action", serde_json::json!(action))
    })?;

    if served_requests == 0 {
        Ok(None)
    } else {
        Ok(Some((served_requests, notes)))
    }
}

fn x11_unicode_paste_attempts(request: &InputRequest) -> Vec<X11UnicodePasteAttempt> {
    if is_x11_terminal_target(request) {
        return vec![
            X11UnicodePasteAttempt {
                selection: ClipboardSelection::Primary,
                shortcut: X11PasteShortcut::ShiftInsert,
            },
            X11UnicodePasteAttempt {
                selection: ClipboardSelection::Clipboard,
                shortcut: X11PasteShortcut::ShiftInsert,
            },
            X11UnicodePasteAttempt {
                selection: ClipboardSelection::Clipboard,
                shortcut: X11PasteShortcut::CtrlShiftV,
            },
            X11UnicodePasteAttempt {
                selection: ClipboardSelection::Clipboard,
                shortcut: X11PasteShortcut::CtrlV,
            },
        ];
    }

    vec![X11UnicodePasteAttempt {
        selection: ClipboardSelection::Clipboard,
        shortcut: X11PasteShortcut::CtrlV,
    }]
}

fn is_terminal_paste_attempt(attempt: X11UnicodePasteAttempt) -> bool {
    matches!(
        attempt.shortcut,
        X11PasteShortcut::ShiftInsert | X11PasteShortcut::CtrlShiftV
    )
}

fn is_x11_terminal_target(request: &InputRequest) -> bool {
    let haystack = x11_target_haystack(request);

    [
        "xterm",
        "uxterm",
        "terminal",
        "gnome-terminal",
        "konsole",
        "xfce4-terminal",
        "mate-terminal",
        "lxterminal",
        "alacritty",
        "kitty",
        "wezterm",
        "foot",
        "rxvt",
        "urxvt",
    ]
    .iter()
    .any(|token| haystack.contains(token))
}

fn is_x11_temporary_keysym_unicode_target(request: &InputRequest) -> bool {
    if is_x11_terminal_target(request) {
        return false;
    }
    let haystack = x11_target_haystack(request);
    ["firefox", "librewolf", "thunderbird"]
        .iter()
        .any(|token| haystack.contains(token))
}

fn x11_target_haystack(request: &InputRequest) -> String {
    let mut haystack = request.target_name.to_ascii_lowercase();
    if let Some(app_name) = &request.app_name {
        haystack.push(' ');
        haystack.push_str(&app_name.to_ascii_lowercase());
    }
    haystack
}

fn clipboard_selection_name(selection: ClipboardSelection) -> &'static str {
    match selection {
        ClipboardSelection::Clipboard => "clipboard",
        ClipboardSelection::Primary => "primary",
    }
}

fn send_x11_paste_shortcut(
    connection: &X11Connection,
    keyboard_map: &KeyboardMap,
    shortcut: X11PasteShortcut,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    match shortcut {
        X11PasteShortcut::CtrlV => send_x11_modified_key_chord(
            connection,
            keyboard_map,
            &[ModifierKey::Ctrl],
            'v',
            "clipboard paste Ctrl+V key events",
            action_index,
            action,
        ),
        X11PasteShortcut::ShiftInsert => send_x11_modified_keysym_chord(
            connection,
            keyboard_map,
            &[ModifierKey::Shift],
            XK_INSERT,
            "terminal paste Shift+Insert key events",
            action_index,
            action,
        ),
        X11PasteShortcut::CtrlShiftV => send_x11_modified_key_chord(
            connection,
            keyboard_map,
            &[ModifierKey::Ctrl, ModifierKey::Shift],
            'v',
            "terminal paste Ctrl+Shift+V key events",
            action_index,
            action,
        ),
    }
}

fn send_x11_modified_key_chord(
    connection: &X11Connection,
    keyboard_map: &KeyboardMap,
    modifiers: &[ModifierKey],
    key: char,
    event_kind: &str,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    let stroke = keyboard_map.stroke_for_char(key).ok_or_else(|| {
        input_execution_error(
            "paste_key_not_mapped",
            format!(
                "key `{key}` is not available in the active X11 keyboard map for paste fallback"
            ),
            action_index,
            action,
        )
    })?;
    send_x11_modified_keysym_stroke(
        connection,
        keyboard_map,
        modifiers,
        stroke,
        &key.to_string(),
        action_index,
        action,
    )?;
    flush_x11_input(connection, action_index, action, event_kind)
}

fn send_x11_modified_keysym_chord(
    connection: &X11Connection,
    keyboard_map: &KeyboardMap,
    modifiers: &[ModifierKey],
    keysym: u32,
    event_kind: &str,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    let stroke = keyboard_map.stroke_for_keysym(keysym).ok_or_else(|| {
        input_execution_error(
            "paste_key_not_mapped",
            "Insert is not available in the active X11 keyboard map for terminal paste fallback",
            action_index,
            action,
        )
    })?;
    send_x11_modified_keysym_stroke(
        connection,
        keyboard_map,
        modifiers,
        stroke,
        "Insert",
        action_index,
        action,
    )?;
    flush_x11_input(connection, action_index, action, event_kind)
}

fn send_x11_modified_keysym_stroke(
    connection: &X11Connection,
    keyboard_map: &KeyboardMap,
    modifiers: &[ModifierKey],
    stroke: KeyStroke,
    key_label: &str,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    if stroke.shift {
        return Err(input_execution_error(
            "paste_key_not_mapped",
            format!(
                "key `{key_label}` requires shift in the active X11 keyboard map; paste fallback chords need an unshifted physical key"
            ),
            action_index,
            action,
        ));
    }

    let mut pressed_modifiers = Vec::new();
    for modifier in modifiers {
        let modifier_stroke = keyboard_map.modifier_stroke(*modifier).ok_or_else(|| {
            input_execution_error(
                "modifier_not_mapped",
                format!("modifier `{modifier:?}` is not available in the active X11 keyboard map for paste fallback"),
                action_index,
                action,
            )
        })?;
        if let Err(error) = fake_key_event(
            connection,
            KEY_PRESS_EVENT,
            modifier_stroke.keycode,
            action_index,
            action,
        ) {
            let _ = release_pressed_modifiers(connection, &pressed_modifiers, action_index, action);
            return Err(error);
        }
        pressed_modifiers.push(modifier_stroke.keycode);
    }

    let key_result = (|| {
        fake_key_event(
            connection,
            KEY_PRESS_EVENT,
            stroke.keycode,
            action_index,
            action,
        )?;
        fake_key_event(
            connection,
            KEY_RELEASE_EVENT,
            stroke.keycode,
            action_index,
            action,
        )
    })();
    let release_result =
        release_pressed_modifiers(connection, &pressed_modifiers, action_index, action);
    key_result?;
    release_result
}

fn release_pressed_modifiers(
    connection: &X11Connection,
    pressed_modifiers: &[u8],
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    let mut first_error = None;
    for keycode in pressed_modifiers.iter().rev() {
        if let Err(error) = fake_key_event(
            connection,
            KEY_RELEASE_EVENT,
            *keycode,
            action_index,
            action,
        ) {
            first_error.get_or_insert(error);
        }
    }
    if let Some(error) = first_error {
        Err(error)
    } else {
        Ok(())
    }
}

fn send_key_stroke(
    connection: &X11Connection,
    keyboard_map: &KeyboardMap,
    stroke: KeyStroke,
    held_modifiers: &HashSet<ModifierKey>,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    let temporary_shift = stroke.shift && !held_modifiers.contains(&ModifierKey::Shift);
    if temporary_shift {
        let shift = keyboard_map
            .modifier_stroke(ModifierKey::Shift)
            .ok_or_else(|| {
                input_execution_error(
                    "modifier_not_mapped",
                    "shift is not available in the active X11 keyboard map",
                    action_index,
                    action,
                )
            })?;
        fake_key_event(
            connection,
            KEY_PRESS_EVENT,
            shift.keycode,
            action_index,
            action,
        )?;
    }

    fake_key_event(
        connection,
        KEY_PRESS_EVENT,
        stroke.keycode,
        action_index,
        action,
    )?;
    fake_key_event(
        connection,
        KEY_RELEASE_EVENT,
        stroke.keycode,
        action_index,
        action,
    )?;

    if temporary_shift {
        let shift = keyboard_map
            .modifier_stroke(ModifierKey::Shift)
            .ok_or_else(|| {
                input_execution_error(
                    "modifier_not_mapped",
                    "shift is not available in the active X11 keyboard map",
                    action_index,
                    action,
                )
            })?;
        fake_key_event(
            connection,
            KEY_RELEASE_EVENT,
            shift.keycode,
            action_index,
            action,
        )?;
    }
    Ok(())
}

fn activate_window(connection: &X11Connection, window: Window) -> Result<(), String> {
    let event = ClientMessageEvent::new(
        32,
        window,
        connection.atoms._NET_ACTIVE_WINDOW,
        [1, CURRENT_TIME, 0, 0, 0],
    );
    connection
        .conn
        .send_event(
            false,
            connection.screen.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )
        .map_err(|error| format!("failed to send _NET_ACTIVE_WINDOW: {error}"))?;
    connection
        .conn
        .configure_window(
            window,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        )
        .map_err(|error| format!("failed to raise the target window: {error}"))?;
    connection
        .conn
        .set_input_focus(InputFocus::PARENT, window, CURRENT_TIME)
        .map_err(|error| format!("failed to set input focus: {error}"))?;
    connection
        .conn
        .flush()
        .map_err(|error| format!("failed to flush the focus request: {error}"))?;
    Ok(())
}

fn ensure_xtest_available(
    connection: &X11Connection,
    platform: PlatformKind,
) -> Result<(), TendrilError> {
    if connection
        .conn
        .extension_information(xtest::X11_EXTENSION_NAME)
        .map_err(|error| {
            TendrilError::from(PlatformAdapterError::adapter_failure(
                AdapterOperation::InputControl,
                platform,
                format!("failed to query the XTEST extension: {error}"),
            ))
        })?
        .is_none()
    {
        return Err(TendrilError::from(PlatformAdapterError::unsupported(
            Capability::InputControl,
            platform,
            CapabilityErrorReason::UnsupportedFeature,
            "X11 input injection requires the XTEST extension on the active X server.",
            Some("Use an X11 server with XTEST enabled, then rerun tendril run."),
        )));
    }
    Ok(())
}

fn lookup_visual(
    setup: &x11rb::protocol::xproto::Setup,
    visual_id: Visualid,
) -> Option<x11rb::protocol::xproto::Visualtype> {
    setup
        .roots
        .iter()
        .flat_map(|screen| screen.allowed_depths.iter())
        .flat_map(|depth| depth.visuals.iter())
        .find(|visual| visual.visual_id == visual_id)
        .copied()
}

fn adapter_failure(
    context: &AdapterContext,
    operation: AdapterOperation,
    error: impl std::fmt::Display,
) -> PlatformAdapterError {
    PlatformAdapterError::adapter_failure(operation, context.platform, error.to_string())
}

fn input_execution_error(
    code: &'static str,
    message: impl Into<String>,
    action_index: Option<usize>,
    action: Option<&str>,
) -> TendrilError {
    let mut error = TendrilError::execution_failure(code, message, action_index)
        .with_detail_entry("stage", serde_json::json!("dispatch"));
    if let Some(action_index) = action_index {
        error = error.with_detail_entry("action_number", serde_json::json!(action_index + 1));
    }
    if let Some(action) = action {
        error = error.with_detail_entry("action", serde_json::json!(action));
    }
    error
}

fn action_label(action: &InputAction) -> String {
    match action {
        InputAction::KeyTap { key } => format!("key({key})"),
        InputAction::Hold { modifier } => format!("hold({modifier:?})").to_lowercase(),
        InputAction::Release { modifier } => format!("release({modifier:?})").to_lowercase(),
        InputAction::Send { text } => format!("send({text:?})"),
        InputAction::Wait { duration_ms } => format!("wait({duration_ms}ms)"),
        InputAction::Click { button, x, y } => format!("{button:?}_click({x},{y})").to_lowercase(),
        InputAction::PointerMove { x, y } => format!("move({x},{y})"),
        InputAction::DoubleClick { x, y } => format!("dblclick({x},{y})"),
        InputAction::Drag { x0, y0, x1, y1 } => format!("drag({x0},{y0},{x1},{y1})"),
        InputAction::Scroll { x, y, dy } => format!("scroll({x},{y},{dy})"),
        InputAction::ElementClick { id } => format!("click({id})"),
    }
}

fn action_is_keyboard(action: &InputAction) -> bool {
    matches!(
        action,
        InputAction::KeyTap { .. }
            | InputAction::Hold { .. }
            | InputAction::Release { .. }
            | InputAction::Send { .. }
    )
}

fn mouse_button_number(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
    }
}

fn format_window_id(window: Window) -> String {
    format!("0x{window:x}")
}

fn parse_window_id(value: &str) -> Result<Window, String> {
    let trimmed = value.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16)
            .map_err(|error| format!("invalid X11 window id `{value}`: {error}"))
    } else {
        trimmed
            .parse::<u32>()
            .map_err(|error| format!("invalid X11 window id `{value}`: {error}"))
    }
}

fn clamp_i16(value: i32) -> Result<i16, String> {
    i16::try_from(value).map_err(|_| format!("coordinate `{value}` exceeds the X11 i16 range"))
}

fn clamp_u16(value: u32) -> Result<u16, String> {
    u16::try_from(value).map_err(|_| format!("dimension `{value}` exceeds the X11 u16 range"))
}

fn key_name_to_keysym(value: &str) -> Option<u32> {
    match value.to_ascii_lowercase().as_str() {
        "enter" | "return" => Some(XK_RETURN),
        "esc" | "escape" => Some(XK_ESCAPE),
        "tab" => Some(XK_TAB),
        "space" => Some(u32::from(' ')),
        "backspace" => Some(XK_BACK_SPACE),
        "insert" | "ins" => Some(XK_INSERT),
        "delete" | "del" => Some(XK_DELETE),
        "left" => Some(XK_LEFT),
        "right" => Some(XK_RIGHT),
        "up" => Some(XK_UP),
        "down" => Some(XK_DOWN),
        "home" => Some(XK_HOME),
        "end" => Some(XK_END),
        "pageup" => Some(XK_PAGE_UP),
        "pagedown" => Some(XK_PAGE_DOWN),
        other if other.chars().count() == 1 => other.chars().next().map(keysym_for_char),
        _ => None,
    }
}

fn keysym_for_char(character: char) -> u32 {
    match character {
        '\n' => XK_RETURN,
        '\t' => XK_TAB,
        other if u32::from(other) <= 0xff => u32::from(other),
        other => 0x01_00_00_00 | u32::from(other),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyStroke {
    keycode: u8,
    shift: bool,
}

struct KeyboardMap {
    min_keycode: u8,
    max_keycode: u8,
    keysyms_per_keycode: u8,
    keysyms: Vec<u32>,
}

impl KeyboardMap {
    fn load(connection: &X11Connection) -> Result<Self, String> {
        let min_keycode = connection.conn.setup().min_keycode;
        let max_keycode = connection.conn.setup().max_keycode;
        let count = max_keycode.saturating_sub(min_keycode).saturating_add(1);
        let reply = connection
            .conn
            .get_keyboard_mapping(min_keycode, count)
            .map_err(|error| error.to_string())?
            .reply()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            min_keycode,
            max_keycode,
            keysyms_per_keycode: reply.keysyms_per_keycode,
            keysyms: reply.keysyms,
        })
    }

    fn modifier_stroke(&self, modifier: ModifierKey) -> Option<KeyStroke> {
        let keysym = match modifier {
            ModifierKey::Ctrl => XK_CONTROL_L,
            ModifierKey::Alt => XK_ALT_L,
            ModifierKey::Shift => XK_SHIFT_L,
            ModifierKey::Meta => XK_SUPER_L,
        };
        self.stroke_for_keysym(keysym)
    }

    fn stroke_for_char(&self, character: char) -> Option<KeyStroke> {
        self.stroke_for_keysym(keysym_for_char(character))
    }

    fn stroke_for_keysym(&self, keysym: u32) -> Option<KeyStroke> {
        for keycode in self.min_keycode..=self.max_keycode {
            let keysyms = self.keysyms_for_keycode(keycode)?;
            if keysyms.first().copied() == Some(keysym) {
                return Some(KeyStroke {
                    keycode,
                    shift: false,
                });
            }
            if keysyms.get(1).copied() == Some(keysym) {
                return Some(KeyStroke {
                    keycode,
                    shift: true,
                });
            }
            if keysyms.iter().copied().any(|candidate| candidate == keysym) {
                return Some(KeyStroke {
                    keycode,
                    shift: false,
                });
            }
        }
        None
    }

    fn keysyms_for_keycode(&self, keycode: u8) -> Option<Vec<u32>> {
        if keycode < self.min_keycode || keycode > self.max_keycode {
            return None;
        }
        let per_keycode = usize::from(self.keysyms_per_keycode);
        let start = usize::from(keycode.saturating_sub(self.min_keycode)) * per_keycode;
        self.keysyms
            .get(start..start + per_keycode)
            .map(<[u32]>::to_vec)
    }

    fn temporary_unicode_keycode(&self) -> u8 {
        (self.min_keycode..=self.max_keycode)
            .rev()
            .find(|keycode| {
                self.keysyms_for_keycode(*keycode)
                    .is_some_and(|keysyms| keysyms.iter().all(|keysym| *keysym == 0))
            })
            .unwrap_or(self.max_keycode)
    }
}

struct X11Connection {
    conn: x11rb::rust_connection::RustConnection,
    screen_num: usize,
    screen: x11rb::protocol::xproto::Screen,
    atoms: Atoms,
}

impl X11Connection {
    fn connect(
        context: &AdapterContext,
        operation: AdapterOperation,
    ) -> Result<Self, PlatformAdapterError> {
        let (conn, screen_num) =
            x11rb::connect(context.x11_display.as_deref()).map_err(|error| {
                PlatformAdapterError::adapter_failure(
                    operation,
                    context.platform,
                    format!("failed to connect to the active X11 display: {error}"),
                )
            })?;
        let screen = conn.setup().roots.get(screen_num).cloned().ok_or_else(|| {
            PlatformAdapterError::adapter_failure(
                operation,
                context.platform,
                format!("X11 screen index `{screen_num}` was not present in setup"),
            )
        })?;
        let atoms = Atoms::new(&conn)
            .map_err(|error| adapter_failure(context, operation, error))?
            .reply()
            .map_err(|error| adapter_failure(context, operation, error))?;
        Ok(Self {
            conn,
            screen_num,
            screen,
            atoms,
        })
    }

    fn atom_name(&self, atom: u32) -> Option<String> {
        if atom == 0 {
            return None;
        }
        self.conn
            .get_atom_name(atom)
            .ok()?
            .reply()
            .ok()
            .and_then(|reply| String::from_utf8(reply.name).ok())
            .filter(|name| !name.is_empty())
    }

    fn window_list(&self, context: &AdapterContext) -> Result<Vec<Window>, PlatformAdapterError> {
        let stacking = self
            .conn
            .get_property(
                false,
                self.screen.root,
                self.atoms._NET_CLIENT_LIST_STACKING,
                AtomEnum::WINDOW,
                0,
                u32::MAX,
            )
            .map_err(|error| adapter_failure(context, AdapterOperation::TargetDiscovery, error))?
            .reply()
            .map(|reply| windows_from_property(&reply))
            .map_err(|error| adapter_failure(context, AdapterOperation::TargetDiscovery, error))?;
        if !stacking.is_empty() {
            return Ok(stacking);
        }

        self.conn
            .get_property(
                false,
                self.screen.root,
                self.atoms._NET_CLIENT_LIST,
                AtomEnum::WINDOW,
                0,
                u32::MAX,
            )
            .map_err(|error| adapter_failure(context, AdapterOperation::TargetDiscovery, error))?
            .reply()
            .map(|reply| windows_from_property(&reply))
            .map_err(|error| adapter_failure(context, AdapterOperation::TargetDiscovery, error))
    }

    fn text_property(&self, window: Window, property: u32) -> Option<String> {
        self.conn
            .get_property(false, window, property, AtomEnum::ANY, 0, 4096)
            .ok()?
            .reply()
            .ok()
            .and_then(|reply| decode_text_property(&reply.value))
    }

    fn class_name(&self, window: Window) -> Option<String> {
        self.conn
            .get_property(
                false,
                window,
                self.atoms.WM_CLASS,
                AtomEnum::STRING,
                0,
                4096,
            )
            .ok()?
            .reply()
            .ok()
            .and_then(|reply| decode_class_name(&reply.value))
    }

    fn cardinal_property(&self, window: Window, property: u32) -> Option<u32> {
        self.conn
            .get_property(false, window, property, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?
            .value32()?
            .next()
    }
}

fn windows_from_property(reply: &x11rb::protocol::xproto::GetPropertyReply) -> Vec<Window> {
    reply.value32().map(Iterator::collect).unwrap_or_default()
}

fn decode_text_property(bytes: &[u8]) -> Option<String> {
    let value = String::from_utf8_lossy(bytes)
        .trim_matches(char::from(0))
        .trim()
        .to_owned();
    (!value.is_empty()).then_some(value)
}

fn decode_class_name(bytes: &[u8]) -> Option<String> {
    let values = bytes
        .split(|byte| *byte == 0)
        .filter_map(|value| {
            let value = String::from_utf8_lossy(value).trim().to_owned();
            (!value.is_empty()).then_some(value)
        })
        .collect::<Vec<_>>();
    values.last().cloned()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{DynamicImage, ImageBuffer, ImageFormat as RasterImageFormat, Rgba};

    use crate::model::Bounds;
    use crate::platform::{CaptureTargetKind, InputRequest};

    use super::{
        DRAG_MIN_STEPS, KeyStroke, KeyboardMap, X11OccludingWindow, X11PasteShortcut,
        X11UnicodePasteAttempt, X11WindowBounds, X11WindowCaptureFallbackReason, XK_INSERT,
        capture_png_is_solid_black, decode_class_name, decode_text_property, drag_motion_points,
        first_unmapped_text_character, is_x11_temporary_keysym_unicode_target,
        is_x11_terminal_target, key_name_to_keysym, keysym_for_char, parse_window_id,
        windows_overlap, x11_unicode_paste_attempts,
    };

    #[test]
    fn parses_hex_window_ids() {
        assert_eq!(parse_window_id("0x4a00007"), Ok(0x04a0_0007));
        assert_eq!(parse_window_id("42"), Ok(42));
    }

    #[test]
    fn decodes_x11_text_properties() {
        assert_eq!(
            decode_text_property(b"Inbox - Mozilla Firefox\0"),
            Some("Inbox - Mozilla Firefox".to_owned())
        );
    }

    #[test]
    fn decodes_wm_class_using_the_application_name() {
        assert_eq!(
            decode_class_name(b"Navigator\0firefox\0"),
            Some("firefox".to_owned())
        );
    }

    #[test]
    fn uses_unicode_keysyms_for_non_latin_text() {
        assert_eq!(keysym_for_char('A'), u32::from('A'));
        assert_eq!(keysym_for_char('é'), u32::from('é'));
        assert_eq!(keysym_for_char('Ж'), 0x0100_0416);
    }

    #[test]
    fn detects_first_unmapped_character_for_x11_clipboard_fallback() {
        let keyboard_map = KeyboardMap {
            min_keycode: 10,
            max_keycode: 15,
            keysyms_per_keycode: 2,
            keysyms: vec![
                u32::from('C'),
                0,
                u32::from('a'),
                0,
                u32::from('f'),
                0,
                u32::from('e'),
                0,
                u32::from(' '),
                0,
                u32::from('x'),
                0,
            ],
        };

        assert_eq!(first_unmapped_text_character(&keyboard_map, "Cafe"), None);
        assert_eq!(
            first_unmapped_text_character(&keyboard_map, "Café π"),
            Some('é')
        );
    }

    #[test]
    fn terminal_targets_use_terminal_compatible_unicode_paste_attempts_first() {
        let request = sample_input_request("Tendril Headless Shell", Some("XTerm"));

        assert!(is_x11_terminal_target(&request));
        assert_eq!(
            x11_unicode_paste_attempts(&request),
            vec![
                X11UnicodePasteAttempt {
                    selection: crate::clipboard::ClipboardSelection::Primary,
                    shortcut: X11PasteShortcut::ShiftInsert,
                },
                X11UnicodePasteAttempt {
                    selection: crate::clipboard::ClipboardSelection::Clipboard,
                    shortcut: X11PasteShortcut::ShiftInsert,
                },
                X11UnicodePasteAttempt {
                    selection: crate::clipboard::ClipboardSelection::Clipboard,
                    shortcut: X11PasteShortcut::CtrlShiftV,
                },
                X11UnicodePasteAttempt {
                    selection: crate::clipboard::ClipboardSelection::Clipboard,
                    shortcut: X11PasteShortcut::CtrlV,
                },
            ]
        );
    }

    #[test]
    fn firefox_targets_use_temporary_keysyms_before_clipboard_paste() {
        let request = sample_input_request("Tendril RichEdit — Mozilla Firefox", Some("firefox"));

        assert!(!is_x11_terminal_target(&request));
        assert!(is_x11_temporary_keysym_unicode_target(&request));
    }

    #[test]
    fn terminal_targets_do_not_use_temporary_keysyms_even_with_browser_title() {
        let request = sample_input_request("xterm running firefox docs", Some("XTerm"));

        assert!(is_x11_terminal_target(&request));
        assert!(!is_x11_temporary_keysym_unicode_target(&request));
    }

    #[test]
    fn generic_non_terminal_targets_keep_browser_clipboard_paste_attempt() {
        let request = sample_input_request("Example Editor", Some("generic-app"));

        assert!(!is_x11_terminal_target(&request));
        assert!(!is_x11_temporary_keysym_unicode_target(&request));
        assert_eq!(
            x11_unicode_paste_attempts(&request),
            vec![X11UnicodePasteAttempt {
                selection: crate::clipboard::ClipboardSelection::Clipboard,
                shortcut: X11PasteShortcut::CtrlV,
            }]
        );
    }

    #[test]
    fn accepts_insert_key_names_for_x11_input() {
        assert_eq!(key_name_to_keysym("insert"), Some(XK_INSERT));
        assert_eq!(key_name_to_keysym("Insert"), Some(XK_INSERT));
        assert_eq!(key_name_to_keysym("ins"), Some(XK_INSERT));
    }

    #[test]
    fn maps_insert_keysym_from_x11_keyboard_layout() {
        let keyboard_map = KeyboardMap {
            min_keycode: 10,
            max_keycode: 12,
            keysyms_per_keycode: 2,
            keysyms: vec![0, 0, XK_INSERT, 0, 0, 0],
        };

        assert_eq!(
            keyboard_map.stroke_for_keysym(XK_INSERT),
            Some(KeyStroke {
                keycode: 11,
                shift: false,
            })
        );
    }

    #[test]
    fn produces_incremental_drag_motion_points() {
        let points = drag_motion_points(10, 20, 130, 80);

        assert!(points.len() >= usize::try_from(DRAG_MIN_STEPS).unwrap());
        assert_eq!(points.last(), Some(&(130, 80)));
        assert!(points.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        assert!(points.windows(2).all(|pair| pair[0].1 <= pair[1].1));

        let long_text_drag = drag_motion_points(95, 350, 850, 350);
        assert!(long_text_drag.len() > 32);
        assert_eq!(long_text_drag.first(), Some(&(110, 350)));
        assert_eq!(long_text_drag.last(), Some(&(850, 350)));
        assert!(
            long_text_drag
                .windows(2)
                .all(|pair| pair[1].0 - pair[0].0 <= 16)
        );
    }

    #[test]
    fn detects_overlapping_x11_window_bounds() {
        let browser = X11WindowBounds {
            x: 40,
            y: 50,
            width: 1200,
            height: 800,
        };
        let xterm = X11WindowBounds {
            x: 100,
            y: 120,
            width: 600,
            height: 320,
        };
        let adjacent = X11WindowBounds {
            x: 1240,
            y: 120,
            width: 300,
            height: 320,
        };

        assert!(windows_overlap(&browser, &xterm));
        assert!(!windows_overlap(&browser, &adjacent));
    }

    #[test]
    fn x11_occlusion_fallback_reason_names_overlapping_windows() {
        let reason = X11WindowCaptureFallbackReason::OverlappingWindows(vec![X11OccludingWindow {
            id: "0x4200010".to_owned(),
            name: "xterm".to_owned(),
            bounds: X11WindowBounds {
                x: 40,
                y: 58,
                width: 605,
                height: 343,
            },
        }]);

        let summary = reason.summary();
        assert!(summary.contains("overlapping X11 window"));
        assert!(summary.contains("0x4200010"));
        assert!(summary.contains("xterm"));
        assert!(summary.contains("605x343"));
    }

    #[test]
    fn detects_solid_black_capture_pngs() {
        assert!(
            capture_png_is_solid_black(&sample_png_from_pixel(Rgba([0, 0, 0, 255])))
                .expect("black png should decode")
        );
        assert!(
            capture_png_is_solid_black(&sample_png_from_pixel(Rgba([2, 1, 0, 255])))
                .expect("near-black png should decode")
        );
    }

    #[test]
    fn rejects_non_black_capture_pngs() {
        assert!(
            !capture_png_is_solid_black(&sample_png_from_pixel(Rgba([0, 0, 8, 255])))
                .expect("blue-tinted png should decode")
        );
        assert!(!capture_png_is_solid_black(&sample_mixed_png()).expect("mixed png should decode"));
    }

    fn sample_input_request(target_name: &str, app_name: Option<&str>) -> InputRequest {
        InputRequest {
            target_id: "0x40000c".to_owned(),
            target: CaptureTargetKind::Window,
            target_name: target_name.to_owned(),
            bounds: Bounds {
                x: 0,
                y: 0,
                width: 80,
                height: 24,
            },
            app_name: app_name.map(str::to_owned),
            process_id: Some(1234),
            restore_focus: true,
            text: None,
            actions: Vec::new(),
        }
    }

    fn sample_png_from_pixel(pixel: Rgba<u8>) -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(3, 2, pixel));
        encode_sample_png(&image)
    }

    fn sample_mixed_png() -> Vec<u8> {
        let mut image = ImageBuffer::from_pixel(3, 2, Rgba([0, 0, 0, 255]));
        image.put_pixel(1, 1, Rgba([255, 255, 255, 255]));
        encode_sample_png(&DynamicImage::ImageRgba8(image))
    }

    fn encode_sample_png(image: &DynamicImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), RasterImageFormat::Png)
            .expect("sample png should encode");
        bytes
    }
}
