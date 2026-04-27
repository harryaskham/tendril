//! Wayland input injection backend.
//!
//! This module powers `tendril run` on Wayland sessions (Hyprland, sway, and
//! other wlroots-based compositors) by delegating actual event delivery to two
//! widely available helper tools:
//!
//! * `ydotool` — kernel-level input injection via `/dev/uinput` driven by a
//!   `ydotoold` daemon. Preferred because it works across every Wayland
//!   compositor and supports both keyboard and pointer events.
//! * `wtype` — keyboard input injection through the wlroots-specific
//!   `virtual-keyboard-v1` Wayland protocol. Used as a keyboard-only fallback
//!   when `ydotool` is not present.
//!
//! Tools are detected at runtime; the public [`detect_backend`] helper returns
//! a [`WaylandInputCapability`] that other modules use to decide whether
//! Wayland targets should advertise `input_supported = true`.

use std::collections::HashSet;
use std::env;
use std::process::Command;
use std::time::Duration;

use serde_json::json;

use crate::error::TendrilError;
use crate::input::{relative_point_to_absolute, reliability_delay};
use crate::model::{InputAction, ModifierKey, MouseButton};
use crate::platform::{
    Capability, CapabilityErrorReason, InputOutcome, InputRequest, PlatformAdapterError,
    PlatformKind,
};

/// Names of the helper binaries that this module knows how to drive.
const YDOTOOL_BIN: &str = "ydotool";
const WTYPE_BIN: &str = "wtype";

/// Linux input event "press" / "release" values. Mirrors `linux/input-event-codes.h`.
const KEY_DOWN: u32 = 1;
const KEY_UP: u32 = 0;

/// Picks which Wayland helper tool will service a given request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WaylandInputBackend {
    /// `ydotool` + `ydotoold`; covers keyboard and pointer events.
    Ydotool,
    /// `wtype`; covers keyboard events only (no pointer support).
    Wtype,
}

/// Snapshot of the Wayland input helpers detected on the current PATH.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WaylandInputCapability {
    pub ydotool: bool,
    pub wtype: bool,
}

impl WaylandInputCapability {
    /// Returns true when at least one helper that can deliver Wayland input is
    /// available. Even a `wtype`-only host can service keyboard sequences, so
    /// we still advertise input support and surface a clear runtime error if a
    /// caller asks for mouse events that the chosen backend cannot handle.
    pub(crate) fn any_supported(&self) -> bool {
        self.ydotool || self.wtype
    }

    /// Backend that should be preferred when both helpers are present.
    pub(crate) fn preferred(&self) -> Option<WaylandInputBackend> {
        if self.ydotool {
            Some(WaylandInputBackend::Ydotool)
        } else if self.wtype {
            Some(WaylandInputBackend::Wtype)
        } else {
            None
        }
    }
}

/// Detects which Wayland input helper tools are reachable on `PATH`.
pub(crate) fn detect_backend() -> WaylandInputCapability {
    WaylandInputCapability {
        ydotool: program_on_path(YDOTOOL_BIN),
        wtype: program_on_path(WTYPE_BIN),
    }
}

/// Returns the diagnostic surfaced by `tendril run` when no Wayland helper is
/// available. Centralised here so the [`crate::platform::LinuxAdapter`] and
/// the runtime executor share identical guidance.
pub(crate) fn missing_backend_error(platform: PlatformKind) -> PlatformAdapterError {
    PlatformAdapterError::unsupported(
        Capability::InputControl,
        platform,
        CapabilityErrorReason::UnsupportedFeature,
        "Wayland input injection requires either `ydotool` (with the `ydotoold` daemon) or `wtype` to be installed and reachable on PATH.",
        Some(
            "Install `ydotool` for full keyboard + pointer support (preferred), or install `wtype` for keyboard-only input on wlroots-based compositors such as Hyprland and sway. Make sure the `ydotoold` daemon is running and that the invoking user can reach its socket (typically `/tmp/.ydotool_socket` or `$YDOTOOL_SOCKET`).",
        ),
    )
}

/// Executes the requested input sequence using a Wayland helper tool.
pub(crate) fn execute_input(
    platform: PlatformKind,
    request: &InputRequest,
) -> Result<InputOutcome, TendrilError> {
    let capability = detect_backend();
    let Some(backend) = capability.preferred() else {
        return Err(TendrilError::from(missing_backend_error(platform)));
    };

    if backend == WaylandInputBackend::Wtype && request_has_pointer(request) {
        // wtype cannot synthesize pointer events. Surface an actionable
        // diagnostic instead of silently dropping the click/drag.
        return Err(TendrilError::from(PlatformAdapterError::unsupported(
            Capability::InputControl,
            platform,
            CapabilityErrorReason::UnsupportedFeature,
            "The detected Wayland input backend `wtype` only supports keyboard events; the requested sequence contains pointer events.",
            Some(
                "Install `ydotool` (and run the `ydotoold` daemon) to enable pointer events on Wayland sessions. wtype intentionally targets the wlroots virtual-keyboard protocol and does not implement pointer injection.",
            ),
        )));
    }

    let mut notes = Vec::new();
    notes.push(match backend {
        WaylandInputBackend::Ydotool => {
            "Wayland input dispatched through ydotool (uinput); requires the ydotoold daemon and PATH access to the helper binary."
                .to_owned()
        }
        WaylandInputBackend::Wtype => {
            "Wayland input dispatched through wtype using the wlroots virtual-keyboard-v1 protocol; pointer events are not supported by this backend."
                .to_owned()
        }
    });

    let keyboard_input = request.text.is_some() || request.actions.iter().any(action_is_keyboard);
    let focus_required = keyboard_input;

    // Wayland compositors do not expose a portable "focus this client"
    // primitive that works for arbitrary surfaces, so we record the focus
    // contract honestly: callers must place focus themselves (or rely on
    // compositor-side keyboard routing) before issuing the run.
    if keyboard_input {
        notes.push(
            "Wayland focus transfer is compositor-mediated; place focus on the intended surface before running keyboard sequences for reliable delivery."
                .to_owned(),
        );
    }

    if let Some(text) = &request.text {
        dispatch_text(backend, text, Some(0), Some("text"))?;
        if request.restore_focus {
            notes.push(
                "Wayland focus restoration is not portable through Tendril; no previous focus snapshot was captured."
                    .to_owned(),
            );
        }
        return Ok(InputOutcome {
            action_count: 1,
            focus_required,
            focus_transferred: false,
            focused_target: None,
            previous_focus: None,
            focus_restored: false,
            pointer_restored: false,
            restore_error: request.restore_focus.then(|| {
                "focus restoration is not supported for generic Wayland sessions".to_owned()
            }),
            notes,
        });
    }

    let mut held_modifiers: HashSet<ModifierKey> = HashSet::new();
    for (action_index, action) in request.actions.iter().enumerate() {
        let label = action_label(action);
        dispatch_action(
            backend,
            request,
            action,
            action_index,
            &label,
            &mut held_modifiers,
        )?;
        if !matches!(action, InputAction::Wait { .. }) {
            std::thread::sleep(reliability_delay());
        }
    }

    if request.restore_focus {
        notes.push(
            "Wayland focus restoration is not portable through Tendril; no previous focus snapshot was captured."
                .to_owned(),
        );
    }

    Ok(InputOutcome {
        action_count: request.actions.len(),
        focus_required,
        focus_transferred: false,
        focused_target: None,
        previous_focus: None,
        focus_restored: false,
        pointer_restored: false,
        restore_error: request
            .restore_focus
            .then(|| "focus restoration is not supported for generic Wayland sessions".to_owned()),
        notes,
    })
}

fn dispatch_action(
    backend: WaylandInputBackend,
    request: &InputRequest,
    action: &InputAction,
    action_index: usize,
    label: &str,
    held_modifiers: &mut HashSet<ModifierKey>,
) -> Result<(), TendrilError> {
    match action {
        InputAction::KeyTap { key } => dispatch_key_tap(
            backend,
            key,
            held_modifiers,
            Some(action_index),
            Some(label),
        ),
        InputAction::Hold { modifier } => {
            dispatch_modifier(backend, *modifier, true, Some(action_index), Some(label))?;
            held_modifiers.insert(*modifier);
            Ok(())
        }
        InputAction::Release { modifier } => {
            dispatch_modifier(backend, *modifier, false, Some(action_index), Some(label))?;
            held_modifiers.remove(modifier);
            Ok(())
        }
        InputAction::Send { text } => dispatch_text(backend, text, Some(action_index), Some(label)),
        InputAction::Wait { duration_ms } => {
            std::thread::sleep(Duration::from_millis(*duration_ms));
            Ok(())
        }
        InputAction::Click { button, x, y } => {
            let (absolute_x, absolute_y) = relative_point_to_absolute(&request.bounds, *x, *y);
            dispatch_click(
                backend,
                *button,
                absolute_x,
                absolute_y,
                Some(action_index),
                Some(label),
            )
        }
        InputAction::Drag { x0, y0, x1, y1 } => {
            let (start_x, start_y) = relative_point_to_absolute(&request.bounds, *x0, *y0);
            let (end_x, end_y) = relative_point_to_absolute(&request.bounds, *x1, *y1);
            dispatch_drag(
                backend,
                start_x,
                start_y,
                end_x,
                end_y,
                Some(action_index),
                Some(label),
            )
        }
        InputAction::Scroll { .. } => Err(input_execution_error(
            "unsupported_scroll_action",
            "scroll(...) is currently implemented for Linux/X11 input delivery; this Wayland adapter does not yet support native wheel injection".to_owned(),
            "dispatch",
            Some(action_index),
            Some(label),
        )),
    }
}

fn dispatch_text(
    backend: WaylandInputBackend,
    text: &str,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    if text.is_empty() {
        return Ok(());
    }

    match backend {
        WaylandInputBackend::Ydotool => run_helper(
            YDOTOOL_BIN,
            &["type", "--", text],
            "dispatch",
            action_index,
            action,
        ),
        WaylandInputBackend::Wtype => {
            // `wtype --` followed by the literal text avoids any further flag parsing.
            run_helper(WTYPE_BIN, &["--", text], "dispatch", action_index, action)
        }
    }
}

fn dispatch_key_tap(
    backend: WaylandInputBackend,
    key: &str,
    held_modifiers: &HashSet<ModifierKey>,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    match backend {
        WaylandInputBackend::Ydotool => {
            let keycode = ydotool_key_code(key).ok_or_else(|| {
                input_execution_error(
                    "unsupported_key",
                    format!("key `{key}` is not supported by the ydotool Wayland backend"),
                    "dispatch",
                    action_index,
                    action,
                )
            })?;
            let press = format!("{keycode}:{KEY_DOWN}");
            let release = format!("{keycode}:{KEY_UP}");
            run_helper(
                YDOTOOL_BIN,
                &["key", "--", press.as_str(), release.as_str()],
                "dispatch",
                action_index,
                action,
            )
        }
        WaylandInputBackend::Wtype => {
            let mut args: Vec<String> = Vec::new();
            // Translate any persistently held modifiers into wtype's per-invocation
            // modifier flags (`-M ctrl ... -m ctrl`).
            for modifier in held_modifiers {
                args.push("-M".to_owned());
                args.push(wtype_modifier_name(*modifier).to_owned());
            }
            let key_argument = wtype_key_argument(key).ok_or_else(|| {
                input_execution_error(
                    "unsupported_key",
                    format!("key `{key}` is not supported by the wtype Wayland backend"),
                    "dispatch",
                    action_index,
                    action,
                )
            })?;
            args.push("-k".to_owned());
            args.push(key_argument);
            for modifier in held_modifiers {
                args.push("-m".to_owned());
                args.push(wtype_modifier_name(*modifier).to_owned());
            }
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            run_helper(WTYPE_BIN, &arg_refs, "dispatch", action_index, action)
        }
    }
}

fn dispatch_modifier(
    backend: WaylandInputBackend,
    modifier: ModifierKey,
    press: bool,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    match backend {
        WaylandInputBackend::Ydotool => {
            let keycode = ydotool_modifier_code(modifier);
            let value = if press { KEY_DOWN } else { KEY_UP };
            let argument = format!("{keycode}:{value}");
            run_helper(
                YDOTOOL_BIN,
                &["key", "--", argument.as_str()],
                "dispatch",
                action_index,
                action,
            )
        }
        WaylandInputBackend::Wtype => {
            let modifier_name = wtype_modifier_name(modifier);
            let direction = if press { "-M" } else { "-m" };
            run_helper(
                WTYPE_BIN,
                &[direction, modifier_name],
                "dispatch",
                action_index,
                action,
            )
        }
    }
}

fn dispatch_click(
    backend: WaylandInputBackend,
    button: MouseButton,
    x: i32,
    y: i32,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    debug_assert!(matches!(backend, WaylandInputBackend::Ydotool));

    move_pointer_absolute(x, y, action_index, action)?;
    // ydotool's `click` argument is a bitmask: low nibble = press mask,
    // high nibble = release mask. `0x40 | 0x4 = 0x44` ⇒ press+release of the
    // requested button (0x1=left, 0x2=right, 0x4=middle in the upstream tool).
    let mask = ydotool_button_combined_mask(button);
    let mask_argument = format!("0x{mask:X}");
    run_helper(
        YDOTOOL_BIN,
        &["click", "--", mask_argument.as_str()],
        "dispatch",
        action_index,
        action,
    )
}

fn dispatch_drag(
    backend: WaylandInputBackend,
    start_x: i32,
    start_y: i32,
    end_x: i32,
    end_y: i32,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    debug_assert!(matches!(backend, WaylandInputBackend::Ydotool));

    let press_mask = format!("0x{:X}", ydotool_button_press_mask(MouseButton::Left));
    let release_mask = format!("0x{:X}", ydotool_button_release_mask(MouseButton::Left));

    move_pointer_absolute(start_x, start_y, action_index, action)?;
    run_helper(
        YDOTOOL_BIN,
        &["click", "--", press_mask.as_str()],
        "dispatch",
        action_index,
        action,
    )?;
    std::thread::sleep(reliability_delay());
    move_pointer_absolute(end_x, end_y, action_index, action)?;
    run_helper(
        YDOTOOL_BIN,
        &["click", "--", release_mask.as_str()],
        "dispatch",
        action_index,
        action,
    )
}

fn move_pointer_absolute(
    x: i32,
    y: i32,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    // ydotool 1.x interprets `mousemove --absolute -x X -y Y` as global
    // workspace coordinates, which is what the InputRequest already supplies
    // after `relative_point_to_absolute` translation.
    let x_str = x.to_string();
    let y_str = y.to_string();
    run_helper(
        YDOTOOL_BIN,
        &[
            "mousemove",
            "--absolute",
            "-x",
            x_str.as_str(),
            "-y",
            y_str.as_str(),
        ],
        "dispatch",
        action_index,
        action,
    )
}

fn run_helper(
    program: &str,
    args: &[&str],
    stage: &'static str,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    let output = Command::new(program).args(args).output().map_err(|error| {
        input_execution_error(
            "input_spawn_failed",
            format!("failed to spawn `{program}`: {error}"),
            stage,
            action_index,
            action,
        )
    })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let combined = if stderr.is_empty() && stdout.is_empty() {
        format!("`{program}` exited with status {}", output.status)
    } else if stderr.is_empty() {
        format!("`{program}` failed: {stdout}")
    } else if stdout.is_empty() {
        format!("`{program}` failed: {stderr}")
    } else {
        format!("`{program}` failed: {stdout} | {stderr}")
    };

    if program == YDOTOOL_BIN && looks_like_ydotool_socket_error(&stderr, &stdout) {
        return Err(input_execution_error(
            "ydotoold_unavailable",
            format!(
                "{combined} — ydotool requires the `ydotoold` daemon to be running and reachable via its socket (set `YDOTOOL_SOCKET` or run the daemon under the invoking user)."
            ),
            stage,
            action_index,
            action,
        ));
    }

    Err(input_execution_error(
        "input_command_failed",
        combined,
        stage,
        action_index,
        action,
    ))
}

fn looks_like_ydotool_socket_error(stderr: &str, stdout: &str) -> bool {
    let haystack = format!("{stderr} {stdout}").to_ascii_lowercase();
    haystack.contains("socket")
        || haystack.contains("connection refused")
        || haystack.contains("no such file")
        || haystack.contains("ydotoold")
        || haystack.contains("permission denied")
}

fn request_has_pointer(request: &InputRequest) -> bool {
    request.actions.iter().any(|action| {
        matches!(
            action,
            InputAction::Click { .. } | InputAction::Drag { .. } | InputAction::Scroll { .. }
        )
    })
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

fn action_label(action: &InputAction) -> String {
    serde_json::to_string(action).unwrap_or_else(|_| format!("{action:?}"))
}

fn input_execution_error(
    code: &'static str,
    message: String,
    stage: &'static str,
    action_index: Option<usize>,
    action: Option<&str>,
) -> TendrilError {
    let mut error = TendrilError::execution_failure(code, message, action_index)
        .with_detail_entry("stage", json!(stage));
    if let Some(action_index) = action_index {
        error = error.with_detail_entry("action_number", json!(action_index + 1));
    }
    if let Some(action) = action {
        error = error.with_detail_entry("action", json!(action));
    }
    error
}

fn program_on_path(program: &str) -> bool {
    env::var_os("PATH").is_some_and(|path| {
        env::split_paths(&path).any(|entry| {
            let candidate = entry.join(program);
            candidate.is_file()
        })
    })
}

/// Maps a Tendril modifier into the upstream Linux input event keycode used by
/// `ydotool key`. The "left" variant is chosen because it is the most widely
/// honored modifier across compositors when synthesising shortcuts.
fn ydotool_modifier_code(modifier: ModifierKey) -> u32 {
    match modifier {
        ModifierKey::Ctrl => 29,  // KEY_LEFTCTRL
        ModifierKey::Shift => 42, // KEY_LEFTSHIFT
        ModifierKey::Alt => 56,   // KEY_LEFTALT
        ModifierKey::Meta => 125, // KEY_LEFTMETA
    }
}

/// Maps a Tendril modifier into the wtype modifier name (`ctrl`, `shift`,
/// `alt`, `logo`).
fn wtype_modifier_name(modifier: ModifierKey) -> &'static str {
    match modifier {
        ModifierKey::Ctrl => "ctrl",
        ModifierKey::Shift => "shift",
        ModifierKey::Alt => "alt",
        ModifierKey::Meta => "logo",
    }
}

/// Translates a key token from the Tendril DSL into the literal argument that
/// `wtype -k` accepts (an XKB keysym name). Returns `None` for unsupported
/// tokens so callers can surface an actionable error.
fn wtype_key_argument(key: &str) -> Option<String> {
    let lower = key.to_ascii_lowercase();
    match lower.as_str() {
        "enter" | "return" => Some("Return".to_owned()),
        "tab" => Some("Tab".to_owned()),
        "esc" | "escape" => Some("Escape".to_owned()),
        "space" => Some("space".to_owned()),
        "backspace" => Some("BackSpace".to_owned()),
        "delete" | "del" => Some("Delete".to_owned()),
        "left" => Some("Left".to_owned()),
        "right" => Some("Right".to_owned()),
        "up" => Some("Up".to_owned()),
        "down" => Some("Down".to_owned()),
        "home" => Some("Home".to_owned()),
        "end" => Some("End".to_owned()),
        "pageup" => Some("Prior".to_owned()),
        "pagedown" => Some("Next".to_owned()),
        other if other.chars().count() == 1 => other.chars().next().map(|c| c.to_string()),
        _ => None,
    }
}

/// Maps a key token from the Tendril DSL onto the Linux input event keycode
/// used by `ydotool key`. Returns `None` for unsupported tokens.
fn ydotool_key_code(key: &str) -> Option<u32> {
    let lower = key.to_ascii_lowercase();
    let code = match lower.as_str() {
        "esc" | "escape" => 1,
        "1" => 2,
        "2" => 3,
        "3" => 4,
        "4" => 5,
        "5" => 6,
        "6" => 7,
        "7" => 8,
        "8" => 9,
        "9" => 10,
        "0" => 11,
        "-" => 12,
        "=" => 13,
        "backspace" => 14,
        "tab" => 15,
        "q" => 16,
        "w" => 17,
        "e" => 18,
        "r" => 19,
        "t" => 20,
        "y" => 21,
        "u" => 22,
        "i" => 23,
        "o" => 24,
        "p" => 25,
        "[" => 26,
        "]" => 27,
        "enter" | "return" => 28,
        "a" => 30,
        "s" => 31,
        "d" => 32,
        "f" => 33,
        "g" => 34,
        "h" => 35,
        "j" => 36,
        "k" => 37,
        "l" => 38,
        ";" => 39,
        "'" => 40,
        "`" => 41,
        "\\" => 43,
        "z" => 44,
        "x" => 45,
        "c" => 46,
        "v" => 47,
        "b" => 48,
        "n" => 49,
        "m" => 50,
        "," => 51,
        "." => 52,
        "/" => 53,
        "space" => 57,
        "f1" => 59,
        "f2" => 60,
        "f3" => 61,
        "f4" => 62,
        "f5" => 63,
        "f6" => 64,
        "f7" => 65,
        "f8" => 66,
        "f9" => 67,
        "f10" => 68,
        "f11" => 87,
        "f12" => 88,
        "home" => 102,
        "up" => 103,
        "pageup" => 104,
        "left" => 105,
        "right" => 106,
        "end" => 107,
        "down" => 108,
        "pagedown" => 109,
        "delete" | "del" => 111,
        _ => return None,
    };
    Some(code)
}

/// Returns the `ydotool click` mask that performs a press immediately followed
/// by a release of `button` (high nibble = release, low nibble = press).
fn ydotool_button_combined_mask(button: MouseButton) -> u32 {
    let press = ydotool_button_press_mask(button);
    let release = ydotool_button_release_mask(button);
    press | release
}

fn ydotool_button_press_mask(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => 0x40,
        MouseButton::Right => 0x41,
        MouseButton::Middle => 0x42,
    }
}

fn ydotool_button_release_mask(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => 0x80,
        MouseButton::Right => 0x81,
        MouseButton::Middle => 0x82,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        WaylandInputBackend, WaylandInputCapability, action_is_keyboard, action_label,
        looks_like_ydotool_socket_error, missing_backend_error, request_has_pointer,
        wtype_key_argument, wtype_modifier_name, ydotool_button_combined_mask,
        ydotool_button_press_mask, ydotool_button_release_mask, ydotool_key_code,
        ydotool_modifier_code,
    };
    use crate::model::{Bounds, InputAction, ModifierKey, MouseButton};
    use crate::platform::{CaptureTargetKind, InputRequest, PlatformAdapterError, PlatformKind};

    #[test]
    fn capability_prefers_ydotool_when_both_available() {
        let capability = WaylandInputCapability {
            ydotool: true,
            wtype: true,
        };
        assert!(capability.any_supported());
        assert_eq!(capability.preferred(), Some(WaylandInputBackend::Ydotool));
    }

    #[test]
    fn capability_falls_back_to_wtype_for_keyboard_only_hosts() {
        let capability = WaylandInputCapability {
            ydotool: false,
            wtype: true,
        };
        assert_eq!(capability.preferred(), Some(WaylandInputBackend::Wtype));
    }

    #[test]
    fn capability_reports_missing_backends_actionably() {
        let error = missing_backend_error(PlatformKind::Linux);
        match error {
            PlatformAdapterError::UnsupportedCapability(capability) => {
                assert!(capability.message.contains("ydotool"));
                assert!(capability.message.contains("wtype"));
                let suggestion = capability
                    .suggested_action
                    .expect("missing-backend error should suggest an install path");
                assert!(suggestion.contains("ydotoold"));
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn ydotool_modifier_codes_match_linux_input_event_codes() {
        // Sanity check against `linux/input-event-codes.h` constants.
        assert_eq!(ydotool_modifier_code(ModifierKey::Ctrl), 29);
        assert_eq!(ydotool_modifier_code(ModifierKey::Shift), 42);
        assert_eq!(ydotool_modifier_code(ModifierKey::Alt), 56);
        assert_eq!(ydotool_modifier_code(ModifierKey::Meta), 125);
    }

    #[test]
    fn ydotool_key_table_covers_alphanumerics_and_named_keys() {
        assert_eq!(ydotool_key_code("c"), Some(46));
        assert_eq!(ydotool_key_code("Enter"), Some(28));
        assert_eq!(ydotool_key_code("escape"), Some(1));
        assert_eq!(ydotool_key_code("tab"), Some(15));
        assert_eq!(ydotool_key_code("space"), Some(57));
        assert_eq!(ydotool_key_code("f5"), Some(63));
        assert_eq!(ydotool_key_code("delete"), Some(111));
        assert_eq!(ydotool_key_code("nonsense-key"), None);
    }

    #[test]
    fn ydotool_button_masks_combine_press_and_release_into_one_invocation() {
        // 0x40 (press left) | 0x80 (release left) == 0xC0.
        assert_eq!(
            ydotool_button_combined_mask(MouseButton::Left),
            ydotool_button_press_mask(MouseButton::Left)
                | ydotool_button_release_mask(MouseButton::Left)
        );
        assert_eq!(ydotool_button_combined_mask(MouseButton::Left), 0xC0);
        assert_eq!(ydotool_button_combined_mask(MouseButton::Right), 0xC1);
        assert_eq!(ydotool_button_combined_mask(MouseButton::Middle), 0xC2);
    }

    #[test]
    fn wtype_modifier_names_use_wayland_canonical_aliases() {
        assert_eq!(wtype_modifier_name(ModifierKey::Ctrl), "ctrl");
        assert_eq!(wtype_modifier_name(ModifierKey::Shift), "shift");
        assert_eq!(wtype_modifier_name(ModifierKey::Alt), "alt");
        // wtype expects `logo` (not `meta` or `super`) for the Windows/Cmd key.
        assert_eq!(wtype_modifier_name(ModifierKey::Meta), "logo");
    }

    #[test]
    fn wtype_key_argument_translates_named_keys_to_xkb_keysyms() {
        assert_eq!(wtype_key_argument("enter").as_deref(), Some("Return"));
        assert_eq!(wtype_key_argument("escape").as_deref(), Some("Escape"));
        assert_eq!(wtype_key_argument("tab").as_deref(), Some("Tab"));
        assert_eq!(wtype_key_argument("pageup").as_deref(), Some("Prior"));
        assert_eq!(
            wtype_key_argument("backspace").as_deref(),
            Some("BackSpace")
        );
        assert_eq!(wtype_key_argument("c").as_deref(), Some("c"));
        assert_eq!(wtype_key_argument("nonsense-key"), None);
    }

    #[test]
    fn keyboard_action_classifier_matches_input_dsl_categories() {
        assert!(action_is_keyboard(&InputAction::KeyTap {
            key: "a".to_owned(),
        }));
        assert!(action_is_keyboard(&InputAction::Hold {
            modifier: ModifierKey::Ctrl,
        }));
        assert!(!action_is_keyboard(&InputAction::Click {
            button: MouseButton::Left,
            x: 0,
            y: 0,
        }));
        assert!(!action_is_keyboard(&InputAction::Wait { duration_ms: 1 }));
    }

    #[test]
    fn pointer_request_detection_flags_clicks_and_drags() {
        let request = sample_request(vec![
            InputAction::KeyTap {
                key: "a".to_owned(),
            },
            InputAction::Click {
                button: MouseButton::Left,
                x: 1,
                y: 2,
            },
        ]);
        assert!(request_has_pointer(&request));

        let request = sample_request(vec![InputAction::KeyTap {
            key: "a".to_owned(),
        }]);
        assert!(!request_has_pointer(&request));
    }

    #[test]
    fn ydotool_socket_diagnostic_classifier_recognises_common_failures() {
        assert!(looks_like_ydotool_socket_error(
            "failed to connect socket /tmp/.ydotool_socket: No such file or directory",
            "",
        ));
        assert!(looks_like_ydotool_socket_error(
            "",
            "ydotoold is not running",
        ));
        assert!(looks_like_ydotool_socket_error("Permission denied", "",));
        assert!(!looks_like_ydotool_socket_error("unknown key 9999", "",));
    }

    #[test]
    fn action_label_serialises_to_compact_json_for_diagnostics() {
        let label = action_label(&InputAction::KeyTap {
            key: "a".to_owned(),
        });
        assert!(label.contains("\"key_tap\""));
        assert!(label.contains("\"a\""));
    }

    fn sample_request(actions: Vec<InputAction>) -> InputRequest {
        InputRequest {
            target: CaptureTargetKind::Display,
            target_id: "1".to_owned(),
            target_name: "Display 1".to_owned(),
            bounds: Bounds {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            app_name: None,
            process_id: None,
            restore_focus: true,
            text: None,
            actions,
        }
    }
}
