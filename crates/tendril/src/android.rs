use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};

use crate::config::ImageFormat;
use crate::error::TendrilError;
use crate::model::{
    Bounds, CapabilitySet, CaptureOutput, CoordinateTransform, ElementDescriptor,
    ElementListOutput, FocusSnapshot, InputAction, ListOutput, MouseButton, RunInputPayload,
    RunOutput, ScaleFactor, TargetDescriptor, TargetKind, TargetSelector,
};
use crate::platform::{AdapterInfo, DesktopSession, PlatformKind};

const ANDROID_XML_PATH: &str = "/sdcard/tendril-window.xml";
const DEFAULT_SWIPE_MS: u64 = 450;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidDeviceSummary {
    pub serial: String,
    pub state: String,
    pub model: Option<String>,
    pub wm_size: Option<String>,
    pub wm_density: Option<String>,
    pub focused_window: Option<String>,
    pub artifact_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct AndroidDevice {
    serial: String,
    artifact_dir: PathBuf,
}

impl AndroidDevice {
    pub fn resolve(selection: Option<&str>) -> Result<Self, TendrilError> {
        let requested = selection
            .map(str::to_owned)
            .or_else(|| std::env::var("TENDRIL_ANDROID_SERIAL").ok())
            .unwrap_or_else(|| "auto".to_owned());
        let serial = if requested == "auto" {
            resolve_auto_serial()?
        } else {
            requested
        };
        let artifact_dir = android_artifact_dir(&serial);
        std::fs::create_dir_all(&artifact_dir).map_err(|error| {
            TendrilError::execution_failure(
                "android_artifact_dir_failed",
                format!(
                    "failed to create Android artifact dir `{}`: {error}",
                    artifact_dir.display()
                ),
                None,
            )
        })?;
        let device = Self {
            serial,
            artifact_dir,
        };
        let state = device.adb_text(["get-state"])?;
        if state.trim() != "device" {
            return Err(TendrilError::execution_failure(
                "android_device_unavailable",
                format!(
                    "Android device `{}` is not ready; adb get-state returned `{}`",
                    device.serial,
                    state.trim()
                ),
                None,
            ));
        }
        Ok(device)
    }

    #[must_use]
    pub fn serial(&self) -> &str {
        &self.serial
    }

    #[must_use]
    pub fn summary(&self) -> AndroidDeviceSummary {
        AndroidDeviceSummary {
            serial: self.serial.clone(),
            state: self
                .adb_text(["get-state"])
                .unwrap_or_else(|_| "unknown".to_owned())
                .trim()
                .to_owned(),
            model: self
                .adb_text(["shell", "getprop", "ro.product.model"])
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            wm_size: self
                .adb_text(["shell", "wm", "size"])
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            wm_density: self
                .adb_text(["shell", "wm", "density"])
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            focused_window: self.focused_window().ok(),
            artifact_dir: self.artifact_dir.clone(),
        }
    }

    #[must_use]
    pub fn list_output(&self) -> ListOutput {
        let bounds = self.screen_bounds().unwrap_or(Bounds {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        });
        ListOutput {
            adapter: android_adapter_info(),
            permissions: Vec::new(),
            targets: vec![TargetDescriptor {
                id: android_target_id(&self.serial),
                kind: TargetKind::Display,
                name: format!("Android device {}", self.serial),
                title: self.summary().focused_window,
                bounds,
                scale_factor: ScaleFactor::identity(),
                capabilities: CapabilitySet {
                    capture: true,
                    input: true,
                    audio: false,
                },
                diagnostics: Vec::new(),
                app_name: self.summary().model,
                process_id: None,
            }],
        }
    }

    pub fn list_elements_output(
        &self,
        include_offscreen: bool,
    ) -> Result<ElementListOutput, TendrilError> {
        let nodes = self.dump_nodes()?;
        let target = TargetSelector::Display {
            id: android_target_id(&self.serial),
        };
        let elements = nodes
            .into_iter()
            .enumerate()
            .filter(|(_, node)| include_offscreen || node.visible())
            .map(|(index, node)| node.to_element(index, &target))
            .collect();
        Ok(ElementListOutput {
            adapter: android_adapter_info(),
            target: Some(target),
            elements,
            notes: vec![format!(
                "Android UIAutomator dump read from `{ANDROID_XML_PATH}` on `{}`; artifacts in `{}`",
                self.serial,
                self.artifact_dir.display()
            )],
        })
    }

    pub fn capture_output(&self, compression: u8) -> Result<CaptureOutput, TendrilError> {
        let bytes = self.adb_bytes(["exec-out", "screencap", "-p"])?;
        let _ = std::fs::write(self.artifact_dir.join("screenshot.png"), &bytes);
        if let Ok(focus) = self.focused_window() {
            let _ = std::fs::write(self.artifact_dir.join("window.txt"), focus);
        }
        let bounds = self.screen_bounds().unwrap_or(Bounds {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        });
        Ok(CaptureOutput {
            adapter: android_adapter_info(),
            target: TargetSelector::Display {
                id: android_target_id(&self.serial),
            },
            original_bounds: bounds.clone(),
            output_bounds: bounds,
            source_to_output: CoordinateTransform::identity(),
            output_to_source: CoordinateTransform::identity(),
            resized: false,
            format: ImageFormat::Png,
            compression,
            media_type: "image/png".to_owned(),
            image_base64: BASE64.encode(bytes),
            captured_at: current_timestamp_string(),
        })
    }

    pub fn execute_payload(&self, payload: &RunInputPayload) -> Result<RunOutput, TendrilError> {
        let (text, actions) = match payload {
            RunInputPayload::Text { text } => (Some(text.as_str()), Vec::new()),
            RunInputPayload::Dsl { sequence: _ } => (None, Vec::new()),
            RunInputPayload::Actions { actions } => (None, actions.clone()),
        };

        let mut action_count = 0;
        let mut notes = Vec::new();
        if let Some(text) = text {
            self.input_text(text)?;
            action_count += 1;
            notes.push("sent Android text input via `adb shell input text`".to_owned());
        }

        let mut cached_nodes: Option<Vec<AndroidNode>> = None;
        for (index, action) in actions.iter().enumerate() {
            self.execute_action(action, index, &mut cached_nodes)?;
            action_count += 1;
        }

        if !actions.is_empty() {
            notes.push(format!(
                "dispatched {action_count} Android action(s) through adb input/uiautomator on `{}`; artifacts in `{}`",
                self.serial,
                self.artifact_dir.display()
            ));
        }

        Ok(RunOutput {
            adapter: android_adapter_info(),
            target: TargetSelector::Display {
                id: android_target_id(&self.serial),
            },
            focus_required: false,
            focus_transferred: false,
            action_count,
            focused_target: self.focused_window().ok(),
            previous_focus: Some(FocusSnapshot {
                id: self.serial.clone(),
                kind: "android_device".to_owned(),
                name: self.summary().model,
            }),
            focus_restored: false,
            pointer_restored: false,
            restore_error: None,
            notes,
            execution_lock: None,
        })
    }

    fn execute_action(
        &self,
        action: &InputAction,
        action_index: usize,
        cached_nodes: &mut Option<Vec<AndroidNode>>,
    ) -> Result<(), TendrilError> {
        match action {
            InputAction::Click {
                button: MouseButton::Left,
                x,
                y,
            } => self.tap(*x, *y),
            InputAction::Click { button, .. } => Err(TendrilError::unsupported_capability(
                "android_mouse_button_unsupported",
                format!("Android adb input supports primary taps, not {button:?} clicks"),
                Some(serde_json::json!({ "action_index": action_index })),
            )),
            InputAction::DoubleClick { x, y } => {
                self.tap(*x, *y)?;
                thread::sleep(Duration::from_millis(80));
                self.tap(*x, *y)
            }
            InputAction::PointerMove { .. } => Ok(()),
            InputAction::Drag { x0, y0, x1, y1 } => {
                self.swipe(*x0, *y0, *x1, *y1, DEFAULT_SWIPE_MS)
            }
            InputAction::Scroll { x, y, dy } => {
                let distance = dy.saturating_mul(80).clamp(-1200, 1200);
                self.swipe(*x, y.saturating_add(distance), *x, *y, DEFAULT_SWIPE_MS)
            }
            InputAction::Send { text } => self.input_text(text),
            InputAction::KeyTap { key } => self.key_event(key),
            InputAction::Wait { duration_ms } => {
                thread::sleep(Duration::from_millis(*duration_ms));
                Ok(())
            }
            InputAction::ElementClick { id } => {
                if let Some(package) = id
                    .strip_prefix("launch:")
                    .or_else(|| id.strip_prefix("package:"))
                {
                    return self.launch_package(package);
                }
                if cached_nodes.is_none() {
                    *cached_nodes = Some(self.dump_nodes()?);
                }
                let nodes = cached_nodes.as_ref().expect("nodes just populated");
                let node = find_node_for_element_id(nodes, id).ok_or_else(|| {
                    TendrilError::target_not_found("android_element", id.clone())
                        .with_detail_entry("action_index", serde_json::json!(action_index))
                })?;
                let (x, y) = node.center().ok_or_else(|| {
                    TendrilError::execution_failure(
                        "android_element_missing_bounds",
                        format!("Android element `{id}` did not expose usable bounds"),
                        Some(action_index),
                    )
                })?;
                self.tap(x, y)
            }
            InputAction::Hold { .. } | InputAction::Release { .. } => {
                Err(TendrilError::unsupported_capability(
                    "android_modifier_hold_unsupported",
                    "Android adb input does not support holding modifier keys in the MVP backend",
                    Some(serde_json::json!({ "action_index": action_index })),
                ))
            }
        }
    }

    fn dump_nodes(&self) -> Result<Vec<AndroidNode>, TendrilError> {
        self.adb_text(["shell", "uiautomator", "dump", ANDROID_XML_PATH])?;
        let xml = self.adb_text(["exec-out", "cat", ANDROID_XML_PATH])?;
        let _ = std::fs::write(self.artifact_dir.join("ui.xml"), &xml);
        Ok(parse_uiautomator_nodes(&xml))
    }

    fn tap(&self, x: i32, y: i32) -> Result<(), TendrilError> {
        self.adb_text(["shell", "input", "tap", &x.to_string(), &y.to_string()])?;
        Ok(())
    }

    fn swipe(
        &self,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        duration_ms: u64,
    ) -> Result<(), TendrilError> {
        self.adb_text([
            "shell",
            "input",
            "swipe",
            &x0.to_string(),
            &y0.to_string(),
            &x1.to_string(),
            &y1.to_string(),
            &duration_ms.to_string(),
        ])?;
        Ok(())
    }

    fn input_text(&self, text: &str) -> Result<(), TendrilError> {
        self.adb_text(["shell", "input", "text", &escape_android_input_text(text)])?;
        Ok(())
    }

    fn launch_package(&self, package: &str) -> Result<(), TendrilError> {
        if package.trim().is_empty() || package.contains(char::is_whitespace) {
            return Err(TendrilError::validation(
                "Android launch package must be a non-empty package name without whitespace",
            )
            .with_code("invalid_run_input")
            .with_field("package"));
        }
        self.adb_text([
            "shell",
            "monkey",
            "-p",
            package,
            "-c",
            "android.intent.category.LAUNCHER",
            "1",
        ])?;
        Ok(())
    }

    fn key_event(&self, key: &str) -> Result<(), TendrilError> {
        let event = android_key_event(key).ok_or_else(|| {
            TendrilError::validation(format!(
                "unsupported Android key `{key}`; use BACK, HOME, ENTER, WAKEUP, MENU, TAB, SPACE, ESCAPE, or a numeric keyevent code"
            ))
            .with_code("invalid_run_input")
            .with_field("key")
        })?;
        self.adb_text(["shell", "input", "keyevent", &event])?;
        Ok(())
    }

    fn screen_bounds(&self) -> Result<Bounds, TendrilError> {
        let size = self.adb_text(["shell", "wm", "size"])?;
        parse_wm_size(&size).ok_or_else(|| {
            TendrilError::execution_failure(
                "android_wm_size_unavailable",
                format!("could not parse Android wm size output `{}`", size.trim()),
                None,
            )
        })
    }

    fn focused_window(&self) -> Result<String, TendrilError> {
        let windows = self.adb_text(["shell", "dumpsys", "window"])?;
        windows
            .lines()
            .find(|line| line.contains("mCurrentFocus") || line.contains("mFocusedApp"))
            .map(|line| line.trim().to_owned())
            .ok_or_else(|| {
                TendrilError::execution_failure(
                    "android_focus_unavailable",
                    "could not find focused Android window/activity in dumpsys window",
                    None,
                )
            })
    }

    fn adb_text<const N: usize>(&self, args: [&str; N]) -> Result<String, TendrilError> {
        let bytes = self.adb_bytes(args)?;
        String::from_utf8(bytes).map_err(|error| {
            TendrilError::execution_failure(
                "android_adb_utf8_error",
                format!("adb output was not valid UTF-8: {error}"),
                None,
            )
        })
    }

    fn adb_bytes<const N: usize>(&self, args: [&str; N]) -> Result<Vec<u8>, TendrilError> {
        let mut command = Command::new("adb");
        command.arg("-s").arg(&self.serial).args(args);
        self.append_command_log(&args);
        let output = command.output().map_err(|error| {
            TendrilError::execution_failure(
                "android_adb_missing",
                format!("failed to execute adb: {error}"),
                None,
            )
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            return Err(TendrilError::execution_failure(
                "android_adb_failed",
                format!(
                    "adb -s {} {} failed with status {}: {}{}{}",
                    self.serial,
                    args.join(" "),
                    output.status,
                    stderr,
                    if stderr.is_empty() || stdout.is_empty() {
                        ""
                    } else {
                        " / stdout: "
                    },
                    stdout
                ),
                None,
            ));
        }
        Ok(output.stdout)
    }

    fn append_command_log<const N: usize>(&self, args: &[&str; N]) {
        let line = format!("adb -s {} {}\n", self.serial, args.join(" "));
        let path = self.artifact_dir.join("commands.log");
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| {
                use std::io::Write as _;
                file.write_all(line.as_bytes())
            });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
struct AndroidNode {
    text: Option<String>,
    content_desc: Option<String>,
    resource_id: Option<String>,
    class_name: Option<String>,
    package_name: Option<String>,
    bounds: Option<Bounds>,
    clickable: bool,
    enabled: bool,
    focusable: bool,
    focused: bool,
    scrollable: bool,
    checked: bool,
    selected: bool,
}

impl AndroidNode {
    fn visible(&self) -> bool {
        self.bounds
            .as_ref()
            .is_some_and(|bounds| bounds.width > 0 && bounds.height > 0)
    }

    fn center(&self) -> Option<(i32, i32)> {
        self.bounds.as_ref().map(|bounds| {
            (
                bounds
                    .x
                    .saturating_add(i32::try_from(bounds.width / 2).unwrap_or(i32::MAX)),
                bounds
                    .y
                    .saturating_add(i32::try_from(bounds.height / 2).unwrap_or(i32::MAX)),
            )
        })
    }

    fn label(&self) -> String {
        self.text
            .as_ref()
            .or(self.content_desc.as_ref())
            .or(self.resource_id.as_ref())
            .or(self.class_name.as_ref())
            .cloned()
            .unwrap_or_else(|| "android node".to_owned())
    }

    fn to_element(&self, index: usize, target: &TargetSelector) -> ElementDescriptor {
        let actions = if self.clickable || self.enabled {
            vec!["click".to_owned()]
        } else {
            Vec::new()
        };
        ElementDescriptor {
            id: format!("android-node-{index}"),
            role: self
                .class_name
                .clone()
                .unwrap_or_else(|| "android_node".to_owned()),
            name: self.label(),
            description: self.content_desc.clone(),
            value: self.resource_id.clone(),
            bounds: self.bounds.clone(),
            target: Some(target.clone()),
            path: self
                .package_name
                .iter()
                .chain(self.class_name.iter())
                .cloned()
                .collect(),
            actions,
            app_name: self.package_name.clone(),
            process_id: None,
        }
    }
}

fn resolve_auto_serial() -> Result<String, TendrilError> {
    let output = Command::new("adb")
        .arg("devices")
        .output()
        .map_err(|error| {
            TendrilError::execution_failure(
                "android_adb_missing",
                format!("failed to execute adb: {error}"),
                None,
            )
        })?;
    if !output.status.success() {
        return Err(TendrilError::execution_failure(
            "android_adb_failed",
            format!(
                "adb devices failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            None,
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let devices = stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = parts.next()?;
            let state = parts.next()?;
            (state == "device").then(|| serial.to_owned())
        })
        .collect::<Vec<_>>();
    match devices.as_slice() {
        [serial] => Ok(serial.clone()),
        [] => Err(TendrilError::validation(
            "--android auto found no connected adb devices; pass --android <serial> or set TENDRIL_ANDROID_SERIAL",
        )
        .with_code("android_device_selection_failed")),
        _ => Err(TendrilError::validation(format!(
            "--android auto found multiple devices {}; pass --android <serial>",
            devices.join(", ")
        ))
        .with_code("android_device_selection_failed")),
    }
}

fn android_adapter_info() -> AdapterInfo {
    AdapterInfo {
        platform: PlatformKind::Android,
        session: DesktopSession::AndroidDevice,
        audio_backend: None,
        stateless: true,
    }
}

fn android_target_id(serial: &str) -> String {
    format!("android:{serial}")
}

fn android_artifact_dir(serial: &str) -> PathBuf {
    if let Some(path) = std::env::var_os("TENDRIL_ANDROID_ARTIFACT_DIR") {
        return PathBuf::from(path);
    }
    std::env::temp_dir().join(format!(
        "tendril-android-{}-{}",
        sanitize_filename(serial),
        current_timestamp_millis()
    ))
}

fn sanitize_filename(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn parse_wm_size(output: &str) -> Option<Bounds> {
    let line = output
        .lines()
        .find(|line| line.contains("Physical size:"))
        .or_else(|| output.lines().find(|line| line.contains('x')))?;
    let size = line.split(':').next_back().unwrap_or(line).trim();
    let (width, height) = size.split_once('x')?;
    Some(Bounds {
        x: 0,
        y: 0,
        width: width.trim().parse().ok()?,
        height: height.trim().parse().ok()?,
    })
}

fn parse_uiautomator_nodes(xml: &str) -> Vec<AndroidNode> {
    let mut nodes = Vec::new();
    let mut rest = xml;
    while let Some(offset) = rest.find("<node") {
        rest = &rest[offset + "<node".len()..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..end];
        nodes.push(AndroidNode {
            text: attr(tag, "text"),
            content_desc: attr(tag, "content-desc"),
            resource_id: attr(tag, "resource-id"),
            class_name: attr(tag, "class"),
            package_name: attr(tag, "package"),
            bounds: attr(tag, "bounds").and_then(|value| parse_bounds(&value)),
            clickable: bool_attr(tag, "clickable"),
            enabled: attr(tag, "enabled").is_none_or(|value| value == "true"),
            focusable: bool_attr(tag, "focusable"),
            focused: bool_attr(tag, "focused"),
            scrollable: bool_attr(tag, "scrollable"),
            checked: bool_attr(tag, "checked"),
            selected: bool_attr(tag, "selected"),
        });
        rest = &rest[end + 1..];
    }
    nodes
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let value = &tag[start..];
    let end = value.find('"')?;
    let value = decode_xml_entities(&value[..end]);
    (!value.is_empty()).then_some(value)
}

fn bool_attr(tag: &str, name: &str) -> bool {
    attr(tag, name).is_some_and(|value| value == "true")
}

fn decode_xml_entities(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn parse_bounds(value: &str) -> Option<Bounds> {
    let value = value.strip_prefix('[')?;
    let (left_top, rest) = value.split_once("][")?;
    let bottom_right = rest.strip_suffix(']')?;
    let (x0, y0) = left_top.split_once(',')?;
    let (x1, y1) = bottom_right.split_once(',')?;
    let x0: i32 = x0.parse().ok()?;
    let y0: i32 = y0.parse().ok()?;
    let x1: i32 = x1.parse().ok()?;
    let y1: i32 = y1.parse().ok()?;
    Some(Bounds {
        x: x0,
        y: y0,
        width: u32::try_from(x1.saturating_sub(x0)).ok()?,
        height: u32::try_from(y1.saturating_sub(y0)).ok()?,
    })
}

fn find_node_for_element_id<'a>(nodes: &'a [AndroidNode], id: &str) -> Option<&'a AndroidNode> {
    if let Some(index) = id
        .strip_prefix("android-node-")
        .and_then(|index| index.parse::<usize>().ok())
    {
        return nodes.get(index);
    }
    nodes.iter().find(|node| {
        node.text.as_deref() == Some(id)
            || node.content_desc.as_deref() == Some(id)
            || node.resource_id.as_deref() == Some(id)
    })
}

fn escape_android_input_text(text: &str) -> String {
    text.replace('%', "%25")
        .replace(' ', "%s")
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\'', "\\'")
}

fn android_key_event(key: &str) -> Option<String> {
    if key.chars().all(|character| character.is_ascii_digit()) {
        return Some(key.to_owned());
    }
    let normalized = key.trim().to_ascii_uppercase().replace('-', "_");
    let code = match normalized.as_str() {
        "BACK" | "ESC" | "ESCAPE" => "KEYCODE_BACK",
        "HOME" => "KEYCODE_HOME",
        "ENTER" | "RETURN" => "KEYCODE_ENTER",
        "WAKEUP" | "WAKE" => "KEYCODE_WAKEUP",
        "MENU" => "KEYCODE_MENU",
        "TAB" => "KEYCODE_TAB",
        "SPACE" => "KEYCODE_SPACE",
        "DELETE" | "BACKSPACE" => "KEYCODE_DEL",
        _ => return None,
    };
    Some(code.to_owned())
}

fn current_timestamp_string() -> String {
    format!("unix-ms:{}", current_timestamp_millis())
}

fn current_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uiautomator_nodes_and_bounds() {
        let xml = r#"<hierarchy><node text="Monitor" content-desc="Route monitor" resource-id="app:id/monitor" class="android.widget.Button" package="com.example" clickable="true" enabled="true" bounds="[140,1838][323,1971]" /></hierarchy>"#;
        let nodes = parse_uiautomator_nodes(xml);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].text.as_deref(), Some("Monitor"));
        assert_eq!(nodes[0].center(), Some((231, 1904)));
        assert!(nodes[0].clickable);
    }

    #[test]
    fn auto_serial_rejects_multiple_devices_from_parser_shape() {
        let bounds = parse_wm_size("Physical size: 1440x3120\n").expect("wm size should parse");
        assert_eq!(bounds.width, 1440);
        assert_eq!(bounds.height, 3120);
    }

    #[test]
    fn escapes_android_input_text_spaces() {
        assert_eq!(escape_android_input_text("hello world"), "hello%sworld");
    }
}
