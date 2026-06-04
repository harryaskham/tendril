use std::env;
use std::io::ErrorKind;
use std::process::Command;

use serde_json::Value;

use crate::model::{Bounds, ScaleFactor};
use crate::platform::{
    AdapterContext, AdapterOperation, Capability, CapabilityErrorReason, CaptureTargetKind,
    DesktopSession, PermissionKind, PlatformAdapterError, PlatformKind, TargetCapabilityDiagnostic,
    TargetDescriptor, TargetDiscoveryRequest, TargetInventory,
};
use crate::x11;

const TARGET_FIXTURE_ENV: &str = "TENDRIL_TARGET_FIXTURE_JSON";

pub fn discover_targets(
    context: &AdapterContext,
    _request: &TargetDiscoveryRequest,
) -> Result<TargetInventory, PlatformAdapterError> {
    if let Some(fixture) = load_fixture_inventory(context)? {
        return Ok(fixture);
    }

    match context.platform {
        PlatformKind::Linux => discover_linux_targets(context),
        PlatformKind::Windows11 => discover_windows_targets(context),
        PlatformKind::MacOs => discover_macos_targets(context),
        PlatformKind::Android => Err(PlatformAdapterError::unsupported(
            Capability::TargetDiscovery,
            context.platform,
            CapabilityErrorReason::UnsupportedPlatform,
            "Use `tendril --android <serial> list` for Android target discovery.",
            Some(
                "Pass an adb serial, or use `--android auto` when exactly one device is connected.",
            ),
        )),
    }
}

fn load_fixture_inventory(
    context: &AdapterContext,
) -> Result<Option<TargetInventory>, PlatformAdapterError> {
    let Some(raw) = env::var(TARGET_FIXTURE_ENV).ok() else {
        return Ok(None);
    };

    let inventory = serde_json::from_str::<TargetInventory>(&raw).map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::TargetDiscovery,
            context.platform,
            format!("failed to parse {TARGET_FIXTURE_ENV}: {error}"),
        )
    })?;
    Ok(Some(sort_inventory(inventory)))
}

fn discover_linux_targets(
    context: &AdapterContext,
) -> Result<TargetInventory, PlatformAdapterError> {
    match context.session {
        DesktopSession::X11 => x11::discover_targets(context).map(sort_inventory),
        DesktopSession::Wayland => discover_wayland_targets(context),
        DesktopSession::Unknown
        | DesktopSession::MacOsWindowServer
        | DesktopSession::WindowsDesktop
        | DesktopSession::AndroidDevice => Err(PlatformAdapterError::unsupported(
            Capability::TargetDiscovery,
            context.platform,
            CapabilityErrorReason::UnsupportedSession,
            "Target discovery requires a detected interactive X11 or Wayland desktop session.",
            Some("Set XDG_SESSION_TYPE or run Tendril from an active graphical login session."),
        )),
    }
}

fn discover_wayland_targets(
    context: &AdapterContext,
) -> Result<TargetInventory, PlatformAdapterError> {
    let input_supported = crate::wayland_input::detect_backend().any_supported();
    let mark = |mut inventory: TargetInventory| {
        if input_supported {
            for target in &mut inventory.targets {
                target.input_supported = true;
            }
        }
        inventory
    };

    if let Some(targets) = discover_hyprland_targets(context)? {
        return Ok(mark(sort_inventory(TargetInventory { targets })));
    }

    if let Some(targets) = discover_sway_targets(context)? {
        return Ok(mark(sort_inventory(TargetInventory { targets })));
    }

    if let Some(displays) = discover_wlr_randr_displays(context)? {
        return Ok(mark(sort_inventory(TargetInventory { targets: displays })));
    }

    Err(wayland_discovery_backend_error(context))
}

fn wayland_discovery_backend_error(context: &AdapterContext) -> PlatformAdapterError {
    let on_path = wayland_discovery_backend_tools_on_path();
    let detected = if on_path.is_empty() {
        "none detected on PATH".to_owned()
    } else {
        format!("detected on PATH: {}", on_path.join(", "))
    };

    PlatformAdapterError::unsupported(
        Capability::TargetDiscovery,
        context.platform,
        CapabilityErrorReason::UnsupportedFeature,
        format!(
            "Wayland discovery requires compositor metadata from one of the supported backends: Hyprland (`hyprctl`), sway (`swaymsg`), or wlroots output enumeration (`wlr-randr`); {detected}."
        ),
        Some(
            "Use the backend that matches the active Wayland session: `hyprctl` for Hyprland, `swaymsg` for sway, or `wlr-randr` for wlroots-based display discovery. Capture no longer requires `grim` when an xdg-desktop-portal screenshot backend is available.",
        ),
    )
}

fn wayland_discovery_backend_tools_on_path() -> Vec<&'static str> {
    ["hyprctl", "swaymsg", "wlr-randr"]
        .into_iter()
        .filter(|program| program_on_path(program))
        .collect()
}

fn program_on_path(program: &str) -> bool {
    env::var_os("PATH").is_some_and(|path| {
        env::split_paths(&path).any(|entry| {
            let candidate = entry.join(program);
            candidate.is_file() || {
                #[cfg(windows)]
                {
                    entry.join(format!("{program}.exe")).is_file()
                }
                #[cfg(not(windows))]
                {
                    false
                }
            }
        })
    })
}

#[allow(clippy::too_many_lines)]
fn discover_hyprland_targets(
    context: &AdapterContext,
) -> Result<Option<Vec<TargetDescriptor>>, PlatformAdapterError> {
    let Some(monitors_output) = run_optional_command(context, "hyprctl", &["monitors", "-j"])?
    else {
        return Ok(None);
    };
    let monitors = serde_json::from_str::<Value>(&monitors_output).map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::TargetDiscovery,
            context.platform,
            format!("failed to parse hyprctl monitors JSON: {error}"),
        )
    })?;
    let clients_output =
        run_optional_command(context, "hyprctl", &["clients", "-j"])?.unwrap_or_default();
    let clients = if clients_output.trim().is_empty() {
        Value::Array(Vec::new())
    } else {
        serde_json::from_str::<Value>(&clients_output).map_err(|error| {
            PlatformAdapterError::adapter_failure(
                AdapterOperation::TargetDiscovery,
                context.platform,
                format!("failed to parse hyprctl clients JSON: {error}"),
            )
        })?
    };

    let all_monitors_headless = monitors.as_array().is_some_and(|monitors| {
        !monitors.is_empty() && monitors.iter().all(is_headless_wayland_monitor)
    });

    let mut targets = Vec::new();
    if let Some(monitors) = monitors.as_array() {
        for (index, monitor) in monitors.iter().enumerate() {
            let width = json_u32(monitor, "width").unwrap_or(0);
            let height = json_u32(monitor, "height").unwrap_or(0);
            if width == 0 || height == 0 {
                continue;
            }
            let name = json_str(monitor, "name")
                .map_or_else(|| format!("Display {}", index + 1), str::to_owned);
            let scale = json_f64(monitor, "scale").unwrap_or(1.0);
            let diagnostics = headless_wayland_capture_diagnostic(&name, monitor)
                .into_iter()
                .collect::<Vec<_>>();
            targets.push(TargetDescriptor {
                id: name.clone(),
                title: None,
                kind: CaptureTargetKind::Display,
                name,
                bounds: Bounds {
                    x: json_i32(monitor, "x").unwrap_or(0),
                    y: json_i32(monitor, "y").unwrap_or(0),
                    width,
                    height,
                },
                scale_factor: scale_factor_from_float(scale),
                capture_supported: diagnostics.is_empty(),
                input_supported: false,
                app_name: None,
                process_id: None,
                diagnostics,
            });
        }
    }

    if let Some(clients) = clients.as_array() {
        for client in clients {
            if json_bool(client, "mapped") == Some(false)
                || json_bool(client, "hidden") == Some(true)
            {
                continue;
            }
            let position = json_array_i32_pair(client, "at");
            let size = json_array_u32_pair(client, "size");
            let (Some((x, y)), Some((width, height))) = (position, size) else {
                continue;
            };
            if width == 0 || height == 0 {
                continue;
            }
            let title = json_str(client, "title")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            let app_name = json_str(client, "class")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            if is_filtered_system_window(app_name.as_deref(), title.as_deref()) {
                continue;
            }
            let name = title
                .clone()
                .or_else(|| app_name.clone())
                .unwrap_or_else(|| json_str(client, "address").unwrap_or("window").to_owned());
            let id = format!("hypr:{}", json_str(client, "address").unwrap_or(&name));
            let diagnostics = all_monitors_headless
                .then(headless_wayland_session_capture_diagnostic)
                .into_iter()
                .collect::<Vec<_>>();
            targets.push(TargetDescriptor {
                id,
                title,
                kind: CaptureTargetKind::Window,
                name,
                bounds: Bounds {
                    x,
                    y,
                    width,
                    height,
                },
                scale_factor: ScaleFactor::identity(),
                capture_supported: diagnostics.is_empty(),
                input_supported: false,
                app_name,
                process_id: json_u32(client, "pid"),
                diagnostics,
            });
        }
    }

    Ok((!targets.is_empty()).then_some(targets))
}

fn is_headless_wayland_monitor(monitor: &Value) -> bool {
    let name_or_description = [json_str(monitor, "name"), json_str(monitor, "description")]
        .into_iter()
        .flatten()
        .any(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("headless") || value.contains("sunshine")
        });

    if name_or_description {
        return true;
    }

    let physical_zero = json_u32(monitor, "physicalWidth").unwrap_or(1) == 0
        && json_u32(monitor, "physicalHeight").unwrap_or(1) == 0;
    let make_model_empty = json_str(monitor, "make")
        .unwrap_or_default()
        .trim()
        .is_empty()
        && json_str(monitor, "model")
            .unwrap_or_default()
            .trim()
            .is_empty();

    physical_zero && make_model_empty
}

fn headless_wayland_capture_diagnostic(
    name: &str,
    monitor: &Value,
) -> Option<TargetCapabilityDiagnostic> {
    is_headless_wayland_monitor(monitor).then(|| {
        TargetCapabilityDiagnostic::capture_unavailable(
            Capability::DisplayCapture,
            "wayland_headless_display_capture_unavailable",
            format!(
                "Wayland output `{name}` appears to be a simulated/headless display. In this class of session xdg-desktop-portal Screenshot can time out waiting for an access dialog and the grim fallback can hang, so Tendril does not advertise display capture for this target."
            ),
            "Use a Wayland session with a working screenshot portal/grim backend, or run automation against the packaged X11/Xvfb headless helper. Incorrect simulated refresh-rate metadata is a known caveat and is not itself treated as the failure.",
        )
    })
}

fn headless_wayland_session_capture_diagnostic() -> TargetCapabilityDiagnostic {
    TargetCapabilityDiagnostic::capture_unavailable(
        Capability::WindowCapture,
        "wayland_headless_session_capture_unavailable",
        "All discovered Wayland outputs appear to be simulated/headless, so window screenshots would use the same portal/grim paths that time out for display capture.",
        "Use a Wayland session with a working screenshot portal/grim backend, or run automation against the packaged X11/Xvfb headless helper.",
    )
}

fn discover_sway_targets(
    context: &AdapterContext,
) -> Result<Option<Vec<TargetDescriptor>>, PlatformAdapterError> {
    let Some(outputs_output) =
        run_optional_command(context, "swaymsg", &["-r", "-t", "get_outputs"])?
    else {
        return Ok(None);
    };
    let outputs = serde_json::from_str::<Value>(&outputs_output).map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::TargetDiscovery,
            context.platform,
            format!("failed to parse sway output JSON: {error}"),
        )
    })?;
    let tree_output =
        run_optional_command(context, "swaymsg", &["-r", "-t", "get_tree"])?.unwrap_or_default();
    let tree = if tree_output.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str::<Value>(&tree_output).map_err(|error| {
            PlatformAdapterError::adapter_failure(
                AdapterOperation::TargetDiscovery,
                context.platform,
                format!("failed to parse sway tree JSON: {error}"),
            )
        })?
    };

    let mut targets = Vec::new();
    if let Some(outputs) = outputs.as_array() {
        for output in outputs {
            if json_bool(output, "active") == Some(false) {
                continue;
            }
            let Some(rect) = output.get("rect") else {
                continue;
            };
            let width = json_u32(rect, "width").unwrap_or(0);
            let height = json_u32(rect, "height").unwrap_or(0);
            if width == 0 || height == 0 {
                continue;
            }
            let Some(name) = json_str(output, "name").map(str::to_owned) else {
                continue;
            };
            let scale = json_f64(output, "scale").unwrap_or(1.0);
            targets.push(TargetDescriptor {
                id: name.clone(),
                title: None,
                kind: CaptureTargetKind::Display,
                name,
                bounds: Bounds {
                    x: json_i32(rect, "x").unwrap_or(0),
                    y: json_i32(rect, "y").unwrap_or(0),
                    width,
                    height,
                },
                scale_factor: scale_factor_from_float(scale),
                capture_supported: true,
                input_supported: false,
                app_name: None,
                process_id: None,
                diagnostics: Vec::new(),
            });
        }
    }

    if !tree.is_null() {
        collect_sway_windows(&tree, &mut targets);
    }

    Ok((!targets.is_empty()).then_some(targets))
}

fn discover_wlr_randr_displays(
    context: &AdapterContext,
) -> Result<Option<Vec<TargetDescriptor>>, PlatformAdapterError> {
    let Some(output) = run_optional_command(context, "wlr-randr", &[])? else {
        return Ok(None);
    };
    let mut displays = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_bounds: Option<Bounds> = None;

    for line in output.lines() {
        if line.trim().is_empty() {
            if let (Some(name), Some(bounds)) = (current_name.take(), current_bounds.take())
                && bounds.width > 0
                && bounds.height > 0
            {
                displays.push(TargetDescriptor {
                    id: name.clone(),
                    title: None,
                    kind: CaptureTargetKind::Display,
                    name,
                    bounds,
                    scale_factor: ScaleFactor::identity(),
                    capture_supported: true,
                    input_supported: false,
                    app_name: None,
                    process_id: None,
                    diagnostics: Vec::new(),
                });
            }
            continue;
        }

        if !line.starts_with(' ') && !line.starts_with('\t') {
            current_name = Some(line.trim().to_owned());
            continue;
        }

        if let Some(mode) = line.trim().strip_prefix("current ") {
            current_bounds = parse_wlr_randr_mode(mode);
        }
    }

    if let (Some(name), Some(bounds)) = (current_name.take(), current_bounds.take())
        && bounds.width > 0
        && bounds.height > 0
    {
        displays.push(TargetDescriptor {
            id: name.clone(),
            title: None,
            kind: CaptureTargetKind::Display,
            name,
            bounds,
            scale_factor: ScaleFactor::identity(),
            capture_supported: true,
            input_supported: false,
            app_name: None,
            process_id: None,
            diagnostics: Vec::new(),
        });
    }

    Ok((!displays.is_empty()).then_some(displays))
}

fn collect_sway_windows(node: &Value, targets: &mut Vec<TargetDescriptor>) {
    if let Some(object) = node.as_object() {
        let node_type = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let visible = object
            .get("visible")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if matches!(node_type, "con" | "floating_con") && visible {
            if let Some(rect) = object.get("rect") {
                let width = json_u32(rect, "width").unwrap_or(0);
                let height = json_u32(rect, "height").unwrap_or(0);
                if width > 0 && height > 0 {
                    let title = object
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned);
                    let app_name = object
                        .get("app_id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                        .or_else(|| {
                            object
                                .get("window_properties")
                                .and_then(|properties| properties.get("class"))
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .map(str::to_owned)
                        });
                    if is_filtered_system_window(app_name.as_deref(), title.as_deref()) {
                        // Skip portal/system dialog windows that pollute capture target lists.
                    } else {
                        let name =
                            title
                                .clone()
                                .or_else(|| app_name.clone())
                                .unwrap_or_else(|| {
                                    object.get("id").and_then(Value::as_i64).map_or_else(
                                        || "window".to_owned(),
                                        |value| format!("window-{value}"),
                                    )
                                });
                        let id = object.get("id").and_then(Value::as_i64).map_or_else(
                            || format!("sway:{name}"),
                            |value| format!("sway:{value}"),
                        );
                        targets.push(TargetDescriptor {
                            id,
                            title,
                            kind: CaptureTargetKind::Window,
                            name,
                            bounds: Bounds {
                                x: json_i32(rect, "x").unwrap_or(0),
                                y: json_i32(rect, "y").unwrap_or(0),
                                width,
                                height,
                            },
                            scale_factor: ScaleFactor::identity(),
                            capture_supported: true,
                            input_supported: false,
                            app_name,
                            process_id: object
                                .get("pid")
                                .and_then(Value::as_u64)
                                .and_then(|value| u32::try_from(value).ok()),
                            diagnostics: Vec::new(),
                        });
                    }
                }
            }
        }

        for key in ["nodes", "floating_nodes"] {
            if let Some(children) = object.get(key).and_then(Value::as_array) {
                for child in children {
                    collect_sway_windows(child, targets);
                }
            }
        }
    }
}

trait WindowsDiscoveryBackend {
    fn discover_displays(&self) -> Result<Vec<tendril_win32::DisplayInfo>, String>;
    fn discover_windows(&self) -> Result<Vec<tendril_win32::WindowInfo>, String>;
}

#[derive(Debug, Default, Clone, Copy)]
struct NativeWindowsDiscoveryBackend;

impl WindowsDiscoveryBackend for NativeWindowsDiscoveryBackend {
    fn discover_displays(&self) -> Result<Vec<tendril_win32::DisplayInfo>, String> {
        tendril_win32::discover_displays()
    }

    fn discover_windows(&self) -> Result<Vec<tendril_win32::WindowInfo>, String> {
        tendril_win32::discover_windows()
    }
}

fn discover_windows_targets(
    context: &AdapterContext,
) -> Result<TargetInventory, PlatformAdapterError> {
    discover_windows_targets_with_backend(context, &NativeWindowsDiscoveryBackend)
}

fn discover_windows_targets_with_backend(
    context: &AdapterContext,
    backend: &dyn WindowsDiscoveryBackend,
) -> Result<TargetInventory, PlatformAdapterError> {
    let mut targets = Vec::new();
    targets.extend(discover_windows_displays_with_backend(context, backend)?);
    targets.extend(discover_windows_windows_with_backend(context, backend)?);
    Ok(sort_inventory(TargetInventory { targets }))
}

fn discover_windows_displays_with_backend(
    context: &AdapterContext,
    backend: &dyn WindowsDiscoveryBackend,
) -> Result<Vec<TargetDescriptor>, PlatformAdapterError> {
    let displays = backend.discover_displays().map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::TargetDiscovery,
            context.platform,
            format!("Windows display discovery failed: {error}"),
        )
    })?;

    Ok(displays
        .into_iter()
        .map(|display| TargetDescriptor {
            id: display.id,
            title: None,
            kind: CaptureTargetKind::Display,
            name: display.name,
            bounds: Bounds {
                x: display.bounds.x,
                y: display.bounds.y,
                width: display.bounds.width,
                height: display.bounds.height,
            },
            scale_factor: ScaleFactor::identity(),
            capture_supported: true,
            input_supported: true,
            app_name: None,
            process_id: None,
            diagnostics: Vec::new(),
        })
        .collect())
}

fn discover_windows_windows_with_backend(
    context: &AdapterContext,
    backend: &dyn WindowsDiscoveryBackend,
) -> Result<Vec<TargetDescriptor>, PlatformAdapterError> {
    let windows = backend.discover_windows().map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::TargetDiscovery,
            context.platform,
            format!("Windows window discovery failed: {error}"),
        )
    })?;

    Ok(windows
        .into_iter()
        .filter(|window| window.bounds.width > 0 && window.bounds.height > 0)
        .map(|window| {
            let app_name = window.app_name;
            let name = app_name.clone().unwrap_or_else(|| window.title.clone());
            TargetDescriptor {
                id: window.id,
                title: Some(window.title),
                kind: CaptureTargetKind::Window,
                name,
                bounds: Bounds {
                    x: window.bounds.x,
                    y: window.bounds.y,
                    width: window.bounds.width,
                    height: window.bounds.height,
                },
                scale_factor: ScaleFactor::identity(),
                capture_supported: true,
                input_supported: true,
                app_name,
                process_id: Some(window.process_id),
                diagnostics: Vec::new(),
            }
        })
        .collect())
}

fn discover_macos_targets(
    context: &AdapterContext,
) -> Result<TargetInventory, PlatformAdapterError> {
    match run_osascript_jxa(context, macos_discovery_script()) {
        Ok(output) => deserialize_json_inventory(&output, context),
        Err(error) if is_macos_permission_error(&error.to_string()) => {
            Err(PlatformAdapterError::missing_permission(
                Capability::TargetDiscovery,
                PermissionKind::ScreenCapture,
                context.platform,
                "macOS target discovery needs Screen Recording consent to enumerate visible windows.",
                crate::platform::screen_recording_remediation_message(),
            ))
        }
        Err(error) => Err(PlatformAdapterError::adapter_failure(
            AdapterOperation::TargetDiscovery,
            context.platform,
            format!("macOS native discovery failed: {error}"),
        )),
    }
}

#[allow(clippy::too_many_lines)]
fn macos_discovery_script() -> &'static str {
    r#"
ObjC.import('AppKit');
ObjC.import('Foundation');
ObjC.import('CoreGraphics');

(function () {
    function emit(jsonObject) {
        var text = JSON.stringify(jsonObject);
        $.NSFileHandle.fileHandleWithStandardOutput.writeData(
            $(text + "\n").dataUsingEncoding($.NSUTF8StringEncoding)
        );
    }

    function trimString(value) {
        if (value === null || value === undefined) {
            return null;
        }
        var text = ObjC.unwrap(value);
        if (typeof text !== 'string') {
            text = String(text);
        }
        text = text.trim();
        return text.length > 0 ? text : null;
    }

    function numberOr(value, fallback) {
        if (value === null || value === undefined) {
            return fallback;
        }
        var parsed = Number(value);
        return isNaN(parsed) ? fallback : parsed;
    }

    function rectValue(bounds, lowerKey, upperKey) {
        if (!bounds || typeof bounds !== 'object') {
            return 0;
        }
        if (Object.prototype.hasOwnProperty.call(bounds, upperKey)) {
            return numberOr(bounds[upperKey], 0);
        }
        if (Object.prototype.hasOwnProperty.call(bounds, lowerKey)) {
            return numberOr(bounds[lowerKey], 0);
        }
        return 0;
    }

    function fileExists(path) {
        return $.NSFileManager.defaultManager.fileExistsAtPath($(path));
    }

    var screens = $.NSScreen.screens;
    var targets = [];
    var screenCount = Number(screens.count);
    for (var index = 0; index < screenCount; index += 1) {
        var screen = screens.objectAtIndex(index);
        var frame = screen.frame;
        var scale = Number(screen.backingScaleFactor);
        var localizedName = trimString(screen.localizedName) || ('Display ' + String(index + 1));
        targets.push({
            id: String(index + 1),
            title: null,
            kind: 'display',
            name: localizedName,
            bounds: {
                x: Math.round(numberOr(frame.origin.x, 0)),
                y: Math.round(numberOr(frame.origin.y, 0)),
                width: Math.round(numberOr(frame.size.width, 0)),
                height: Math.round(numberOr(frame.size.height, 0)),
            },
            scale_factor: {
                numerator: Math.round(scale * 1000),
                denominator: 1000,
            },
            capture_supported: true,
            input_supported: true,
            app_name: null,
            process_id: null,
        });
    }

    var options = $.kCGWindowListOptionOnScreenOnly | $.kCGWindowListExcludeDesktopElements;
    // CGWindowListCopyWindowInfo returns a CFArrayRef; JXA needs castRefToObject
    // before deepUnwrap can iterate it, otherwise we silently observe zero windows.
    var windowListRef = $.CGWindowListCopyWindowInfo(options, $.kCGNullWindowID);
    var windowInfo = windowListRef === null || windowListRef === undefined
        ? []
        : (ObjC.deepUnwrap(ObjC.castRefToObject(windowListRef)) || []);
    var environment = $.NSProcessInfo.processInfo.environment;
    var targetIsYabai = environment.objectForKey($('YABAI_SOCKET')) !== null
        || fileExists('/opt/homebrew/bin/yabai')
        || fileExists('/usr/local/bin/yabai');

    for (var i = 0; i < windowInfo.length; i += 1) {
        var entry = windowInfo[i];
        if (numberOr(entry.kCGWindowLayer, -1) !== 0) {
            continue;
        }
        if (entry.kCGWindowIsOnscreen === false) {
            continue;
        }

        var windowNumber = entry.kCGWindowNumber;
        if (windowNumber === null || windowNumber === undefined) {
            continue;
        }

        var bounds = entry.kCGWindowBounds || {};
        var width = Math.round(rectValue(bounds, 'width', 'Width'));
        var height = Math.round(rectValue(bounds, 'height', 'Height'));
        if (width <= 0 || height <= 0) {
            continue;
        }

        var ownerName = trimString(entry.kCGWindowOwnerName);
        var title = trimString(entry.kCGWindowName);
        var windowId = String(windowNumber);
        targets.push({
            id: windowId,
            title: title,
            kind: 'window',
            name: title || ownerName || ('window-' + windowId),
            bounds: {
                x: Math.round(rectValue(bounds, 'x', 'X')),
                y: Math.round(rectValue(bounds, 'y', 'Y')),
                width: width,
                height: height,
            },
            scale_factor: {
                numerator: 1,
                denominator: 1,
            },
            capture_supported: true,
            input_supported: true,
            app_name: ownerName,
            process_id: entry.kCGWindowOwnerPID === null || entry.kCGWindowOwnerPID === undefined
                ? null
                : Math.round(numberOr(entry.kCGWindowOwnerPID, 0)),
            notes: targetIsYabai
                ? ['yabai detected in session; native Quartz discovery remains authoritative.']
                : [],
        });
    }

    emit({ targets: targets });
}());
"#
}

fn run_command(
    context: &AdapterContext,
    program: &str,
    args: &[&str],
) -> Result<String, PlatformAdapterError> {
    let output = Command::new(program).args(args).output().map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::TargetDiscovery,
            context.platform,
            format!("failed to execute `{program}`: {error}"),
        )
    })?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(PlatformAdapterError::adapter_failure(
            AdapterOperation::TargetDiscovery,
            context.platform,
            format!(
                "`{program}` exited with status {}: {}{}{}",
                output.status,
                stdout.trim(),
                if stdout.trim().is_empty() || stderr.trim().is_empty() {
                    ""
                } else {
                    " | "
                },
                stderr.trim()
            ),
        ))
    }
}

fn run_osascript_jxa(
    context: &AdapterContext,
    script: &str,
) -> Result<String, PlatformAdapterError> {
    run_command(context, "osascript", &["-l", "JavaScript", "-e", script])
}

fn run_optional_command(
    context: &AdapterContext,
    program: &str,
    args: &[&str],
) -> Result<Option<String>, PlatformAdapterError> {
    let output = Command::new(program).args(args).output();
    match output {
        Ok(output) if output.status.success() => {
            Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
            let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
            if command_output_means_backend_unavailable(&stdout, &stderr) {
                Ok(None)
            } else {
                Err(PlatformAdapterError::adapter_failure(
                    AdapterOperation::TargetDiscovery,
                    context.platform,
                    format!(
                        "`{program}` exited with status {}: {}{}{}",
                        output.status,
                        stdout.trim(),
                        if stdout.trim().is_empty() || stderr.trim().is_empty() {
                            ""
                        } else {
                            " | "
                        },
                        stderr.trim()
                    ),
                ))
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(PlatformAdapterError::adapter_failure(
            AdapterOperation::TargetDiscovery,
            context.platform,
            format!("failed to execute `{program}`: {error}"),
        )),
    }
}

fn command_output_means_backend_unavailable(stdout: &str, stderr: &str) -> bool {
    stderr.contains("unknown socket")
        || stderr.contains("unable to retrieve socket path")
        || stderr.contains("unable to connect")
        || stderr.contains("hyprland_instance_signature not set")
        || stderr.contains("is hyprland running")
        || stderr.contains("ipc")
        || stdout.contains("unable to connect")
        || stdout.contains("no running instance")
        || stdout.contains("hyprland_instance_signature not set")
        || stdout.contains("is hyprland running")
}

fn deserialize_json_inventory(
    output: &str,
    context: &AdapterContext,
) -> Result<TargetInventory, PlatformAdapterError> {
    let inventory = serde_json::from_str::<TargetInventory>(output).map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::TargetDiscovery,
            context.platform,
            format!("failed to parse discovery JSON: {error}"),
        )
    })?;
    Ok(sort_inventory(inventory))
}

fn json_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn json_i32(value: &Value, key: &str) -> Option<i32> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|raw| i32::try_from(raw).ok())
}

fn json_u32(value: &Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|raw| u32::try_from(raw).ok())
}

fn json_f64(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn json_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn json_array_i32_pair(value: &Value, key: &str) -> Option<(i32, i32)> {
    let values = value.get(key)?.as_array()?;
    let x = values
        .first()?
        .as_i64()
        .and_then(|raw| i32::try_from(raw).ok())?;
    let y = values
        .get(1)?
        .as_i64()
        .and_then(|raw| i32::try_from(raw).ok())?;
    Some((x, y))
}

fn json_array_u32_pair(value: &Value, key: &str) -> Option<(u32, u32)> {
    let values = value.get(key)?.as_array()?;
    let width = values
        .first()?
        .as_u64()
        .and_then(|raw| u32::try_from(raw).ok())?;
    let height = values
        .get(1)?
        .as_u64()
        .and_then(|raw| u32::try_from(raw).ok())?;
    Some((width, height))
}

fn scale_factor_from_float(scale: f64) -> ScaleFactor {
    let normalized = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let numerator = format!("{:.0}", normalized * 1000.0)
        .parse::<u32>()
        .unwrap_or(u32::MAX)
        .max(1);
    ScaleFactor::new(numerator, 1000)
}

fn parse_wlr_randr_mode(line: &str) -> Option<Bounds> {
    let geometry = line
        .split_whitespace()
        .find(|token| token.contains('x') && token.contains('+'))?;
    parse_simple_geometry(geometry)
}

fn parse_simple_geometry(token: &str) -> Option<Bounds> {
    let (width, rest) = token.split_once('x')?;
    let x_index = rest.find(['+', '-'])?;
    let (height, rest) = rest.split_at(x_index);
    let y_index = rest[1..].find(['+', '-'])? + 1;
    let (x, y) = rest.split_at(y_index);

    Some(Bounds {
        x: x.parse::<i32>().ok()?,
        y: y.parse::<i32>().ok()?,
        width: width.parse::<u32>().ok()?,
        height: height.parse::<u32>().ok()?,
    })
}

fn is_macos_permission_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("not authorized")
        || lower.contains("not permitted")
        || lower.contains("assistive access")
        || lower.contains("screen recording")
        || lower.contains("screen capture")
        || lower.contains("apple events")
        || lower.contains("-1719")
        || lower.contains("-1743")
}

fn sort_inventory(mut inventory: TargetInventory) -> TargetInventory {
    inventory.targets.sort_by(|left, right| {
        let left_key = (
            target_kind_rank(left.kind),
            left.bounds.y,
            left.bounds.x,
            left.name.to_ascii_lowercase(),
            left.id.to_ascii_lowercase(),
        );
        let right_key = (
            target_kind_rank(right.kind),
            right.bounds.y,
            right.bounds.x,
            right.name.to_ascii_lowercase(),
            right.id.to_ascii_lowercase(),
        );
        left_key.cmp(&right_key)
    });

    // Assign stable sequential numeric IDs to display targets so users can
    // simply write `--display 1`, `--display 2`, etc.
    let mut display_index: u32 = 0;
    for target in &mut inventory.targets {
        if target.kind == CaptureTargetKind::Display {
            display_index += 1;
            target.id = display_index.to_string();
        }
    }

    inventory
}

const fn target_kind_rank(kind: CaptureTargetKind) -> u8 {
    match kind {
        CaptureTargetKind::Display => 0,
        CaptureTargetKind::Window => 1,
    }
}

/// Apps/windows that are owned by the desktop session itself (e.g. xdg-desktop-portal
/// authorization dialogs, notification daemons) and should never appear as capture
/// targets. These windows leak into compositor client lists when portal interactions
/// fail (see bd-b6adf6) and pollute target discovery for agents.
///
/// Matching is case-insensitive on the application identifier (Hyprland `class` /
/// sway `app_id`/window-class) with a small allowance for matching by window title
/// when the app id is unavailable.
pub(crate) fn is_filtered_system_window(app_name: Option<&str>, title: Option<&str>) -> bool {
    const FILTERED_APP_PREFIXES: &[&str] = &[
        "xdg-desktop-portal",
        "org.freedesktop.impl.portal",
        "org.freedesktop.portal",
    ];

    if let Some(app) = app_name {
        let app_lower = app.to_ascii_lowercase();
        if FILTERED_APP_PREFIXES
            .iter()
            .any(|prefix| app_lower.starts_with(prefix))
        {
            return true;
        }
    }

    // Fall back to title only when there is no app id at all, since titles are
    // user-facing strings that could legitimately mention these names.
    if app_name.is_none()
        && let Some(title) = title
    {
        let title_lower = title.to_ascii_lowercase();
        if FILTERED_APP_PREFIXES
            .iter()
            .any(|prefix| title_lower.starts_with(prefix))
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::{
        Bounds, WindowsDiscoveryBackend, command_output_means_backend_unavailable,
        discover_windows_targets_with_backend, headless_wayland_capture_diagnostic,
        is_filtered_system_window, is_headless_wayland_monitor, is_macos_permission_error,
        json_array_i32_pair, json_array_u32_pair, macos_discovery_script, parse_simple_geometry,
        parse_wlr_randr_mode, scale_factor_from_float, wayland_discovery_backend_error,
        wayland_discovery_backend_tools_on_path,
    };
    use crate::model::ScaleFactor;
    use crate::platform::{
        AdapterContext, Capability, CaptureTargetKind, DesktopSession, PlatformAdapterError,
        PlatformKind,
    };
    use serde_json::json;

    #[test]
    fn filters_xdg_desktop_portal_windows_by_app_id() {
        // Hyprland reports portal dialogs with class "xdg-desktop-portal-gtk" (bd-b6adf6).
        assert!(is_filtered_system_window(
            Some("xdg-desktop-portal-gtk"),
            Some("Open File")
        ));
        assert!(is_filtered_system_window(
            Some("xdg-desktop-portal-hyprland"),
            None
        ));
        assert!(is_filtered_system_window(
            Some("org.freedesktop.impl.portal.desktop.gtk"),
            None
        ));
        assert!(is_filtered_system_window(
            Some("XDG-Desktop-Portal-GTK"),
            None
        ));
    }

    #[test]
    fn keeps_regular_application_windows() {
        assert!(!is_filtered_system_window(
            Some("firefox"),
            Some("Mozilla Firefox")
        ));
        assert!(!is_filtered_system_window(Some("Alacritty"), None));
        assert!(!is_filtered_system_window(
            Some("editor"),
            Some("xdg-desktop-portal docs")
        ));
    }

    #[test]
    fn falls_back_to_title_when_app_id_missing() {
        assert!(is_filtered_system_window(
            None,
            Some("xdg-desktop-portal-gtk")
        ));
        assert!(!is_filtered_system_window(None, Some("Untitled document")));
    }

    #[test]
    fn parses_negative_geometry_offsets() {
        assert_eq!(
            parse_simple_geometry("800x600-10+20"),
            Some(Bounds {
                x: -10,
                y: 20,
                width: 800,
                height: 600,
            })
        );
    }

    #[test]
    fn parses_wlr_randr_current_mode_line() {
        assert_eq!(
            parse_wlr_randr_mode(
                "1920x1080 px, 60.000 Hz, position 0,0, transform normal, scale 1.000000"
            ),
            None
        );
        assert_eq!(
            parse_wlr_randr_mode("1920x1080+0+0"),
            Some(Bounds {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            })
        );
    }

    struct MockWindowsDiscoveryBackend;

    impl WindowsDiscoveryBackend for MockWindowsDiscoveryBackend {
        fn discover_displays(&self) -> Result<Vec<tendril_win32::DisplayInfo>, String> {
            Ok(vec![tendril_win32::DisplayInfo {
                id: "1".to_owned(),
                name: r"\\.\DISPLAY1".to_owned(),
                bounds: tendril_win32::Bounds {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
            }])
        }

        fn discover_windows(&self) -> Result<Vec<tendril_win32::WindowInfo>, String> {
            Ok(vec![tendril_win32::WindowInfo {
                id: "0x10".to_owned(),
                title: "Inbox".to_owned(),
                app_name: Some("OUTLOOK".to_owned()),
                process_id: 4242,
                bounds: tendril_win32::Bounds {
                    x: 120,
                    y: 80,
                    width: 1440,
                    height: 900,
                },
            }])
        }
    }

    #[test]
    fn windows_native_discovery_backend_maps_display_and_window_targets() {
        let inventory = discover_windows_targets_with_backend(
            &AdapterContext::windows11(),
            &MockWindowsDiscoveryBackend,
        )
        .expect("mocked windows discovery should succeed");

        assert_eq!(inventory.targets.len(), 2);
        assert_eq!(inventory.targets[0].kind, CaptureTargetKind::Display);
        assert_eq!(inventory.targets[0].id, "1");
        assert_eq!(inventory.targets[0].name, r"\\.\DISPLAY1");
        assert_eq!(inventory.targets[1].kind, CaptureTargetKind::Window);
        assert_eq!(inventory.targets[1].id, "0x10");
        assert_eq!(inventory.targets[1].title.as_deref(), Some("Inbox"));
        assert_eq!(inventory.targets[1].name, "OUTLOOK");
        assert_eq!(inventory.targets[1].process_id, Some(4242));
    }

    #[test]
    fn windows_native_discovery_errors_are_structured() {
        struct FailingBackend;

        impl WindowsDiscoveryBackend for FailingBackend {
            fn discover_displays(&self) -> Result<Vec<tendril_win32::DisplayInfo>, String> {
                Err("EnumDisplayMonitors failed".to_owned())
            }

            fn discover_windows(&self) -> Result<Vec<tendril_win32::WindowInfo>, String> {
                Ok(Vec::new())
            }
        }

        let error =
            discover_windows_targets_with_backend(&AdapterContext::windows11(), &FailingBackend)
                .expect_err("mocked discovery failure should surface as an adapter error");

        match error {
            PlatformAdapterError::AdapterFailure {
                platform, message, ..
            } => {
                assert_eq!(platform, PlatformKind::Windows11);
                assert!(message.contains("EnumDisplayMonitors failed"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn macos_permission_classifier_catches_osascript_permission_failures() {
        assert!(is_macos_permission_error(
            "osascript exited with status 1: System Events got an error: osascript is not allowed assistive access. (-1719)"
        ));
        assert!(is_macos_permission_error(
            "Platform adapter failure during TargetDiscovery on MacOs: screen recording access denied"
        ));
    }

    #[test]
    fn macos_discovery_script_uses_built_in_jxa_bridge() {
        let script = macos_discovery_script();

        assert!(script.contains("ObjC.import('CoreGraphics')"));
        assert!(script.contains("NSScreen.screens"));
        assert!(script.contains("CGWindowListCopyWindowInfo"));
        assert!(!script.contains("import AppKit\nimport ApplicationServices"));
    }

    /// Regression test for bd-845b47.
    ///
    /// `CGWindowListCopyWindowInfo` returns a `CFArrayRef`. JXA's `ObjC.deepUnwrap`
    /// silently treats the raw ref as an empty array unless we first call
    /// `ObjC.castRefToObject` to bridge it to an `NSArray`. Without the cast the
    /// macOS `tendril list` output included only displays — no windows — making
    /// `tendril --window <id>` unreachable from remote agents. Lock the cast in
    /// place so future edits don't silently drop window enumeration again.
    #[test]
    fn macos_discovery_script_casts_cfarrayref_before_deep_unwrap() {
        let script = macos_discovery_script();

        assert!(
            script.contains("ObjC.castRefToObject("),
            "macOS discovery script must castRefToObject the CFArrayRef returned by \
             CGWindowListCopyWindowInfo before deepUnwrap, otherwise window \
             enumeration silently returns zero entries (bd-845b47)."
        );
        assert!(
            script.contains("ObjC.deepUnwrap(ObjC.castRefToObject("),
            "deepUnwrap must wrap castRefToObject(windowListRef) so the bridged \
             NSArray is iterated (bd-845b47)."
        );
        // Also assert that we still emit window-kind targets in the script body.
        assert!(
            script.contains("kind: 'window'"),
            "macOS discovery script must emit kind: 'window' targets (bd-845b47)."
        );
    }

    #[test]
    fn inactive_wayland_compositor_output_is_treated_as_backend_unavailable() {
        assert!(command_output_means_backend_unavailable(
            "",
            "hyprland_instance_signature not set! (is hyprland running?)"
        ));
        assert!(command_output_means_backend_unavailable(
            "unable to connect to socket",
            ""
        ));
        assert!(!command_output_means_backend_unavailable(
            "{not-json}",
            "syntax error"
        ));
    }

    #[test]
    fn wayland_backend_diagnostic_only_reports_supported_matrix_tools() {
        let detected = wayland_discovery_backend_tools_on_path();

        assert!(
            detected
                .iter()
                .all(|tool| matches!(*tool, "hyprctl" | "swaymsg" | "wlr-randr"))
        );
    }

    #[test]
    fn wayland_headless_monitor_diagnostic_marks_capture_unavailable() {
        let monitor = json!({
            "name": "sunshine-headless",
            "description": "",
            "make": "",
            "model": "",
            "physicalWidth": 0,
            "physicalHeight": 0,
        });

        assert!(is_headless_wayland_monitor(&monitor));
        let diagnostic = headless_wayland_capture_diagnostic("sunshine-headless", &monitor)
            .expect("headless monitor should have a capture diagnostic");
        assert_eq!(diagnostic.capability, Capability::DisplayCapture);
        assert_eq!(
            diagnostic.code,
            "wayland_headless_display_capture_unavailable"
        );
        assert!(diagnostic.message.contains("xdg-desktop-portal"));
        assert!(diagnostic.message.contains("grim"));
        assert!(
            diagnostic
                .suggested_action
                .as_deref()
                .is_some_and(|action| {
                    action.contains("X11/Xvfb") && action.contains("refresh-rate")
                })
        );
    }

    #[test]
    fn wayland_backend_error_describes_supported_matrices_without_grim_requirement() {
        let error =
            wayland_discovery_backend_error(&AdapterContext::linux(DesktopSession::Wayland, None));

        match error {
            PlatformAdapterError::UnsupportedCapability(capability) => {
                assert!(capability.message.contains("hyprctl"));
                assert!(capability.message.contains("swaymsg"));
                assert!(capability.message.contains("wlr-randr"));
                assert!(!capability.message.contains("grim"));
                assert!(
                    capability.suggested_action.as_deref().is_some_and(
                        |message| message.contains("Capture no longer requires `grim`")
                    )
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn scale_factor_from_float_reduces_and_falls_back_for_invalid_scales() {
        // Whole and fractional scales reduce to their simplest ratio.
        assert_eq!(scale_factor_from_float(1.0), ScaleFactor::identity());
        let two_x = scale_factor_from_float(2.0);
        assert_eq!((two_x.numerator, two_x.denominator), (2, 1));
        let one_point_five = scale_factor_from_float(1.5);
        assert_eq!((one_point_five.numerator, one_point_five.denominator), (3, 2));
        // Non-finite or non-positive scales fall back to identity (1/1).
        assert_eq!(scale_factor_from_float(0.0), ScaleFactor::identity());
        assert_eq!(scale_factor_from_float(-2.0), ScaleFactor::identity());
        assert_eq!(scale_factor_from_float(f64::NAN), ScaleFactor::identity());
        assert_eq!(scale_factor_from_float(f64::INFINITY), ScaleFactor::identity());
    }

    #[test]
    fn backend_unavailable_classifier_matches_known_markers_only() {
        // Markers in stderr are detected.
        assert!(command_output_means_backend_unavailable(
            "",
            "error: is hyprland running?"
        ));
        assert!(command_output_means_backend_unavailable("", "unknown socket"));
        // Markers in stdout are detected.
        assert!(command_output_means_backend_unavailable(
            "no running instance",
            ""
        ));
        assert!(command_output_means_backend_unavailable(
            "unable to connect",
            ""
        ));
        // Unrelated output is not classified as a backend-unavailable signal.
        assert!(!command_output_means_backend_unavailable(
            "[{\"id\":1}]",
            ""
        ));
    }

    #[test]
    fn json_array_i32_pair_reads_pair_and_rejects_malformed_inputs() {
        let object = json!({
            "origin": [-10, 20],
            "too_short": [1],
            "not_array": "10,20",
            "out_of_range": [i64::from(i32::MAX) + 1, 0],
        });
        // A well-formed two-element array (including negatives) is read.
        assert_eq!(json_array_i32_pair(&object, "origin"), Some((-10, 20)));
        // Missing key, non-array, short array, and out-of-range all return None.
        assert_eq!(json_array_i32_pair(&object, "missing"), None);
        assert_eq!(json_array_i32_pair(&object, "not_array"), None);
        assert_eq!(json_array_i32_pair(&object, "too_short"), None);
        assert_eq!(json_array_i32_pair(&object, "out_of_range"), None);
    }

    #[test]
    fn json_array_u32_pair_reads_pair_and_rejects_negative_or_out_of_range() {
        let object = json!({
            "size": [1920, 1080],
            "negative": [-1, 1080],
            "out_of_range": [i64::from(u32::MAX) + 1, 0],
            "short": [800],
        });
        // A well-formed size pair is read.
        assert_eq!(json_array_u32_pair(&object, "size"), Some((1920, 1080)));
        // Negative, out-of-range, short, and missing all return None.
        assert_eq!(json_array_u32_pair(&object, "negative"), None);
        assert_eq!(json_array_u32_pair(&object, "out_of_range"), None);
        assert_eq!(json_array_u32_pair(&object, "short"), None);
        assert_eq!(json_array_u32_pair(&object, "missing"), None);
    }
}
