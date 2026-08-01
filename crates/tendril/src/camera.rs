//! Cross-platform camera / non-display video-device discovery and capture.
//!
//! `tendril list` surfaces video capture devices (webcams, Continuity Camera,
//! virtual cameras) alongside windows and displays so an agent can discover
//! them before calling `tendril capture --camera <id>`. Discovery uses the
//! built-in macOS system profiler, Linux's V4L2 sysfs inventory, and ffmpeg's
//! `DirectShow` inventory on Windows. A single frame is captured through the
//! matching ffmpeg input backend on every supported desktop platform.

use std::path::Path;

use crate::error::TendrilError;
use crate::model::CameraDescriptor;

const FFMPEG_BIN: &str = "ffmpeg";
const AVFOUNDATION_DEFAULT_FRAMERATE: &str = "30";

/// Parse the JSON emitted by `system_profiler SPCameraDataType -json` into the
/// camera descriptor list. The relevant shape is:
///
/// ```json
/// {
///   "SPCameraDataType": [
///     {
///       "_name": "MacBook Pro Camera",
///       "spcamera_model-id": "MacBook Pro Camera",
///       "spcamera_unique-id": "6C707041-05AC-0010-0006-000000000001"
///     }
///   ]
/// }
/// ```
///
/// Returns an empty list for malformed JSON or a missing `SPCameraDataType`
/// array rather than failing, so enumeration degrades quietly.
#[must_use]
pub fn parse_spcamera_json(json: &str) -> Vec<CameraDescriptor> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(entries) = value
        .get("SPCameraDataType")
        .and_then(|entry| entry.as_array())
    else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let name = entry
                .get("_name")
                .and_then(|value| value.as_str())?
                .to_owned();
            let model_id = entry
                .get("spcamera_model-id")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned);
            let unique_id = entry
                .get("spcamera_unique-id")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned);
            Some(CameraDescriptor {
                // The localized device name is the handle `ffmpeg -f
                // avfoundation -i "<name>"` matches on.
                id: name.clone(),
                name,
                model_id,
                unique_id,
            })
        })
        .collect()
}

/// Parse ffmpeg's `DirectShow` device-list diagnostics.
///
/// `DirectShow` writes its inventory to stderr and exits unsuccessfully because
/// `dummy` is not a real input. A video entry is followed by an optional
/// alternative `PnP` name. Tendril exposes that alternative name as the stable id
/// when available while retaining the friendly camera name for display and
/// name-based selection.
#[must_use]
pub fn parse_dshow_devices(stderr: &str) -> Vec<CameraDescriptor> {
    let mut cameras: Vec<CameraDescriptor> = Vec::new();
    let mut pending_video_alternative = false;
    for line in stderr.lines() {
        if line.contains("(video)") {
            pending_video_alternative = false;
            if let Some(name) = first_quoted_value(line) {
                cameras.push(CameraDescriptor {
                    id: name.clone(),
                    name,
                    model_id: Some("DirectShow".to_owned()),
                    unique_id: None,
                });
                pending_video_alternative = true;
            }
        } else if line.contains("(audio)") {
            // The following alternative name belongs to the audio source, not
            // to the most recently listed camera.
            pending_video_alternative = false;
        } else if pending_video_alternative && line.contains("Alternative name") {
            if let Some(alternative_name) = first_quoted_value(line)
                && let Some(camera) = cameras.last_mut()
            {
                camera.id.clone_from(&alternative_name);
                camera.unique_id = Some(alternative_name);
            }
            pending_video_alternative = false;
        }
    }
    cameras
}

fn first_quoted_value(line: &str) -> Option<String> {
    let start = line.find('"')? + 1;
    let end = line[start..].find('"')?;
    Some(line[start..start + end].to_owned())
}

/// Enumerate Linux V4L2 devices from sysfs. This helper accepts roots so the
/// filesystem mapping can be tested without touching the host's `/sys` or
/// `/dev` trees.
#[must_use]
pub fn enumerate_v4l2_cameras(sys_class_root: &Path, dev_root: &Path) -> Vec<CameraDescriptor> {
    let Ok(entries) = std::fs::read_dir(sys_class_root) else {
        return Vec::new();
    };
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(std::fs::DirEntry::file_name);

    entries
        .into_iter()
        .filter_map(|entry| {
            let node = entry.file_name();
            let node = node.to_str()?;
            if !node.starts_with("video") {
                return None;
            }
            let device_path = dev_root.join(node);
            if !device_path.exists() {
                return None;
            }
            let name = std::fs::read_to_string(entry.path().join("name"))
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| node.to_owned());
            let unique_id = entry
                .path()
                .join("device")
                .canonicalize()
                .ok()
                .map(|path| path.display().to_string());
            Some(CameraDescriptor {
                id: device_path.display().to_string(),
                name,
                model_id: Some("V4L2".to_owned()),
                unique_id,
            })
        })
        .collect()
}

/// Enumerate video capture devices for the host platform. Returns an empty list
/// when the platform inventory is unavailable or the required Windows ffmpeg
/// backend is not installed. Capture itself reports a structured backend error.
#[must_use]
pub fn enumerate_cameras() -> Vec<CameraDescriptor> {
    enumerate_platform_cameras()
}

#[cfg(target_os = "macos")]
fn enumerate_platform_cameras() -> Vec<CameraDescriptor> {
    let output = std::process::Command::new("system_profiler")
        .args(["SPCameraDataType", "-json"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            parse_spcamera_json(&String::from_utf8_lossy(&out.stdout))
        }
        _ => Vec::new(),
    }
}

#[cfg(target_os = "linux")]
fn enumerate_platform_cameras() -> Vec<CameraDescriptor> {
    enumerate_v4l2_cameras(Path::new("/sys/class/video4linux"), Path::new("/dev"))
}

#[cfg(target_os = "windows")]
fn enumerate_platform_cameras() -> Vec<CameraDescriptor> {
    let output = std::process::Command::new(FFMPEG_BIN)
        .args([
            "-hide_banner",
            "-nostdin",
            "-list_devices",
            "true",
            "-f",
            "dshow",
            "-i",
            "dummy",
        ])
        .output();
    output.map_or_else(
        |_| Vec::new(),
        |output| parse_dshow_devices(&String::from_utf8_lossy(&output.stderr)),
    )
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn enumerate_platform_cameras() -> Vec<CameraDescriptor> {
    Vec::new()
}

fn common_capture_args(output: &Path) -> Vec<String> {
    vec![
        "-frames:v".to_owned(),
        "1".to_owned(),
        "-an".to_owned(),
        "-y".to_owned(),
        output.display().to_string(),
    ]
}

/// Build the ffmpeg argument vector for a single-frame `AVFoundation` grab.
///
/// `AVFoundation` otherwise defaults to the NTSC rate 29.970030 fps. Many USB
/// cameras, including the Logitech C930e, advertise 30.000030 but not 29.97 and
/// reject that default. Explicitly requesting 30 lets `AVFoundation` negotiate
/// the camera's supported 30 fps mode.
#[must_use]
pub fn avfoundation_capture_args(device: &str, output: &Path, framerate: &str) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".to_owned(),
        "-nostdin".to_owned(),
        "-loglevel".to_owned(),
        "error".to_owned(),
        "-f".to_owned(),
        "avfoundation".to_owned(),
        "-framerate".to_owned(),
        framerate.to_owned(),
        "-i".to_owned(),
        device.to_owned(),
    ];
    args.extend(common_capture_args(output));
    args
}

/// Build the ffmpeg argument vector for a single-frame Linux V4L2 grab.
#[must_use]
pub fn v4l2_capture_args(device: &str, output: &Path) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".to_owned(),
        "-nostdin".to_owned(),
        "-loglevel".to_owned(),
        "error".to_owned(),
        "-f".to_owned(),
        "v4l2".to_owned(),
        "-i".to_owned(),
        device.to_owned(),
    ];
    args.extend(common_capture_args(output));
    args
}

/// Build the ffmpeg argument vector for a single-frame Windows `DirectShow`
/// grab. `device` may be either a friendly name or the alternative `PnP` id
/// returned by `parse_dshow_devices`.
#[must_use]
pub fn dshow_capture_args(device: &str, output: &Path) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".to_owned(),
        "-nostdin".to_owned(),
        "-loglevel".to_owned(),
        "error".to_owned(),
        "-f".to_owned(),
        "dshow".to_owned(),
        "-i".to_owned(),
        format!("video={device}"),
    ];
    args.extend(common_capture_args(output));
    args
}

/// Extract the first advertised `AVFoundation` frame rate from ffmpeg's mode
/// diagnostics. Used as a retry when a camera does not support the preferred
/// 30 fps mode.
#[must_use]
pub fn parse_avfoundation_supported_framerate(stderr: &str) -> Option<String> {
    stderr.lines().find_map(|line| {
        let marker = line.find("@[")? + 2;
        let token = line[marker..]
            .split(|character: char| character.is_whitespace() || character == ']')
            .next()?;
        token
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|_| token.to_owned())
    })
}

/// Read width/height from a PNG's IHDR header without decoding pixels. Returns
/// `None` for input that is not a PNG. Dependency-free so the camera capture
/// path stays light.
#[must_use]
pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    // signature(8) + IHDR length(4) + "IHDR"(4) + width(4) + height(4) = 24
    if bytes.len() < 24 || &bytes[0..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((width, height))
}

/// Capture a single still frame from a camera, returning PNG bytes.
///
/// The selector should normally be an id returned by `tendril list`. Linux
/// additionally accepts an unambiguous friendly device name so the same
/// operator-shaped command works across hosts.
pub fn capture_camera_frame(device: &str) -> Result<Vec<u8>, TendrilError> {
    capture_camera_frame_impl(device)
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn capture_camera_frame_impl(device: &str) -> Result<Vec<u8>, TendrilError> {
    let output_path = unique_capture_path();
    #[cfg(target_os = "macos")]
    let resolved_device = device.to_owned();
    #[cfg(target_os = "linux")]
    let resolved_device = resolve_linux_camera_device(device)?;
    #[cfg(target_os = "windows")]
    let resolved_device = resolve_windows_camera_device(device);

    #[cfg(target_os = "macos")]
    let (backend, args) = (
        "AVFoundation",
        avfoundation_capture_args(
            &resolved_device,
            &output_path,
            AVFOUNDATION_DEFAULT_FRAMERATE,
        ),
    );
    #[cfg(target_os = "linux")]
    let (backend, args) = ("V4L2", v4l2_capture_args(&resolved_device, &output_path));
    #[cfg(target_os = "windows")]
    let (backend, args) = (
        "DirectShow",
        dshow_capture_args(&resolved_device, &output_path),
    );

    let mut completed = run_ffmpeg(&args, backend, device)?;

    // A minority of AVFoundation devices do not offer 30 fps. ffmpeg prints
    // their supported modes with the failure, so retry once at the first
    // advertised rate rather than requiring users to reverse-engineer it.
    #[cfg(target_os = "macos")]
    if !completed.status.success()
        && let Some(framerate) =
            parse_avfoundation_supported_framerate(&String::from_utf8_lossy(&completed.stderr))
        && framerate != AVFOUNDATION_DEFAULT_FRAMERATE
    {
        let retry_args = avfoundation_capture_args(&resolved_device, &output_path, &framerate);
        completed = run_ffmpeg(&retry_args, backend, device)?;
    }

    if !completed.status.success() {
        let _ = std::fs::remove_file(&output_path);
        let stderr = String::from_utf8_lossy(&completed.stderr);
        return Err(TendrilError::execution_failure(
            "camera_capture_failed",
            format!(
                "ffmpeg {backend} failed to capture from camera `{device}`: {}",
                stderr.trim()
            ),
            None,
        )
        .with_detail_entry("backend", serde_json::json!(backend))
        .with_detail_entry("device", serde_json::json!(device)));
    }

    let bytes = std::fs::read(&output_path).map_err(|error| {
        TendrilError::execution_failure(
            "camera_capture_failed",
            format!("ffmpeg reported success but the camera frame could not be read: {error}"),
            None,
        )
        .with_detail_entry("backend", serde_json::json!(backend))
        .with_detail_entry("device", serde_json::json!(device))
    });
    let _ = std::fs::remove_file(&output_path);
    bytes
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn run_ffmpeg(
    args: &[String],
    backend: &'static str,
    device: &str,
) -> Result<std::process::Output, TendrilError> {
    use std::io::ErrorKind;

    std::process::Command::new(FFMPEG_BIN)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                TendrilError::unsupported_capability(
                    "camera_capture_backend_unavailable",
                    ffmpeg_install_hint(),
                    Some(serde_json::json!({
                        "backend": backend,
                        "device": device,
                        "program": FFMPEG_BIN,
                    })),
                )
            } else {
                TendrilError::execution_failure(
                    "camera_capture_failed",
                    format!("failed to launch ffmpeg {backend} camera capture: {error}"),
                    None,
                )
                .with_detail_entry("backend", serde_json::json!(backend))
                .with_detail_entry("device", serde_json::json!(device))
            }
        })
}

#[cfg(target_os = "macos")]
fn ffmpeg_install_hint() -> &'static str {
    "camera capture requires `ffmpeg` on PATH; install it with `brew install ffmpeg` and retry"
}

#[cfg(target_os = "linux")]
fn ffmpeg_install_hint() -> &'static str {
    "camera capture requires `ffmpeg` with V4L2 support on PATH; install it with your system package manager and retry"
}

#[cfg(target_os = "windows")]
fn ffmpeg_install_hint() -> &'static str {
    "camera capture requires `ffmpeg.exe` with DirectShow support on PATH; install FFmpeg (for example `winget install Gyan.FFmpeg`) and retry"
}

#[cfg(target_os = "linux")]
fn resolve_linux_camera_device(device: &str) -> Result<String, TendrilError> {
    if Path::new(device).exists() {
        return Ok(device.to_owned());
    }

    let cameras = enumerate_platform_cameras();
    let matches = cameras
        .iter()
        .filter(|camera| camera.id == device || camera.name == device)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [camera] => Ok(camera.id.clone()),
        [] => Err(TendrilError::target_not_found("camera", device)),
        _ => Err(TendrilError::validation(format!(
            "camera name `{device}` is ambiguous; use one of the device ids from `tendril list`: {}",
            matches
                .iter()
                .map(|camera| camera.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .with_code("ambiguous_camera_selector")
        .with_field("camera")),
    }
}

#[cfg(target_os = "windows")]
fn resolve_windows_camera_device(device: &str) -> String {
    let cameras = enumerate_platform_cameras();
    cameras
        .iter()
        .find(|camera| camera.id == device)
        .or_else(|| camera_by_unique_name(&cameras, device))
        .map_or_else(|| device.to_owned(), |camera| camera.id.clone())
}

#[cfg(target_os = "windows")]
fn camera_by_unique_name<'a>(
    cameras: &'a [CameraDescriptor],
    name: &str,
) -> Option<&'a CameraDescriptor> {
    let mut matching = cameras.iter().filter(|camera| camera.name == name);
    let camera = matching.next()?;
    matching.next().is_none().then_some(camera)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn capture_camera_frame_impl(device: &str) -> Result<Vec<u8>, TendrilError> {
    Err(TendrilError::unsupported_capability(
        "camera_capture_unsupported_platform",
        "camera capture is supported on macOS, Linux, and Windows",
        Some(serde_json::json!({ "device": device })),
    ))
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn unique_capture_path() -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let pid = std::process::id();
    std::env::temp_dir().join(format!("tendril-camera-{pid}-{nanos}.png"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "SPCameraDataType" : [
        {
          "_name" : "MacBook Pro Camera",
          "spcamera_model-id" : "MacBook Pro Camera",
          "spcamera_unique-id" : "6C707041-05AC-0010-0006-000000000001"
        },
        {
          "_name" : "OBS Virtual Camera",
          "spcamera_model-id" : "OBS Camera Extension",
          "spcamera_unique-id" : "7626645E-4425-469E-9D8B-97E0FA59AC75"
        },
        {
          "_name" : "Harry’s iPhone (2) Camera",
          "spcamera_model-id" : "iPhone14,4",
          "spcamera_unique-id" : "E46361EB-14D9-4A38-A639-3C2F00000001"
        }
      ]
    }"#;

    #[test]
    fn parses_spcamera_json_into_descriptors() {
        let cameras = parse_spcamera_json(SAMPLE);
        assert_eq!(cameras.len(), 3);
        assert_eq!(cameras[0].id, "MacBook Pro Camera");
        assert_eq!(cameras[0].name, "MacBook Pro Camera");
        assert_eq!(
            cameras[0].unique_id.as_deref(),
            Some("6C707041-05AC-0010-0006-000000000001")
        );
        assert_eq!(cameras[1].name, "OBS Virtual Camera");
        assert_eq!(cameras[1].model_id.as_deref(), Some("OBS Camera Extension"));
        assert_eq!(cameras[2].name, "Harry’s iPhone (2) Camera");
        assert_eq!(cameras[2].id, "Harry’s iPhone (2) Camera");
    }

    #[test]
    fn parse_spcamera_json_rejects_garbage() {
        assert!(parse_spcamera_json("not json at all").is_empty());
    }

    #[test]
    fn parse_spcamera_json_handles_missing_key() {
        assert!(parse_spcamera_json("{}").is_empty());
        assert!(parse_spcamera_json(r#"{"SPCameraDataType": []}"#).is_empty());
    }

    #[test]
    fn parse_spcamera_json_skips_entries_without_name() {
        let json = r#"{"SPCameraDataType": [{"spcamera_unique-id": "x"}, {"_name": "Cam"}]}"#;
        let cameras = parse_spcamera_json(json);
        assert_eq!(cameras.len(), 1);
        assert_eq!(cameras[0].name, "Cam");
        assert!(cameras[0].unique_id.is_none());
    }

    #[test]
    fn parses_directshow_video_devices_and_alternative_ids() {
        let stderr = r#"
[dshow @ 000001] "Logitech Webcam C930e" (video)
[dshow @ 000001]   Alternative name "@device_pnp_\\?\\usb#vid_046d&pid_0843"
[dshow @ 000001] "Microphone (C930e)" (audio)
[dshow @ 000001]   Alternative name "@device_cm_\\?\\audio#c930e"
[dshow @ 000001] "OBS Virtual Camera" (video)
"#;
        let cameras = parse_dshow_devices(stderr);
        assert_eq!(cameras.len(), 2);
        assert_eq!(cameras[0].name, "Logitech Webcam C930e");
        assert_eq!(cameras[0].id, r"@device_pnp_\\?\\usb#vid_046d&pid_0843");
        assert_eq!(cameras[0].unique_id, Some(cameras[0].id.clone()));
        assert_eq!(cameras[1].id, "OBS Virtual Camera");
    }

    #[test]
    fn enumerates_v4l2_devices_from_sysfs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let sys = temp.path().join("sys");
        let dev = temp.path().join("dev");
        std::fs::create_dir_all(sys.join("video0")).expect("sys camera");
        std::fs::create_dir_all(&dev).expect("dev root");
        std::fs::write(sys.join("video0/name"), "Logitech Webcam C930e\n").expect("name");
        std::fs::write(dev.join("video0"), []).expect("device node fixture");
        std::fs::create_dir_all(sys.join("not-a-camera")).expect("other sys entry");

        let cameras = enumerate_v4l2_cameras(&sys, &dev);
        assert_eq!(cameras.len(), 1);
        assert_eq!(cameras[0].name, "Logitech Webcam C930e");
        assert_eq!(cameras[0].id, dev.join("video0").display().to_string());
        assert_eq!(cameras[0].model_id.as_deref(), Some("V4L2"));
    }

    #[test]
    fn avfoundation_capture_args_select_supported_rate_and_single_frame() {
        let path = Path::new("/tmp/frame.png");
        let args = avfoundation_capture_args("MacBook Pro Camera", path, "30");
        let device_pos = args.iter().position(|arg| arg == "-i").expect("-i present");
        assert_eq!(args[device_pos + 1], "MacBook Pro Camera");
        assert_eq!(argument_value(&args, "-f"), Some("avfoundation"));
        assert_eq!(argument_value(&args, "-framerate"), Some("30"));
        assert_eq!(argument_value(&args, "-frames:v"), Some("1"));
        assert_eq!(args.last().map(String::as_str), Some("/tmp/frame.png"));
        assert!(args.iter().any(|arg| arg == "-y"));
    }

    #[test]
    fn v4l2_capture_args_use_device_node() {
        let args = v4l2_capture_args("/dev/video3", Path::new("/tmp/frame.png"));
        assert_eq!(argument_value(&args, "-f"), Some("v4l2"));
        assert_eq!(argument_value(&args, "-i"), Some("/dev/video3"));
        assert_eq!(argument_value(&args, "-frames:v"), Some("1"));
    }

    #[test]
    fn dshow_capture_args_prefix_video_selector() {
        let args = dshow_capture_args("Logitech Webcam C930e", Path::new("C:/Temp/frame.png"));
        assert_eq!(argument_value(&args, "-f"), Some("dshow"));
        assert_eq!(
            argument_value(&args, "-i"),
            Some("video=Logitech Webcam C930e")
        );
        assert_eq!(argument_value(&args, "-frames:v"), Some("1"));
    }

    #[test]
    fn parses_avfoundation_supported_rate() {
        let stderr = "Selected framerate (29.970030) is not supported\n  640x480@[30.000030 30.000030]fps\n  640x480@[24.000038 24.000038]fps";
        assert_eq!(
            parse_avfoundation_supported_framerate(stderr).as_deref(),
            Some("30.000030")
        );
        assert!(parse_avfoundation_supported_framerate("no modes").is_none());
    }

    #[test]
    fn png_dimensions_reads_ihdr() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&320u32.to_be_bytes());
        bytes.extend_from_slice(&240u32.to_be_bytes());
        assert_eq!(png_dimensions(&bytes), Some((320, 240)));
    }

    #[test]
    fn png_dimensions_rejects_non_png() {
        assert!(png_dimensions(b"not a png").is_none());
        assert!(png_dimensions(&[]).is_none());
    }

    fn argument_value<'a>(args: &'a [String], argument: &str) -> Option<&'a str> {
        args.windows(2)
            .find(|window| window[0] == argument)
            .map(|window| window[1].as_str())
    }
}
