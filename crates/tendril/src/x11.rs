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

use crate::error::TendrilError;
use crate::input::reliability_delay;
use crate::model::{Bounds, InputAction, ModifierKey, MouseButton, ScaleFactor};
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
    request: &InputRequest,
) -> Result<InputOutcome, TendrilError> {
    let context = AdapterContext::linux(crate::platform::DesktopSession::X11, None);
    let connection = X11Connection::connect(&context, AdapterOperation::InputControl)
        .map_err(TendrilError::from)?;

    ensure_xtest_available(&connection, platform)?;

    let keyboard_input = request.text.is_some() || request.actions.iter().any(action_is_keyboard);
    let mut focus_required = false;
    let mut focus_transferred = false;
    let mut notes = Vec::new();

    if keyboard_input {
        focus_required = true;
        if matches!(request.target, CaptureTargetKind::Window) {
            let window = parse_window_id(&request.target_id).map_err(|message| {
                input_execution_error("invalid_target", message, None, Some("focus"))
            })?;
            activate_window(&connection, window).map_err(|error| {
                input_execution_error(
                    "focus_failed",
                    format!("failed to focus target window: {error}"),
                    None,
                    Some("focus"),
                )
            })?;
            focus_transferred = true;
            notes.push(
                "Activated the target window before keyboard delivery for X11 reliability."
                    .to_owned(),
            );
            std::thread::sleep(reliability_delay());
        } else {
            notes.push(
                "Display-scoped keyboard input uses the currently focused control; place focus explicitly if a different app should receive text or key taps."
                    .to_owned(),
            );
        }
    }

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
        type_text(
            &connection,
            &keyboard_map,
            text,
            &held_modifiers,
            Some(0),
            Some("text"),
        )?;
        connection.conn.flush().map_err(|error| {
            input_execution_error(
                "dispatch_failed",
                format!("failed to flush X11 text events: {error}"),
                Some(0),
                Some("text"),
            )
        })?;
        return Ok(InputOutcome {
            action_count: 1,
            focus_required,
            focus_transferred,
            focused_target: focus_transferred.then(|| request.target_id.clone()),
            notes,
        });
    }

    for (action_index, action) in request.actions.iter().enumerate() {
        let label = action_label(action);
        dispatch_action(
            &connection,
            &keyboard_map,
            request,
            action,
            action_index,
            &label,
            &mut held_modifiers,
        )?;
        if !matches!(action, InputAction::Wait { .. }) {
            connection.conn.flush().map_err(|error| {
                input_execution_error(
                    "dispatch_failed",
                    format!("failed to flush X11 input events: {error}"),
                    Some(action_index),
                    Some(&label),
                )
            })?;
            std::thread::sleep(reliability_delay());
        }
    }

    Ok(InputOutcome {
        action_count: request.actions.len(),
        focus_required,
        focus_transferred,
        focused_target: focus_transferred.then(|| request.target_id.clone()),
        notes,
    })
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
    capture_region(
        context,
        connection,
        window,
        0,
        0,
        geometry.width,
        geometry.height,
        Some(connection.screen.root_visual),
    )
}

fn capture_display(
    context: &AdapterContext,
    connection: &X11Connection,
    target_id: &str,
) -> Result<Vec<u8>, PlatformAdapterError> {
    let display = discover_displays(context, connection)?
        .into_iter()
        .find(|target| target.id == target_id)
        .ok_or_else(|| {
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

#[allow(clippy::too_many_lines)]
fn dispatch_action(
    connection: &X11Connection,
    keyboard_map: &KeyboardMap,
    request: &InputRequest,
    action: &InputAction,
    action_index: usize,
    label: &str,
    held_modifiers: &mut HashSet<ModifierKey>,
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
        InputAction::Send { text } => type_text(
            connection,
            keyboard_map,
            text,
            held_modifiers,
            Some(action_index),
            Some(label),
        ),
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
    }
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
    let button = mouse_button_number(button);
    fake_button_event(connection, BUTTON_PRESS_EVENT, button, action_index, action)?;
    fake_button_event(
        connection,
        BUTTON_RELEASE_EVENT,
        button,
        action_index,
        action,
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
    fake_button_event(connection, BUTTON_PRESS_EVENT, 1, action_index, action)?;
    std::thread::sleep(reliability_delay());
    move_pointer(connection, end_x, end_y, action_index, action)?;
    fake_button_event(connection, BUTTON_RELEASE_EVENT, 1, action_index, action)
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
    text: &str,
    held_modifiers: &HashSet<ModifierKey>,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
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
    Ok(())
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
        InputAction::Drag { x0, y0, x1, y1 } => format!("drag({x0},{y0},{x1},{y1})"),
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

#[derive(Debug, Clone, Copy)]
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
        let per_keycode = usize::from(self.keysyms_per_keycode);
        for keycode in self.min_keycode..=self.max_keycode {
            let start = usize::from(keycode.saturating_sub(self.min_keycode)) * per_keycode;
            let keysyms = self.keysyms.get(start..start + per_keycode)?;
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
        let (conn, screen_num) = x11rb::connect(None).map_err(|error| {
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
    use super::{decode_class_name, decode_text_property, keysym_for_char, parse_window_id};

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
}
