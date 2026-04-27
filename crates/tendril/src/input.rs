use std::time::Duration;

use serde_json::json;

use crate::error::TendrilError;
use crate::model::{
    Bounds, CoordinateTransform, InputAction, MAX_SCROLL_TICKS, ModifierKey, MouseButton, RunInput,
    RunInputPayload, RunOutput, TargetSelector,
};
use crate::platform::{
    AdapterInfo, CaptureTargetKind, DesktopSession, InputRequest as PlatformInputRequest,
    PlatformAdapter, PlatformKind, TargetDescriptor as PlatformTargetDescriptor,
    TargetDiscoveryRequest,
};

const RELIABILITY_DELAY_MS: u64 = 20;

pub(crate) fn parse_input_definition(input: &str) -> Result<RunInputPayload, TendrilError> {
    if let Some(offset) = top_level_semicolon_offset(input) {
        return Err(dsl_error(
            "unexpected `;`; the DSL separator is `,`",
            None,
            None,
            Some("parse"),
        )
        .with_detail_entry("offset", json!(offset)));
    }

    if input.contains('(') {
        return Ok(RunInputPayload::Actions {
            actions: parse_dsl_sequence(input)?,
        });
    }

    if contains_top_level_comma(input) && looks_like_bare_key_sequence(input) {
        if let Ok(actions) = parse_dsl_sequence(input) {
            return Ok(RunInputPayload::Actions { actions });
        }
    }

    if let Some(error) = ambiguous_single_dsl_like_input_error(input) {
        return Err(error);
    }

    Ok(RunInputPayload::Text {
        text: input.to_owned(),
    })
}

pub(crate) fn parse_dsl_sequence(sequence: &str) -> Result<Vec<InputAction>, TendrilError> {
    let trimmed = sequence.trim();
    if trimmed.is_empty() {
        return Err(dsl_error(
            "dsl sequence cannot be empty",
            None,
            None,
            Some("parse"),
        ));
    }

    let segments = split_top_level(trimmed)?;
    let mut actions = Vec::with_capacity(segments.len());
    for (action_index, segment) in segments.iter().enumerate() {
        let action = parse_action_segment(segment, action_index)?;
        actions.push(action);
    }

    Ok(actions)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn remap_output_point_to_source(
    transform: &CoordinateTransform,
    x: i32,
    y: i32,
) -> (i32, i32) {
    (
        scaled_coordinate(x, transform.x_numerator, transform.x_denominator),
        scaled_coordinate(y, transform.y_numerator, transform.y_denominator),
    )
}

pub(crate) fn relative_point_to_absolute(bounds: &Bounds, x: i32, y: i32) -> (i32, i32) {
    (bounds.x.saturating_add(x), bounds.y.saturating_add(y))
}

pub(crate) fn execute_run(
    input: &RunInput,
    adapter: &dyn PlatformAdapter,
) -> Result<RunOutput, TendrilError> {
    let target = resolve_target(input, adapter)?;
    // Probe adapter-level input support first so platform-specific diagnostics
    // (for example the Wayland missing-backend error that names `ydotool` and
    // `wtype`) surface before the generic per-target capability check, which
    // would otherwise mask the actionable remediation guidance with
    // `input_not_supported_for_target` (bd-da01d3).
    adapter.input_support().map_err(TendrilError::from)?;
    ensure_input_supported(&target)?;

    let adapter_info = adapter.info();
    let (text, actions) = normalize_payload(&input.payload);
    validate_actions_for_target(&target, &actions)?;
    reject_unsafe_browser_navigation_chord(&adapter_info, &target, &actions)?;

    let outcome = adapter.execute_input(&PlatformInputRequest {
        target_id: target.id.clone(),
        target: target.kind,
        target_name: target.name.clone(),
        bounds: target.bounds.clone(),
        app_name: target.app_name.clone(),
        process_id: target.process_id,
        restore_focus: input.restore_focus,
        text,
        actions: actions.clone(),
    })?;

    Ok(RunOutput {
        adapter: adapter_info,
        target: input.target.clone(),
        focus_required: outcome.focus_required,
        focus_transferred: outcome.focus_transferred,
        action_count: outcome.action_count,
        focused_target: outcome.focused_target,
        previous_focus: outcome.previous_focus,
        focus_restored: outcome.focus_restored,
        pointer_restored: outcome.pointer_restored,
        restore_error: outcome.restore_error,
        notes: outcome.notes,
        execution_lock: None,
    })
}

pub(crate) fn render_run_human(output: &RunOutput) -> String {
    let notes = if output.notes.is_empty() {
        String::from("none")
    } else {
        output.notes.join(" ")
    };

    format!(
        "run target: {:?} {}\nplatform: {:?} / {:?}\naction_count: {}\nfocus_required: {}\nfocus_transferred: {}\nfocused_target: {}\nprevious_focus: {}\nfocus_restored: {}\npointer_restored: {}\nrestore_error: {}\nexecution_lock: {}\nnotes: {}\n",
        output.target.kind(),
        output.target.id(),
        output.adapter.platform,
        output.adapter.session,
        output.action_count,
        output.focus_required,
        output.focus_transferred,
        output.focused_target.as_deref().unwrap_or("<none>"),
        output
            .previous_focus
            .as_ref()
            .map_or("<none>", |focus| focus.id.as_str()),
        output.focus_restored,
        output.pointer_restored,
        output.restore_error.as_deref().unwrap_or("<none>"),
        render_execution_lock_summary(output.execution_lock.as_ref()),
        notes,
    )
}

pub(crate) fn reliability_delay() -> Duration {
    Duration::from_millis(RELIABILITY_DELAY_MS)
}

fn render_execution_lock_summary(
    report: Option<&crate::execution_lock::ExecutionLockReport>,
) -> String {
    report.map_or_else(
        || "<not reported>".to_owned(),
        |report| {
            format!(
                "enabled={} acquired={} wait_ms={} queue_position_at_join={} queue_depth_at_join={} stale_locks_reaped={} stale_tickets_reaped={}",
                report.enabled,
                report.acquired,
                report.wait_ms,
                report.queue_position_at_join,
                report.queue_depth_at_join,
                report.stale_locks_reaped,
                report.stale_tickets_reaped,
            )
        },
    )
}

fn normalize_payload(payload: &RunInputPayload) -> (Option<String>, Vec<InputAction>) {
    match payload {
        RunInputPayload::Text { text } => (Some(text.clone()), Vec::new()),
        RunInputPayload::Dsl { sequence } => {
            (None, parse_dsl_sequence(sequence).unwrap_or_default())
        }
        RunInputPayload::Actions { actions } => (None, actions.clone()),
    }
}

fn resolve_target(
    input: &RunInput,
    adapter: &dyn PlatformAdapter,
) -> Result<PlatformTargetDescriptor, TendrilError> {
    let inventory = adapter.discover_targets(&TargetDiscoveryRequest)?;
    inventory
        .targets
        .into_iter()
        .find(|target| {
            target.id == input.target.id() && matches_target_kind(&input.target, target.kind)
        })
        .ok_or_else(|| {
            TendrilError::target_not_found(
                match input.target.kind() {
                    crate::model::TargetKind::Window => "window",
                    crate::model::TargetKind::Display => "display",
                    crate::model::TargetKind::AudioSource => "audio_source",
                },
                input.target.id(),
            )
        })
}

fn reject_unsafe_browser_navigation_chord(
    adapter_info: &AdapterInfo,
    target: &PlatformTargetDescriptor,
    actions: &[InputAction],
) -> Result<(), TendrilError> {
    if adapter_info.platform != PlatformKind::Linux
        || adapter_info.session != DesktopSession::X11
        || target.kind != CaptureTargetKind::Window
        || !looks_like_browser_target(target)
    {
        return Ok(());
    }

    let Some((send_action_index, navigation_text)) = vulnerable_ctrl_l_navigation(actions) else {
        return Ok(());
    };

    Err(TendrilError::validation(format!(
        "refusing X11 browser navigation through Ctrl+L followed by navigation text `{}` because synthetic browser-chrome shortcuts can stay in the focused page control (observed with Firefox page inputs); use capture -> click the visible address bar -> Ctrl+A -> send URL/path -> Return -> recapture/verify instead",
        summarize_navigation_text(navigation_text)
    ))
    .with_code("invalid_run_input")
    .with_field("input_definition")
    .with_detail_entry("stage", json!("browser_navigation_preflight"))
    .with_detail_entry("pattern", json!("x11_browser_ctrl_l_url_send"))
    .with_detail_entry("target_id", json!(target.id))
    .with_detail_entry("target_name", json!(target.name))
    .with_detail_entry("target_app_name", json!(target.app_name))
    .with_detail_entry("target_title", json!(target.title))
    .with_detail_entry("action_index", json!(send_action_index))
    .with_detail_entry("url_preview", json!(summarize_navigation_text(navigation_text)))
    .with_detail_entry(
        "navigation_text_preview",
        json!(summarize_navigation_text(navigation_text)),
    )
    .with_detail_entry(
        "remediation",
        json!("Do not rely on Ctrl+L/Cmd+L for browser navigation on X11 Firefox when a page input may already be focused. Capture the browser, click the visible address bar coordinates, run hold(ctrl),a,release(ctrl),send(\"URL_OR_PATH\"),Return, then recapture and verify the page changed before continuing."),
    ))
}

fn looks_like_browser_target(target: &PlatformTargetDescriptor) -> bool {
    [
        target.name.as_str(),
        target.title.as_deref().unwrap_or_default(),
        target.app_name.as_deref().unwrap_or_default(),
    ]
    .into_iter()
    .map(str::to_ascii_lowercase)
    .any(|value| {
        [
            "firefox",
            "mozilla",
            "chromium",
            "chrome",
            "google-chrome",
            "browser",
            "brave",
            "edge",
        ]
        .into_iter()
        .any(|token| value.contains(token))
    })
}

fn vulnerable_ctrl_l_navigation(actions: &[InputAction]) -> Option<(usize, &str)> {
    let significant_actions = actions
        .iter()
        .enumerate()
        .filter(|(_, action)| !matches!(action, InputAction::Wait { .. }))
        .collect::<Vec<_>>();
    if significant_actions.len() < 3 {
        return None;
    }

    for start in 0..=significant_actions.len() - 3 {
        let (_, first) = significant_actions[start];
        let (_, second) = significant_actions[start + 1];
        let (_, third) = significant_actions[start + 2];
        if !(is_ctrl_hold(first) && is_key(second, "l") && is_ctrl_release(third)) {
            continue;
        }

        let mut pending_url_send = None;
        for (action_index, action) in significant_actions.iter().skip(start + 3).copied() {
            match action {
                InputAction::Click { .. }
                | InputAction::Drag { .. }
                | InputAction::Scroll { .. } => break,
                InputAction::Send { text } if looks_like_navigation_text(text) => {
                    pending_url_send = Some((action_index, text.as_str()));
                }
                _ if is_return_key(action) => {
                    if let Some((send_index, navigation_text)) = pending_url_send {
                        return Some((send_index, navigation_text));
                    }
                }
                _ => {}
            }
        }
    }

    None
}

fn is_ctrl_hold(action: &InputAction) -> bool {
    matches!(
        action,
        InputAction::Hold {
            modifier: ModifierKey::Ctrl
        }
    )
}

fn is_ctrl_release(action: &InputAction) -> bool {
    matches!(
        action,
        InputAction::Release {
            modifier: ModifierKey::Ctrl
        }
    )
}

fn is_key(action: &InputAction, expected: &str) -> bool {
    matches!(action, InputAction::KeyTap { key } if key.eq_ignore_ascii_case(expected))
}

fn is_return_key(action: &InputAction) -> bool {
    matches!(action, InputAction::KeyTap { key } if matches!(key.as_str(), "return" | "enter"))
}

fn looks_like_navigation_text(text: &str) -> bool {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    lower.contains("://")
        || lower.starts_with("about:")
        || lower.starts_with("chrome:")
        || lower.starts_with("edge:")
        || lower.starts_with("localhost:")
        || lower.starts_with("127.0.0.1:")
        || looks_like_absolute_filesystem_path(trimmed)
}

fn looks_like_absolute_filesystem_path(text: &str) -> bool {
    // Linux/X11 browsers accept absolute POSIX paths such as `/tmp/file.txt`
    // in the address bar and navigate to them as local filesystem targets.
    is_absolute_unix_path(text)
        || is_absolute_windows_drive_path(text)
        || is_absolute_windows_unc_path(text)
}

fn is_absolute_unix_path(text: &str) -> bool {
    text.starts_with('/')
}

fn is_absolute_windows_drive_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

fn is_absolute_windows_unc_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 5 || !matches!(bytes[0], b'\\' | b'/') || !matches!(bytes[1], b'\\' | b'/') {
        return false;
    }

    let mut components = text[2..]
        .split(['\\', '/'])
        .filter(|component| !component.is_empty());
    components.next().is_some() && components.next().is_some()
}

fn summarize_navigation_text(text: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 96;
    let mut preview = text.chars().take(MAX_PREVIEW_CHARS).collect::<String>();
    if text.chars().count() > MAX_PREVIEW_CHARS {
        preview.push('…');
    }
    preview
}

fn ensure_input_supported(target: &PlatformTargetDescriptor) -> Result<(), TendrilError> {
    if target.input_supported {
        Ok(())
    } else {
        Err(TendrilError::unsupported_capability(
            "input_not_supported_for_target",
            format!("target `{}` does not support input execution", target.id),
            Some(json!({
                "target_id": target.id,
                "target_kind": target.kind,
            })),
        ))
    }
}

fn matches_target_kind(target: &TargetSelector, platform_kind: CaptureTargetKind) -> bool {
    matches!(
        (target, platform_kind),
        (TargetSelector::Window { .. }, CaptureTargetKind::Window)
            | (TargetSelector::Display { .. }, CaptureTargetKind::Display)
    )
}

fn validate_actions_for_target(
    target: &PlatformTargetDescriptor,
    actions: &[InputAction],
) -> Result<(), TendrilError> {
    for (action_index, action) in actions.iter().enumerate() {
        match action {
            InputAction::KeyTap { .. }
            | InputAction::Hold { .. }
            | InputAction::Release { .. }
            | InputAction::Send { .. }
            | InputAction::Wait { .. } => {}
            InputAction::Click { x, y, .. } => {
                validate_relative_point(*x, *y, &target.bounds, action_index, "click")?;
            }
            InputAction::Drag { x0, y0, x1, y1 } => {
                validate_relative_point(*x0, *y0, &target.bounds, action_index, "drag_start")?;
                validate_relative_point(*x1, *y1, &target.bounds, action_index, "drag_end")?;
            }
            InputAction::Scroll { x, y, .. } => {
                validate_relative_point(*x, *y, &target.bounds, action_index, "scroll")?;
            }
        }
    }

    Ok(())
}

fn validate_relative_point(
    x: i32,
    y: i32,
    bounds: &Bounds,
    action_index: usize,
    field: &'static str,
) -> Result<(), TendrilError> {
    let width = i32::try_from(bounds.width).unwrap_or(i32::MAX);
    let height = i32::try_from(bounds.height).unwrap_or(i32::MAX);
    if x < 0 || y < 0 || x >= width || y >= height {
        return Err(TendrilError::validation(format!(
            "{field} coordinates ({x}, {y}) are outside target bounds {}x{}",
            bounds.width, bounds.height
        ))
        .with_code("invalid_run_input")
        .with_detail_entry("stage", json!("validate"))
        .with_detail_entry("field", json!(field))
        .with_detail_entry("action_index", json!(action_index))
        .with_detail_entry("action_number", json!(action_index + 1)));
    }

    Ok(())
}

fn top_level_semicolon_offset(input: &str) -> Option<usize> {
    let mut in_string = false;
    let mut escape = false;
    let mut depth = 0_u32;

    for (index, character) in input.char_indices() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            match character {
                '\\' => escape = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match character {
            '"' => in_string = true,
            '(' => depth = depth.saturating_add(1),
            ')' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => return Some(index),
            _ => {}
        }
    }

    None
}

fn contains_top_level_comma(input: &str) -> bool {
    let mut in_string = false;
    let mut escape = false;
    let mut depth = 0_u32;

    for character in input.chars() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            match character {
                '\\' => escape = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match character {
            '"' => in_string = true,
            '(' => depth = depth.saturating_add(1),
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return true,
            _ => {}
        }
    }

    false
}

fn looks_like_bare_key_sequence(input: &str) -> bool {
    let mut start = 0_usize;
    let mut in_string = false;
    let mut escape = false;
    let mut depth = 0_i32;

    for (index, character) in input.char_indices() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            match character {
                '\\' => escape = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match character {
            '"' => in_string = true,
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                let raw = &input[start..index];
                if raw != raw.trim() || parse_key_token(raw).is_none() {
                    return false;
                }
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }

    let raw = &input[start..];
    raw == raw.trim() && parse_key_token(raw).is_some()
}

fn ambiguous_single_dsl_like_input_error(input: &str) -> Option<TendrilError> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.contains('(') || contains_top_level_comma(trimmed) {
        return None;
    }

    if is_known_bare_key_token(trimmed) {
        return Some(
            dsl_error(
                format!(
                    "ambiguous run input `{trimmed}` looks like a DSL key token; refusing to type it as literal text"
                ),
                None,
                Some(trimmed),
                Some("parse"),
            )
            .with_detail_entry("hint", json!(bare_key_token_hint(trimmed)))
            .with_detail_entry("ambiguous_token", json!(trimmed))
            .with_detail_entry("ambiguity", json!("bare_key_token")),
        );
    }

    if let Some((verb, hint)) = reserved_verb_without_parens_hint(trimmed) {
        return Some(
            dsl_error(
                format!(
                    "ambiguous run input `{trimmed}` looks like a DSL action missing parentheses; refusing to type it as literal text"
                ),
                None,
                Some(trimmed),
                Some("parse"),
            )
            .with_detail_entry("hint", json!(hint))
            .with_detail_entry("verb", json!(verb))
            .with_detail_entry("ambiguity", json!("reserved_verb_without_parens")),
        );
    }

    None
}

fn is_known_bare_key_token(input: &str) -> bool {
    let lower = input.trim().to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11" | "f12"
    ) {
        return true;
    }

    matches!(
        lower.as_str(),
        "enter"
            | "return"
            | "tab"
            | "esc"
            | "escape"
            | "space"
            | "backspace"
            | "insert"
            | "ins"
            | "delete"
            | "del"
            | "left"
            | "right"
            | "up"
            | "down"
            | "home"
            | "end"
            | "pageup"
            | "pagedown"
            | "page-up"
            | "page-down"
            | "page_up"
            | "page_down"
            | "pgup"
            | "pgdn"
    )
}

fn bare_key_token_hint(token: &str) -> String {
    let literal = serde_json::to_string(token).unwrap_or_else(|_| String::from("\"...\""));
    format!(
        "Use a DSL sequence containing a comma for key taps, for example `wait(1ms),{token}`. If you intended to type the literal text, use `send({literal})`."
    )
}

fn reserved_verb_without_parens_hint(input: &str) -> Option<(&'static str, String)> {
    let trimmed = input.trim();
    let split_at = trimmed
        .char_indices()
        .find_map(|(index, character)| character.is_whitespace().then_some(index))?;
    let verb = &trimmed[..split_at];
    let rest = trimmed[split_at..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }

    let canonical_verb = match verb.to_ascii_lowercase().as_str() {
        "type" | "send" => "send",
        "key" => "key",
        "click" => "click",
        "scroll" => "scroll",
        _ => return None,
    };

    let quoted_hint = parse_quoted_string(rest, 0, input).ok().map_or_else(
        || String::from("\"...\""),
        |text| serde_json::to_string(&text).unwrap_or_else(|_| String::from("\"...\"")),
    );
    let hint = if canonical_verb == "send" {
        format!(
            "Use Tendril's canonical text action as `send({quoted_hint})`; for example `send({quoted_hint}),Return` when text should be followed by Return."
        )
    } else {
        format!(
            "Tendril DSL actions require parentheses. Use `send({quoted_hint})` for text, or rewrite the `{canonical_verb}` action with explicit Tendril DSL syntax."
        )
    };

    Some((canonical_verb, hint))
}

fn split_top_level(input: &str) -> Result<Vec<&str>, TendrilError> {
    let mut parts = Vec::new();
    let mut start = 0_usize;
    let mut in_string = false;
    let mut escape = false;
    let mut depth = 0_i32;

    for (index, character) in input.char_indices() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            match character {
                '\\' => escape = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match character {
            '"' => in_string = true,
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Err(dsl_error(
                        "unexpected `)` in input sequence",
                        None,
                        None,
                        Some("parse"),
                    )
                    .with_detail_entry("offset", json!(index)));
                }
                depth -= 1;
            }
            ';' if depth == 0 => {
                return Err(dsl_error(
                    "unexpected `;`; the DSL separator is `,`",
                    Some(parts.len()),
                    None,
                    Some("parse"),
                )
                .with_detail_entry("offset", json!(index)));
            }
            ',' if depth == 0 => {
                let part = input[start..index].trim();
                if part.is_empty() {
                    return Err(dsl_error(
                        "empty action in DSL sequence",
                        Some(parts.len()),
                        None,
                        Some("parse"),
                    )
                    .with_detail_entry("offset", json!(index)));
                }
                parts.push(part);
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }

    if in_string {
        return Err(dsl_error(
            "unterminated string literal in input sequence",
            None,
            None,
            Some("parse"),
        ));
    }

    if depth != 0 {
        return Err(dsl_error(
            "unbalanced parentheses in input sequence",
            None,
            None,
            Some("parse"),
        ));
    }

    let tail = input[start..].trim();
    if tail.is_empty() {
        return Err(dsl_error(
            "empty action in DSL sequence",
            Some(parts.len()),
            None,
            Some("parse"),
        ));
    }
    parts.push(tail);

    Ok(parts)
}

#[allow(clippy::too_many_lines)]
fn parse_action_segment(segment: &str, action_index: usize) -> Result<InputAction, TendrilError> {
    if !segment.contains('(') {
        let key = parse_key_token(segment).ok_or_else(|| {
            dsl_error(
                format!("invalid key tap `{segment}`"),
                Some(action_index),
                Some(segment),
                Some("parse"),
            )
        })?;
        return Ok(InputAction::KeyTap { key });
    }

    let open = segment.find('(').ok_or_else(|| {
        dsl_error(
            format!("invalid DSL action `{segment}`"),
            Some(action_index),
            Some(segment),
            Some("parse"),
        )
    })?;
    let close = segment.rfind(')').ok_or_else(|| {
        dsl_error(
            format!("missing closing `)` in `{segment}`"),
            Some(action_index),
            Some(segment),
            Some("parse"),
        )
    })?;

    if !segment[close + 1..].trim().is_empty() {
        return Err(dsl_error(
            format!("unexpected trailing input after `{segment}`"),
            Some(action_index),
            Some(segment),
            Some("parse"),
        ));
    }

    let name = segment[..open].trim().to_ascii_lowercase();
    let inner = &segment[open + 1..close];

    match name.as_str() {
        "hold" => {
            let argument = expect_single_argument(inner, action_index, segment)?;
            Ok(InputAction::Hold {
                modifier: parse_modifier(argument, action_index, segment)?,
            })
        }
        "release" => {
            let argument = expect_single_argument(inner, action_index, segment)?;
            Ok(InputAction::Release {
                modifier: parse_modifier(argument, action_index, segment)?,
            })
        }
        "send" => {
            let argument = expect_single_argument(inner, action_index, segment)?;
            let text = parse_quoted_string(argument, action_index, segment)?;
            if text.is_empty() {
                return Err(dsl_error(
                    "send(\"...\") requires a non-empty string",
                    Some(action_index),
                    Some(segment),
                    Some("validate"),
                ));
            }
            Ok(InputAction::Send { text })
        }
        "wait" => {
            let argument = expect_single_argument(inner, action_index, segment)?;
            Ok(InputAction::Wait {
                duration_ms: parse_duration_ms(argument, action_index, segment)?,
            })
        }
        "lclick" | "rclick" | "mclick" => {
            let arguments = split_arguments(inner, action_index, segment)?;
            if arguments.len() != 2 {
                return Err(dsl_error(
                    format!("`{name}` expects exactly two coordinates"),
                    Some(action_index),
                    Some(segment),
                    Some("parse"),
                ));
            }
            Ok(InputAction::Click {
                button: match name.as_str() {
                    "lclick" => MouseButton::Left,
                    "rclick" => MouseButton::Right,
                    _ => MouseButton::Middle,
                },
                x: parse_i32(arguments[0], action_index, segment, "x")?,
                y: parse_i32(arguments[1], action_index, segment, "y")?,
            })
        }
        "drag" => {
            let arguments = split_arguments(inner, action_index, segment)?;
            if arguments.len() != 4 {
                return Err(dsl_error(
                    "`drag` expects exactly four coordinates",
                    Some(action_index),
                    Some(segment),
                    Some("parse"),
                ));
            }
            Ok(InputAction::Drag {
                x0: parse_i32(arguments[0], action_index, segment, "x0")?,
                y0: parse_i32(arguments[1], action_index, segment, "y0")?,
                x1: parse_i32(arguments[2], action_index, segment, "x1")?,
                y1: parse_i32(arguments[3], action_index, segment, "y1")?,
            })
        }
        "scroll" => {
            let arguments = split_arguments(inner, action_index, segment)?;
            if arguments.len() != 3 {
                return Err(dsl_error(
                    "`scroll` expects exactly x, y, and dy arguments",
                    Some(action_index),
                    Some(segment),
                    Some("parse"),
                ));
            }
            Ok(InputAction::Scroll {
                x: parse_i32(arguments[0], action_index, segment, "x")?,
                y: parse_i32(arguments[1], action_index, segment, "y")?,
                dy: parse_scroll_delta(arguments[2], action_index, segment)?,
            })
        }
        _ => Err(dsl_error(
            format!("unknown DSL action `{name}`"),
            Some(action_index),
            Some(segment),
            Some("parse"),
        )),
    }
}

fn expect_single_argument<'a>(
    inner: &'a str,
    action_index: usize,
    segment: &str,
) -> Result<&'a str, TendrilError> {
    let arguments = split_arguments(inner, action_index, segment)?;
    if arguments.len() != 1 {
        return Err(dsl_error(
            format!("`{segment}` expects exactly one argument"),
            Some(action_index),
            Some(segment),
            Some("parse"),
        ));
    }
    Ok(arguments[0])
}

fn split_arguments<'a>(
    input: &'a str,
    action_index: usize,
    segment: &str,
) -> Result<Vec<&'a str>, TendrilError> {
    let mut parts = Vec::new();
    let mut start = 0_usize;
    let mut in_string = false;
    let mut escape = false;
    let mut depth = 0_i32;

    for (index, character) in input.char_indices() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            match character {
                '\\' => escape = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match character {
            '"' => in_string = true,
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(input[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }

    if in_string {
        return Err(dsl_error(
            "unterminated string literal in action arguments",
            Some(action_index),
            Some(segment),
            Some("parse"),
        ));
    }

    parts.push(input[start..].trim());

    if parts.iter().any(|part| part.is_empty()) {
        return Err(dsl_error(
            "action arguments cannot be empty",
            Some(action_index),
            Some(segment),
            Some("parse"),
        ));
    }

    Ok(parts)
}

fn parse_modifier(
    input: &str,
    action_index: usize,
    segment: &str,
) -> Result<ModifierKey, TendrilError> {
    match input.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Ok(ModifierKey::Ctrl),
        "alt" | "option" => Ok(ModifierKey::Alt),
        "shift" => Ok(ModifierKey::Shift),
        "meta" | "cmd" | "command" | "super" | "win" | "windows" => Ok(ModifierKey::Meta),
        other => Err(dsl_error(
            format!("unsupported modifier `{other}`"),
            Some(action_index),
            Some(segment),
            Some("parse"),
        )),
    }
}

fn parse_key_token(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        return None;
    }

    if trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '+'))
    {
        Some(trimmed.to_ascii_lowercase())
    } else {
        None
    }
}

fn parse_quoted_string(
    input: &str,
    action_index: usize,
    segment: &str,
) -> Result<String, TendrilError> {
    let trimmed = input.trim();
    if !(trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2) {
        return Err(dsl_error(
            "send(...) expects a double-quoted string literal",
            Some(action_index),
            Some(segment),
            Some("parse"),
        ));
    }

    let mut result = String::new();
    let mut escape = false;
    for character in trimmed[1..trimmed.len() - 1].chars() {
        if escape {
            match character {
                '"' => result.push('"'),
                '\\' => result.push('\\'),
                'n' => result.push('\n'),
                'r' => result.push('\r'),
                't' => result.push('\t'),
                other => {
                    return Err(dsl_error(
                        format!("unsupported escape `\\{other}` in string literal"),
                        Some(action_index),
                        Some(segment),
                        Some("parse"),
                    ));
                }
            }
            escape = false;
            continue;
        }

        if character == '\\' {
            escape = true;
        } else {
            result.push(character);
        }
    }

    if escape {
        return Err(dsl_error(
            "unterminated escape sequence in string literal",
            Some(action_index),
            Some(segment),
            Some("parse"),
        ));
    }

    Ok(result)
}

fn parse_duration_ms(input: &str, action_index: usize, segment: &str) -> Result<u64, TendrilError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(dsl_error(
            "wait(...) requires a duration",
            Some(action_index),
            Some(segment),
            Some("parse"),
        ));
    }

    let (number, unit) = if let Some(value) = trimmed.strip_suffix("ms") {
        (value.trim(), "ms")
    } else if let Some(value) = trimmed.strip_suffix('s') {
        (value.trim(), "s")
    } else {
        (trimmed, "ms")
    };

    if number.is_empty() {
        return Err(dsl_error(
            "wait(...) requires a numeric duration",
            Some(action_index),
            Some(segment),
            Some("parse"),
        ));
    }

    let millis = if unit == "ms" {
        number.parse::<u64>().map_err(|_| {
            dsl_error(
                format!("invalid millisecond duration `{trimmed}`"),
                Some(action_index),
                Some(segment),
                Some("parse"),
            )
        })?
    } else {
        let seconds = number.parse::<f64>().map_err(|_| {
            dsl_error(
                format!("invalid second duration `{trimmed}`"),
                Some(action_index),
                Some(segment),
                Some("parse"),
            )
        })?;
        if !seconds.is_finite() || seconds <= 0.0 {
            return Err(dsl_error(
                format!("duration `{trimmed}` must be greater than zero"),
                Some(action_index),
                Some(segment),
                Some("validate"),
            ));
        }
        u64::try_from(Duration::from_secs_f64(seconds).as_millis()).map_err(|_| {
            dsl_error(
                format!("duration `{trimmed}` is too large"),
                Some(action_index),
                Some(segment),
                Some("validate"),
            )
        })?
    };

    if millis == 0 {
        return Err(dsl_error(
            format!("duration `{trimmed}` must be greater than zero"),
            Some(action_index),
            Some(segment),
            Some("validate"),
        ));
    }

    Ok(millis)
}

fn parse_i32(
    input: &str,
    action_index: usize,
    segment: &str,
    field: &'static str,
) -> Result<i32, TendrilError> {
    input.trim().parse::<i32>().map_err(|_| {
        dsl_error(
            format!("{field} must be an integer in `{segment}`"),
            Some(action_index),
            Some(segment),
            Some("parse"),
        )
    })
}

fn parse_scroll_delta(
    input: &str,
    action_index: usize,
    segment: &str,
) -> Result<i32, TendrilError> {
    let dy = parse_i32(input, action_index, segment, "dy")?;
    if dy == 0 {
        return Err(dsl_error(
            "scroll dy must be non-zero",
            Some(action_index),
            Some(segment),
            Some("validate"),
        ));
    }
    if dy.unsigned_abs() > MAX_SCROLL_TICKS {
        return Err(dsl_error(
            format!(
                "scroll dy must be between -{MAX_SCROLL_TICKS} and {MAX_SCROLL_TICKS} wheel ticks"
            ),
            Some(action_index),
            Some(segment),
            Some("validate"),
        ));
    }
    Ok(dy)
}

fn dsl_error(
    message: impl Into<String>,
    action_index: Option<usize>,
    action: Option<&str>,
    stage: Option<&str>,
) -> TendrilError {
    let mut error = TendrilError::validation(message)
        .with_code("invalid_run_input")
        .with_field("input_definition");
    if let Some(action_index) = action_index {
        error = error
            .with_detail_entry("action_index", json!(action_index))
            .with_detail_entry("action_number", json!(action_index + 1));
    }
    if let Some(action) = action {
        error = error.with_detail_entry("action", json!(action));
    }
    if let Some(stage) = stage {
        error = error.with_detail_entry("stage", json!(stage));
    }
    error
}

#[cfg_attr(not(test), allow(dead_code))]
fn scaled_coordinate(value: i32, numerator: u32, denominator: u32) -> i32 {
    if denominator == 0 {
        return value;
    }

    let scaled = (i64::from(value) * i64::from(numerator) + i64::from(denominator / 2))
        / i64::from(denominator);
    i32::try_from(scaled).unwrap_or(if scaled.is_negative() {
        i32::MIN
    } else {
        i32::MAX
    })
}

#[cfg(test)]
mod tests {
    use super::{
        looks_like_navigation_text, parse_dsl_sequence, parse_input_definition,
        reject_unsafe_browser_navigation_chord, relative_point_to_absolute,
        remap_output_point_to_source,
    };
    use crate::model::{
        Bounds, CoordinateTransform, InputAction, ModifierKey, MouseButton, RunInputPayload,
        ScaleFactor,
    };
    use crate::platform::{
        AdapterInfo, AudioBackend, CaptureTargetKind, DesktopSession, PlatformKind,
        TargetDescriptor,
    };
    use proptest::prelude::*;

    #[test]
    fn x11_browser_ctrl_l_url_navigation_is_rejected_with_remediation() {
        let actions = parse_dsl_sequence(
            r#"hold(ctrl),l,release(ctrl),send("file:///tmp/mouse-buttons-task.html"),Return,wait(1000ms),lclick(70,150)"#,
        )
        .expect("dsl should parse");
        let error = reject_unsafe_browser_navigation_chord(
            &x11_adapter_info(),
            &browser_target(
                "0x600016",
                "firefox",
                Some("Tendril Smoke Browser — Mozilla Firefox"),
            ),
            &actions,
        )
        .expect_err("unsafe browser navigation chord should be rejected");

        assert_eq!(error.code(), "invalid_run_input");
        let details = error.details().expect("details");
        assert_eq!(details["stage"], "browser_navigation_preflight");
        assert_eq!(details["pattern"], "x11_browser_ctrl_l_url_send");
        assert_eq!(details["action_index"], 3);
        let remediation = details["remediation"].as_str().expect("remediation");
        assert!(
            remediation.contains("click the visible address bar")
                && remediation.contains("recapture and verify"),
            "unexpected remediation: {remediation}"
        );

        let ctrl_a_after_ctrl_l = parse_dsl_sequence(
            r#"hold(ctrl),l,release(ctrl),hold(ctrl),a,release(ctrl),send("https://example.com"),Return"#,
        )
        .expect("dsl should parse");
        reject_unsafe_browser_navigation_chord(
            &x11_adapter_info(),
            &browser_target("0x600016", "firefox", Some("Mozilla Firefox")),
            &ctrl_a_after_ctrl_l,
        )
        .expect_err("Ctrl+A after an unsafe Ctrl+L should still be rejected before URL text");
    }

    #[test]
    fn x11_browser_ctrl_l_absolute_filesystem_paths_are_rejected() {
        let browser = browser_target("0x600016", "firefox", Some("Mozilla Firefox"));

        for path in [
            "/home/harry/upload-proof.txt",
            " /tmp/folder with spaces/upload-proof.txt ",
            "/Users/alice/upload-proof.txt",
        ] {
            let actions = parse_dsl_sequence(&format!(
                r#"hold(ctrl),l,release(ctrl),send("{path}"),Return"#
            ))
            .expect("dsl should parse");
            let error =
                reject_unsafe_browser_navigation_chord(&x11_adapter_info(), &browser, &actions)
                    .expect_err("absolute filesystem path navigation should be rejected");

            assert_eq!(error.code(), "invalid_run_input");
            let details = error.details().expect("details");
            assert_eq!(details["action_index"], 3);
            assert_eq!(details["url_preview"], path);
        }

        for path in [
            r"C:\Users\agent\Desktop\upload-proof.txt",
            "C:/Users/agent/Desktop/upload-proof.txt",
            r"\\server\share\upload-proof.txt",
        ] {
            let actions = ctrl_l_send_return_actions(path);
            assert!(
                reject_unsafe_browser_navigation_chord(&x11_adapter_info(), &browser, &actions)
                    .is_err(),
                "absolute filesystem path `{path}` should be rejected"
            );
        }
    }

    #[test]
    fn navigation_text_classifier_preserves_search_text() {
        assert!(looks_like_navigation_text("/home/harry/upload-proof.txt"));
        assert!(looks_like_navigation_text(
            " /Users/alice/upload-proof.txt "
        ));
        assert!(looks_like_navigation_text("file:///tmp/upload-proof.txt"));
        assert!(looks_like_navigation_text(
            r"C:\Users\agent\Desktop\upload-proof.txt"
        ));
        assert!(looks_like_navigation_text(
            "C:/Users/agent/Desktop/upload-proof.txt"
        ));
        assert!(looks_like_navigation_text(
            r"\\server\share\upload-proof.txt"
        ));
        assert!(!looks_like_navigation_text("literal search text"));
        assert!(!looks_like_navigation_text("how to use /tmp on linux"));
        assert!(!looks_like_navigation_text(
            "relative/path/upload-proof.txt"
        ));
        assert!(!looks_like_navigation_text("~/upload-proof.txt"));
        assert!(!looks_like_navigation_text("C: drive letter search"));
    }

    #[test]
    fn x11_browser_ctrl_l_non_url_and_click_address_bar_patterns_are_allowed() {
        let browser = browser_target("0x600016", "firefox", Some("Mozilla Firefox"));

        for text in [
            "literal search text",
            "search for /home/harry/upload-proof.txt",
            "relative/path/upload-proof.txt",
            "~/upload-proof.txt",
            "C: drive letter search",
        ] {
            let actions = ctrl_l_send_return_actions(text);
            reject_unsafe_browser_navigation_chord(&x11_adapter_info(), &browser, &actions)
                .unwrap_or_else(|_| panic!("non-navigation text `{text}` should remain allowed"));
        }

        let click_address_bar = parse_dsl_sequence(
            r#"lclick(220,60),hold(ctrl),a,release(ctrl),send("file:///tmp/mouse-buttons-task.html"),Return"#,
        )
        .expect("dsl should parse");
        reject_unsafe_browser_navigation_chord(&x11_adapter_info(), &browser, &click_address_bar)
            .expect("capture-click-address-bar navigation pattern should remain valid");
    }

    #[test]
    fn ctrl_l_url_sequence_remains_valid_for_non_browser_or_non_x11_targets() {
        let actions =
            parse_dsl_sequence(r#"hold(ctrl),l,release(ctrl),send("https://example.com"),Return"#)
                .expect("dsl should parse");
        reject_unsafe_browser_navigation_chord(
            &x11_adapter_info(),
            &browser_target("window-1", "FixtureApp", Some("Fixture Window")),
            &actions,
        )
        .expect("non-browser targets should not trigger the browser navigation guard");

        let wayland_info = AdapterInfo {
            platform: PlatformKind::Linux,
            session: DesktopSession::Wayland,
            audio_backend: Some(AudioBackend::PipeWire),
            stateless: true,
        };
        reject_unsafe_browser_navigation_chord(
            &wayland_info,
            &browser_target("0x600016", "firefox", Some("Mozilla Firefox")),
            &actions,
        )
        .expect("the guard is scoped to the known Linux/X11 failure mode");
    }

    #[test]
    fn parser_accepts_initial_action_set() {
        let actions = parse_dsl_sequence(
            r#"hold(ctrl),c,release(ctrl),wait(1.5s),send("abc"),lclick(10,20),rclick(30,40),mclick(50,60),drag(1,2,3,4),scroll(100,200,7),scroll(100,200,-3)"#,
        )
        .expect("dsl should parse");

        assert_eq!(actions.len(), 11);
        assert_eq!(
            actions[0],
            InputAction::Hold {
                modifier: ModifierKey::Ctrl
            }
        );
        assert_eq!(
            actions[1],
            InputAction::KeyTap {
                key: "c".to_owned()
            }
        );
        assert_eq!(
            actions[2],
            InputAction::Release {
                modifier: ModifierKey::Ctrl
            }
        );
        assert_eq!(actions[3], InputAction::Wait { duration_ms: 1_500 });
        assert_eq!(
            actions[4],
            InputAction::Send {
                text: "abc".to_owned()
            }
        );
        assert_eq!(
            actions[5],
            InputAction::Click {
                button: MouseButton::Left,
                x: 10,
                y: 20,
            }
        );
        assert_eq!(
            actions[6],
            InputAction::Click {
                button: MouseButton::Right,
                x: 30,
                y: 40,
            }
        );
        assert_eq!(
            actions[7],
            InputAction::Click {
                button: MouseButton::Middle,
                x: 50,
                y: 60,
            }
        );
        assert_eq!(
            actions[8],
            InputAction::Drag {
                x0: 1,
                y0: 2,
                x1: 3,
                y1: 4,
            }
        );
        assert_eq!(
            actions[9],
            InputAction::Scroll {
                x: 100,
                y: 200,
                dy: 7,
            }
        );
        assert_eq!(
            actions[10],
            InputAction::Scroll {
                x: 100,
                y: 200,
                dy: -3,
            }
        );
    }

    #[test]
    fn parser_accepts_shift_insert_terminal_paste_chord() {
        let actions = parse_dsl_sequence("hold(shift),Insert,release(shift)")
            .expect("Shift+Insert terminal paste chord should parse");

        assert_eq!(
            actions,
            vec![
                InputAction::Hold {
                    modifier: ModifierKey::Shift,
                },
                InputAction::KeyTap {
                    key: "insert".to_owned(),
                },
                InputAction::Release {
                    modifier: ModifierKey::Shift,
                },
            ]
        );
    }

    #[test]
    fn parser_reports_precise_invalid_syntax_details() {
        let error = parse_dsl_sequence(r"hold(ctrl),send(abc),wait(1s)")
            .expect_err("invalid send syntax should fail");

        assert_eq!(error.code(), "invalid_run_input");
        assert_eq!(error.details().expect("details")["stage"], "parse");
        assert_eq!(error.details().expect("details")["action_index"], 1);
        assert_eq!(error.details().expect("details")["action"], "send(abc)");
    }

    #[test]
    fn parser_rejects_branchy_invalid_sequences_with_precise_stages() {
        for (sequence, stage, detail_key) in [
            ("hold(ctrl),,tab", "parse", Some("action_index")),
            ("hold(ctrl))", "parse", Some("offset")),
            (r#"send("unterminated)"#, "parse", None),
            ("hold(ctrl", "parse", None),
            (r#"send("hi") trailing"#, "parse", Some("action")),
            ("drag(1,2,3)", "parse", Some("action")),
            ("scroll(10,20)", "parse", Some("action")),
            ("scroll(10,20,0)", "validate", Some("action")),
            ("scroll(10,20,121)", "validate", Some("action")),
            ("scroll(10,20,down)", "parse", Some("action")),
            ("wait(0)", "validate", Some("action")),
            (r#"send("")"#, "validate", Some("action")),
            (r#"send("bad\q")"#, "parse", Some("action")),
        ] {
            let error = parse_dsl_sequence(sequence).expect_err("sequence should be rejected");
            let details = error.details().expect("details should be present");
            assert_eq!(details["stage"], stage, "unexpected stage for `{sequence}`");
            if let Some(detail_key) = detail_key {
                assert!(
                    details.get(detail_key).is_some(),
                    "expected `{detail_key}` in details for `{sequence}`"
                );
            }
        }
    }

    #[test]
    fn plain_text_with_commas_remains_text_when_dsl_parse_fails() {
        let payload = parse_input_definition("hello, world").expect("text payload should parse");
        assert_eq!(
            payload,
            RunInputPayload::Text {
                text: "hello, world".to_owned()
            }
        );
    }

    #[test]
    fn ambiguous_single_bare_key_tokens_are_rejected_with_hint() {
        for token in ["Return", "tab", "Escape", "F12", "Up", "Insert"] {
            let error = parse_input_definition(token).expect_err("bare key token should fail");
            assert_eq!(error.code(), "invalid_run_input");
            let details = error.details().expect("details");
            assert_eq!(details["stage"], "parse");
            assert_eq!(details["ambiguity"], "bare_key_token");
            assert_eq!(details["ambiguous_token"], token);
            let hint = details["hint"].as_str().expect("hint should be a string");
            assert!(
                hint.contains("send(") && hint.contains("DSL sequence"),
                "unexpected hint: {hint}"
            );
        }
    }

    #[test]
    fn reserved_verb_with_quoted_argument_without_parens_is_rejected_with_hint() {
        let error = parse_input_definition(r#"type "hi""#)
            .expect_err("reserved verb missing parentheses should fail");
        assert_eq!(error.code(), "invalid_run_input");
        let details = error.details().expect("details");
        assert_eq!(details["stage"], "parse");
        assert_eq!(details["ambiguity"], "reserved_verb_without_parens");
        assert_eq!(details["verb"], "send");
        let hint = details["hint"].as_str().expect("hint should be a string");
        assert!(
            hint.contains(r#"send("hi")"#),
            "hint should suggest canonical send syntax: {hint}"
        );
    }

    #[test]
    fn genuine_plain_text_payloads_still_parse_as_text() {
        for text in ["hello world", "hello", "Return key", "type hi"] {
            let payload = parse_input_definition(text).expect("text payload should parse");
            assert_eq!(
                payload,
                RunInputPayload::Text {
                    text: text.to_owned()
                }
            );
        }
    }

    #[test]
    fn bare_parenthesized_text_promotes_to_dsl_error() {
        let error = parse_input_definition("send(hello)").expect_err("invalid dsl should fail");
        assert_eq!(error.code(), "invalid_run_input");
        assert_eq!(error.details().expect("details")["stage"], "parse");
    }

    #[test]
    fn top_level_semicolon_emits_clear_diagnostic() {
        let error = parse_input_definition("hold(ctrl); tab")
            .expect_err("top-level `;` should be rejected");
        assert_eq!(error.code(), "invalid_run_input");
        let details = error.details().expect("details");
        assert_eq!(details["stage"], "parse");
        assert_eq!(details["offset"], 10);
        let msg = mcp_cli::StructuredError::message(&error);
        assert!(
            msg.contains("the DSL separator is `,`"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn semicolon_inside_string_or_parens_is_not_diagnosed() {
        // Inside a quoted string the `;` is part of text content.
        let payload = parse_input_definition(r#"send("a; b")"#)
            .expect("quoted `;` should parse as DSL text payload");
        assert!(matches!(payload, RunInputPayload::Actions { .. }));
    }

    #[test]
    fn top_level_commas_promote_valid_sequences_to_actions() {
        let payload = parse_input_definition("ctrl+c,tab").expect("dsl payload should parse");
        assert!(matches!(payload, RunInputPayload::Actions { .. }));
    }

    #[test]
    fn quoted_commas_remain_text_payloads() {
        let payload = parse_input_definition(r#"send("hello, world")"#)
            .expect("quoted dsl payload should parse");
        match payload {
            RunInputPayload::Actions { actions } => assert_eq!(actions.len(), 1),
            other => panic!("expected action payload, got {other:?}"),
        }
    }

    #[test]
    fn capture_mapping_metadata_remaps_output_space_to_source_space() {
        let transform = CoordinateTransform {
            x_numerator: 400,
            x_denominator: 100,
            y_numerator: 200,
            y_denominator: 50,
        };

        assert_eq!(remap_output_point_to_source(&transform, 25, 10), (100, 40));
    }

    #[test]
    fn remap_rounds_fractional_scale_factors_to_nearest_source_coordinate() {
        let transform = CoordinateTransform {
            x_numerator: 3,
            x_denominator: 2,
            y_numerator: 5,
            y_denominator: 4,
        };

        assert_eq!(remap_output_point_to_source(&transform, 7, 7), (11, 9));
    }

    #[test]
    fn relative_coordinates_map_to_absolute_source_space() {
        let bounds = crate::model::Bounds {
            x: 50,
            y: 80,
            width: 300,
            height: 200,
        };

        assert_eq!(relative_point_to_absolute(&bounds, 10, 20), (60, 100));
    }

    fn x11_adapter_info() -> AdapterInfo {
        AdapterInfo {
            platform: PlatformKind::Linux,
            session: DesktopSession::X11,
            audio_backend: Some(AudioBackend::PipeWire),
            stateless: true,
        }
    }

    fn ctrl_l_send_return_actions(text: &str) -> Vec<InputAction> {
        vec![
            InputAction::Hold {
                modifier: ModifierKey::Ctrl,
            },
            InputAction::KeyTap {
                key: "l".to_owned(),
            },
            InputAction::Release {
                modifier: ModifierKey::Ctrl,
            },
            InputAction::Send {
                text: text.to_owned(),
            },
            InputAction::KeyTap {
                key: "return".to_owned(),
            },
        ]
    }

    fn browser_target(id: &str, app_name: &str, title: Option<&str>) -> TargetDescriptor {
        TargetDescriptor {
            id: id.to_owned(),
            title: title.map(str::to_owned),
            kind: CaptureTargetKind::Window,
            name: app_name.to_owned(),
            bounds: Bounds {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            scale_factor: ScaleFactor {
                numerator: 1,
                denominator: 1,
            },
            capture_supported: true,
            input_supported: true,
            app_name: Some(app_name.to_owned()),
            process_id: Some(4242),
            diagnostics: Vec::new(),
        }
    }

    proptest! {
        #[test]
        fn bare_key_sequences_round_trip_into_ordered_actions(
            keys in proptest::collection::vec("[A-Za-z0-9_+-]{1,8}", 2..8)
        ) {
            let sequence = keys.join(",");
            let payload = parse_input_definition(&sequence).expect("generated sequence should parse");

            match payload {
                RunInputPayload::Actions { actions } => {
                    let expected = keys
                        .into_iter()
                        .map(|key| InputAction::KeyTap { key: key.to_ascii_lowercase() })
                        .collect::<Vec<_>>();
                    prop_assert_eq!(actions, expected);
                }
                other => prop_assert!(false, "expected actions payload, got {other:?}"),
            }
        }

        #[test]
        fn coordinate_remap_is_monotonic_with_positive_scale_factors(
            x0 in 0i32..5_000,
            dx in 0i32..128,
            y0 in 0i32..5_000,
            dy in 0i32..128,
            x_num in 1u32..32,
            x_den in 1u32..32,
            y_num in 1u32..32,
            y_den in 1u32..32,
        ) {
            let transform = CoordinateTransform {
                x_numerator: x_num,
                x_denominator: x_den,
                y_numerator: y_num,
                y_denominator: y_den,
            };
            let (mapped_x0, mapped_y0) = remap_output_point_to_source(&transform, x0, y0);
            let (mapped_x1, mapped_y1) = remap_output_point_to_source(&transform, x0 + dx, y0 + dy);

            prop_assert!(mapped_x1 >= mapped_x0);
            prop_assert!(mapped_y1 >= mapped_y0);
        }
    }
}
