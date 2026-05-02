use std::process::Command;

use serde::Deserialize;
use serde_json::Value;

use crate::error::TendrilError;
use crate::model::{
    Bounds, ElementDescriptor, ElementListInput, ElementListOutput, TargetKind, TargetSelector,
};
use crate::platform::{
    AdapterInfo, AdapterOperation, CaptureTargetKind, DesktopSession, PlatformAdapterError,
    PlatformKind, TargetDescriptor as PlatformTargetDescriptor, TargetInventory,
};

const ELEMENT_FIXTURE_ENV: &str = "TENDRIL_ELEMENT_FIXTURE_JSON";
const MAX_MACOS_ELEMENTS: usize = 512;

#[derive(Debug, Deserialize)]
struct ElementFixture {
    #[serde(default)]
    elements: Vec<ElementDescriptor>,
    #[serde(default)]
    notes: Vec<String>,
}

pub fn discover_elements(
    adapter: &AdapterInfo,
    inventory: &TargetInventory,
    input: &ElementListInput,
) -> Result<ElementListOutput, TendrilError> {
    input.validate()?;

    if let Some(fixture) = load_fixture(adapter, input)? {
        return Ok(fixture);
    }

    let targets = matching_targets(inventory, input)?;
    let mut notes = Vec::new();
    let mut elements = match (adapter.platform, adapter.session) {
        (PlatformKind::MacOs, DesktopSession::MacOsWindowServer) => {
            discover_macos_accessibility_elements(&targets, input.include_offscreen, &mut notes)
        }
        (PlatformKind::Linux, DesktopSession::X11) => {
            discover_x11_window_elements(&targets, input.include_offscreen, &mut notes)
        }
        (PlatformKind::Linux, DesktopSession::Wayland) => {
            discover_wayland_elements(&targets, &mut notes)
        }
        _ => {
            notes.push(
                "No platform element backend is available for this session; returning target roots only."
                    .to_owned(),
            );
            Vec::new()
        }
    };

    if elements.is_empty() {
        notes.push("No child UI elements were reported; returning target roots.".to_owned());
        elements = root_elements_from_targets(&targets);
    }

    assign_snapshot_ids(&mut elements);
    Ok(ElementListOutput {
        adapter: adapter.clone(),
        target: input.target.clone(),
        elements,
        notes,
    })
}

pub fn root_elements_from_targets(targets: &[PlatformTargetDescriptor]) -> Vec<ElementDescriptor> {
    targets.iter().map(root_element_from_target).collect()
}

fn load_fixture(
    adapter: &AdapterInfo,
    input: &ElementListInput,
) -> Result<Option<ElementListOutput>, TendrilError> {
    let Some(raw) = std::env::var(ELEMENT_FIXTURE_ENV).ok() else {
        return Ok(None);
    };

    let fixture = serde_json::from_str::<ElementFixture>(&raw).map_err(|error| {
        TendrilError::from(PlatformAdapterError::adapter_failure(
            AdapterOperation::ElementDiscovery,
            adapter.platform,
            format!("failed to parse {ELEMENT_FIXTURE_ENV}: {error}"),
        ))
    })?;

    let mut elements = fixture.elements;
    assign_snapshot_ids(&mut elements);
    Ok(Some(ElementListOutput {
        adapter: adapter.clone(),
        target: input.target.clone(),
        elements,
        notes: fixture.notes,
    }))
}

fn matching_targets(
    inventory: &TargetInventory,
    input: &ElementListInput,
) -> Result<Vec<PlatformTargetDescriptor>, TendrilError> {
    let targets = inventory
        .targets
        .iter()
        .filter(|target| match &input.target {
            None => true,
            Some(selector) => {
                target.id == selector.id() && selector_matches_kind(selector, target.kind)
            }
        })
        .cloned()
        .collect::<Vec<_>>();

    if targets.is_empty() {
        if let Some(selector) = &input.target {
            return Err(TendrilError::target_not_found(
                match selector.kind() {
                    TargetKind::Window => "window",
                    TargetKind::Display => "display",
                    TargetKind::AudioSource => "audio_source",
                },
                selector.id(),
            ));
        }
    }

    Ok(targets)
}

fn selector_matches_kind(selector: &TargetSelector, kind: CaptureTargetKind) -> bool {
    matches!(
        (selector, kind),
        (TargetSelector::Window { .. }, CaptureTargetKind::Window)
            | (TargetSelector::Display { .. }, CaptureTargetKind::Display)
    )
}

fn root_element_from_target(target: &PlatformTargetDescriptor) -> ElementDescriptor {
    ElementDescriptor {
        id: String::new(),
        role: match target.kind {
            CaptureTargetKind::Window => "window".to_owned(),
            CaptureTargetKind::Display => "display".to_owned(),
        },
        name: target.name.clone(),
        description: target.title.clone(),
        value: None,
        bounds: Some(target.bounds.clone()),
        target: Some(target_selector_from_platform(target)),
        path: target
            .app_name
            .iter()
            .chain(std::iter::once(&target.name))
            .cloned()
            .collect(),
        actions: vec!["click".to_owned()],
        app_name: target.app_name.clone(),
        process_id: target.process_id,
    }
}

fn target_selector_from_platform(target: &PlatformTargetDescriptor) -> TargetSelector {
    match target.kind {
        CaptureTargetKind::Window => TargetSelector::Window {
            id: target.id.clone(),
        },
        CaptureTargetKind::Display => TargetSelector::Display {
            id: target.id.clone(),
        },
    }
}

fn assign_snapshot_ids(elements: &mut [ElementDescriptor]) {
    for (index, element) in elements.iter_mut().enumerate() {
        if element.id.trim().is_empty() || element.id.starts_with("auto:") {
            element.id = (index + 1).to_string();
        }
    }
}

fn discover_macos_accessibility_elements(
    targets: &[PlatformTargetDescriptor],
    include_offscreen: bool,
    notes: &mut Vec<String>,
) -> Vec<ElementDescriptor> {
    let mut elements = Vec::new();
    for target in targets {
        let Some(process_id) = target.process_id else {
            elements.push(root_element_from_target(target));
            notes.push(format!(
                "Target `{}` did not expose a process id, so macOS Accessibility elements could not be queried.",
                target.id
            ));
            continue;
        };

        match run_macos_accessibility_listing(process_id, target, include_offscreen) {
            Ok(mut discovered) => elements.append(&mut discovered),
            Err(error) => {
                notes.push(format!(
                    "macOS Accessibility element listing failed for `{}`: {error}; returning its root target instead.",
                    target.id
                ));
                elements.push(root_element_from_target(target));
            }
        }
    }
    elements
}

fn run_macos_accessibility_listing(
    process_id: u32,
    target: &PlatformTargetDescriptor,
    include_offscreen: bool,
) -> Result<Vec<ElementDescriptor>, String> {
    let script = macos_accessibility_listing_jxa(process_id, target, include_offscreen);
    let output = Command::new("osascript")
        .args(["-l", "JavaScript", "-e", &script])
        .output()
        .map_err(|error| format!("failed to spawn osascript: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        return Err(if stdout.is_empty() || stderr.is_empty() {
            format!("{stdout}{stderr}")
        } else {
            format!("{stdout} | {stderr}")
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let values = serde_json::from_str::<Value>(stdout.trim())
        .map_err(|error| format!("failed to parse osascript JSON: {error}"))?;
    let mut elements = Vec::new();
    if let Some(array) = values.as_array() {
        for value in array {
            let role = json_str(value, "role").unwrap_or("element").to_owned();
            let name = json_str(value, "name")
                .or_else(|| json_str(value, "description"))
                .or_else(|| json_str(value, "value"))
                .unwrap_or(&role)
                .to_owned();
            let bounds = value.get("bounds").map(|bounds| Bounds {
                x: json_i32(bounds, "x").unwrap_or(target.bounds.x),
                y: json_i32(bounds, "y").unwrap_or(target.bounds.y),
                width: json_u32(bounds, "width").unwrap_or(1),
                height: json_u32(bounds, "height").unwrap_or(1),
            });
            elements.push(ElementDescriptor {
                id: String::new(),
                role,
                name,
                description: json_str(value, "description").map(str::to_owned),
                value: json_str(value, "value").map(str::to_owned),
                bounds,
                target: Some(target_selector_from_platform(target)),
                path: value.get("path").and_then(Value::as_array).map_or_else(
                    || target.app_name.iter().cloned().collect(),
                    |items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    },
                ),
                actions: vec!["click".to_owned(), "press".to_owned()],
                app_name: target.app_name.clone(),
                process_id: target.process_id,
            });
        }
    }
    Ok(elements)
}

fn macos_accessibility_listing_jxa(
    process_id: u32,
    target: &PlatformTargetDescriptor,
    include_offscreen: bool,
) -> String {
    let target_x = target.bounds.x;
    let target_y = target.bounds.y;
    let target_width = target.bounds.width;
    let target_height = target.bounds.height;
    let include_offscreen = if include_offscreen { "true" } else { "false" };
    format!(
        r"ObjC.import('ApplicationServices');
(function () {{
  var pid = {process_id};
  var target = {{ x: {target_x}, y: {target_y}, width: {target_width}, height: {target_height} }};
  var includeOffscreen = {include_offscreen};
  var maxElements = {MAX_MACOS_ELEMENTS};
  var app = $.AXUIElementCreateApplication(pid);
  if (!app) throw new Error('AXUIElementCreateApplication failed');
  var out = [];
  function unwrap(value) {{
    try {{ return ObjC.unwrap(value); }} catch (e) {{ return String(value); }}
  }}
  function attr(element, name) {{
    var ref = Ref();
    var err = $.AXUIElementCopyAttributeValue(element, $(name), ref);
    if (err !== 0 || !ref[0]) return null;
    return ObjC.castRefToObject(ref[0]);
  }}
  function point(value) {{
    if (!value) return null;
    var p = $.CGPointMake(0, 0);
    if (!$.AXValueGetValue(value, 1, Ref(p))) return null;
    return {{ x: Math.round(p.x), y: Math.round(p.y) }};
  }}
  function size(value) {{
    if (!value) return null;
    var s = $.CGSizeMake(0, 0);
    if (!$.AXValueGetValue(value, 2, Ref(s))) return null;
    return {{ width: Math.round(s.width), height: Math.round(s.height) }};
  }}
  function intersects(bounds) {{
    if (includeOffscreen || !bounds) return true;
    return bounds.x < target.x + target.width && bounds.x + bounds.width > target.x &&
           bounds.y < target.y + target.height && bounds.y + bounds.height > target.y;
  }}
  function textAttr(element, name) {{
    var value = attr(element, name);
    if (!value) return null;
    var text = String(unwrap(value));
    return text.length === 0 ? null : text;
  }}
  function roleName(role) {{
    if (!role) return 'element';
    return String(unwrap(role)).replace(/^AX/, '').toLowerCase();
  }}
  function walk(element, depth, path) {{
    if (out.length >= maxElements || depth > 8) return;
    var role = roleName(attr(element, 'AXRole'));
    var title = textAttr(element, 'AXTitle');
    var desc = textAttr(element, 'AXDescription');
    var value = textAttr(element, 'AXValue');
    var pos = point(attr(element, 'AXPosition'));
    var sz = size(attr(element, 'AXSize'));
    var bounds = (pos && sz) ? {{ x: pos.x, y: pos.y, width: sz.width, height: sz.height }} : null;
    var name = title || desc || value || role;
    if (intersects(bounds)) {{
      out.push({{ role: role, name: name, description: desc, value: value, bounds: bounds, path: path.concat([name]) }});
    }}
    var children = attr(element, 'AXChildren');
    if (!children) return;
    var count = children.count;
    for (var i = 0; i < count && out.length < maxElements; i += 1) {{
      walk(children.objectAtIndex(i), depth + 1, path.concat([name]));
    }}
  }}
  walk(app, 0, []);
  return JSON.stringify(out);
}}());
"
    )
}

fn discover_x11_window_elements(
    targets: &[PlatformTargetDescriptor],
    include_offscreen: bool,
    notes: &mut Vec<String>,
) -> Vec<ElementDescriptor> {
    let mut elements = Vec::new();
    for target in targets {
        if target.kind != CaptureTargetKind::Window || !program_on_path("xwininfo") {
            elements.push(root_element_from_target(target));
            if target.kind == CaptureTargetKind::Window {
                notes.push(
                    "xwininfo was not available; returning X11 target roots only for element listing."
                        .to_owned(),
                );
            }
            continue;
        }
        match Command::new("xwininfo")
            .args(["-tree", "-id", &target.id])
            .output()
        {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut discovered = parse_xwininfo_tree(&stdout, target, include_offscreen);
                if discovered.is_empty() {
                    elements.push(root_element_from_target(target));
                } else {
                    elements.append(&mut discovered);
                }
            }
            Ok(output) => {
                notes.push(format!(
                    "xwininfo failed for `{}`: {}; returning its root target.",
                    target.id,
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
                elements.push(root_element_from_target(target));
            }
            Err(error) => {
                notes.push(format!(
                    "failed to spawn xwininfo for `{}`: {error}; returning its root target.",
                    target.id
                ));
                elements.push(root_element_from_target(target));
            }
        }
    }
    elements
}

fn parse_xwininfo_tree(
    output: &str,
    target: &PlatformTargetDescriptor,
    include_offscreen: bool,
) -> Vec<ElementDescriptor> {
    output
        .lines()
        .filter_map(|line| parse_xwininfo_line(line, target, include_offscreen))
        .collect()
}

fn parse_xwininfo_line(
    line: &str,
    target: &PlatformTargetDescriptor,
    include_offscreen: bool,
) -> Option<ElementDescriptor> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("0x") {
        return None;
    }
    let xid = trimmed.split_whitespace().next()?.to_owned();
    let quoted_name = quoted_segment(trimmed).unwrap_or_else(|| xid.clone());
    let (bounds, _) = parse_x11_geometry_from_line(trimmed, &target.bounds)?;
    if !include_offscreen && !bounds_overlap(&bounds, &target.bounds) {
        return None;
    }
    Some(ElementDescriptor {
        id: String::new(),
        role: if xid == target.id {
            "window"
        } else {
            "x11_window"
        }
        .to_owned(),
        name: quoted_name,
        description: Some(format!("X11 window {xid}")),
        value: None,
        bounds: Some(bounds),
        target: Some(target_selector_from_platform(target)),
        path: target
            .app_name
            .iter()
            .chain(std::iter::once(&target.name))
            .cloned()
            .collect(),
        actions: vec!["click".to_owned()],
        app_name: target.app_name.clone(),
        process_id: target.process_id,
    })
}

fn quoted_segment(line: &str) -> Option<String> {
    let start = line.find('"')? + 1;
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_owned())
}

fn parse_x11_geometry_from_line(line: &str, target: &Bounds) -> Option<(Bounds, bool)> {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    let geometry = tokens.iter().find_map(|token| parse_size_offset(token))?;
    let absolute_offset = tokens.iter().find_map(|token| parse_offset_only(token));
    let (width, height, rel_x, rel_y) = geometry;
    let (x, y, absolute) = if let Some((abs_x, abs_y)) = absolute_offset {
        (abs_x, abs_y, true)
    } else {
        (
            target.x.saturating_add(rel_x),
            target.y.saturating_add(rel_y),
            false,
        )
    };
    Some((
        Bounds {
            x,
            y,
            width,
            height,
        },
        absolute,
    ))
}

fn parse_size_offset(token: &str) -> Option<(u32, u32, i32, i32)> {
    let (width_text, rest) = token.split_once('x')?;
    let (height_text, x_text, y_text) = split_geometry_offsets(rest)?;
    Some((
        width_text.parse().ok()?,
        height_text.parse().ok()?,
        x_text.parse().ok()?,
        y_text.parse().ok()?,
    ))
}

fn parse_offset_only(token: &str) -> Option<(i32, i32)> {
    if token.contains('x') {
        return None;
    }
    let (_, x_text, y_text) = split_geometry_offsets(token)?;
    Some((x_text.parse().ok()?, y_text.parse().ok()?))
}

fn split_geometry_offsets(value: &str) -> Option<(&str, &str, &str)> {
    let sign_index = value
        .char_indices()
        .skip(1)
        .find_map(|(index, ch)| matches!(ch, '+' | '-').then_some(index))?;
    let first = &value[..sign_index];
    let rest = &value[sign_index..];
    let second_sign = rest
        .char_indices()
        .skip(1)
        .find_map(|(index, ch)| matches!(ch, '+' | '-').then_some(index))?;
    Some((first, &rest[..second_sign], &rest[second_sign..]))
}

fn discover_wayland_elements(
    targets: &[PlatformTargetDescriptor],
    notes: &mut Vec<String>,
) -> Vec<ElementDescriptor> {
    notes.push(
        "Wayland does not expose a compositor-neutral widget tree to clients; returning compositor-discovered surface roots as clickable elements. Apps that publish AT-SPI accessibility metadata can be wired as a future backend without changing the list-elements/run contract."
            .to_owned(),
    );
    root_elements_from_targets(targets)
}

fn bounds_overlap(left: &Bounds, right: &Bounds) -> bool {
    let left_right = i64::from(left.x) + i64::from(left.width);
    let left_bottom = i64::from(left.y) + i64::from(left.height);
    let right_right = i64::from(right.x) + i64::from(right.width);
    let right_bottom = i64::from(right.y) + i64::from(right.height);
    i64::from(left.x) < right_right
        && left_right > i64::from(right.x)
        && i64::from(left.y) < right_bottom
        && left_bottom > i64::from(right.y)
}

fn program_on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|entry| {
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

fn json_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

fn json_i32(value: &Value, key: &str) -> Option<i32> {
    value
        .get(key)?
        .as_i64()
        .and_then(|number| i32::try_from(number).ok())
}

fn json_u32(value: &Value, key: &str) -> Option<u32> {
    value
        .get(key)?
        .as_u64()
        .and_then(|number| u32::try_from(number).ok())
}

#[cfg(test)]
mod tests {
    use super::{parse_x11_geometry_from_line, parse_xwininfo_line};
    use crate::model::{Bounds, ScaleFactor};
    use crate::platform::{CaptureTargetKind, TargetDescriptor};

    fn target() -> TargetDescriptor {
        TargetDescriptor {
            id: "0x400001".to_owned(),
            title: Some("App".to_owned()),
            kind: CaptureTargetKind::Window,
            name: "App".to_owned(),
            bounds: Bounds {
                x: 100,
                y: 200,
                width: 800,
                height: 600,
            },
            scale_factor: ScaleFactor::identity(),
            capture_supported: true,
            input_supported: true,
            app_name: Some("App".to_owned()),
            process_id: Some(42),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn parses_xwininfo_geometry_with_absolute_offsets() {
        let bounds = parse_x11_geometry_from_line(
            "0x400123 \"OK\": ()  80x24+10+20  +110+220",
            &target().bounds,
        )
        .expect("geometry should parse")
        .0;

        assert_eq!(bounds.x, 110);
        assert_eq!(bounds.y, 220);
        assert_eq!(bounds.width, 80);
        assert_eq!(bounds.height, 24);
    }

    #[test]
    fn parses_xwininfo_line_as_clickable_element() {
        let element = parse_xwininfo_line(
            "     0x400123 \"New Note\": ()  80x24+10+20  +110+220",
            &target(),
            false,
        )
        .expect("element should parse");

        assert_eq!(element.role, "x11_window");
        assert_eq!(element.name, "New Note");
        assert_eq!(element.bounds.expect("bounds").x, 110);
        assert_eq!(element.actions, vec!["click"]);
    }
}
