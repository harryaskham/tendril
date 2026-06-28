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
}
