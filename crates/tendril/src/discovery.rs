use std::env;
use std::process::Command;

use serde::Deserialize;

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
        DesktopSession::Wayland => Err(PlatformAdapterError::unsupported(
            Capability::TargetDiscovery,
            context.platform,
            CapabilityErrorReason::UnsupportedSession,
            "Generic Wayland discovery is not yet portable enough to expose stable window and display identifiers.",
            Some("Use an X11 session for now or add a compositor-specific discovery backend."),
        )),
        DesktopSession::Unknown
        | DesktopSession::MacOsWindowServer
        | DesktopSession::WindowsDesktop => Err(PlatformAdapterError::unsupported(
            Capability::TargetDiscovery,
            context.platform,
            CapabilityErrorReason::UnsupportedSession,
            "Target discovery requires a detected interactive X11 desktop session.",
            Some("Set DISPLAY or run Tendril from an active graphical login session."),
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
    let script = r#"
set json_text to do shell script "/usr/bin/python3 - <<'PY'
import json
from AppKit import NSScreen
from Quartz import CGWindowListCopyWindowInfo, kCGNullWindowID, kCGWindowListOptionOnScreenOnly

screens = []
for index, screen in enumerate(NSScreen.screens()):
    frame = screen.frame()
    name = screen.localizedName() if hasattr(screen, 'localizedName') else f'Display {index + 1}'
    screens.append({
        'id': f'display-{index + 1}',
        'title': None,
        'kind': 'display',
        'name': str(name),
        'bounds': {
            'x': int(frame.origin.x),
            'y': int(frame.origin.y),
            'width': int(frame.size.width),
            'height': int(frame.size.height),
        },
        'scale_factor': {
            'numerator': int(round(screen.backingScaleFactor() * 1000)),
            'denominator': 1000,
        },
        'capture_supported': True,
        'input_supported': True,
        'app_name': None,
        'process_id': None,
    })

windows = []
for entry in CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly, kCGNullWindowID) or []:
    bounds = entry.get('kCGWindowBounds') or {}
    window_id = entry.get('kCGWindowNumber')
    owner = entry.get('kCGWindowOwnerName')
    title = entry.get('kCGWindowName')
    if not window_id or not bounds.get('Width') or not bounds.get('Height'):
        continue
    windows.append({
        'id': str(window_id),
        'title': title,
        'kind': 'window',
        'name': owner or title or str(window_id),
        'bounds': {
            'x': int(bounds.get('X', 0)),
            'y': int(bounds.get('Y', 0)),
            'width': int(bounds.get('Width', 0)),
            'height': int(bounds.get('Height', 0)),
        },
        'scale_factor': {'numerator': 1, 'denominator': 1},
        'capture_supported': True,
        'input_supported': True,
        'app_name': owner,
        'process_id': entry.get('kCGWindowOwnerPID'),
    })
print(json.dumps({'targets': screens + windows}))
PY"
return json_text
"#;

    match run_osascript(context, script) {
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
        Err(_) => Err(PlatformAdapterError::unsupported(
            Capability::TargetDiscovery,
            context.platform,
            CapabilityErrorReason::UnsupportedFeature,
            "macOS discovery requires system Python with AppKit and Quartz available to the invoking user session.",
            Some(
                "Run Tendril from a logged-in macOS desktop session with Screen Recording access.",
            ),
        )),
    }
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

fn run_osascript(context: &AdapterContext, script: &str) -> Result<String, PlatformAdapterError> {
    run_command(context, "osascript", &["-e", script])
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
        || lower.contains("-1743")
        || lower.contains("screen recording")
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
        Bounds, X11WindowMetadata, extract_all_quoted_strings, parse_simple_geometry,
        parse_window_id_list, parse_xprop_window_metadata, parse_xrandr_geometry,
        parse_xwininfo_geometry,
    };

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
}
