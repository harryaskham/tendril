use std::time::Duration;

use serde_json::json;

use crate::error::TendrilError;
use crate::model::{
    Bounds, CoordinateTransform, InputAction, ModifierKey, MouseButton, RunInput, RunInputPayload,
    RunOutput, TargetSelector,
};
use crate::platform::{
    CaptureTargetKind, InputRequest as PlatformInputRequest, PlatformAdapter,
    TargetDescriptor as PlatformTargetDescriptor, TargetDiscoveryRequest,
};

const RELIABILITY_DELAY_MS: u64 = 20;

pub(crate) fn parse_input_definition(input: &str) -> Result<RunInputPayload, TendrilError> {
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
    ensure_input_supported(&target)?;
    adapter.input_support().map_err(TendrilError::from)?;

    let (text, actions) = normalize_payload(&input.payload);
    validate_actions_for_target(&target, &actions)?;

    let outcome = adapter.execute_input(&PlatformInputRequest {
        target_id: target.id.clone(),
        target: target.kind,
        target_name: target.name.clone(),
        bounds: target.bounds.clone(),
        app_name: target.app_name.clone(),
        process_id: target.process_id,
        text,
        actions: actions.clone(),
    })?;

    Ok(RunOutput {
        adapter: adapter.info(),
        target: input.target.clone(),
        focus_required: outcome.focus_required,
        focus_transferred: outcome.focus_transferred,
        action_count: outcome.action_count,
        focused_target: outcome.focused_target,
        notes: outcome.notes,
    })
}

pub(crate) fn render_run_human(output: &RunOutput) -> String {
    let notes = if output.notes.is_empty() {
        String::from("none")
    } else {
        output.notes.join(" ")
    };

    format!(
        "run target: {:?} {}\nplatform: {:?} / {:?}\naction_count: {}\nfocus_required: {}\nfocus_transferred: {}\nfocused_target: {}\nnotes: {}\n",
        output.target.kind(),
        output.target.id(),
        output.adapter.platform,
        output.adapter.session,
        output.action_count,
        output.focus_required,
        output.focus_transferred,
        output.focused_target.as_deref().unwrap_or("<none>"),
        notes,
    )
}

pub(crate) fn reliability_delay() -> Duration {
    Duration::from_millis(RELIABILITY_DELAY_MS)
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
        parse_dsl_sequence, parse_input_definition, relative_point_to_absolute,
        remap_output_point_to_source,
    };
    use crate::model::{
        CoordinateTransform, InputAction, ModifierKey, MouseButton, RunInputPayload,
    };

    #[test]
    fn parser_accepts_initial_action_set() {
        let actions = parse_dsl_sequence(
            r#"hold(ctrl),c,release(ctrl),wait(1.5s),send("abc"),lclick(10,20),rclick(30,40),mclick(50,60),drag(1,2,3,4)"#,
        )
        .expect("dsl should parse");

        assert_eq!(actions.len(), 9);
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
    fn top_level_commas_promote_valid_sequences_to_actions() {
        let payload = parse_input_definition("ctrl+c,tab").expect("dsl payload should parse");
        assert!(matches!(payload, RunInputPayload::Actions { .. }));
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
    fn relative_coordinates_map_to_absolute_source_space() {
        let bounds = crate::model::Bounds {
            x: 50,
            y: 80,
            width: 300,
            height: 200,
        };

        assert_eq!(relative_point_to_absolute(&bounds, 10, 20), (60, 100));
    }
}
