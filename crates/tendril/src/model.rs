use serde::{Deserialize, Serialize};

use crate::config::ImageFormat;
use crate::error::TendrilError;
use crate::platform::{AdapterInfo, PermissionStatus};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScaleFactor {
    pub numerator: u32,
    pub denominator: u32,
}

impl ScaleFactor {
    #[must_use]
    pub fn identity() -> Self {
        Self {
            numerator: 1,
            denominator: 1,
        }
    }
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
pub struct CaptureInput {
    pub target: TargetSelector,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
    pub format: ImageFormat,
    pub compression: u8,
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
            .map_err(|error| error.with_code("invalid_capture_input"))
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputAction {
    KeyTap { key: String },
    Hold { modifier: ModifierKey },
    Release { modifier: ModifierKey },
    Send { text: String },
    Wait { duration_ms: u64 },
    Click { button: MouseButton, x: i32, y: i32 },
    Drag { x0: i32, y0: i32, x1: i32, y1: i32 },
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
pub struct RunOutput {
    pub adapter: AdapterInfo,
    pub target: TargetSelector,
    pub focus_required: bool,
    pub focus_transferred: bool,
    pub action_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focused_target: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

        Ok(())
    }
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
        ListInput, ListenInput, RunInput, RunInputPayload, ShellKind, TargetSelector,
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
        };

        let error = input
            .validate()
            .expect_err("compression should be validated");

        assert_eq!(error.code(), "invalid_capture_input");
    }

    #[test]
    fn run_validation_rejects_empty_text_payload() {
        let input = RunInput {
            target: TargetSelector::Display {
                id: "1".into(),
            },
            payload: RunInputPayload::Text {
                text: String::new(),
            },
        };

        let error = input
            .validate()
            .expect_err("text payload should be validated");

        assert_eq!(error.code(), "invalid_run_input");
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
}
