use serde::{Deserialize, Serialize};

use crate::config::ImageFormat;
use crate::error::TendrilError;
use crate::execution_lock::ExecutionLockReport;
use crate::platform::{AdapterInfo, PermissionStatus, TargetCapabilityDiagnostic};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Window,
    Display,
    AudioSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TargetSelector {
    Window { id: String },
    Display { id: String },
}

impl TargetSelector {
    #[must_use]
    pub fn kind(&self) -> TargetKind {
        match self {
            Self::Window { .. } => TargetKind::Window,
            Self::Display { .. } => TargetKind::Display,
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Window { id } | Self::Display { id } => id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScaleFactor {
    pub numerator: u32,
    pub denominator: u32,
}

impl<'de> Deserialize<'de> for ScaleFactor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            numerator: u32,
            denominator: u32,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self::new(raw.numerator, raw.denominator))
    }
}

impl ScaleFactor {
    #[must_use]
    pub fn identity() -> Self {
        Self {
            numerator: 1,
            denominator: 1,
        }
    }

    /// Construct a `ScaleFactor` reduced to its simplest form via GCD.
    ///
    /// Both fields are clamped to a minimum of 1 to avoid zero values
    /// (a denominator of 0 would be meaningless and a numerator of 0
    /// would represent a 0x scale, which is also invalid).
    #[must_use]
    pub fn new(numerator: u32, denominator: u32) -> Self {
        let numerator = numerator.max(1);
        let denominator = denominator.max(1);
        let divisor = gcd(numerator, denominator);
        Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        }
    }
}

const fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet {
    pub capture: bool,
    pub input: bool,
    pub audio: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetDescriptor {
    pub id: String,
    pub kind: TargetKind,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub bounds: Bounds,
    pub scale_factor: ScaleFactor,
    pub capabilities: CapabilitySet,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<TargetCapabilityDiagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListInput {
    pub include_windows: bool,
    pub include_displays: bool,
    pub include_audio_sources: bool,
}

impl Default for ListInput {
    fn default() -> Self {
        Self {
            include_windows: true,
            include_displays: true,
            include_audio_sources: false,
        }
    }
}

impl ListInput {
    pub fn validate(&self) -> Result<(), TendrilError> {
        if self.include_windows || self.include_displays || self.include_audio_sources {
            Ok(())
        } else {
            Err(
                TendrilError::validation("at least one target class must be requested")
                    .with_code("invalid_list_input")
                    .with_field("include_windows"),
            )
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListOutput {
    pub adapter: AdapterInfo,
    pub permissions: Vec<PermissionStatus>,
    pub targets: Vec<TargetDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementListInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetSelector>,
    #[serde(default)]
    pub include_offscreen: bool,
}

impl ElementListInput {
    pub fn validate(&self) -> Result<(), TendrilError> {
        if let Some(target) = &self.target {
            validate_identifier(target.id(), target.kind())
                .map_err(|error| error.with_code("invalid_list_elements_input"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementDescriptor {
    /// Stable within the current target snapshot. Pass this value to
    /// `tendril run 'click(<id>)'` to activate the element without manually
    /// choosing screenshot coordinates.
    pub id: String,
    pub role: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Bounds>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetSelector>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementListOutput {
    pub adapter: AdapterInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetSelector>,
    pub elements: Vec<ElementDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureInput {
    pub target: TargetSelector,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
    pub format: ImageFormat,
    pub compression: u8,
    /// Per-call deadline in milliseconds for backend capture work (portal/grim/etc).
    /// `None` means use the platform-default deadline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

impl CaptureInput {
    pub fn validate(&self) -> Result<(), TendrilError> {
        if let Some(max_width) = self.max_width
            && max_width == 0
        {
            return Err(
                TendrilError::validation("max_width must be greater than zero")
                    .with_code("invalid_capture_input")
                    .with_field("max_width"),
            );
        }

        if let Some(max_height) = self.max_height
            && max_height == 0
        {
            return Err(
                TendrilError::validation("max_height must be greater than zero")
                    .with_code("invalid_capture_input")
                    .with_field("max_height"),
            );
        }

        if self.compression > 100 {
            return Err(
                TendrilError::validation("compression must be between 0 and 100")
                    .with_code("invalid_capture_input")
                    .with_field("compression"),
            );
        }

        validate_identifier(self.target.id(), self.target.kind())
            .map_err(|error| error.with_code("invalid_capture_input"))?;

        if let Some(timeout_ms) = self.timeout_ms
            && timeout_ms == 0
        {
            return Err(
                TendrilError::validation("timeout_ms must be greater than zero")
                    .with_code("invalid_capture_input")
                    .with_field("timeout_ms"),
            );
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinateTransform {
    pub x_numerator: u32,
    pub x_denominator: u32,
    pub y_numerator: u32,
    pub y_denominator: u32,
}

impl CoordinateTransform {
    #[must_use]
    pub fn identity() -> Self {
        Self {
            x_numerator: 1,
            x_denominator: 1,
            y_numerator: 1,
            y_denominator: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureOutput {
    pub adapter: AdapterInfo,
    pub target: TargetSelector,
    pub original_bounds: Bounds,
    pub output_bounds: Bounds,
    pub source_to_output: CoordinateTransform,
    pub output_to_source: CoordinateTransform,
    pub resized: bool,
    pub format: ImageFormat,
    pub compression: u8,
    pub media_type: String,
    pub image_base64: String,
    pub captured_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModifierKey {
    Ctrl,
    Alt,
    Shift,
    Meta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Maximum wheel ticks accepted in a single typed scroll action.
///
/// Wheel events are delivered as repeated native button/input events, so this
/// cap keeps malformed typed-model requests from generating unbounded event
/// loops while still allowing large page or list movements in one action.
pub const MAX_SCROLL_TICKS: u32 = 120;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputAction {
    KeyTap {
        key: String,
    },
    Hold {
        modifier: ModifierKey,
    },
    Release {
        modifier: ModifierKey,
    },
    Send {
        text: String,
    },
    Wait {
        duration_ms: u64,
    },
    Click {
        button: MouseButton,
        x: i32,
        y: i32,
    },
    /// Move the pointer to source-space coordinates without pressing a button.
    ///
    /// This is exposed in the DSL as `move(x,y)` and `hover(x,y)` and
    /// serializes in the shared typed model as `type: "pointer_move"`.
    PointerMove {
        x: i32,
        y: i32,
    },
    /// Double-click the primary/left mouse button at source-space coordinates.
    ///
    /// This is exposed in the DSL as `dblclick(x,y)` and `doubleclick(x,y)` and
    /// serializes in the shared typed model as `type: "double_click"`.
    DoubleClick {
        x: i32,
        y: i32,
    },
    Drag {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
    },
    /// Scroll the wheel under source-space coordinates.
    ///
    /// `dy` is measured in wheel ticks. Positive values scroll down; negative
    /// values scroll up. A zero delta is rejected by validation.
    Scroll {
        x: i32,
        y: i32,
        dy: i32,
    },
    /// Activate an element returned by `tendril list-elements`.
    ///
    /// The DSL form is `click(<id>)`, `press(<id>)`, or `element(<id>)`.
    /// Tendril resolves the element from platform metadata for the target and
    /// dispatches an activation/click without the caller choosing pixels by
    /// hand.
    ElementClick {
        id: String,
    },
}

impl InputAction {
    pub fn validate(&self) -> Result<(), TendrilError> {
        match self {
            Self::KeyTap { key } if key.trim().is_empty() => {
                Err(TendrilError::validation("key tap requires a non-empty key")
                    .with_code("invalid_run_input")
                    .with_field("actions"))
            }
            Self::Send { text } if text.is_empty() => Err(TendrilError::validation(
                "send action requires a non-empty string",
            )
            .with_code("invalid_run_input")
            .with_field("actions")),
            Self::Wait { duration_ms } if *duration_ms == 0 => Err(TendrilError::validation(
                "wait action duration must be greater than zero",
            )
            .with_code("invalid_run_input")
            .with_field("actions")),
            Self::Scroll { dy, .. } if *dy == 0 => Err(TendrilError::validation(
                "scroll action dy must be non-zero",
            )
            .with_code("invalid_run_input")
            .with_field("actions")),
            Self::Scroll { dy, .. } if dy.unsigned_abs() > MAX_SCROLL_TICKS => Err(
                TendrilError::validation(format!(
                    "scroll action dy must be between -{MAX_SCROLL_TICKS} and {MAX_SCROLL_TICKS} wheel ticks"
                ))
                .with_code("invalid_run_input")
                .with_field("actions"),
            ),
            Self::ElementClick { id } if id.trim().is_empty() => Err(TendrilError::validation(
                "element click action requires a non-empty element id",
            )
            .with_code("invalid_run_input")
            .with_field("actions")),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunInputPayload {
    Text { text: String },
    Dsl { sequence: String },
    Actions { actions: Vec<InputAction> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInput {
    pub target: TargetSelector,
    pub payload: RunInputPayload,
    /// Restore the previously focused window/application after dispatching
    /// input when the platform adapter can observe and restore focus.
    #[serde(default = "default_restore_focus")]
    pub restore_focus: bool,
}

fn default_restore_focus() -> bool {
    true
}

impl RunInput {
    pub fn validate(&self) -> Result<(), TendrilError> {
        validate_identifier(self.target.id(), self.target.kind())
            .map_err(|error| error.with_code("invalid_run_input"))?;

        match &self.payload {
            RunInputPayload::Text { text } if text.is_empty() => {
                Err(TendrilError::validation("text input cannot be empty")
                    .with_code("invalid_run_input")
                    .with_field("text"))
            }
            RunInputPayload::Dsl { sequence } if sequence.trim().is_empty() => {
                Err(TendrilError::validation("dsl sequence cannot be empty")
                    .with_code("invalid_run_input")
                    .with_field("sequence"))
            }
            RunInputPayload::Actions { actions } if actions.is_empty() => {
                Err(TendrilError::validation("actions cannot be empty")
                    .with_code("invalid_run_input")
                    .with_field("actions"))
            }
            RunInputPayload::Actions { actions } => {
                for (index, action) in actions.iter().enumerate() {
                    action.validate().map_err(|error| {
                        error
                            .with_detail_entry("action_index", serde_json::json!(index))
                            .with_detail_entry("action_number", serde_json::json!(index + 1))
                    })?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusSnapshot {
    pub id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// Keep these independently named status flags in the public JSON output so
// existing Tendril clients do not need to unpack an internal state object.
#[allow(clippy::struct_excessive_bools)]
pub struct RunOutput {
    pub adapter: AdapterInfo,
    pub target: TargetSelector,
    pub focus_required: bool,
    pub focus_transferred: bool,
    pub action_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focused_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_focus: Option<FocusSnapshot>,
    pub focus_restored: bool,
    pub pointer_restored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restore_error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Host-local execution lock/queue metadata for this run invocation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_lock: Option<ExecutionLockReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioSourceKind {
    System,
    Microphone,
    Device,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioSourceSelector {
    pub kind: AudioSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioFormat {
    Wav,
    Flac,
    Opus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenInput {
    pub source: AudioSourceSelector,
    pub duration_ms: u64,
    pub format: AudioFormat,
}

impl Default for ListenInput {
    fn default() -> Self {
        Self {
            source: AudioSourceSelector {
                kind: AudioSourceKind::System,
                id: None,
            },
            duration_ms: 5_000,
            format: AudioFormat::Wav,
        }
    }
}

impl ListenInput {
    pub fn validate(&self) -> Result<(), TendrilError> {
        if self.duration_ms == 0 {
            return Err(
                TendrilError::validation("duration_ms must be greater than zero")
                    .with_code("invalid_listen_input")
                    .with_field("duration_ms"),
            );
        }

        if matches!(self.source.kind, AudioSourceKind::Device)
            && self.source.id.as_deref().unwrap_or_default().is_empty()
        {
            return Err(TendrilError::validation(
                "device audio capture requires a device identifier",
            )
            .with_code("invalid_listen_input")
            .with_field("source.id"));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenOutput {
    pub source: AudioSourceSelector,
    pub duration_ms: u64,
    pub format: AudioFormat,
    pub channels: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasInput {
    pub target: TargetSelector,
    pub shell: ShellKind,
    pub name: String,
}

impl AliasInput {
    pub fn validate(&self) -> Result<(), TendrilError> {
        validate_identifier(self.target.id(), self.target.kind())
            .map_err(|error| error.with_code("invalid_alias_input"))?;

        if self.name.is_empty() {
            return Err(TendrilError::validation("alias name cannot be empty")
                .with_code("invalid_alias_input")
                .with_field("name"));
        }

        if !self
            .name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
        {
            return Err(TendrilError::validation(
                "alias name may only contain ASCII letters, digits, underscore, or hyphen",
            )
            .with_code("invalid_alias_input")
            .with_field("name"));
        }

        if is_shell_reserved_word(&self.name) {
            return Err(TendrilError::validation(format!(
                "alias name '{}' is a shell reserved word and cannot be used as a function name",
                self.name
            ))
            .with_code("invalid_alias_input")
            .with_field("name"));
        }

        Ok(())
    }
}

/// Returns true if `name` is a reserved word in bash/zsh that cannot be used
/// as a shell function name. Using one of these as an alias name produces a
/// syntax error when the generated `name() { ... }` definition is evaluated.
fn is_shell_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "if" | "then"
            | "else"
            | "elif"
            | "fi"
            | "for"
            | "while"
            | "until"
            | "do"
            | "done"
            | "case"
            | "esac"
            | "function"
            | "select"
            | "in"
            | "time"
            | "coproc"
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasOutput {
    pub shell: ShellKind,
    pub name: String,
    pub command: String,
    pub argv: Vec<String>,
    pub shell_code: String,
    pub target: TargetSelector,
}

fn validate_identifier(id: &str, kind: TargetKind) -> Result<(), TendrilError> {
    if id.trim().is_empty() {
        Err(TendrilError::validation(format!(
            "{} identifier cannot be empty",
            match kind {
                TargetKind::Window => "window",
                TargetKind::Display => "display",
                TargetKind::AudioSource => "audio source",
            }
        ))
        .with_field("id"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AliasInput, AudioFormat, AudioSourceKind, AudioSourceSelector, CaptureInput, ImageFormat,
        InputAction, ListInput, ListenInput, MAX_SCROLL_TICKS, RunInput, RunInputPayload,
        ScaleFactor, ShellKind, TargetSelector,
    };

    #[test]
    fn capture_validation_rejects_out_of_range_compression() {
        let input = CaptureInput {
            target: TargetSelector::Window {
                id: "window-1".into(),
            },
            max_width: Some(1024),
            max_height: None,
            format: ImageFormat::Png,
            compression: 255,
            timeout_ms: None,
        };

        let error = input
            .validate()
            .expect_err("compression should be validated");

        assert_eq!(error.code(), "invalid_capture_input");
    }

    #[test]
    fn run_validation_rejects_empty_text_payload() {
        let input = RunInput {
            target: TargetSelector::Display { id: "1".into() },
            payload: RunInputPayload::Text {
                text: String::new(),
            },
            restore_focus: true,
        };

        let error = input
            .validate()
            .expect_err("text payload should be validated");

        assert_eq!(error.code(), "invalid_run_input");
    }

    #[test]
    fn pointer_move_action_round_trips_through_shared_model() {
        let json = serde_json::to_value(InputAction::PointerMove { x: 10, y: 20 })
            .expect("pointer-move action should serialize");
        assert_eq!(json["type"], "pointer_move");
        assert_eq!(json["x"], 10);
        assert_eq!(json["y"], 20);

        let decoded: InputAction = serde_json::from_value(json)
            .expect("pointer-move action should deserialize through shared model");
        assert_eq!(decoded, InputAction::PointerMove { x: 10, y: 20 });
    }

    #[test]
    fn double_click_action_round_trips_through_shared_model() {
        let json = serde_json::to_value(InputAction::DoubleClick { x: 10, y: 20 })
            .expect("double-click action should serialize");
        assert_eq!(json["type"], "double_click");
        assert_eq!(json["x"], 10);
        assert_eq!(json["y"], 20);

        let decoded: InputAction = serde_json::from_value(json)
            .expect("double-click action should deserialize through shared model");
        assert_eq!(decoded, InputAction::DoubleClick { x: 10, y: 20 });
    }

    #[test]
    fn run_validation_rejects_invalid_typed_scroll_delta() {
        let over_limit = i32::try_from(MAX_SCROLL_TICKS + 1).expect("scroll limit fits i32");
        for dy in [0, over_limit, -over_limit] {
            let input = RunInput {
                target: TargetSelector::Display { id: "1".into() },
                payload: RunInputPayload::Actions {
                    actions: vec![InputAction::Scroll { x: 10, y: 20, dy }],
                },
                restore_focus: true,
            };

            let error = input.validate().expect_err("scroll dy should be validated");

            assert_eq!(error.code(), "invalid_run_input");
            assert_eq!(error.details().expect("action details")["action_index"], 0);
        }
    }

    #[test]
    fn listen_validation_requires_device_identifier() {
        let input = ListenInput {
            source: AudioSourceSelector {
                kind: AudioSourceKind::Device,
                id: None,
            },
            duration_ms: 1_000,
            format: AudioFormat::Wav,
        };

        let error = input
            .validate()
            .expect_err("device listen input should require an id");

        assert_eq!(error.code(), "invalid_listen_input");
    }

    #[test]
    fn alias_validation_accepts_agent_friendly_name() {
        let input = AliasInput {
            target: TargetSelector::Window {
                id: "window-1".into(),
            },
            shell: ShellKind::Bash,
            name: "desk_helper-1".into(),
        };

        input.validate().expect("alias name should be valid");
    }

    #[test]
    fn alias_validation_rejects_shell_reserved_words() {
        for reserved in [
            "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case",
            "esac", "function", "select", "in", "time", "coproc",
        ] {
            let input = AliasInput {
                target: TargetSelector::Window {
                    id: "window-1".into(),
                },
                shell: ShellKind::Bash,
                name: reserved.into(),
            };

            let error = input
                .validate()
                .expect_err(&format!("reserved word '{reserved}' should be rejected"));
            assert_eq!(error.code(), "invalid_alias_input");
        }
    }

    #[test]
    fn list_validation_requires_any_requested_class() {
        let input = ListInput {
            include_windows: false,
            include_displays: false,
            include_audio_sources: false,
        };

        let error = input
            .validate()
            .expect_err("list input should require at least one target class");

        assert_eq!(error.code(), "invalid_list_input");
    }

    #[test]
    fn scale_factor_new_reduces_to_simplest_form_so_displays_match_windows() {
        // Both 1000/1000 (display origin) and 1/1 (window origin) must collapse
        // to the same canonical 1/1 representation, removing the agent-facing
        // inconsistency that motivated bd-e123b8.
        assert_eq!(ScaleFactor::new(1000, 1000), ScaleFactor::identity());
        assert_eq!(ScaleFactor::new(1, 1), ScaleFactor::identity());

        // Common HiDPI scales reduce predictably.
        let two_x = ScaleFactor::new(2000, 1000);
        assert_eq!((two_x.numerator, two_x.denominator), (2, 1));

        let one_point_five = ScaleFactor::new(1500, 1000);
        assert_eq!(
            (one_point_five.numerator, one_point_five.denominator),
            (3, 2)
        );

        let one_point_two_five = ScaleFactor::new(1250, 1000);
        assert_eq!(
            (one_point_two_five.numerator, one_point_two_five.denominator),
            (5, 4)
        );
    }

    #[test]
    fn scale_factor_new_clamps_zero_components_to_one() {
        // Zero numerator or denominator would represent an invalid scale and
        // could trigger divide-by-zero in downstream coordinate math.
        let zero_num = ScaleFactor::new(0, 1000);
        assert!(zero_num.numerator >= 1 && zero_num.denominator >= 1);

        let zero_den = ScaleFactor::new(1000, 0);
        assert!(zero_den.numerator >= 1 && zero_den.denominator >= 1);
    }

    #[test]
    fn scale_factor_deserialization_normalizes_legacy_payloads() {
        // Existing on-the-wire payloads from older adapters may still carry
        // the un-reduced 1000/1000 form. Deserialization must canonicalize so
        // consumers see a uniform shape regardless of producer version.
        let value: ScaleFactor =
            serde_json::from_str(r#"{"numerator": 1000, "denominator": 1000}"#)
                .expect("legacy 1000/1000 payload should deserialize");
        assert_eq!(value, ScaleFactor::identity());
    }

    fn capture_with_target(target: TargetSelector) -> CaptureInput {
        CaptureInput {
            target,
            max_width: None,
            max_height: None,
            format: ImageFormat::Png,
            compression: 0,
            timeout_ms: None,
        }
    }

    #[test]
    fn validate_identifier_rejects_empty_window_id_with_id_field() {
        let error = capture_with_target(TargetSelector::Window { id: String::new() })
            .validate()
            .expect_err("empty window id should be rejected");
        assert_eq!(error.code(), "invalid_capture_input");
        // The shared identifier guard tags the offending field as `id`.
        assert_eq!(error.details().expect("details")["field"], "id");
    }

    #[test]
    fn validate_identifier_rejects_whitespace_only_id() {
        // A whitespace-only id is empty after trim() and must be rejected so
        // blank-but-non-empty selectors cannot slip past the guard.
        let error = capture_with_target(TargetSelector::Window { id: "   ".into() })
            .validate()
            .expect_err("whitespace-only id should be rejected");
        assert_eq!(error.code(), "invalid_capture_input");
        assert_eq!(error.details().expect("details")["field"], "id");
    }

    #[test]
    fn validate_identifier_rejects_empty_display_id() {
        let error = capture_with_target(TargetSelector::Display { id: String::new() })
            .validate()
            .expect_err("empty display id should be rejected");
        assert_eq!(error.code(), "invalid_capture_input");
        assert_eq!(error.details().expect("details")["field"], "id");
    }

    #[test]
    fn validate_identifier_accepts_non_empty_id() {
        capture_with_target(TargetSelector::Window {
            id: "window-1".into(),
        })
        .validate()
        .expect("a non-empty target id should pass the identifier guard");
    }
}
