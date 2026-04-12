use std::env;
use std::io::ErrorKind;
use std::process::Command;

use serde::Deserialize;
use serde_json::Value;

use crate::model::{Bounds, ScaleFactor};
use crate::platform::{
    AdapterContext, AdapterOperation, Capability, CapabilityErrorReason, CaptureTargetKind,
    DesktopSession, PermissionKind, PlatformAdapterError, PlatformKind, TargetDescriptor,
    TargetDiscoveryRequest, TargetInventory,
};

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
        DesktopSession::X11 => {
            let mut targets = Vec::new();
            targets.extend(discover_x11_displays(context)?);
            targets.extend(discover_x11_windows(context)?);
            Ok(sort_inventory(TargetInventory { targets }))
        }
        DesktopSession::Wayland => discover_wayland_targets(context),
        DesktopSession::Unknown
        | DesktopSession::MacOsWindowServer
        | DesktopSession::WindowsDesktop => Err(PlatformAdapterError::unsupported(
            Capability::TargetDiscovery,
            context.platform,
            CapabilityErrorReason::UnsupportedSession,
            "Target discovery requires a detected interactive X11 or Wayland desktop session.",
            Some("Set XDG_SESSION_TYPE or run Tendril from an active graphical login session."),
        )),
    }
}

fn discover_x11_displays(
    context: &AdapterContext,
) -> Result<Vec<TargetDescriptor>, PlatformAdapterError> {
    let output = run_command(context, "xrandr", &["--query"])?;
    let mut displays = Vec::new();

    for line in output.lines() {
        if !line.contains(" connected") {
            continue;
        }

        let mut parts = line.split_whitespace();
        let Some(id) = parts.next() else {
            continue;
        };

        let bounds = line
            .split_whitespace()
            .find_map(parse_xrandr_geometry)
            .unwrap_or(Bounds {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            });

        if bounds.width == 0 || bounds.height == 0 {
            continue;
        }

        displays.push(TargetDescriptor {
            id: id.to_owned(),
            title: None,
            kind: CaptureTargetKind::Display,
            name: id.to_owned(),
            bounds,
            scale_factor: ScaleFactor::identity(),
            capture_supported: true,
            input_supported: true,
            app_name: None,
            process_id: None,
        });
    }

    Ok(displays)
}

fn discover_x11_windows(
    context: &AdapterContext,
) -> Result<Vec<TargetDescriptor>, PlatformAdapterError> {
    let output = run_command(context, "xprop", &["-root", "_NET_CLIENT_LIST_STACKING"])?;
    let window_ids = parse_window_id_list(&output);
    let mut windows = Vec::new();

    for window_id in window_ids {
        let Some(descriptor) = discover_x11_window(context, &window_id) else {
            continue;
        };
        windows.push(descriptor);
    }

    Ok(windows)
}

fn discover_x11_window(context: &AdapterContext, window_id: &str) -> Option<TargetDescriptor> {
    let xwininfo = run_command(context, "xwininfo", &["-id", window_id]).ok()?;
    let geometry = parse_xwininfo_geometry(&xwininfo)?;
    if geometry.width == 0 || geometry.height == 0 {
        return None;
    }
    if !xwininfo
        .lines()
        .any(|line| line.contains("Map State: IsViewable"))
    {
        return None;
    }

    let xprop = run_command(
        context,
        "xprop",
        &[
            "-id",
            window_id,
            "_NET_WM_NAME",
            "WM_NAME",
            "WM_CLASS",
            "_NET_WM_PID",
        ],
    )
    .unwrap_or_default();
    let metadata = parse_xprop_window_metadata(&xprop);
    let title = metadata
        .title
        .or_else(|| parse_xwininfo_title(&xwininfo))
        .filter(|value| !value.is_empty());
    let name = metadata
        .app_name
        .clone()
        .or_else(|| title.clone())
        .unwrap_or_else(|| window_id.to_owned());

    Some(TargetDescriptor {
        id: window_id.to_owned(),
        title,
        kind: CaptureTargetKind::Window,
        name,
        bounds: geometry,
        scale_factor: ScaleFactor::identity(),
        capture_supported: true,
        input_supported: true,
        app_name: metadata.app_name,
        process_id: metadata.process_id,
    })
}

fn discover_wayland_targets(
    context: &AdapterContext,
) -> Result<TargetInventory, PlatformAdapterError> {
    if let Some(targets) = discover_hyprland_targets(context)? {
        return Ok(sort_inventory(TargetInventory { targets }));
    }

    if let Some(targets) = discover_sway_targets(context)? {
        return Ok(sort_inventory(TargetInventory { targets }));
    }

    if let Some(displays) = discover_wlr_randr_displays(context)? {
        return Ok(sort_inventory(TargetInventory { targets: displays }));
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

    let mut targets = Vec::new();
    if let Some(monitors) = monitors.as_array() {
        for (index, monitor) in monitors.iter().enumerate() {
            let width = json_u32(monitor, "width").unwrap_or(0);
            let height = json_u32(monitor, "height").unwrap_or(0);
            if width == 0 || height == 0 {
                continue;
            }
            let name = json_str(monitor, "name")
                .map_or_else(|| format!("display-{}", index + 1), str::to_owned);
            let scale = json_f64(monitor, "scale").unwrap_or(1.0);
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
                capture_supported: true,
                input_supported: false,
                app_name: None,
                process_id: None,
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
            let name = title
                .clone()
                .or_else(|| app_name.clone())
                .unwrap_or_else(|| json_str(client, "address").unwrap_or("window").to_owned());
            let id = format!("hypr:{}", json_str(client, "address").unwrap_or(&name));
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
                capture_supported: true,
                input_supported: false,
                app_name,
                process_id: json_u32(client, "pid"),
            });
        }
    }

    Ok((!targets.is_empty()).then_some(targets))
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
                    let name = title
                        .clone()
                        .or_else(|| app_name.clone())
                        .unwrap_or_else(|| {
                            object.get("id").and_then(Value::as_i64).map_or_else(
                                || "window".to_owned(),
                                |value| format!("window-{value}"),
                            )
                        });
                    let id = object
                        .get("id")
                        .and_then(Value::as_i64)
                        .map_or_else(|| format!("sway:{name}"), |value| format!("sway:{value}"));
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
                    });
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

fn discover_windows_targets(
    context: &AdapterContext,
) -> Result<TargetInventory, PlatformAdapterError> {
    let mut targets = Vec::new();
    targets.extend(discover_windows_displays(context)?);
    targets.extend(discover_windows_windows(context)?);
    Ok(sort_inventory(TargetInventory { targets }))
}

fn discover_windows_displays(
    context: &AdapterContext,
) -> Result<Vec<TargetDescriptor>, PlatformAdapterError> {
    #[derive(Debug, Deserialize)]
    struct ScreenBounds {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    }

    #[derive(Debug, Deserialize)]
    struct ScreenRecord {
        id: String,
        name: String,
        bounds: ScreenBounds,
    }

    let script = r"
Add-Type -AssemblyName System.Windows.Forms
[System.Windows.Forms.Screen]::AllScreens |
  ForEach-Object {
    [pscustomobject]@{
      id = $_.DeviceName
      name = $_.DeviceName
      bounds = [pscustomobject]@{
        x = $_.Bounds.X
        y = $_.Bounds.Y
        width = $_.Bounds.Width
        height = $_.Bounds.Height
      }
    }
  } | ConvertTo-Json -Compress
";
    let output = run_powershell(context, script)?;
    let screens = deserialize_json_array::<ScreenRecord>(&output, context)?;

    Ok(screens
        .into_iter()
        .map(|screen| TargetDescriptor {
            id: screen.id,
            title: None,
            kind: CaptureTargetKind::Display,
            name: screen.name,
            bounds: Bounds {
                x: screen.bounds.x,
                y: screen.bounds.y,
                width: screen.bounds.width,
                height: screen.bounds.height,
            },
            scale_factor: ScaleFactor::identity(),
            capture_supported: true,
            input_supported: true,
            app_name: None,
            process_id: None,
        })
        .collect())
}

fn discover_windows_windows(
    context: &AdapterContext,
) -> Result<Vec<TargetDescriptor>, PlatformAdapterError> {
    #[derive(Debug, Deserialize)]
    struct WindowBounds {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    }

    #[derive(Debug, Deserialize)]
    struct WindowRecord {
        id: String,
        title: String,
        app_name: String,
        process_id: u32,
        bounds: WindowBounds,
    }

    let script = r#"
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
public static class TendrilNative {
  [DllImport("user32.dll")]
  public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
}
"@
Get-Process |
  Where-Object { $_.MainWindowHandle -ne 0 -and -not [string]::IsNullOrWhiteSpace($_.MainWindowTitle) } |
  ForEach-Object {
    $rect = New-Object RECT
    [void][TendrilNative]::GetWindowRect($_.MainWindowHandle, [ref]$rect)
    [pscustomobject]@{
      id = ('0x{0:X}' -f $_.MainWindowHandle)
      title = $_.MainWindowTitle
      app_name = $_.ProcessName
      process_id = $_.Id
      bounds = [pscustomobject]@{
        x = $rect.Left
        y = $rect.Top
        width = [Math]::Max(0, $rect.Right - $rect.Left)
        height = [Math]::Max(0, $rect.Bottom - $rect.Top)
      }
    }
  } | ConvertTo-Json -Compress
"#;
    let output = run_powershell(context, script)?;
    let windows = deserialize_json_array::<WindowRecord>(&output, context)?;

    Ok(windows
        .into_iter()
        .filter(|window| window.bounds.width > 0 && window.bounds.height > 0)
        .map(|window| TargetDescriptor {
            id: window.id,
            title: Some(window.title.clone()),
            kind: CaptureTargetKind::Window,
            name: window.app_name.clone(),
            bounds: Bounds {
                x: window.bounds.x,
                y: window.bounds.y,
                width: window.bounds.width,
                height: window.bounds.height,
            },
            scale_factor: ScaleFactor::identity(),
            capture_supported: true,
            input_supported: true,
            app_name: Some(window.app_name),
            process_id: Some(window.process_id),
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
                "Grant Screen Recording to the invoking terminal or tendril binary, then rerun tendril list.",
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
            id: 'display-' + String(index + 1),
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
    var windowInfo = ObjC.deepUnwrap($.CGWindowListCopyWindowInfo(options, $.kCGNullWindowID)) || [];
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

fn run_powershell(context: &AdapterContext, script: &str) -> Result<String, PlatformAdapterError> {
    run_command(context, "powershell", &["-NoProfile", "-Command", script])
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
            if stderr.contains("unknown socket")
                || stderr.contains("unable to retrieve socket path")
                || stderr.contains("unable to connect")
                || stderr.contains("ipc")
                || stdout.contains("unable to connect")
                || stdout.contains("no running instance")
            {
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

fn deserialize_json_array<T>(
    output: &str,
    context: &AdapterContext,
) -> Result<Vec<T>, PlatformAdapterError>
where
    T: for<'de> Deserialize<'de>,
{
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    serde_json::from_str::<Vec<T>>(trimmed)
        .or_else(|_| serde_json::from_str::<T>(trimmed).map(|value| vec![value]))
        .map_err(|error| {
            PlatformAdapterError::adapter_failure(
                AdapterOperation::TargetDiscovery,
                context.platform,
                format!("failed to parse discovery JSON array: {error}"),
            )
        })
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
    ScaleFactor {
        numerator,
        denominator: 1000,
    }
}

fn parse_wlr_randr_mode(line: &str) -> Option<Bounds> {
    let geometry = line
        .split_whitespace()
        .find(|token| token.contains('x') && token.contains('+'))?;
    parse_simple_geometry(geometry)
}

fn parse_xrandr_geometry(token: &str) -> Option<Bounds> {
    if token.contains('/') {
        return None;
    }
    parse_simple_geometry(token)
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

fn parse_window_id_list(output: &str) -> Vec<String> {
    output
        .split(|character: char| character.is_whitespace() || character == ',')
        .filter(|token| token.starts_with("0x"))
        .map(|token| token.trim().to_owned())
        .collect()
}

fn parse_xwininfo_geometry(output: &str) -> Option<Bounds> {
    let mut x = None;
    let mut y = None;
    let mut width = None;
    let mut height = None;

    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("Absolute upper-left X:") {
            x = value.trim().parse::<i32>().ok();
        } else if let Some(value) = trimmed.strip_prefix("Absolute upper-left Y:") {
            y = value.trim().parse::<i32>().ok();
        } else if let Some(value) = trimmed.strip_prefix("Width:") {
            width = value.trim().parse::<u32>().ok();
        } else if let Some(value) = trimmed.strip_prefix("Height:") {
            height = value.trim().parse::<u32>().ok();
        }
    }

    Some(Bounds {
        x: x?,
        y: y?,
        width: width?,
        height: height?,
    })
}

fn parse_xwininfo_title(output: &str) -> Option<String> {
    output
        .lines()
        .next()
        .and_then(extract_last_quoted_string)
        .map(str::to_owned)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct X11WindowMetadata {
    title: Option<String>,
    app_name: Option<String>,
    process_id: Option<u32>,
}

fn parse_xprop_window_metadata(output: &str) -> X11WindowMetadata {
    let mut metadata = X11WindowMetadata::default();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("_NET_WM_NAME") || trimmed.starts_with("WM_NAME") {
            if metadata.title.is_none() {
                metadata.title = extract_last_quoted_string(trimmed).map(str::to_owned);
            }
        } else if trimmed.starts_with("WM_CLASS") {
            let quoted = extract_all_quoted_strings(trimmed);
            metadata.app_name = quoted.last().map(|value| (*value).to_owned());
        } else if let Some((_, value)) = trimmed.split_once('=')
            && trimmed.starts_with("_NET_WM_PID")
        {
            metadata.process_id = value.trim().parse::<u32>().ok();
        }
    }

    metadata
}

fn extract_last_quoted_string(input: &str) -> Option<&str> {
    extract_all_quoted_strings(input).last().copied()
}

fn extract_all_quoted_strings(input: &str) -> Vec<&str> {
    let mut values = Vec::new();
    let mut rest = input;

    while let Some(start) = rest.find('"') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('"') else {
            break;
        };
        values.push(&after_start[..end]);
        rest = &after_start[end + 1..];
    }

    values
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
    inventory
}

const fn target_kind_rank(kind: CaptureTargetKind) -> u8 {
    match kind {
        CaptureTargetKind::Display => 0,
        CaptureTargetKind::Window => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Bounds, X11WindowMetadata, extract_all_quoted_strings, is_macos_permission_error,
        macos_discovery_script, parse_simple_geometry, parse_window_id_list, parse_wlr_randr_mode,
        parse_xprop_window_metadata, parse_xrandr_geometry, parse_xwininfo_geometry,
        wayland_discovery_backend_error, wayland_discovery_backend_tools_on_path,
    };
    use crate::platform::{AdapterContext, DesktopSession, PlatformAdapterError};

    #[test]
    fn parses_xrandr_connected_monitor_geometry() {
        let token = "3840x2160+1920+0";

        assert_eq!(
            parse_xrandr_geometry(token),
            Some(Bounds {
                x: 1920,
                y: 0,
                width: 3840,
                height: 2160,
            })
        );
    }

    #[test]
    fn rejects_xrandr_physical_size_tokens() {
        assert_eq!(parse_xrandr_geometry("1920/309x1080/174+0+0"), None);
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
    fn parses_xprop_client_list() {
        let ids = parse_window_id_list(
            "_NET_CLIENT_LIST_STACKING(WINDOW): window id # 0x4a00007, 0x5200003, 0x6200012",
        );

        assert_eq!(ids, vec!["0x4a00007", "0x5200003", "0x6200012"]);
    }

    #[test]
    fn parses_xwininfo_geometry_block() {
        let output = r"
  Absolute upper-left X:  42
  Absolute upper-left Y:  77
  Width: 1280
  Height: 720
";

        assert_eq!(
            parse_xwininfo_geometry(output),
            Some(Bounds {
                x: 42,
                y: 77,
                width: 1280,
                height: 720,
            })
        );
    }

    #[test]
    fn parses_xprop_window_metadata() {
        let output = r#"
_NET_WM_NAME(UTF8_STRING) = "Inbox - Mozilla Firefox"
WM_CLASS(STRING) = "Navigator", "firefox"
_NET_WM_PID(CARDINAL) = 12345
"#;

        assert_eq!(
            parse_xprop_window_metadata(output),
            X11WindowMetadata {
                title: Some("Inbox - Mozilla Firefox".to_owned()),
                app_name: Some("firefox".to_owned()),
                process_id: Some(12345),
            }
        );
    }

    #[test]
    fn quoted_string_extraction_preserves_all_values() {
        assert_eq!(
            extract_all_quoted_strings(r#"WM_CLASS(STRING) = "Navigator", "firefox""#),
            vec!["Navigator", "firefox"]
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
        assert!(script.contains("CGWindowListCopyWindowInfo"));
        assert!(!script.contains("import AppKit\nimport ApplicationServices"));
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
}
