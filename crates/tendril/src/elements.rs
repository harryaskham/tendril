use std::collections::HashSet;
use std::process::Command;

use serde::Deserialize;
use serde_json::Value;
use zbus::{
    blocking::{Connection, Proxy, connection::Builder as ConnectionBuilder},
    zvariant::OwnedObjectPath,
};

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
const MAX_ATSPI_ELEMENTS: usize = 512;
const MAX_ATSPI_DEPTH: usize = 12;
const ATSPI_COORD_TYPE_SCREEN: u32 = 0;

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
            discover_macos_accessibility_elements(
                inventory,
                &targets,
                input.include_offscreen,
                &mut notes,
            )
        }
        (PlatformKind::Linux, DesktopSession::X11) => {
            discover_x11_elements(&targets, input.include_offscreen, &mut notes)
        }
        (PlatformKind::Linux, DesktopSession::Wayland) => {
            discover_wayland_elements(&targets, input.include_offscreen, &mut notes)
        }
        (PlatformKind::Windows11, DesktopSession::WindowsDesktop) => {
            discover_windows_elements(inventory, &targets, input.include_offscreen, &mut notes)
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
    inventory: &TargetInventory,
    targets: &[PlatformTargetDescriptor],
    include_offscreen: bool,
    notes: &mut Vec<String>,
) -> Vec<ElementDescriptor> {
    let mut elements = Vec::new();
    let query_targets = macos_accessibility_query_targets(inventory, targets, notes);
    for target in &query_targets {
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

fn macos_accessibility_query_targets(
    inventory: &TargetInventory,
    targets: &[PlatformTargetDescriptor],
    notes: &mut Vec<String>,
) -> Vec<PlatformTargetDescriptor> {
    let mut query_targets = Vec::new();
    let mut seen = HashSet::new();

    for target in targets {
        match target.kind {
            CaptureTargetKind::Window => push_unique_target(&mut query_targets, &mut seen, target),
            CaptureTargetKind::Display => {
                let before = query_targets.len();
                for window in inventory.targets.iter().filter(|candidate| {
                    candidate.kind == CaptureTargetKind::Window
                        && bounds_overlap(&candidate.bounds, &target.bounds)
                }) {
                    push_unique_target(&mut query_targets, &mut seen, window);
                }
                if query_targets.len() == before {
                    notes.push(format!(
                        "Display `{}` did not contain any discovered windows for macOS Accessibility listing; returning its root target instead.",
                        target.id
                    ));
                    push_unique_target(&mut query_targets, &mut seen, target);
                }
            }
        }
    }

    query_targets
}

fn push_unique_target(
    targets: &mut Vec<PlatformTargetDescriptor>,
    seen: &mut HashSet<String>,
    target: &PlatformTargetDescriptor,
) {
    let key = format!("{:?}:{}", target.kind, target.id);
    if seen.insert(key) {
        targets.push(target.clone());
    }
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

fn discover_x11_elements(
    targets: &[PlatformTargetDescriptor],
    include_offscreen: bool,
    notes: &mut Vec<String>,
) -> Vec<ElementDescriptor> {
    match run_atspi_accessibility_listing(targets, include_offscreen) {
        Ok(elements) if !elements.is_empty() => {
            notes.push(
                "X11 element discovery used AT-SPI accessibility metadata; element bounds are screen coordinates and click(<id>) resolves them through the existing target-relative DSL contract."
                    .to_owned(),
            );
            elements
        }
        Ok(_) => {
            notes.push(
                "AT-SPI was reachable but did not report accessible child elements for the requested X11 target; falling back to the X11 window tree."
                    .to_owned(),
            );
            discover_x11_window_elements(targets, include_offscreen, notes)
        }
        Err(error) => {
            notes.push(format!(
                "X11 AT-SPI element listing failed: {error}; falling back to the X11 window tree."
            ));
            discover_x11_window_elements(targets, include_offscreen, notes)
        }
    }
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

fn discover_windows_elements(
    inventory: &TargetInventory,
    targets: &[PlatformTargetDescriptor],
    include_offscreen: bool,
    notes: &mut Vec<String>,
) -> Vec<ElementDescriptor> {
    let query_targets = window_targets_for_scope(inventory, targets);
    let mut elements = Vec::new();
    for target in &query_targets {
        if target.kind != CaptureTargetKind::Window {
            elements.push(root_element_from_target(target));
            continue;
        }
        match tendril_win32::discover_window_elements(&target.id) {
            Ok(discovered) if !discovered.is_empty() => {
                elements.extend(discovered.into_iter().filter_map(|element| {
                    windows_element_descriptor(target, element, include_offscreen)
                }));
            }
            Ok(_) => elements.push(root_element_from_target(target)),
            Err(error) => {
                notes.push(format!(
                    "Windows native element listing failed for `{}`: {error}; returning its root target instead.",
                    target.id
                ));
                elements.push(root_element_from_target(target));
            }
        }
    }
    if !elements.is_empty() {
        notes.push(
            "Windows element discovery used native Win32 window/control enumeration; element bounds are screen coordinates and click(<id>) resolves them through the shared target-relative DSL contract."
                .to_owned(),
        );
    }
    elements
}

fn window_targets_for_scope(
    inventory: &TargetInventory,
    targets: &[PlatformTargetDescriptor],
) -> Vec<PlatformTargetDescriptor> {
    let mut query_targets = Vec::new();
    let mut seen = HashSet::new();
    for target in targets {
        match target.kind {
            CaptureTargetKind::Window => push_unique_target(&mut query_targets, &mut seen, target),
            CaptureTargetKind::Display => {
                for window in inventory.targets.iter().filter(|candidate| {
                    candidate.kind == CaptureTargetKind::Window
                        && bounds_overlap(&candidate.bounds, &target.bounds)
                }) {
                    push_unique_target(&mut query_targets, &mut seen, window);
                }
            }
        }
    }
    if query_targets.is_empty() {
        targets.to_vec()
    } else {
        query_targets
    }
}

fn windows_element_descriptor(
    target: &PlatformTargetDescriptor,
    element: tendril_win32::ElementInfo,
    include_offscreen: bool,
) -> Option<ElementDescriptor> {
    let bounds = Bounds {
        x: element.bounds.x,
        y: element.bounds.y,
        width: element.bounds.width,
        height: element.bounds.height,
    };
    if !include_offscreen && !bounds_overlap(&bounds, &target.bounds) {
        return None;
    }
    Some(ElementDescriptor {
        id: element.id,
        role: element.role,
        name: element.name,
        description: element.description,
        value: None,
        bounds: Some(bounds),
        target: Some(target_selector_from_platform(target)),
        path: if element.path.is_empty() {
            target
                .app_name
                .iter()
                .chain(std::iter::once(&target.name))
                .cloned()
                .collect()
        } else {
            element.path
        },
        actions: element.actions,
        app_name: target.app_name.clone(),
        process_id: element.process_id.or(target.process_id),
    })
}

fn discover_wayland_elements(
    targets: &[PlatformTargetDescriptor],
    include_offscreen: bool,
    notes: &mut Vec<String>,
) -> Vec<ElementDescriptor> {
    match run_atspi_accessibility_listing(targets, include_offscreen) {
        Ok(elements) if !elements.is_empty() => {
            notes.push(
                "Wayland element discovery used AT-SPI accessibility metadata; element bounds are screen coordinates and click(<id>) resolves them through the existing target-relative DSL contract."
                    .to_owned(),
            );
            elements
        }
        Ok(_) => {
            notes.push(
                "AT-SPI was reachable but did not report accessible child elements for the requested Wayland target; returning compositor-discovered surface roots."
                    .to_owned(),
            );
            root_elements_from_targets(targets)
        }
        Err(error) => {
            notes.push(format!(
                "Wayland AT-SPI element listing failed: {error}; returning compositor-discovered surface roots."
            ));
            root_elements_from_targets(targets)
        }
    }
}

#[derive(Debug, Clone)]
struct AtspiObjectRef {
    destination: String,
    path: OwnedObjectPath,
}

impl AtspiObjectRef {
    fn from_tuple((destination, path): (String, OwnedObjectPath)) -> Self {
        Self { destination, path }
    }

    fn key(&self) -> String {
        format!("{}{}", self.destination, self.path.as_str())
    }
}

struct AtspiClient {
    connection: Connection,
}

impl AtspiClient {
    fn connect() -> Result<Self, String> {
        let address = std::env::var("AT_SPI_BUS_ADDRESS")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map_or_else(query_atspi_bus_address, Ok)?;
        let connection = ConnectionBuilder::address(address.as_str())
            .map_err(|error| format!("failed to configure AT-SPI bus connection: {error}"))?
            .build()
            .map_err(|error| format!("failed to connect to AT-SPI bus: {error}"))?;
        Ok(Self { connection })
    }

    fn applications(&self) -> Result<Vec<AtspiObjectRef>, String> {
        let registry = Proxy::new(
            &self.connection,
            "org.a11y.atspi.Registry",
            "/org/a11y/atspi/registry",
            "org.a11y.atspi.Registry",
        )
        .map_err(|error| format!("failed to create AT-SPI registry proxy: {error}"))?;
        let applications: Vec<(String, OwnedObjectPath)> = registry
            .call("GetApplications", &())
            .map_err(|error| format!("AT-SPI Registry.GetApplications failed: {error}"))?;
        Ok(applications
            .into_iter()
            .map(AtspiObjectRef::from_tuple)
            .collect())
    }

    fn accessible_proxy(&self, object: &AtspiObjectRef) -> Result<Proxy<'static>, String> {
        Proxy::new_owned(
            self.connection.clone(),
            object.destination.clone(),
            object.path.clone(),
            "org.a11y.atspi.Accessible".to_owned(),
        )
        .map_err(|error| format!("failed to create AT-SPI Accessible proxy: {error}"))
    }

    fn component_proxy(&self, object: &AtspiObjectRef) -> Result<Proxy<'static>, String> {
        Proxy::new_owned(
            self.connection.clone(),
            object.destination.clone(),
            object.path.clone(),
            "org.a11y.atspi.Component".to_owned(),
        )
        .map_err(|error| format!("failed to create AT-SPI Component proxy: {error}"))
    }

    fn action_proxy(&self, object: &AtspiObjectRef) -> Result<Proxy<'static>, String> {
        Proxy::new_owned(
            self.connection.clone(),
            object.destination.clone(),
            object.path.clone(),
            "org.a11y.atspi.Action".to_owned(),
        )
        .map_err(|error| format!("failed to create AT-SPI Action proxy: {error}"))
    }

    fn application_proxy(&self, object: &AtspiObjectRef) -> Result<Proxy<'static>, String> {
        Proxy::new_owned(
            self.connection.clone(),
            object.destination.clone(),
            object.path.clone(),
            "org.a11y.atspi.Application".to_owned(),
        )
        .map_err(|error| format!("failed to create AT-SPI Application proxy: {error}"))
    }
}

fn query_atspi_bus_address() -> Result<String, String> {
    let session = Connection::session()
        .map_err(|error| format!("failed to connect to session bus for org.a11y.Bus: {error}"))?;
    let bus = Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus")
        .map_err(|error| format!("failed to create org.a11y.Bus proxy: {error}"))?;
    bus.call("GetAddress", &())
        .map_err(|error| format!("org.a11y.Bus.GetAddress failed: {error}"))
}

fn run_atspi_accessibility_listing(
    targets: &[PlatformTargetDescriptor],
    include_offscreen: bool,
) -> Result<Vec<ElementDescriptor>, String> {
    let client = AtspiClient::connect()?;
    let applications = client.applications()?;
    let mut elements = Vec::new();

    for target in targets {
        for application in &applications {
            if elements.len() >= MAX_ATSPI_ELEMENTS {
                return Ok(elements);
            }
            let process_id = atspi_application_process_id(&client, application);
            if target.kind == CaptureTargetKind::Window
                && target.process_id.is_some()
                && process_id.is_some()
                && target.process_id != process_id
            {
                continue;
            }
            let app_name = atspi_accessible_name(&client, application)
                .filter(|name| !name.trim().is_empty())
                .or_else(|| target.app_name.clone());
            let mut visited = HashSet::new();
            let root_path = app_name.clone().into_iter().collect::<Vec<_>>();
            walk_atspi_tree(
                &client,
                application,
                target,
                include_offscreen,
                &root_path,
                app_name.as_ref(),
                process_id.or(target.process_id),
                &mut visited,
                &mut elements,
                0,
            );
        }
    }

    Ok(elements)
}

#[allow(clippy::too_many_arguments)]
fn walk_atspi_tree(
    client: &AtspiClient,
    object: &AtspiObjectRef,
    target: &PlatformTargetDescriptor,
    include_offscreen: bool,
    parent_path: &[String],
    app_name: Option<&String>,
    process_id: Option<u32>,
    visited: &mut HashSet<String>,
    elements: &mut Vec<ElementDescriptor>,
    depth: usize,
) {
    if elements.len() >= MAX_ATSPI_ELEMENTS || depth > MAX_ATSPI_DEPTH {
        return;
    }
    if !visited.insert(object.key()) {
        return;
    }

    let Ok(accessible) = client.accessible_proxy(object) else {
        return;
    };
    let role = atspi_role_name(&accessible);
    let name = atspi_string_property(&accessible, "Name")
        .or_else(|| atspi_string_property(&accessible, "Description"))
        .unwrap_or_else(|| role.clone());
    let description = atspi_string_property(&accessible, "Description");
    let value = atspi_value_text(client, object);
    let bounds = atspi_component_extents(client, object);
    let mut path = parent_path.to_vec();
    if path.last() != Some(&name) {
        path.push(name.clone());
    }

    if atspi_element_in_scope(bounds.as_ref(), &target.bounds, include_offscreen) {
        let mut actions = atspi_action_names(client, object);
        if bounds.is_some() && !actions.iter().any(|action| action == "click") {
            actions.insert(0, "click".to_owned());
        }
        if bounds.is_some() && !actions.iter().any(|action| action == "press") {
            actions.push("press".to_owned());
        }
        elements.push(ElementDescriptor {
            id: String::new(),
            role,
            name: name.clone(),
            description,
            value,
            bounds,
            target: Some(target_selector_from_platform(target)),
            path,
            actions,
            app_name: app_name.cloned(),
            process_id,
        });
    }

    for child in atspi_children(&accessible) {
        let child_parent_path = parent_path_for_child(parent_path, &name);
        walk_atspi_tree(
            client,
            &child,
            target,
            include_offscreen,
            &child_parent_path,
            app_name,
            process_id,
            visited,
            elements,
            depth + 1,
        );
    }
}

fn parent_path_for_child(parent_path: &[String], name: &str) -> Vec<String> {
    let mut path = parent_path.to_vec();
    if path.last().is_none_or(|last| last != name) {
        path.push(name.to_owned());
    }
    path
}

fn atspi_application_process_id(client: &AtspiClient, object: &AtspiObjectRef) -> Option<u32> {
    let application = client.application_proxy(object).ok()?;
    let id = application
        .get_property::<i32>("Id")
        .ok()
        .or_else(|| application.call::<_, _, i32>("GetId", &()).ok())?;
    u32::try_from(id).ok()
}

fn atspi_accessible_name(client: &AtspiClient, object: &AtspiObjectRef) -> Option<String> {
    let accessible = client.accessible_proxy(object).ok()?;
    atspi_string_property(&accessible, "Name")
}

fn atspi_role_name(accessible: &Proxy<'_>) -> String {
    accessible
        .get_property::<String>("RoleName")
        .ok()
        .or_else(|| accessible.call::<_, _, String>("GetRoleName", &()).ok())
        .map(|role| normalize_atspi_role(&role))
        .filter(|role| !role.is_empty())
        .unwrap_or_else(|| "element".to_owned())
}

fn normalize_atspi_role(role: &str) -> String {
    let normalized = role
        .trim()
        .trim_start_matches("ROLE_")
        .chars()
        .map(|ch| match ch {
            '-' | ' ' => '_',
            other => other.to_ascii_lowercase(),
        })
        .collect::<String>();
    normalized
        .strip_prefix("atspi_role_")
        .unwrap_or(&normalized)
        .to_owned()
}

fn atspi_string_property(proxy: &Proxy<'_>, property: &str) -> Option<String> {
    proxy
        .get_property::<String>(property)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn atspi_value_text(client: &AtspiClient, object: &AtspiObjectRef) -> Option<String> {
    let value = Proxy::new(
        &client.connection,
        object.destination.as_str(),
        object.path.as_str(),
        "org.a11y.atspi.Value",
    )
    .ok()?;
    value
        .get_property::<f64>("CurrentValue")
        .ok()
        .map(|number| number.to_string())
        .filter(|text| !text.is_empty())
}

fn atspi_component_extents(client: &AtspiClient, object: &AtspiObjectRef) -> Option<Bounds> {
    let component = client.component_proxy(object).ok()?;
    let (x, y, width, height): (i32, i32, i32, i32) = component
        .call("GetExtents", &(ATSPI_COORD_TYPE_SCREEN,))
        .ok()?;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(Bounds {
        x,
        y,
        width: u32::try_from(width).ok()?,
        height: u32::try_from(height).ok()?,
    })
}

fn atspi_action_names(client: &AtspiClient, object: &AtspiObjectRef) -> Vec<String> {
    let Ok(action) = client.action_proxy(object) else {
        return Vec::new();
    };
    let count = action
        .get_property::<i32>("NActions")
        .ok()
        .or_else(|| action.call::<_, _, i32>("GetNActions", &()).ok())
        .unwrap_or(0);
    (0..count)
        .filter_map(|index| action.call::<_, _, String>("GetName", &(index,)).ok())
        .map(|name| normalize_atspi_action(&name))
        .filter(|name| !name.is_empty())
        .fold(Vec::new(), |mut acc, name| {
            if !acc.contains(&name) {
                acc.push(name);
            }
            acc
        })
}

fn normalize_atspi_action(action: &str) -> String {
    match action.trim().to_ascii_lowercase().as_str() {
        "click" | "press" | "activate" | "default" => "press".to_owned(),
        "toggle" => "toggle".to_owned(),
        "expand" => "expand".to_owned(),
        "collapse" => "collapse".to_owned(),
        other => other.replace([' ', '-'], "_"),
    }
}

fn atspi_children(accessible: &Proxy<'_>) -> Vec<AtspiObjectRef> {
    if let Ok(children) =
        accessible.call::<_, _, Vec<(String, OwnedObjectPath)>>("GetChildren", &())
    {
        return children
            .into_iter()
            .map(AtspiObjectRef::from_tuple)
            .collect();
    }

    let child_count = accessible
        .get_property::<i32>("ChildCount")
        .ok()
        .or_else(|| accessible.call::<_, _, i32>("GetChildCount", &()).ok())
        .unwrap_or(0);
    (0..child_count)
        .filter_map(|index| {
            accessible
                .call::<_, _, (String, OwnedObjectPath)>("GetChildAtIndex", &(index,))
                .ok()
        })
        .map(AtspiObjectRef::from_tuple)
        .collect()
}

fn atspi_element_in_scope(
    bounds: Option<&Bounds>,
    target_bounds: &Bounds,
    include_offscreen: bool,
) -> bool {
    if include_offscreen {
        return true;
    }
    bounds.is_some_and(|bounds| bounds_overlap(bounds, target_bounds))
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
    use super::{
        assign_snapshot_ids, atspi_element_in_scope, bounds_overlap, json_i32, json_str, json_u32,
        macos_accessibility_listing_jxa, macos_accessibility_query_targets, normalize_atspi_action,
        normalize_atspi_role, parent_path_for_child, parse_offset_only, parse_size_offset,
        parse_x11_geometry_from_line, parse_xwininfo_line, selector_matches_kind,
        split_geometry_offsets, target_selector_from_platform, window_targets_for_scope,
    };
    use crate::model::{Bounds, ElementDescriptor, ScaleFactor, TargetSelector};
    use crate::platform::{CaptureTargetKind, TargetDescriptor};

    fn target() -> TargetDescriptor {
        window_target("0x400001", 100, 200, 800, 600)
    }

    fn window_target(id: &str, x: i32, y: i32, width: u32, height: u32) -> TargetDescriptor {
        TargetDescriptor {
            id: id.to_owned(),
            title: Some("App".to_owned()),
            kind: CaptureTargetKind::Window,
            name: "App".to_owned(),
            bounds: Bounds {
                x,
                y,
                width,
                height,
            },
            scale_factor: ScaleFactor::identity(),
            capture_supported: true,
            input_supported: true,
            app_name: Some("App".to_owned()),
            process_id: Some(42),
            diagnostics: Vec::new(),
        }
    }

    fn display_target(id: &str, x: i32, y: i32, width: u32, height: u32) -> TargetDescriptor {
        TargetDescriptor {
            id: id.to_owned(),
            title: None,
            kind: CaptureTargetKind::Display,
            name: format!("Display {id}"),
            bounds: Bounds {
                x,
                y,
                width,
                height,
            },
            scale_factor: ScaleFactor::identity(),
            capture_supported: true,
            input_supported: true,
            app_name: None,
            process_id: None,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn selector_matches_kind_pairs_window_and_display_only() {
        let window = TargetSelector::Window {
            id: "0x1".to_owned(),
        };
        let display = TargetSelector::Display {
            id: "1".to_owned(),
        };
        assert!(selector_matches_kind(&window, CaptureTargetKind::Window));
        assert!(selector_matches_kind(&display, CaptureTargetKind::Display));
        assert!(!selector_matches_kind(&window, CaptureTargetKind::Display));
        assert!(!selector_matches_kind(&display, CaptureTargetKind::Window));
    }

    #[test]
    fn target_selector_from_platform_maps_kind_and_clones_id() {
        let mut window = window_target("0x400001", 0, 0, 100, 100);
        window.kind = CaptureTargetKind::Window;
        assert_eq!(
            target_selector_from_platform(&window),
            TargetSelector::Window {
                id: "0x400001".to_owned(),
            }
        );

        let mut display = window_target("2", 0, 0, 1920, 1080);
        display.kind = CaptureTargetKind::Display;
        assert_eq!(
            target_selector_from_platform(&display),
            TargetSelector::Display {
                id: "2".to_owned(),
            }
        );
    }

    #[test]
    fn assign_snapshot_ids_renumbers_blank_and_auto_ids_but_keeps_real_ones() {
        fn element(id: &str) -> ElementDescriptor {
            ElementDescriptor {
                id: id.to_owned(),
                role: "button".to_owned(),
                name: "OK".to_owned(),
                description: None,
                value: None,
                bounds: None,
                target: None,
                path: Vec::new(),
                actions: Vec::new(),
                app_name: None,
                process_id: None,
            }
        }

        let mut elements = vec![element("btn"), element("   "), element("auto:xyz")];
        assign_snapshot_ids(&mut elements);
        // A real id is preserved; a blank id and an auto: id are replaced with
        // their 1-based positions.
        assert_eq!(elements[0].id, "btn");
        assert_eq!(elements[1].id, "2");
        assert_eq!(elements[2].id, "3");
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

    #[test]
    fn expands_macos_display_targets_to_windows_on_that_display() {
        let display = display_target("2", 0, 0, 1000, 800);
        let left_window = window_target("left", 100, 100, 200, 200);
        let overlapping_window = window_target("overlap", 900, 100, 300, 200);
        let other_display_window = window_target("right", 1500, 100, 200, 200);
        let inventory = crate::platform::TargetInventory {
            targets: vec![
                display.clone(),
                left_window.clone(),
                overlapping_window.clone(),
                other_display_window,
            ],
        };
        let mut notes = Vec::new();

        let query_targets = macos_accessibility_query_targets(&inventory, &[display], &mut notes);

        assert_eq!(
            query_targets
                .iter()
                .map(|target| target.id.as_str())
                .collect::<Vec<_>>(),
            vec!["left", "overlap"]
        );
        assert!(notes.is_empty());
    }

    #[test]
    fn keeps_macos_window_targets_once_when_also_selected_by_a_display() {
        let display = display_target("2", 0, 0, 1000, 800);
        let window = window_target("left", 100, 100, 200, 200);
        let inventory = crate::platform::TargetInventory {
            targets: vec![display.clone(), window.clone()],
        };
        let mut notes = Vec::new();

        let query_targets =
            macos_accessibility_query_targets(&inventory, &[display, window], &mut notes);

        assert_eq!(query_targets.len(), 1);
        assert_eq!(query_targets[0].id, "left");
    }

    #[test]
    fn expands_display_scoped_native_window_element_backends_to_windows() {
        let display = display_target("display-1", 0, 0, 1000, 800);
        let window = window_target("window-1", 100, 100, 200, 200);
        let other = window_target("window-2", 1500, 100, 200, 200);
        let inventory = crate::platform::TargetInventory {
            targets: vec![display.clone(), window.clone(), other],
        };

        let query_targets = window_targets_for_scope(&inventory, &[display]);

        assert_eq!(query_targets.len(), 1);
        assert_eq!(query_targets[0].id, "window-1");
    }

    #[test]
    fn normalizes_atspi_roles_to_contract_taxonomy_names() {
        assert_eq!(normalize_atspi_role("push button"), "push_button");
        assert_eq!(normalize_atspi_role("ROLE_TEXT"), "text");
        assert_eq!(normalize_atspi_role("ATSPI_ROLE_MENU-ITEM"), "menu_item");
    }

    #[test]
    fn normalizes_atspi_actions_for_element_dsl() {
        assert_eq!(normalize_atspi_action("activate"), "press");
        assert_eq!(normalize_atspi_action("show menu"), "show_menu");
    }

    #[test]
    fn filters_atspi_elements_to_target_bounds_by_default() {
        let target_bounds = target().bounds;
        let visible = Bounds {
            x: 120,
            y: 220,
            width: 20,
            height: 20,
        };
        let offscreen = Bounds {
            x: 2_000,
            y: 2_000,
            width: 20,
            height: 20,
        };

        assert!(atspi_element_in_scope(
            Some(&visible),
            &target_bounds,
            false
        ));
        assert!(!atspi_element_in_scope(
            Some(&offscreen),
            &target_bounds,
            false
        ));
        assert!(atspi_element_in_scope(None, &target_bounds, true));
    }

    #[test]
    fn macos_jxa_script_interpolates_pid_bounds_and_cap() {
        // window_target -> process_id 42, bounds (100, 200, 800, 600).
        let target = window_target("0x400001", 100, 200, 800, 600);
        let script = macos_accessibility_listing_jxa(7531, &target, false);

        // Process id is interpolated into the AX application lookup path.
        assert!(
            script.contains("var pid = 7531;"),
            "script should interpolate the process id, got:\n{script}"
        );
        // Target bounds are interpolated into the intersect filter target.
        assert!(
            script.contains("x: 100")
                && script.contains("y: 200")
                && script.contains("width: 800")
                && script.contains("height: 600"),
            "script should interpolate target bounds, got:\n{script}"
        );
        // The element cap must match the Rust-side constant so the walk is bounded.
        assert!(
            script.contains("var maxElements = 512;"),
            "script should interpolate MAX_MACOS_ELEMENTS, got:\n{script}"
        );
        // Core ApplicationServices entry points the listing depends on.
        assert!(script.contains("ObjC.import('ApplicationServices');"));
        assert!(script.contains("$.AXUIElementCreateApplication(pid)"));
        assert!(script.contains("'AXRole'") && script.contains("'AXPosition'"));
        assert!(
            script.trim_end().ends_with("}());"),
            "script should be a self-invoking JXA program, got:\n{script}"
        );
    }

    #[test]
    fn macos_jxa_script_maps_include_offscreen_to_js_boolean_literal() {
        let target = window_target("0x400001", 0, 0, 10, 10);

        let on = macos_accessibility_listing_jxa(1, &target, true);
        assert!(
            on.contains("var includeOffscreen = true;"),
            "include_offscreen=true should map to JS literal true, got:\n{on}"
        );
        assert!(!on.contains("var includeOffscreen = false;"));

        let off = macos_accessibility_listing_jxa(1, &target, false);
        assert!(
            off.contains("var includeOffscreen = false;"),
            "include_offscreen=false should map to JS literal false, got:\n{off}"
        );
        assert!(!off.contains("var includeOffscreen = true;"));
    }

    #[test]
    fn split_geometry_offsets_requires_three_segments() {
        // The sign scan skips index 0, so a leading non-sign segment plus two
        // signed offsets yields three parts.
        assert_eq!(
            split_geometry_offsets("24+10+20"),
            Some(("24", "+10", "+20"))
        );
        // Negative offsets are captured with their sign intact.
        assert_eq!(
            split_geometry_offsets("24-10-20"),
            Some(("24", "-10", "-20"))
        );
        // Only two segments (a single leading sign + one offset) is rejected:
        // there is no second sign after index 0 of the remainder.
        assert_eq!(split_geometry_offsets("+110+220"), None);
        // A bare value with no signs is rejected.
        assert_eq!(split_geometry_offsets("800"), None);
    }

    #[test]
    fn parse_size_offset_reads_width_height_and_relative_offsets() {
        assert_eq!(parse_size_offset("80x24+10+20"), Some((80, 24, 10, 20)));
        // Negative relative offsets are preserved as signed values.
        assert_eq!(parse_size_offset("5x6-1-2"), Some((5, 6, -1, -2)));
        // A token without the `WxH` size prefix is not a size+offset token.
        assert_eq!(parse_size_offset("0+110+220"), None);
    }

    #[test]
    fn parse_offset_only_rejects_size_tokens_and_two_part_offsets() {
        // An offset-only token must have a non-sign leading segment plus two
        // signed offsets; `0+110+220` qualifies and yields the two offsets.
        assert_eq!(parse_offset_only("0+110+220"), Some((110, 220)));
        // Tokens containing `x` are size tokens, not pure offsets.
        assert_eq!(parse_offset_only("80x24+10+20"), None);
        // A leading-sign two-part token has no second sign and is rejected.
        assert_eq!(parse_offset_only("+110+220"), None);
    }

    #[test]
    fn bounds_overlap_is_true_only_for_real_intersection() {
        let base = Bounds {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        let overlapping = Bounds {
            x: 50,
            y: 50,
            width: 100,
            height: 100,
        };
        assert!(bounds_overlap(&base, &overlapping));
        assert!(bounds_overlap(&overlapping, &base));
        // Edge-touching rectangles (base right edge == other left edge) do not
        // count as overlapping.
        let edge_touch = Bounds {
            x: 100,
            y: 0,
            width: 10,
            height: 100,
        };
        assert!(!bounds_overlap(&base, &edge_touch));
        // Fully disjoint rectangles do not overlap.
        let disjoint = Bounds {
            x: 500,
            y: 500,
            width: 10,
            height: 10,
        };
        assert!(!bounds_overlap(&base, &disjoint));
    }

    #[test]
    fn parent_path_for_child_appends_unless_duplicate_tail() {
        let parent = vec!["root".to_owned(), "window".to_owned()];
        assert_eq!(
            parent_path_for_child(&parent, "button"),
            vec!["root", "window", "button"]
        );
        // If the child name already equals the last segment, it is not
        // appended again (avoids self-duplicating paths).
        assert_eq!(
            parent_path_for_child(&parent, "window"),
            vec!["root", "window"]
        );
        // Appending to an empty path always pushes the name.
        assert_eq!(parent_path_for_child(&[], "only"), vec!["only"]);
    }

    #[test]
    fn json_helpers_extract_typed_values_with_range_checks() {
        let value = serde_json::json!({
            "name": "widget",
            "x": -5,
            "width": 42,
            "huge": 5_000_000_000_i64,
            "negative_width": -1,
        });
        assert_eq!(json_str(&value, "name"), Some("widget"));
        assert_eq!(json_str(&value, "missing"), None);
        // i32 accepts negatives; u32 does not.
        assert_eq!(json_i32(&value, "x"), Some(-5));
        assert_eq!(json_u32(&value, "width"), Some(42));
        assert_eq!(json_u32(&value, "negative_width"), None);
        // Out-of-range integers fail the checked conversion and return None.
        assert_eq!(json_i32(&value, "huge"), None);
        assert_eq!(json_u32(&value, "huge"), None);
        // A string field is not a number.
        assert_eq!(json_i32(&value, "name"), None);
    }
}
