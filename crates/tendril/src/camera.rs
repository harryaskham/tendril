//! Camera / non-display video-device discovery (bd-aed538).
//!
//! `tendril list` surfaces video capture devices (webcams, Continuity Camera,
//! virtual cameras) alongside windows and displays so an agent can discover
//! them the same way. macOS enumerates them via the built-in
//! `system_profiler SPCameraDataType -json`, which needs no extra dependency
//! and reports only real cameras (it excludes the screen-capture pseudo-devices
//! that `ffmpeg -f avfoundation` lists). Linux (V4L2) and Windows (Media
//! Foundation) enumeration, plus the `tendril capture --camera` single-frame
//! grab, are tracked as follow-ups.

use crate::error::TendrilError;
use crate::model::CameraDescriptor;

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
                // The localized device name is the handle `ffmpeg -f avfoundation
                // -i "<name>"` matches on, so it doubles as the stable
                // `--camera` id the upcoming capture path will consume.
                id: name.clone(),
                name,
                model_id,
                unique_id,
            })
        })
        .collect()
}

/// Enumerate video capture devices for the host platform. Returns an empty list
/// on platforms without an enumeration backend yet (Linux/Windows) or when the
/// backend command is unavailable or fails.
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

#[cfg(not(target_os = "macos"))]
fn enumerate_platform_cameras() -> Vec<CameraDescriptor> {
    Vec::new()
}

/// Build the `ffmpeg` argument vector for a single-frame `AVFoundation` grab from
/// the named video device into `output` (a `.png` path). The `device` string is
/// the same id `tendril list` surfaces (the localized device name), which
/// `ffmpeg -f avfoundation -i` matches directly; passing it as one argv element
/// means names containing spaces or apostrophes need no shell quoting.
#[must_use]
pub fn avfoundation_capture_args(device: &str, output: &std::path::Path) -> Vec<String> {
    vec![
        "-hide_banner".to_owned(),
        "-nostdin".to_owned(),
        "-loglevel".to_owned(),
        "error".to_owned(),
        "-f".to_owned(),
        "avfoundation".to_owned(),
        "-i".to_owned(),
        device.to_owned(),
        "-frames:v".to_owned(),
        "1".to_owned(),
        "-y".to_owned(),
        output.display().to_string(),
    ]
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

/// Capture a single still frame from the named video device, returning PNG
/// bytes. macOS-only for now (`AVFoundation` via `ffmpeg`); other platforms
/// return an unsupported-capability error. The first live grab activates the
/// camera (its indicator light), so this is never exercised in tests.
pub fn capture_camera_frame(device: &str) -> Result<Vec<u8>, TendrilError> {
    capture_camera_frame_impl(device)
}

#[cfg(target_os = "macos")]
fn capture_camera_frame_impl(device: &str) -> Result<Vec<u8>, TendrilError> {
    use std::io::ErrorKind;

    let output_path = unique_capture_path();
    let args = avfoundation_capture_args(device, &output_path);
    let result = std::process::Command::new("ffmpeg")
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output();
    let completed = match result {
        Ok(completed) => completed,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(TendrilError::unsupported_capability(
                "camera_capture_backend_unavailable",
                "camera capture requires `ffmpeg` on PATH (macOS AVFoundation backend); install it (e.g. `brew install ffmpeg`) and retry",
                Some(serde_json::json!({ "backend": "ffmpeg", "device": device })),
            ));
        }
        Err(error) => {
            return Err(TendrilError::execution_failure(
                "camera_capture_failed",
                format!("failed to launch ffmpeg for camera capture: {error}"),
                None,
            ));
        }
    };
    if !completed.status.success() {
        let _ = std::fs::remove_file(&output_path);
        let stderr = String::from_utf8_lossy(&completed.stderr);
        return Err(TendrilError::execution_failure(
            "camera_capture_failed",
            format!(
                "ffmpeg failed to capture from camera `{device}`: {}",
                stderr.trim()
            ),
            None,
        ));
    }
    let bytes = std::fs::read(&output_path).map_err(|error| {
        TendrilError::execution_failure(
            "camera_capture_failed",
            format!("ffmpeg reported success but the camera frame could not be read: {error}"),
            None,
        )
    });
    let _ = std::fs::remove_file(&output_path);
    bytes
}

#[cfg(not(target_os = "macos"))]
fn capture_camera_frame_impl(device: &str) -> Result<Vec<u8>, TendrilError> {
    Err(TendrilError::unsupported_capability(
        "camera_capture_unsupported_platform",
        "camera capture is currently only implemented on macOS (AVFoundation via ffmpeg)",
        Some(serde_json::json!({ "device": device })),
    ))
}

#[cfg(target_os = "macos")]
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
        // The capture handle (id) round-trips the localized name verbatim,
        // including non-ASCII apostrophes, so it can be passed to `--camera`.
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
    fn avfoundation_capture_args_select_device_and_single_frame_png() {
        let path = std::path::Path::new("/tmp/frame.png");
        let args = avfoundation_capture_args("MacBook Pro Camera", path);
        // Device name is a single argv element (no shell quoting needed).
        let device_pos = args.iter().position(|a| a == "-i").expect("-i present");
        assert_eq!(args[device_pos + 1], "MacBook Pro Camera");
        assert_eq!(
            args.windows(2)
                .find(|w| w[0] == "-f")
                .map(|w| w[1].as_str()),
            Some("avfoundation")
        );
        assert_eq!(
            args.windows(2)
                .find(|w| w[0] == "-frames:v")
                .map(|w| w[1].as_str()),
            Some("1")
        );
        assert_eq!(args.last().map(String::as_str), Some("/tmp/frame.png"));
        assert!(args.iter().any(|a| a == "-y"));
    }

    #[test]
    fn png_dimensions_reads_ihdr() {
        // Minimal PNG signature + IHDR length + "IHDR" + 320x240.
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
}
