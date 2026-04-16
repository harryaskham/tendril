use std::env;
use std::time::Duration;

use ashpd::desktop::screenshot::Screenshot as PortalScreenshot;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use pollster::block_on;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use mcp_cli::ErrorCategory;

use crate::capture::{current_timestamp, read_and_remove_temp_capture, unique_temp_path};
use crate::discovery;
use crate::error::TendrilError;
use crate::input::{relative_point_to_absolute, reliability_delay};
use crate::model::{Bounds, InputAction, ModifierKey, MouseButton, ScaleFactor};
use crate::x11;

const CAPTURE_FIXTURE_ENV: &str = "TENDRIL_CAPTURE_FIXTURE_JSON";
const INPUT_FIXTURE_ENV: &str = "TENDRIL_INPUT_FIXTURE_JSON";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKind {
    MacOs,
    Linux,
    Windows11,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopSession {
    MacOsWindowServer,
    X11,
    Wayland,
    WindowsDesktop,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioBackend {
    CoreAudio,
    PipeWire,
    PulseAudio,
    Wasapi,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterContext {
    pub platform: PlatformKind,
    pub session: DesktopSession,
    pub audio_backend: Option<AudioBackend>,
}

impl AdapterContext {
    #[must_use]
    pub fn detect() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::macos()
        }

        #[cfg(target_os = "linux")]
        {
            Self::linux(
                detect_linux_session(
                    env::var("XDG_SESSION_TYPE").ok().as_deref(),
                    env::var_os("DISPLAY").as_deref(),
                    env::var_os("WAYLAND_DISPLAY").as_deref(),
                ),
                detect_linux_audio_backend(
                    env::var_os("PIPEWIRE_RUNTIME_DIR").as_deref(),
                    env::var_os("PULSE_SERVER").as_deref(),
                    env::var_os("PULSE_RUNTIME_PATH").as_deref(),
                ),
            )
        }

        #[cfg(target_os = "windows")]
        {
            Self::windows11()
        }
    }

    #[must_use]
    pub const fn macos() -> Self {
        Self {
            platform: PlatformKind::MacOs,
            session: DesktopSession::MacOsWindowServer,
            audio_backend: Some(AudioBackend::CoreAudio),
        }
    }

    #[must_use]
    pub const fn linux(session: DesktopSession, audio_backend: Option<AudioBackend>) -> Self {
        Self {
            platform: PlatformKind::Linux,
            session,
            audio_backend,
        }
    }

    #[must_use]
    pub const fn windows11() -> Self {
        Self {
            platform: PlatformKind::Windows11,
            session: DesktopSession::WindowsDesktop,
            audio_backend: Some(AudioBackend::Wasapi),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterInfo {
    pub platform: PlatformKind,
    pub session: DesktopSession,
    pub audio_backend: Option<AudioBackend>,
    pub stateless: bool,
}

impl AdapterInfo {
    #[must_use]
    pub fn from_context(context: &AdapterContext) -> Self {
        Self {
            platform: context.platform,
            session: context.session,
            audio_backend: context.audio_backend,
            stateless: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    TargetDiscovery,
    WindowCapture,
    DisplayCapture,
    InputControl,
    AudioLoopbackCapture,
    AudioInputCapture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureTargetKind {
    Window,
    Display,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioSourceKind {
    SystemLoopback,
    Microphone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    ScreenCapture,
    Accessibility,
    Microphone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    NotRequired,
    Unknown,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionStatus {
    pub permission: PermissionKind,
    pub state: PermissionState,
    pub summary: String,
    pub suggested_action: Option<String>,
}

impl PermissionStatus {
    #[must_use]
    pub fn unknown(
        permission: PermissionKind,
        summary: impl Into<String>,
        suggested_action: impl Into<String>,
    ) -> Self {
        Self {
            permission,
            state: PermissionState::Unknown,
            summary: summary.into(),
            suggested_action: Some(suggested_action.into()),
        }
    }

    #[must_use]
    pub fn not_required(permission: PermissionKind, summary: impl Into<String>) -> Self {
        Self {
            permission,
            state: PermissionState::NotRequired,
            summary: summary.into(),
            suggested_action: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureSupport {
    pub capability: Capability,
    pub permissions: Vec<PermissionStatus>,
    pub notes: Vec<String>,
}

impl FeatureSupport {
    #[must_use]
    pub fn available(capability: Capability) -> Self {
        Self {
            capability,
            permissions: Vec::new(),
            notes: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_permissions(mut self, permissions: Vec<PermissionStatus>) -> Self {
        self.permissions = permissions;
        self
    }

    #[must_use]
    pub fn with_notes(mut self, notes: Vec<String>) -> Self {
        self.notes = notes;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityErrorReason {
    UnsupportedPlatform,
    UnsupportedSession,
    UnsupportedFeature,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityError {
    pub capability: Capability,
    pub platform: PlatformKind,
    pub reason: CapabilityErrorReason,
    pub message: String,
    pub suggested_action: Option<String>,
}

impl std::fmt::Display for CapabilityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Unsupported capability {:?} on {:?}: {}",
            self.capability, self.platform, self.message
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterOperation {
    TargetDiscovery,
    Capture,
    InputControl,
    PermissionCheck,
    AudioProbe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
pub enum PlatformAdapterError {
    #[error("{0}")]
    UnsupportedCapability(CapabilityError),

    #[error("Missing permission {permission:?} for {capability:?} on {platform:?}: {message}")]
    MissingPermission {
        capability: Capability,
        permission: PermissionKind,
        platform: PlatformKind,
        message: String,
        suggested_action: String,
    },

    #[error("Platform adapter failure during {operation:?} on {platform:?}: {message}")]
    AdapterFailure {
        operation: AdapterOperation,
        platform: PlatformKind,
        message: String,
    },
}

impl PlatformAdapterError {
    #[must_use]
    pub fn unsupported(
        capability: Capability,
        platform: PlatformKind,
        reason: CapabilityErrorReason,
        message: impl Into<String>,
        suggested_action: Option<&str>,
    ) -> Self {
        Self::UnsupportedCapability(CapabilityError {
            capability,
            platform,
            reason,
            message: message.into(),
            suggested_action: suggested_action.map(str::to_owned),
        })
    }

    #[must_use]
    pub fn missing_permission(
        capability: Capability,
        permission: PermissionKind,
        platform: PlatformKind,
        message: impl Into<String>,
        suggested_action: impl Into<String>,
    ) -> Self {
        Self::MissingPermission {
            capability,
            permission,
            platform,
            message: message.into(),
            suggested_action: suggested_action.into(),
        }
    }

    #[must_use]
    pub fn adapter_failure(
        operation: AdapterOperation,
        platform: PlatformKind,
        message: impl Into<String>,
    ) -> Self {
        Self::AdapterFailure {
            operation,
            platform,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::UnsupportedCapability(_) => ErrorCategory::UnsupportedCapability,
            Self::MissingPermission { .. } => ErrorCategory::MissingPermission,
            Self::AdapterFailure { .. } => ErrorCategory::PlatformAdapterFailure,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetDiscoveryRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetDescriptor {
    pub id: String,
    pub title: Option<String>,
    pub kind: CaptureTargetKind,
    pub name: String,
    pub bounds: Bounds,
    pub scale_factor: ScaleFactor,
    pub capture_supported: bool,
    pub input_supported: bool,
    pub app_name: Option<String>,
    pub process_id: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetInventory {
    pub targets: Vec<TargetDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRequest {
    pub target: CaptureTargetKind,
    pub target_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureArtifact {
    pub target_id: String,
    pub media_type: String,
    pub image_bytes: Vec<u8>,
    pub captured_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputRequest {
    pub target_id: String,
    pub target: CaptureTargetKind,
    pub target_name: String,
    pub bounds: Bounds,
    pub app_name: Option<String>,
    pub process_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<InputAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputOutcome {
    pub action_count: usize,
    pub focus_required: bool,
    pub focus_transferred: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focused_target: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CaptureFixture {
    #[serde(default = "default_capture_fixture_media_type")]
    media_type: String,
    image_base64: String,
    #[serde(default)]
    captured_at: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct InputFixture {
    #[serde(default)]
    action_count: Option<usize>,
    #[serde(default)]
    focus_required: bool,
    #[serde(default)]
    focus_transferred: bool,
    #[serde(default)]
    focused_target: Option<String>,
    #[serde(default)]
    notes: Vec<String>,
}

fn default_capture_fixture_media_type() -> String {
    "image/png".to_owned()
}

fn load_capture_fixture(
    platform: PlatformKind,
    request: &CaptureRequest,
) -> Result<Option<CaptureArtifact>, PlatformAdapterError> {
    let Some(raw) = env::var(CAPTURE_FIXTURE_ENV).ok() else {
        return Ok(None);
    };

    let fixture = serde_json::from_str::<CaptureFixture>(&raw).map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            platform,
            format!("failed to parse {CAPTURE_FIXTURE_ENV}: {error}"),
        )
    })?;
    let image_bytes = BASE64.decode(&fixture.image_base64).map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            platform,
            format!("failed to decode {CAPTURE_FIXTURE_ENV} image bytes: {error}"),
        )
    })?;

    Ok(Some(CaptureArtifact {
        target_id: request.target_id.clone(),
        media_type: fixture.media_type,
        image_bytes,
        captured_at: fixture.captured_at.unwrap_or_else(current_timestamp),
    }))
}

fn default_input_action_count(request: &InputRequest) -> usize {
    request.actions.len() + usize::from(request.text.is_some())
}

fn load_input_fixture(
    platform: PlatformKind,
    request: &InputRequest,
) -> Result<Option<InputOutcome>, TendrilError> {
    let Some(raw) = env::var(INPUT_FIXTURE_ENV).ok() else {
        return Ok(None);
    };

    let fixture = serde_json::from_str::<InputFixture>(&raw).map_err(|error| {
        TendrilError::from(PlatformAdapterError::adapter_failure(
            AdapterOperation::InputControl,
            platform,
            format!("failed to parse {INPUT_FIXTURE_ENV}: {error}"),
        ))
    })?;

    Ok(Some(InputOutcome {
        action_count: fixture
            .action_count
            .unwrap_or_else(|| default_input_action_count(request)),
        focus_required: fixture.focus_required,
        focus_transferred: fixture.focus_transferred,
        focused_target: fixture
            .focused_target
            .or_else(|| fixture.focus_transferred.then(|| request.target_id.clone())),
        notes: fixture.notes,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioProbeRequest {
    pub source: AudioSourceKind,
    pub duration_hint_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioCapabilityReport {
    pub source: AudioSourceKind,
    pub backend: AudioBackend,
    pub supported_sample_rates_hz: Vec<u32>,
    pub supported_channel_counts: Vec<u8>,
    pub permissions: Vec<PermissionStatus>,
    pub notes: Vec<String>,
}

pub trait TargetDiscoveryAdapter {
    fn target_discovery_support(&self) -> Result<FeatureSupport, PlatformAdapterError>;

    fn discover_targets(
        &self,
        _request: &TargetDiscoveryRequest,
    ) -> Result<TargetInventory, PlatformAdapterError>;
}

pub trait CaptureAdapter {
    fn capture_support(
        &self,
        target: CaptureTargetKind,
    ) -> Result<FeatureSupport, PlatformAdapterError>;

    fn capture(&self, _request: &CaptureRequest) -> Result<CaptureArtifact, PlatformAdapterError>;
}

pub trait InputControlAdapter {
    fn input_support(&self) -> Result<FeatureSupport, PlatformAdapterError>;

    fn execute_input(&self, _request: &InputRequest) -> Result<InputOutcome, TendrilError>;
}

pub trait PermissionAdapter {
    fn permissions(&self) -> Vec<PermissionStatus>;
}

pub trait AudioCapabilityProbe {
    fn probe_audio_capture(
        &self,
        request: &AudioProbeRequest,
    ) -> Result<AudioCapabilityReport, PlatformAdapterError>;
}

pub trait PlatformAdapter:
    Send
    + Sync
    + TargetDiscoveryAdapter
    + CaptureAdapter
    + InputControlAdapter
    + PermissionAdapter
    + AudioCapabilityProbe
{
    fn info(&self) -> AdapterInfo;
}

#[derive(Debug, Clone)]
pub struct MacOsAdapter {
    context: AdapterContext,
}

impl MacOsAdapter {
    #[must_use]
    pub fn new(context: AdapterContext) -> Self {
        Self { context }
    }

    fn platform(&self) -> PlatformKind {
        self.context.platform
    }

    fn screen_capture_permission() -> PermissionStatus {
        PermissionStatus::unknown(
            PermissionKind::ScreenCapture,
            "macOS capture requires Screen Recording consent for the invoking terminal or binary.",
            "Grant Screen Recording in System Settings > Privacy & Security > Screen Recording, then rerun tendril.",
        )
    }

    fn accessibility_permission() -> PermissionStatus {
        PermissionStatus::unknown(
            PermissionKind::Accessibility,
            "macOS input control requires Accessibility consent.",
            "Grant Accessibility access in System Settings > Privacy & Security > Accessibility, then rerun tendril.",
        )
    }

    fn microphone_permission() -> PermissionStatus {
        PermissionStatus::unknown(
            PermissionKind::Microphone,
            "macOS microphone capture requires Microphone consent.",
            "Grant Microphone access in System Settings > Privacy & Security > Microphone, then rerun tendril.",
        )
    }
}

impl TargetDiscoveryAdapter for MacOsAdapter {
    fn target_discovery_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
        Ok(
            FeatureSupport::available(Capability::TargetDiscovery).with_notes(vec![
                "Window metadata can be enumerated without a background daemon.".to_owned(),
            ]),
        )
    }

    fn discover_targets(
        &self,
        request: &TargetDiscoveryRequest,
    ) -> Result<TargetInventory, PlatformAdapterError> {
        discovery::discover_targets(&self.context, request)
    }
}

impl CaptureAdapter for MacOsAdapter {
    fn capture_support(
        &self,
        target: CaptureTargetKind,
    ) -> Result<FeatureSupport, PlatformAdapterError> {
        let note = match target {
            CaptureTargetKind::Window => {
                "Window capture uses the same permission boundary as display capture on macOS."
            }
            CaptureTargetKind::Display => {
                "Display capture remains stateless; every request must specify the target explicitly."
            }
        };

        Ok(FeatureSupport::available(match target {
            CaptureTargetKind::Window => Capability::WindowCapture,
            CaptureTargetKind::Display => Capability::DisplayCapture,
        })
        .with_permissions(vec![Self::screen_capture_permission()])
        .with_notes(vec![note.to_owned()]))
    }

    fn capture(&self, request: &CaptureRequest) -> Result<CaptureArtifact, PlatformAdapterError> {
        if let Some(artifact) = load_capture_fixture(self.platform(), request)? {
            return Ok(artifact);
        }

        let path = unique_temp_path("png");
        let mut command = std::process::Command::new("screencapture");
        command.arg("-x").arg("-t").arg("png");

        match request.target {
            CaptureTargetKind::Window => {
                command.arg("-l").arg(&request.target_id);
            }
            CaptureTargetKind::Display => {
                command
                    .arg("-D")
                    .arg(parse_display_index(&request.target_id, self.platform())?.to_string());
            }
        }

        command.arg(&path);
        let output = command.output().map_err(|error| {
            PlatformAdapterError::adapter_failure(
                AdapterOperation::Capture,
                self.platform(),
                format!("failed to spawn screencapture: {error}"),
            )
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PlatformAdapterError::missing_permission(
                match request.target {
                    CaptureTargetKind::Window => Capability::WindowCapture,
                    CaptureTargetKind::Display => Capability::DisplayCapture,
                },
                PermissionKind::ScreenCapture,
                self.platform(),
                if stderr.trim().is_empty() {
                    "Capture execution is gated on explicit Screen Recording consent.".to_owned()
                } else {
                    format!("Capture failed: {}", stderr.trim())
                },
                "Grant Screen Recording access before invoking capture commands.",
            ));
        }

        let image_bytes = read_and_remove_temp_capture(&path).map_err(|error| {
            PlatformAdapterError::adapter_failure(
                AdapterOperation::Capture,
                self.platform(),
                error.to_string(),
            )
        })?;

        Ok(CaptureArtifact {
            target_id: request.target_id.clone(),
            media_type: "image/png".to_owned(),
            image_bytes,
            captured_at: current_timestamp(),
        })
    }
}

impl InputControlAdapter for MacOsAdapter {
    fn input_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
        Ok(FeatureSupport::available(Capability::InputControl)
            .with_permissions(vec![Self::accessibility_permission()])
            .with_notes(vec![
                "The adapter may need to transfer focus for reliable text entry on secure fields."
                    .to_owned(),
            ]))
    }

    fn execute_input(&self, request: &InputRequest) -> Result<InputOutcome, TendrilError> {
        if let Some(outcome) = load_input_fixture(self.platform(), request)? {
            return Ok(outcome);
        }

        execute_macos_input(self.platform(), request)
    }
}

impl PermissionAdapter for MacOsAdapter {
    fn permissions(&self) -> Vec<PermissionStatus> {
        vec![
            Self::screen_capture_permission(),
            Self::accessibility_permission(),
            Self::microphone_permission(),
        ]
    }
}

impl AudioCapabilityProbe for MacOsAdapter {
    fn probe_audio_capture(
        &self,
        request: &AudioProbeRequest,
    ) -> Result<AudioCapabilityReport, PlatformAdapterError> {
        match request.source {
            AudioSourceKind::SystemLoopback => Err(PlatformAdapterError::unsupported(
                Capability::AudioLoopbackCapture,
                self.platform(),
                CapabilityErrorReason::UnsupportedFeature,
                "The macOS adapter spine does not expose system loopback capture in v0.0.1.",
                Some(
                    "Use microphone capture or provide an explicit virtual loopback device in a future adapter implementation.",
                ),
            )),
            AudioSourceKind::Microphone => Ok(AudioCapabilityReport {
                source: AudioSourceKind::Microphone,
                backend: AudioBackend::CoreAudio,
                supported_sample_rates_hz: vec![16_000, 44_100, 48_000],
                supported_channel_counts: vec![1, 2],
                permissions: vec![Self::microphone_permission()],
                notes: vec![
                    "Audio capture remains command-scoped and does not create a background daemon."
                        .to_owned(),
                ],
            }),
        }
    }
}

impl PlatformAdapter for MacOsAdapter {
    fn info(&self) -> AdapterInfo {
        AdapterInfo::from_context(&self.context)
    }
}

#[derive(Debug, Clone)]
pub struct LinuxAdapter {
    context: AdapterContext,
}

impl LinuxAdapter {
    #[must_use]
    pub fn new(context: AdapterContext) -> Self {
        Self { context }
    }

    fn platform(&self) -> PlatformKind {
        self.context.platform
    }

    fn audio_backend(&self) -> AudioBackend {
        self.context.audio_backend.unwrap_or(AudioBackend::Unknown)
    }
}

impl TargetDiscoveryAdapter for LinuxAdapter {
    fn target_discovery_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
        match self.context.session {
            DesktopSession::X11 => Ok(FeatureSupport::available(Capability::TargetDiscovery)
                .with_notes(vec![
                    "X11 target enumeration does not require a Tendril daemon.".to_owned(),
                ])),
            DesktopSession::Wayland => Ok(FeatureSupport::available(Capability::TargetDiscovery)
                .with_notes(vec![
                    "Wayland discovery may depend on compositor- or portal-provided metadata."
                        .to_owned(),
                ])),
            _ => Err(PlatformAdapterError::unsupported(
                Capability::TargetDiscovery,
                self.platform(),
                CapabilityErrorReason::UnsupportedSession,
                "The Linux adapter could not determine an X11 or Wayland desktop session.",
                Some("Set XDG_SESSION_TYPE or run Tendril inside an interactive desktop session."),
            )),
        }
    }

    fn discover_targets(
        &self,
        request: &TargetDiscoveryRequest,
    ) -> Result<TargetInventory, PlatformAdapterError> {
        discovery::discover_targets(&self.context, request)
    }
}

impl CaptureAdapter for LinuxAdapter {
    fn capture_support(
        &self,
        target: CaptureTargetKind,
    ) -> Result<FeatureSupport, PlatformAdapterError> {
        let capability = match target {
            CaptureTargetKind::Window => Capability::WindowCapture,
            CaptureTargetKind::Display => Capability::DisplayCapture,
        };

        match self.context.session {
            DesktopSession::X11 => Ok(FeatureSupport::available(capability).with_notes(vec![
                "X11 capture support is adapter-local and command-scoped.".to_owned(),
            ])),
            DesktopSession::Wayland => Ok(FeatureSupport::available(capability).with_notes(vec![
                "Wayland capture prefers xdg-desktop-portal screenshot capture, then crops the requested target bounds locally."
                    .to_owned(),
                "If the portal screenshot backend is unavailable, Tendril falls back to grim on compositor stacks that permit geometry-scoped screenshots."
                    .to_owned(),
                "Wayland discovery still requires a supported compositor metadata backend: Hyprland (`hyprctl`), sway (`swaymsg`), or wlroots output enumeration (`wlr-randr`)."
                    .to_owned(),
            ])),
            _ => Err(PlatformAdapterError::unsupported(
                capability,
                self.platform(),
                CapabilityErrorReason::UnsupportedSession,
                "Capture requires a detected X11 or Wayland desktop session.",
                Some("Run Tendril from a graphical login session and retry."),
            )),
        }
    }

    fn capture(&self, request: &CaptureRequest) -> Result<CaptureArtifact, PlatformAdapterError> {
        if let Some(artifact) = load_capture_fixture(self.platform(), request)? {
            return Ok(artifact);
        }

        match self.context.session {
            DesktopSession::X11 => {}
            DesktopSession::Wayland => {
                let image_bytes = capture_wayland_target(&self.context, request)?;
                return Ok(CaptureArtifact {
                    target_id: request.target_id.clone(),
                    media_type: "image/png".to_owned(),
                    image_bytes,
                    captured_at: current_timestamp(),
                });
            }
            _ => {
                return Err(PlatformAdapterError::unsupported(
                    match request.target {
                        CaptureTargetKind::Window => Capability::WindowCapture,
                        CaptureTargetKind::Display => Capability::DisplayCapture,
                    },
                    self.platform(),
                    CapabilityErrorReason::UnsupportedSession,
                    "Capture requires a detected graphical desktop session.",
                    Some("Run Tendril from a graphical login session and retry."),
                ));
            }
        }

        let image_bytes = x11::capture_target(&self.context, request.target, &request.target_id)?;

        Ok(CaptureArtifact {
            target_id: request.target_id.clone(),
            media_type: "image/png".to_owned(),
            image_bytes,
            captured_at: current_timestamp(),
        })
    }
}

impl InputControlAdapter for LinuxAdapter {
    fn input_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
        match self.context.session {
            DesktopSession::X11 => Ok(FeatureSupport::available(Capability::InputControl)
                .with_notes(vec![
                    "Input injection remains stateless and target-scoped on X11.".to_owned(),
                ])),
            DesktopSession::Wayland => Err(PlatformAdapterError::unsupported(
                Capability::InputControl,
                self.platform(),
                CapabilityErrorReason::UnsupportedSession,
                "Wayland input injection is compositor-specific and is not exposed by the generic Linux adapter spine.",
                Some(
                    "Use an X11 session or implement a compositor-specific backend with explicit permissions.",
                ),
            )),
            _ => Err(PlatformAdapterError::unsupported(
                Capability::InputControl,
                self.platform(),
                CapabilityErrorReason::UnsupportedSession,
                "Input control requires a detected graphical desktop session.",
                Some("Run Tendril inside X11 or a supported Wayland compositor session."),
            )),
        }
    }

    fn execute_input(&self, request: &InputRequest) -> Result<InputOutcome, TendrilError> {
        if let Some(outcome) = load_input_fixture(self.platform(), request)? {
            return Ok(outcome);
        }

        execute_linux_input(self.platform(), self.context.session, request)
    }
}

impl PermissionAdapter for LinuxAdapter {
    fn permissions(&self) -> Vec<PermissionStatus> {
        vec![
            PermissionStatus::not_required(
                PermissionKind::ScreenCapture,
                "Linux desktop capture typically relies on session capabilities rather than OS privacy prompts.",
            ),
            PermissionStatus::not_required(
                PermissionKind::Accessibility,
                "Linux input control generally depends on session support instead of a centralized permission prompt.",
            ),
            PermissionStatus::not_required(
                PermissionKind::Microphone,
                "Audio device access is usually mediated by the active audio stack and user session.",
            ),
        ]
    }
}

impl AudioCapabilityProbe for LinuxAdapter {
    fn probe_audio_capture(
        &self,
        request: &AudioProbeRequest,
    ) -> Result<AudioCapabilityReport, PlatformAdapterError> {
        let backend = self.audio_backend();
        if backend == AudioBackend::Unknown {
            return Err(PlatformAdapterError::unsupported(
                match request.source {
                    AudioSourceKind::SystemLoopback => Capability::AudioLoopbackCapture,
                    AudioSourceKind::Microphone => Capability::AudioInputCapture,
                },
                self.platform(),
                CapabilityErrorReason::UnsupportedSession,
                "The Linux adapter could not detect a supported audio backend.",
                Some("Run Tendril inside a PipeWire or PulseAudio user session and retry."),
            ));
        }

        Ok(AudioCapabilityReport {
            source: request.source,
            backend,
            supported_sample_rates_hz: vec![16_000, 44_100, 48_000],
            supported_channel_counts: vec![1, 2],
            permissions: vec![PermissionStatus::not_required(
                PermissionKind::Microphone,
                "Audio capture support is determined by the active Linux audio backend.",
            )],
            notes: vec![match request.source {
                AudioSourceKind::SystemLoopback => {
                    "Loopback capture support depends on monitor/portal exposure from the active backend."
                        .to_owned()
                }
                AudioSourceKind::Microphone => {
                    "Microphone capture is explicit and command-scoped; no hidden recording session is retained."
                        .to_owned()
                }
            }],
        })
    }
}

impl PlatformAdapter for LinuxAdapter {
    fn info(&self) -> AdapterInfo {
        AdapterInfo::from_context(&self.context)
    }
}

#[derive(Debug, Clone)]
pub struct WindowsAdapter {
    context: AdapterContext,
}

impl WindowsAdapter {
    #[must_use]
    pub fn new(context: AdapterContext) -> Self {
        Self { context }
    }

    fn platform(&self) -> PlatformKind {
        self.context.platform
    }

    fn microphone_permission() -> PermissionStatus {
        PermissionStatus::unknown(
            PermissionKind::Microphone,
            "Windows microphone capture may be gated by the Privacy > Microphone setting for desktop apps.",
            "Enable microphone access for desktop apps in Settings > Privacy & security > Microphone before retrying.",
        )
    }

    fn capture_display(&self, target_id: &str) -> Result<Vec<u8>, PlatformAdapterError> {
        WINDOWS_RUNTIME.capture_display(self.platform(), target_id)
    }

    fn capture_window(&self, target_id: &str) -> Result<Vec<u8>, PlatformAdapterError> {
        WINDOWS_RUNTIME.capture_window(self.platform(), target_id)
    }
}

impl TargetDiscoveryAdapter for WindowsAdapter {
    fn target_discovery_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
        Ok(
            FeatureSupport::available(Capability::TargetDiscovery).with_notes(vec![
            "The Windows adapter remains stateless; targets are discovered fresh for each command."
                .to_owned(),
        ]),
        )
    }

    fn discover_targets(
        &self,
        request: &TargetDiscoveryRequest,
    ) -> Result<TargetInventory, PlatformAdapterError> {
        discovery::discover_targets(&self.context, request)
    }
}

impl CaptureAdapter for WindowsAdapter {
    fn capture_support(
        &self,
        target: CaptureTargetKind,
    ) -> Result<FeatureSupport, PlatformAdapterError> {
        Ok(FeatureSupport::available(match target {
            CaptureTargetKind::Window => Capability::WindowCapture,
            CaptureTargetKind::Display => Capability::DisplayCapture,
        })
        .with_notes(vec![
            "Windows capture support does not require a Tendril-managed background service."
                .to_owned(),
        ]))
    }

    fn capture(&self, request: &CaptureRequest) -> Result<CaptureArtifact, PlatformAdapterError> {
        if let Some(artifact) = load_capture_fixture(self.platform(), request)? {
            return Ok(artifact);
        }

        let image_bytes = match request.target {
            CaptureTargetKind::Display => self.capture_display(&request.target_id)?,
            CaptureTargetKind::Window => self.capture_window(&request.target_id)?,
        };

        Ok(CaptureArtifact {
            target_id: request.target_id.clone(),
            media_type: "image/png".to_owned(),
            image_bytes,
            captured_at: current_timestamp(),
        })
    }
}

impl InputControlAdapter for WindowsAdapter {
    fn input_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
        Ok(FeatureSupport::available(Capability::InputControl).with_notes(vec![
            "The adapter may still need to report focus transfer when a target cannot receive background input."
                .to_owned(),
        ]))
    }

    fn execute_input(&self, request: &InputRequest) -> Result<InputOutcome, TendrilError> {
        if let Some(outcome) = load_input_fixture(self.platform(), request)? {
            return Ok(outcome);
        }

        execute_windows_input(self.platform(), request)
    }
}

impl PermissionAdapter for WindowsAdapter {
    fn permissions(&self) -> Vec<PermissionStatus> {
        vec![
            PermissionStatus::not_required(
                PermissionKind::ScreenCapture,
                "Desktop capture does not require a separate Windows privacy prompt for classic desktop apps.",
            ),
            PermissionStatus::not_required(
                PermissionKind::Accessibility,
                "Desktop input control uses user-session APIs rather than a dedicated accessibility prompt.",
            ),
            Self::microphone_permission(),
        ]
    }
}

impl AudioCapabilityProbe for WindowsAdapter {
    fn probe_audio_capture(
        &self,
        request: &AudioProbeRequest,
    ) -> Result<AudioCapabilityReport, PlatformAdapterError> {
        Ok(AudioCapabilityReport {
            source: request.source,
            backend: AudioBackend::Wasapi,
            supported_sample_rates_hz: vec![16_000, 44_100, 48_000],
            supported_channel_counts: vec![1, 2],
            permissions: match request.source {
                AudioSourceKind::SystemLoopback => vec![PermissionStatus::not_required(
                    PermissionKind::Microphone,
                    "WASAPI loopback capture does not require microphone access.",
                )],
                AudioSourceKind::Microphone => vec![Self::microphone_permission()],
            },
            notes: vec![match request.source {
                AudioSourceKind::SystemLoopback => {
                    "System loopback is exposed via WASAPI and remains command-scoped."
                        .to_owned()
                }
                AudioSourceKind::Microphone => {
                    "Microphone capture may require desktop-app privacy approval in addition to device availability."
                        .to_owned()
                }
            }],
        })
    }
}

impl PlatformAdapter for WindowsAdapter {
    fn info(&self) -> AdapterInfo {
        AdapterInfo::from_context(&self.context)
    }
}

fn parse_display_index(
    target_id: &str,
    platform: PlatformKind,
) -> Result<u32, PlatformAdapterError> {
    target_id.parse::<u32>().map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            platform,
            format!("display target id `{target_id}` must be a numeric index (e.g. 1, 2): {error}"),
        )
    })
}

#[derive(Debug, Clone)]
struct WaylandCaptureBackendFailure {
    message: String,
    missing_backend: bool,
}

fn capture_wayland_target(
    context: &AdapterContext,
    request: &CaptureRequest,
) -> Result<Vec<u8>, PlatformAdapterError> {
    let inventory = discovery::discover_targets(context, &TargetDiscoveryRequest)?;
    let target = inventory
        .targets
        .iter()
        .find(|target| target.kind == request.target && target.id == request.target_id)
        .cloned()
        .ok_or_else(|| {
            PlatformAdapterError::adapter_failure(
                AdapterOperation::Capture,
                context.platform,
                format!(
                    "target `{}` was not found during Wayland capture",
                    request.target_id
                ),
            )
        })?;

    let portal_error = match capture_wayland_target_via_portal(&target, &inventory) {
        Ok(image_bytes) => return Ok(image_bytes),
        Err(error) => error,
    };

    if !wayland_capture_program_on_path("grim") {
        return if portal_error.missing_backend {
            Err(PlatformAdapterError::unsupported(
                wayland_capture_capability(request.target),
                context.platform,
                CapabilityErrorReason::UnsupportedFeature,
                format!(
                    "Wayland capture needs either an xdg-desktop-portal screenshot backend or the `grim` compatibility fallback for this session; portal capture failed: {}",
                    portal_error.message
                ),
                Some(
                    "Install and run an xdg-desktop-portal screenshot backend for the active compositor, or install `grim` as a compatibility fallback if that compositor permits geometry-scoped screenshots.",
                ),
            ))
        } else {
            Err(PlatformAdapterError::adapter_failure(
                AdapterOperation::Capture,
                context.platform,
                format!(
                    "Wayland capture via xdg-desktop-portal screenshot failed (`{}`), and the `grim` compatibility fallback is not installed.",
                    portal_error.message
                ),
            ))
        };
    }

    capture_wayland_target_with_grim(&target).map_err(|grim_error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            context.platform,
            format!(
                "Wayland capture failed via xdg-desktop-portal screenshot (`{}`) and grim fallback (`{grim_error}`).",
                portal_error.message
            ),
        )
    })
}

fn capture_wayland_target_via_portal(
    target: &TargetDescriptor,
    inventory: &TargetInventory,
) -> Result<Vec<u8>, WaylandCaptureBackendFailure> {
    let screenshot = block_on(async {
        PortalScreenshot::request()
            .interactive(false)
            .modal(true)
            .send()
            .await?
            .response()
    })
    .map_err(|error| classify_wayland_portal_capture_error(&error))?;

    let path = screenshot
        .uri()
        .to_file_path()
        .map_err(|()| WaylandCaptureBackendFailure {
            message: format!(
                "xdg-desktop-portal screenshot returned a non-file URI: {}",
                screenshot.uri()
            ),
            missing_backend: false,
        })?;
    let image_bytes = std::fs::read(&path).map_err(|error| WaylandCaptureBackendFailure {
        message: format!(
            "failed to read xdg-desktop-portal screenshot artifact `{}`: {error}",
            path.display()
        ),
        missing_backend: false,
    })?;
    let _ = std::fs::remove_file(&path);

    crop_wayland_portal_capture_to_target(&image_bytes, target, inventory)
}

fn classify_wayland_portal_capture_error(error: &ashpd::Error) -> WaylandCaptureBackendFailure {
    let missing_backend = match error {
        ashpd::Error::NoResponse
        | ashpd::Error::RequiresVersion(_, _)
        | ashpd::Error::Portal(ashpd::PortalError::NotFound(_)) => true,
        ashpd::Error::Zbus(zbus_error) => {
            let message = zbus_error.to_string().to_ascii_lowercase();
            message.contains("serviceunknown")
                || message.contains("unknownmethod")
                || message.contains("name has no owner")
                || message.contains("org.freedesktop.portal.desktop")
                || message.contains("org.freedesktop.portal.screenshot")
        }
        _ => false,
    };

    WaylandCaptureBackendFailure {
        message: error.to_string(),
        missing_backend,
    }
}

fn crop_wayland_portal_capture_to_target(
    image_bytes: &[u8],
    target: &TargetDescriptor,
    inventory: &TargetInventory,
) -> Result<Vec<u8>, WaylandCaptureBackendFailure> {
    let image =
        image::load_from_memory(image_bytes).map_err(|error| WaylandCaptureBackendFailure {
            message: format!(
                "failed to decode xdg-desktop-portal screenshot image for `{}`: {error}",
                target.id
            ),
            missing_backend: false,
        })?;

    let (origin_x, origin_y) = wayland_workspace_origin(inventory);
    let crop_x = i64::from(target.bounds.x) - i64::from(origin_x);
    let crop_y = i64::from(target.bounds.y) - i64::from(origin_y);

    if crop_x < 0 || crop_y < 0 {
        return Err(WaylandCaptureBackendFailure {
            message: format!(
                "xdg-desktop-portal screenshot for `{}` did not cover the requested target origin after workspace translation (crop origin {crop_x},{crop_y}).",
                target.id
            ),
            missing_backend: false,
        });
    }

    let crop_x = u32::try_from(crop_x).expect("validated crop x should fit u32");
    let crop_y = u32::try_from(crop_y).expect("validated crop y should fit u32");
    if crop_x == 0
        && crop_y == 0
        && target.bounds.width == image.width()
        && target.bounds.height == image.height()
    {
        return Ok(image_bytes.to_vec());
    }

    if crop_x.saturating_add(target.bounds.width) > image.width()
        || crop_y.saturating_add(target.bounds.height) > image.height()
    {
        return Err(WaylandCaptureBackendFailure {
            message: format!(
                "xdg-desktop-portal screenshot dimensions {}x{} did not cover target `{}` at {}x{}+{}+{}.",
                image.width(),
                image.height(),
                target.id,
                target.bounds.width,
                target.bounds.height,
                target.bounds.x,
                target.bounds.y,
            ),
            missing_backend: false,
        });
    }

    let cropped = image.crop_imm(crop_x, crop_y, target.bounds.width, target.bounds.height);
    let mut encoded = Vec::new();
    cropped
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Png,
        )
        .map_err(|error| WaylandCaptureBackendFailure {
            message: format!(
                "failed to encode cropped xdg-desktop-portal screenshot for `{}`: {error}",
                target.id
            ),
            missing_backend: false,
        })?;
    Ok(encoded)
}

fn wayland_workspace_origin(inventory: &TargetInventory) -> (i32, i32) {
    let mut displays = inventory
        .targets
        .iter()
        .filter(|target| target.kind == CaptureTargetKind::Display)
        .map(|target| (target.bounds.x, target.bounds.y));
    let Some((mut min_x, mut min_y)) = displays.next() else {
        return (0, 0);
    };

    for (x, y) in displays {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
    }

    (min_x, min_y)
}

fn capture_wayland_target_with_grim(target: &TargetDescriptor) -> Result<Vec<u8>, String> {
    let path = unique_temp_path("png");
    let geometry = format!(
        "{}x{}+{}+{}",
        target.bounds.width, target.bounds.height, target.bounds.x, target.bounds.y
    );
    let mut command = std::process::Command::new("grim");
    command
        .arg("-t")
        .arg("png")
        .arg("-g")
        .arg(geometry)
        .arg(&path);
    let output = command
        .output()
        .map_err(|error| format!("failed to spawn grim: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(if stderr.trim().is_empty() {
            format!("grim exited with status {}", output.status)
        } else {
            format!("grim failed: {}", stderr.trim())
        });
    }

    read_and_remove_temp_capture(&path).map_err(|error| error.to_string())
}

fn wayland_capture_capability(target: CaptureTargetKind) -> Capability {
    match target {
        CaptureTargetKind::Window => Capability::WindowCapture,
        CaptureTargetKind::Display => Capability::DisplayCapture,
    }
}

fn wayland_capture_program_on_path(program: &str) -> bool {
    env::var_os("PATH").is_some_and(|path| {
        env::split_paths(&path).any(|entry| {
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

fn execute_linux_input(
    platform: PlatformKind,
    session: DesktopSession,
    request: &InputRequest,
) -> Result<InputOutcome, TendrilError> {
    if session != DesktopSession::X11 {
        return Err(TendrilError::from(PlatformAdapterError::unsupported(
            Capability::InputControl,
            platform,
            CapabilityErrorReason::UnsupportedFeature,
            "Wayland input injection remains compositor-specific and is not enabled by the generic Linux run surface.",
            Some(
                "Use X11 for input automation today; Wayland support in this bead covers native discovery plus capture backends.",
            ),
        )));
    }

    x11::execute_input(platform, request)
}

fn execute_macos_input(
    _platform: PlatformKind,
    request: &InputRequest,
) -> Result<InputOutcome, TendrilError> {
    let keyboard_input = request.text.is_some() || request.actions.iter().any(action_is_keyboard);
    let mut focus_required = keyboard_input || matches!(request.target, CaptureTargetKind::Window);
    let mut focus_transferred = false;
    let mut notes = Vec::new();

    if matches!(request.target, CaptureTargetKind::Window) {
        if let Some(process_id) = request.process_id {
            let script = macos_focus_pid_jxa_script(process_id);
            run_macos_osascript_jxa_for_input(&script, "focus", None, None)?;
            focus_transferred = true;
            notes.push(
                "Activated the target macOS application via NSRunningApplication before dispatching input."
                    .to_owned(),
            );
            std::thread::sleep(reliability_delay());
        } else if let Some(app_name) = &request.app_name {
            let script = macos_focus_app_jxa_script(app_name);
            run_macos_osascript_jxa_for_input(&script, "focus", None, None)?;
            focus_transferred = true;
            notes.push(
                "Activated the target macOS application by name before dispatching input."
                    .to_owned(),
            );
            std::thread::sleep(reliability_delay());
        } else {
            notes.push(
                "Target focus could not be transferred automatically because discovery did not expose a process or app name."
                    .to_owned(),
            );
        }
    } else if keyboard_input {
        notes.push(
            "Display-scoped keyboard input posts into the current macOS focus chain; transfer focus manually when needed."
                .to_owned(),
        );
    }

    if let Some(text) = &request.text {
        let script = macos_text_jxa_script(text);
        run_macos_osascript_jxa_for_input(&script, "dispatch", Some(0), Some("text"))?;
        return Ok(InputOutcome {
            action_count: 1,
            focus_required,
            focus_transferred,
            focused_target: if focus_transferred {
                Some(request.target_id.clone())
            } else {
                None
            },
            notes,
        });
    }

    for (action_index, action) in request.actions.iter().enumerate() {
        let label = action_label(action);
        dispatch_macos_action(request, action, action_index, &label)?;
        if !matches!(action, InputAction::Wait { .. }) {
            std::thread::sleep(reliability_delay());
        }
    }

    if !keyboard_input && matches!(request.target, CaptureTargetKind::Display) {
        focus_required = false;
    }

    Ok(InputOutcome {
        action_count: request.actions.len(),
        focus_required,
        focus_transferred,
        focused_target: if focus_transferred {
            Some(request.target_id.clone())
        } else {
            None
        },
        notes,
    })
}

fn dispatch_macos_action(
    request: &InputRequest,
    action: &InputAction,
    action_index: usize,
    label: &str,
) -> Result<(), TendrilError> {
    match action {
        InputAction::KeyTap { key } => {
            let script = macos_key_jxa_script(key, Some(true), Some(true))?;
            run_macos_osascript_jxa_for_input(&script, "dispatch", Some(action_index), Some(label))
        }
        InputAction::Hold { modifier } => {
            let script = macos_modifier_jxa_script(*modifier, true)?;
            run_macos_osascript_jxa_for_input(&script, "dispatch", Some(action_index), Some(label))
        }
        InputAction::Release { modifier } => {
            let script = macos_modifier_jxa_script(*modifier, false)?;
            run_macos_osascript_jxa_for_input(&script, "dispatch", Some(action_index), Some(label))
        }
        InputAction::Send { text } => {
            let script = macos_text_jxa_script(text);
            run_macos_osascript_jxa_for_input(&script, "dispatch", Some(action_index), Some(label))
        }
        InputAction::Wait { duration_ms } => {
            std::thread::sleep(Duration::from_millis(*duration_ms));
            Ok(())
        }
        InputAction::Click { button, x, y } => {
            let (absolute_x, absolute_y) = relative_point_to_absolute(&request.bounds, *x, *y);
            let script = macos_mouse_jxa_script(*button, absolute_x, absolute_y, None);
            run_macos_osascript_jxa_for_input(&script, "dispatch", Some(action_index), Some(label))
        }
        InputAction::Drag { x0, y0, x1, y1 } => {
            let (start_x, start_y) = relative_point_to_absolute(&request.bounds, *x0, *y0);
            let (end_x, end_y) = relative_point_to_absolute(&request.bounds, *x1, *y1);
            let script =
                macos_mouse_jxa_script(MouseButton::Left, start_x, start_y, Some((end_x, end_y)));
            run_macos_osascript_jxa_for_input(&script, "dispatch", Some(action_index), Some(label))
        }
    }
}

trait WindowsRuntimeBackend {
    fn capture_display(
        &self,
        platform: PlatformKind,
        target_id: &str,
    ) -> Result<Vec<u8>, PlatformAdapterError>;
    fn capture_window(
        &self,
        platform: PlatformKind,
        target_id: &str,
    ) -> Result<Vec<u8>, PlatformAdapterError>;
    fn focus_window(&self, target_id: &str) -> Result<(), String>;
    fn send_text(&self, text: &str) -> Result<(), String>;
    fn tap_key(&self, key: &str) -> Result<(), String>;
    fn hold_modifier(&self, modifier: ModifierKey) -> Result<(), String>;
    fn release_modifier(&self, modifier: ModifierKey) -> Result<(), String>;
    fn click_mouse(&self, button: MouseButton, x: i32, y: i32) -> Result<(), String>;
    fn drag_mouse(
        &self,
        button: MouseButton,
        start_x: i32,
        start_y: i32,
        end_x: i32,
        end_y: i32,
    ) -> Result<(), String>;
}

#[derive(Debug, Default, Clone, Copy)]
struct NativeWindowsRuntime;

static WINDOWS_RUNTIME: NativeWindowsRuntime = NativeWindowsRuntime;

impl WindowsRuntimeBackend for NativeWindowsRuntime {
    fn capture_display(
        &self,
        platform: PlatformKind,
        target_id: &str,
    ) -> Result<Vec<u8>, PlatformAdapterError> {
        tendril_win32::capture_display_png(target_id).map_err(|error| {
            PlatformAdapterError::adapter_failure(
                AdapterOperation::Capture,
                platform,
                format!("Windows display capture failed: {error}"),
            )
        })
    }

    fn capture_window(
        &self,
        platform: PlatformKind,
        target_id: &str,
    ) -> Result<Vec<u8>, PlatformAdapterError> {
        tendril_win32::capture_window_png(target_id).map_err(|error| {
            PlatformAdapterError::adapter_failure(
                AdapterOperation::Capture,
                platform,
                format!("Windows window capture failed: {error}"),
            )
        })
    }

    fn focus_window(&self, target_id: &str) -> Result<(), String> {
        tendril_win32::focus_window(target_id)
    }

    fn send_text(&self, text: &str) -> Result<(), String> {
        tendril_win32::send_text(text)
    }

    fn tap_key(&self, key: &str) -> Result<(), String> {
        tendril_win32::tap_key(key)
    }

    fn hold_modifier(&self, modifier: ModifierKey) -> Result<(), String> {
        tendril_win32::hold_modifier(windows_runtime_modifier(modifier))
    }

    fn release_modifier(&self, modifier: ModifierKey) -> Result<(), String> {
        tendril_win32::release_modifier(windows_runtime_modifier(modifier))
    }

    fn click_mouse(&self, button: MouseButton, x: i32, y: i32) -> Result<(), String> {
        tendril_win32::click_mouse(windows_runtime_mouse_button(button), x, y)
    }

    fn drag_mouse(
        &self,
        button: MouseButton,
        start_x: i32,
        start_y: i32,
        end_x: i32,
        end_y: i32,
    ) -> Result<(), String> {
        tendril_win32::drag_mouse(
            windows_runtime_mouse_button(button),
            start_x,
            start_y,
            end_x,
            end_y,
        )
    }
}

fn execute_windows_input(
    platform: PlatformKind,
    request: &InputRequest,
) -> Result<InputOutcome, TendrilError> {
    execute_windows_input_with_runtime(platform, request, &WINDOWS_RUNTIME)
}

fn execute_windows_input_with_runtime(
    _platform: PlatformKind,
    request: &InputRequest,
    runtime: &dyn WindowsRuntimeBackend,
) -> Result<InputOutcome, TendrilError> {
    let keyboard_input = request.text.is_some() || request.actions.iter().any(action_is_keyboard);
    let mut focus_required = keyboard_input || matches!(request.target, CaptureTargetKind::Window);
    let mut focus_transferred = false;
    let mut notes = Vec::new();

    if matches!(request.target, CaptureTargetKind::Window) {
        run_windows_step(
            runtime.focus_window(&request.target_id),
            "focus",
            None,
            None,
        )?;
        focus_transferred = true;
        notes.push(
            "Activated the target window with native Win32 focus APIs before dispatching Windows input."
                .to_owned(),
        );
        std::thread::sleep(reliability_delay());
    } else if keyboard_input {
        notes.push(
            "Display-scoped keyboard input uses the currently focused Windows control; transfer focus manually when required."
                .to_owned(),
        );
    }

    if let Some(text) = &request.text {
        run_windows_step(runtime.send_text(text), "dispatch", Some(0), Some("text"))?;
        return Ok(InputOutcome {
            action_count: 1,
            focus_required,
            focus_transferred,
            focused_target: if focus_transferred {
                Some(request.target_id.clone())
            } else {
                None
            },
            notes,
        });
    }

    for (action_index, action) in request.actions.iter().enumerate() {
        let label = action_label(action);
        dispatch_windows_action_with_runtime(runtime, request, action, action_index, &label)?;
        if !matches!(action, InputAction::Wait { .. }) {
            std::thread::sleep(reliability_delay());
        }
    }

    if !keyboard_input && matches!(request.target, CaptureTargetKind::Display) {
        focus_required = false;
    }

    Ok(InputOutcome {
        action_count: request.actions.len(),
        focus_required,
        focus_transferred,
        focused_target: if focus_transferred {
            Some(request.target_id.clone())
        } else {
            None
        },
        notes,
    })
}

fn dispatch_windows_action_with_runtime(
    runtime: &dyn WindowsRuntimeBackend,
    request: &InputRequest,
    action: &InputAction,
    action_index: usize,
    label: &str,
) -> Result<(), TendrilError> {
    match action {
        InputAction::KeyTap { key } => {
            if !windows_key_is_supported(key) {
                return Err(TendrilError::execution_failure(
                    "unsupported_key",
                    format!("unsupported Windows key `{key}`"),
                    None,
                ));
            }
            run_windows_step(
                runtime.tap_key(key),
                "dispatch",
                Some(action_index),
                Some(label),
            )
        }
        InputAction::Hold { modifier } => run_windows_step(
            runtime.hold_modifier(*modifier),
            "dispatch",
            Some(action_index),
            Some(label),
        ),
        InputAction::Release { modifier } => run_windows_step(
            runtime.release_modifier(*modifier),
            "dispatch",
            Some(action_index),
            Some(label),
        ),
        InputAction::Send { text } => run_windows_step(
            runtime.send_text(text),
            "dispatch",
            Some(action_index),
            Some(label),
        ),
        InputAction::Wait { duration_ms } => {
            std::thread::sleep(Duration::from_millis(*duration_ms));
            Ok(())
        }
        InputAction::Click { button, x, y } => {
            let (absolute_x, absolute_y) = relative_point_to_absolute(&request.bounds, *x, *y);
            run_windows_step(
                runtime.click_mouse(*button, absolute_x, absolute_y),
                "dispatch",
                Some(action_index),
                Some(label),
            )
        }
        InputAction::Drag { x0, y0, x1, y1 } => {
            let (start_x, start_y) = relative_point_to_absolute(&request.bounds, *x0, *y0);
            let (end_x, end_y) = relative_point_to_absolute(&request.bounds, *x1, *y1);
            run_windows_step(
                runtime.drag_mouse(MouseButton::Left, start_x, start_y, end_x, end_y),
                "dispatch",
                Some(action_index),
                Some(label),
            )
        }
    }
}

fn run_windows_step(
    result: Result<(), String>,
    stage: &'static str,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    result.map_err(|error| {
        input_execution_error(
            "input_command_failed",
            format!("Windows input failed: {error}"),
            stage,
            action_index,
            action,
        )
    })
}

fn windows_runtime_modifier(modifier: ModifierKey) -> tendril_win32::ModifierKey {
    match modifier {
        ModifierKey::Ctrl => tendril_win32::ModifierKey::Ctrl,
        ModifierKey::Alt => tendril_win32::ModifierKey::Alt,
        ModifierKey::Shift => tendril_win32::ModifierKey::Shift,
        ModifierKey::Meta => tendril_win32::ModifierKey::Meta,
    }
}

fn windows_runtime_mouse_button(button: MouseButton) -> tendril_win32::MouseButton {
    match button {
        MouseButton::Left => tendril_win32::MouseButton::Left,
        MouseButton::Middle => tendril_win32::MouseButton::Middle,
        MouseButton::Right => tendril_win32::MouseButton::Right,
    }
}

fn windows_key_is_supported(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "enter"
            | "return"
            | "esc"
            | "escape"
            | "tab"
            | "space"
            | "backspace"
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
    ) || key.chars().count() == 1
}

fn action_is_keyboard(action: &InputAction) -> bool {
    matches!(
        action,
        InputAction::KeyTap { .. }
            | InputAction::Hold { .. }
            | InputAction::Release { .. }
            | InputAction::Send { .. }
    )
}

fn action_label(action: &InputAction) -> String {
    serde_json::to_string(action).unwrap_or_else(|_| format!("{action:?}"))
}

fn input_execution_error(
    code: &'static str,
    message: String,
    stage: &'static str,
    action_index: Option<usize>,
    action: Option<&str>,
) -> TendrilError {
    let mut error = TendrilError::execution_failure(code, message, action_index)
        .with_detail_entry("stage", json!(stage));
    if let Some(action_index) = action_index {
        error = error.with_detail_entry("action_number", json!(action_index + 1));
    }
    if let Some(action) = action {
        error = error.with_detail_entry("action", json!(action));
    }
    error
}

fn javascript_string_literal(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| String::from("\"\""))
}

fn macos_focus_pid_jxa_script(process_id: u32) -> String {
    format!(
        r"ObjC.import('AppKit');
(function () {{
    var app = $.NSRunningApplication.runningApplicationWithProcessIdentifier({process_id});
    if (!app) {{
        throw new Error('target process could not be activated');
    }}
    if (!app.activateWithOptions($.NSApplicationActivateIgnoringOtherApps)) {{
        throw new Error('target process could not be activated');
    }}
}}());
"
    )
}

fn macos_focus_app_jxa_script(app_name: &str) -> String {
    let app_name = javascript_string_literal(app_name);
    format!(
        r"var app = Application({app_name});
app.activate();
"
    )
}

fn macos_text_jxa_script(text: &str) -> String {
    let text = javascript_string_literal(text);
    format!(
        r"var systemEvents = Application('System Events');
systemEvents.includeStandardAdditions = true;
systemEvents.keystroke({text});
"
    )
}

fn macos_modifier_jxa_script(modifier: ModifierKey, down: bool) -> Result<String, TendrilError> {
    let key_code = macos_key_code(match modifier {
        ModifierKey::Ctrl => "ctrl",
        ModifierKey::Alt => "alt",
        ModifierKey::Shift => "shift",
        ModifierKey::Meta => "meta",
    })?;
    Ok(macos_keyboard_jxa_script(key_code, down, false))
}

fn macos_key_jxa_script(
    key: &str,
    down: Option<bool>,
    up: Option<bool>,
) -> Result<String, TendrilError> {
    let key_code = macos_key_code(key)?;
    Ok(macos_keyboard_jxa_script(
        key_code,
        down.unwrap_or(true),
        up.unwrap_or(true),
    ))
}

fn macos_keyboard_jxa_script(key_code: u16, post_down: bool, post_up: bool) -> String {
    format!(
        r"ObjC.import('CoreGraphics');
(function () {{
    function post(keyDown) {{
        var event = $.CGEventCreateKeyboardEvent(null, {key_code}, keyDown);
        if (!event) {{
            throw new Error('failed to create keyboard event');
        }}
        $.CGEventPost($.kCGHIDEventTap, event);
    }}
    {post_down_call}{post_up_call}
}}());
",
        post_down_call = if post_down { "post(true);\n    " } else { "" },
        post_up_call = if post_up { "post(false);\n" } else { "" },
    )
}

fn macos_mouse_jxa_script(
    button: MouseButton,
    x: i32,
    y: i32,
    drag_end: Option<(i32, i32)>,
) -> String {
    let (down_event, up_event, drag_event, button_code) = match button {
        MouseButton::Left => (
            "kCGEventLeftMouseDown",
            "kCGEventLeftMouseUp",
            "kCGEventLeftMouseDragged",
            0,
        ),
        MouseButton::Right => (
            "kCGEventRightMouseDown",
            "kCGEventRightMouseUp",
            "kCGEventRightMouseDragged",
            1,
        ),
        MouseButton::Middle => (
            "kCGEventOtherMouseDown",
            "kCGEventOtherMouseUp",
            "kCGEventOtherMouseDragged",
            2,
        ),
    };
    if let Some((end_x, end_y)) = drag_end {
        format!(
            r"ObjC.import('CoreGraphics');
(function () {{
    function point(px, py) {{
        return $.CGPointMake(px, py);
    }}
    function postMouse(eventType, px, py) {{
        var event = $.CGEventCreateMouseEvent(null, $[eventType], point(px, py), {button_code});
        if (!event) {{
            throw new Error('failed to create mouse event');
        }}
        $.CGEventPost($.kCGHIDEventTap, event);
    }}
    $.CGWarpMouseCursorPosition(point({x}, {y}));
    postMouse('kCGEventMouseMoved', {x}, {y});
    postMouse('{down_event}', {x}, {y});
    postMouse('{drag_event}', {end_x}, {end_y});
    postMouse('{up_event}', {end_x}, {end_y});
}}());
"
        )
    } else {
        format!(
            r"ObjC.import('CoreGraphics');
(function () {{
    function point(px, py) {{
        return $.CGPointMake(px, py);
    }}
    function postMouse(eventType, px, py) {{
        var event = $.CGEventCreateMouseEvent(null, $[eventType], point(px, py), {button_code});
        if (!event) {{
            throw new Error('failed to create mouse event');
        }}
        $.CGEventPost($.kCGHIDEventTap, event);
    }}
    $.CGWarpMouseCursorPosition(point({x}, {y}));
    postMouse('kCGEventMouseMoved', {x}, {y});
    postMouse('{down_event}', {x}, {y});
    postMouse('{up_event}', {x}, {y});
}}());
"
        )
    }
}

fn run_macos_osascript_jxa_for_input(
    script: &str,
    stage: &'static str,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    let output = std::process::Command::new("osascript")
        .args(["-l", "JavaScript", "-e", script])
        .output()
        .map_err(|error| {
            input_execution_error(
                "input_spawn_failed",
                format!("failed to spawn osascript: {error}"),
                stage,
                action_index,
                action,
            )
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let message = if stdout.is_empty() || stderr.is_empty() {
        format!("{stdout}{stderr}")
    } else {
        format!("{stdout} | {stderr}")
    };

    if is_macos_input_permission_error(&message) {
        let mut error = TendrilError::from(PlatformAdapterError::missing_permission(
            Capability::InputControl,
            PermissionKind::Accessibility,
            PlatformKind::MacOs,
            if message.is_empty() {
                "macOS input control requires Accessibility consent.".to_owned()
            } else {
                format!("macOS input control requires Accessibility consent: {message}")
            },
            "Grant Accessibility access in System Settings > Privacy & Security > Accessibility, then rerun tendril run.",
        ))
        .with_detail_entry("stage", json!(stage));
        if let Some(action_index) = action_index {
            error = error.with_detail_entry("action_number", json!(action_index + 1));
        }
        if let Some(action) = action {
            error = error.with_detail_entry("action", json!(action));
        }
        return Err(error);
    }

    Err(input_execution_error(
        "input_command_failed",
        if message.is_empty() {
            format!("osascript exited with status {}", output.status)
        } else {
            format!("osascript failed: {message}")
        },
        stage,
        action_index,
        action,
    ))
}

fn is_macos_input_permission_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("accessibility")
        || lower.contains("assistive access")
        || lower.contains("system events got an error")
        || lower.contains("apple events")
        || lower.contains("not authorized")
        || lower.contains("not permitted")
        || lower.contains("-1719")
        || lower.contains("-1743")
}

fn macos_key_code(key: &str) -> Result<u16, TendrilError> {
    match key.to_ascii_lowercase().as_str() {
        "a" => Ok(0),
        "s" => Ok(1),
        "d" => Ok(2),
        "f" => Ok(3),
        "h" => Ok(4),
        "g" => Ok(5),
        "z" => Ok(6),
        "x" => Ok(7),
        "c" => Ok(8),
        "v" => Ok(9),
        "b" => Ok(11),
        "q" => Ok(12),
        "w" => Ok(13),
        "e" => Ok(14),
        "r" => Ok(15),
        "y" => Ok(16),
        "t" => Ok(17),
        "1" => Ok(18),
        "2" => Ok(19),
        "3" => Ok(20),
        "4" => Ok(21),
        "6" => Ok(22),
        "5" => Ok(23),
        "=" => Ok(24),
        "9" => Ok(25),
        "7" => Ok(26),
        "-" => Ok(27),
        "8" => Ok(28),
        "0" => Ok(29),
        "]" => Ok(30),
        "o" => Ok(31),
        "u" => Ok(32),
        "[" => Ok(33),
        "i" => Ok(34),
        "p" => Ok(35),
        "enter" | "return" => Ok(36),
        "l" => Ok(37),
        "j" => Ok(38),
        "'" => Ok(39),
        "k" => Ok(40),
        ";" => Ok(41),
        "\\" => Ok(42),
        "," => Ok(43),
        "/" => Ok(44),
        "n" => Ok(45),
        "m" => Ok(46),
        "." => Ok(47),
        "tab" => Ok(48),
        "space" => Ok(49),
        "`" => Ok(50),
        "backspace" => Ok(51),
        "esc" | "escape" => Ok(53),
        "meta" | "cmd" | "command" => Ok(55),
        "shift" => Ok(56),
        "alt" | "option" => Ok(58),
        "ctrl" | "control" => Ok(59),
        "right_shift" => Ok(60),
        "right_alt" => Ok(61),
        "right_ctrl" => Ok(62),
        "left" => Ok(123),
        "right" => Ok(124),
        "down" => Ok(125),
        "up" => Ok(126),
        other => Err(TendrilError::execution_failure(
            "unsupported_key",
            format!("unsupported macOS key `{other}`"),
            None,
        )),
    }
}

#[must_use]
pub fn adapter_for_context(context: AdapterContext) -> Box<dyn PlatformAdapter> {
    match context.platform {
        PlatformKind::MacOs => Box::new(MacOsAdapter::new(context)),
        PlatformKind::Linux => Box::new(LinuxAdapter::new(context)),
        PlatformKind::Windows11 => Box::new(WindowsAdapter::new(context)),
    }
}

#[must_use]
pub fn current_adapter() -> Box<dyn PlatformAdapter> {
    adapter_for_context(AdapterContext::detect())
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn detect_linux_session(
    xdg_session_type: Option<&str>,
    display: Option<&std::ffi::OsStr>,
    wayland_display: Option<&std::ffi::OsStr>,
) -> DesktopSession {
    match xdg_session_type.map(str::to_ascii_lowercase).as_deref() {
        Some("x11") => DesktopSession::X11,
        Some("wayland") => DesktopSession::Wayland,
        Some(_) | None => {
            if wayland_display.is_some() {
                DesktopSession::Wayland
            } else if display.is_some() {
                DesktopSession::X11
            } else {
                DesktopSession::Unknown
            }
        }
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn detect_linux_audio_backend(
    pipewire_runtime_dir: Option<&std::ffi::OsStr>,
    pulse_server: Option<&std::ffi::OsStr>,
    pulse_runtime_path: Option<&std::ffi::OsStr>,
) -> Option<AudioBackend> {
    if pipewire_runtime_dir.is_some() {
        Some(AudioBackend::PipeWire)
    } else if pulse_server.is_some() || pulse_runtime_path.is_some() {
        Some(AudioBackend::PulseAudio)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AdapterContext, AudioBackend, AudioCapabilityProbe, AudioSourceKind, Bounds,
        CaptureAdapter, CaptureTargetKind, DesktopSession, InputControlAdapter, InputRequest,
        LinuxAdapter, MacOsAdapter, ModifierKey, MouseButton, PermissionAdapter, PermissionKind,
        PermissionState, PlatformAdapter, PlatformAdapterError, PlatformKind, TargetDescriptor,
        TargetInventory, WindowsAdapter, WindowsRuntimeBackend,
        crop_wayland_portal_capture_to_target, detect_linux_audio_backend, detect_linux_session,
        execute_windows_input_with_runtime, is_macos_input_permission_error,
        javascript_string_literal, macos_focus_pid_jxa_script, macos_text_jxa_script,
        wayland_capture_program_on_path, wayland_workspace_origin, windows_key_is_supported,
    };
    use crate::{TendrilError, model::InputAction};
    use mcp_cli::ErrorCategory;

    #[test]
    fn linux_session_detection_prefers_declared_session_type() {
        let detected =
            detect_linux_session(Some("wayland"), Some(std::ffi::OsStr::new(":0")), None);

        assert_eq!(detected, DesktopSession::Wayland);
    }

    #[test]
    fn linux_audio_backend_detection_uses_pipewire_before_pulseaudio() {
        let detected = detect_linux_audio_backend(
            Some(std::ffi::OsStr::new("/run/user/1000")),
            Some(std::ffi::OsStr::new("unix:/tmp/pulse")),
            None,
        );

        assert_eq!(detected, Some(AudioBackend::PipeWire));
    }

    #[test]
    fn macos_loopback_probe_returns_structured_capability_error() {
        let adapter = MacOsAdapter::new(AdapterContext::macos());
        let error = adapter
            .probe_audio_capture(&super::AudioProbeRequest {
                source: AudioSourceKind::SystemLoopback,
                duration_hint_ms: Some(500),
            })
            .expect_err("loopback should be unsupported on the macOS spine");

        match error {
            PlatformAdapterError::UnsupportedCapability(capability) => {
                assert_eq!(
                    capability.capability,
                    super::Capability::AudioLoopbackCapture
                );
                assert_eq!(capability.platform, PlatformKind::MacOs);
                assert_eq!(
                    capability.reason,
                    super::CapabilityErrorReason::UnsupportedFeature
                );
                assert!(capability.suggested_action.is_some());
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn linux_wayland_input_probe_is_actionable() {
        let adapter = LinuxAdapter::new(AdapterContext::linux(
            DesktopSession::Wayland,
            Some(AudioBackend::PipeWire),
        ));
        let error = adapter
            .input_support()
            .expect_err("wayland input should not be generically supported yet");

        match error {
            PlatformAdapterError::UnsupportedCapability(capability) => {
                assert_eq!(capability.capability, super::Capability::InputControl);
                assert_eq!(
                    capability.reason,
                    super::CapabilityErrorReason::UnsupportedSession
                );
                assert!(
                    capability
                        .suggested_action
                        .as_deref()
                        .is_some_and(|message| message.contains("X11"))
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn windows_microphone_probe_exposes_permission_guidance() {
        let adapter = WindowsAdapter::new(AdapterContext::windows11());
        let report = adapter
            .probe_audio_capture(&super::AudioProbeRequest {
                source: AudioSourceKind::Microphone,
                duration_hint_ms: Some(1_000),
            })
            .expect("windows microphone probe should succeed");

        assert_eq!(report.backend, AudioBackend::Wasapi);
        assert_eq!(report.permissions.len(), 1);
        assert_eq!(report.permissions[0].permission, PermissionKind::Microphone);
        assert_eq!(report.permissions[0].state, PermissionState::Unknown);
        assert!(report.permissions[0].suggested_action.is_some());
    }

    #[test]
    fn macos_capture_support_requires_screen_permission() {
        let adapter = MacOsAdapter::new(AdapterContext::macos());
        let support = adapter
            .capture_support(CaptureTargetKind::Display)
            .expect("capture support should be described on macOS");

        assert_eq!(support.permissions.len(), 1);
        assert_eq!(
            support.permissions[0].permission,
            PermissionKind::ScreenCapture
        );
    }

    #[test]
    fn adapter_permissions_are_explicit_and_stateless() {
        let adapter = WindowsAdapter::new(AdapterContext::windows11());
        let permissions = adapter.permissions();

        assert_eq!(adapter.info().platform, PlatformKind::Windows11);
        assert!(adapter.info().stateless);
        assert_eq!(permissions.len(), 3);
    }

    #[derive(Default)]
    struct MockWindowsRuntime {
        calls: std::sync::Mutex<Vec<String>>,
    }

    impl MockWindowsRuntime {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("mutex poisoned").clone()
        }

        fn push(&self, call: impl Into<String>) {
            self.calls.lock().expect("mutex poisoned").push(call.into());
        }
    }

    impl WindowsRuntimeBackend for MockWindowsRuntime {
        fn capture_display(
            &self,
            _platform: PlatformKind,
            target_id: &str,
        ) -> Result<Vec<u8>, PlatformAdapterError> {
            self.push(format!("capture_display:{target_id}"));
            Ok(vec![1, 2, 3])
        }

        fn capture_window(
            &self,
            _platform: PlatformKind,
            target_id: &str,
        ) -> Result<Vec<u8>, PlatformAdapterError> {
            self.push(format!("capture_window:{target_id}"));
            Ok(vec![4, 5, 6])
        }

        fn focus_window(&self, target_id: &str) -> Result<(), String> {
            self.push(format!("focus:{target_id}"));
            Ok(())
        }

        fn send_text(&self, text: &str) -> Result<(), String> {
            self.push(format!("text:{text}"));
            Ok(())
        }

        fn tap_key(&self, key: &str) -> Result<(), String> {
            self.push(format!("tap:{key}"));
            Ok(())
        }

        fn hold_modifier(&self, modifier: ModifierKey) -> Result<(), String> {
            self.push(format!("hold:{modifier:?}"));
            Ok(())
        }

        fn release_modifier(&self, modifier: ModifierKey) -> Result<(), String> {
            self.push(format!("release:{modifier:?}"));
            Ok(())
        }

        fn click_mouse(&self, button: MouseButton, x: i32, y: i32) -> Result<(), String> {
            self.push(format!("click:{button:?}:{x}:{y}"));
            Ok(())
        }

        fn drag_mouse(
            &self,
            button: MouseButton,
            start_x: i32,
            start_y: i32,
            end_x: i32,
            end_y: i32,
        ) -> Result<(), String> {
            self.push(format!(
                "drag:{button:?}:{start_x}:{start_y}:{end_x}:{end_y}"
            ));
            Ok(())
        }
    }

    #[test]
    fn windows_native_runtime_flow_focuses_and_dispatches_actions() {
        let runtime = MockWindowsRuntime::default();
        let request = InputRequest {
            target: CaptureTargetKind::Window,
            target_id: "0x10".to_owned(),
            target_name: "Inbox".to_owned(),
            bounds: Bounds {
                x: 100,
                y: 200,
                width: 400,
                height: 300,
            },
            app_name: Some("Notepad".to_owned()),
            process_id: Some(42),
            text: None,
            actions: vec![
                InputAction::Hold {
                    modifier: ModifierKey::Ctrl,
                },
                InputAction::KeyTap {
                    key: "c".to_owned(),
                },
                InputAction::Release {
                    modifier: ModifierKey::Ctrl,
                },
                InputAction::Click {
                    button: MouseButton::Left,
                    x: 5,
                    y: 7,
                },
                InputAction::Drag {
                    x0: 1,
                    y0: 2,
                    x1: 20,
                    y1: 25,
                },
            ],
        };

        let outcome =
            execute_windows_input_with_runtime(PlatformKind::Windows11, &request, &runtime)
                .expect("mocked windows input should succeed");

        assert!(outcome.focus_required);
        assert!(outcome.focus_transferred);
        assert_eq!(outcome.focused_target.as_deref(), Some("0x10"));
        assert!(
            outcome
                .notes
                .iter()
                .any(|note| note.contains("native Win32 focus APIs"))
        );
        assert_eq!(
            runtime.calls(),
            vec![
                "focus:0x10",
                "hold:Ctrl",
                "tap:c",
                "release:Ctrl",
                "click:Left:105:207",
                "drag:Left:101:202:120:225",
            ]
        );
    }

    #[test]
    fn windows_native_runtime_text_flow_preserves_display_focus_guidance() {
        let runtime = MockWindowsRuntime::default();
        let request = InputRequest {
            target: CaptureTargetKind::Display,
            target_id: "1".to_owned(),
            target_name: "Display 1".to_owned(),
            bounds: Bounds {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            app_name: None,
            process_id: None,
            text: Some("hello".to_owned()),
            actions: Vec::new(),
        };

        let outcome =
            execute_windows_input_with_runtime(PlatformKind::Windows11, &request, &runtime)
                .expect("mocked windows text input should succeed");

        assert!(outcome.focus_required);
        assert!(!outcome.focus_transferred);
        assert_eq!(runtime.calls(), vec!["text:hello"]);
        assert!(
            outcome
                .notes
                .iter()
                .any(|note| note.contains("currently focused Windows control"))
        );
    }

    #[test]
    fn windows_key_support_matches_native_runtime_contract() {
        assert!(windows_key_is_supported("enter"));
        assert!(windows_key_is_supported("A"));
        assert!(windows_key_is_supported("💡"));
        assert!(!windows_key_is_supported("ctrl+alt+delete"));
    }

    #[test]
    fn tendril_error_category_tracks_adapter_error_category() {
        let adapter_error = PlatformAdapterError::unsupported(
            super::Capability::AudioLoopbackCapture,
            PlatformKind::MacOs,
            super::CapabilityErrorReason::UnsupportedFeature,
            "unsupported",
            Some("pick another source"),
        );
        let tendril_error: TendrilError = adapter_error.into();

        assert_eq!(
            tendril_error.category(),
            ErrorCategory::UnsupportedCapability
        );
    }

    #[test]
    fn javascript_string_literal_uses_json_escaping() {
        assert_eq!(
            javascript_string_literal("line \"one\"\nline two"),
            "\"line \\\"one\\\"\\nline two\""
        );
    }

    #[test]
    fn macos_input_permission_classifier_catches_accessibility_failures() {
        assert!(is_macos_input_permission_error(
            "System Events got an error: osascript is not allowed assistive access. (-1719)"
        ));
        assert!(is_macos_input_permission_error(
            "Not authorized to send Apple events to System Events. (-1743)"
        ));
    }

    #[test]
    fn macos_osascript_scripts_are_self_contained() {
        let focus_script = macos_focus_pid_jxa_script(42);
        let text_script = macos_text_jxa_script("hello");

        assert!(focus_script.contains("NSRunningApplication"));
        assert!(text_script.contains("System Events"));
        assert!(!focus_script.contains("swift"));
        assert!(!text_script.contains("swift"));
    }

    #[test]
    fn wayland_capture_support_notes_describe_portal_first_strategy() {
        let adapter = LinuxAdapter::new(AdapterContext::linux(
            DesktopSession::Wayland,
            Some(AudioBackend::PipeWire),
        ));
        let support = adapter
            .capture_support(CaptureTargetKind::Window)
            .expect("wayland capture support should be described");

        assert!(
            support
                .notes
                .iter()
                .any(|note| note.contains("xdg-desktop-portal"))
        );
        assert!(support.notes.iter().any(|note| note.contains("grim")));
        assert!(support.notes.iter().any(|note| note.contains("hyprctl")));
    }

    #[test]
    fn wayland_workspace_origin_tracks_leftmost_display() {
        let inventory = TargetInventory {
            targets: vec![
                display_target("1", -1920, 40, 1920, 1080),
                display_target("2", 0, 0, 2560, 1440),
            ],
        };

        assert_eq!(wayland_workspace_origin(&inventory), (-1920, 0));
    }

    #[test]
    fn wayland_portal_crop_translates_global_bounds_into_workspace_space() {
        let inventory = TargetInventory {
            targets: vec![
                display_target("1", -2, 0, 2, 2),
                display_target("2", 0, 0, 2, 2),
            ],
        };
        let target = TargetDescriptor {
            id: "window-1".to_owned(),
            title: Some("Example".to_owned()),
            kind: CaptureTargetKind::Window,
            name: "Example".to_owned(),
            bounds: crate::model::Bounds {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
            scale_factor: crate::model::ScaleFactor::identity(),
            capture_supported: true,
            input_supported: false,
            app_name: Some("example".to_owned()),
            process_id: Some(7),
        };

        let cropped = crop_wayland_portal_capture_to_target(
            &sample_png(
                4,
                2,
                image::Rgba([0, 0, 255, 255]),
                image::Rgba([255, 0, 0, 255]),
            ),
            &target,
            &inventory,
        )
        .expect("portal crop should succeed");
        let decoded = image::load_from_memory(&cropped)
            .expect("cropped image should decode")
            .to_rgba8();

        assert_eq!(decoded.width(), 2);
        assert_eq!(decoded.height(), 2);
        assert_eq!(*decoded.get_pixel(0, 0), image::Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn wayland_capture_path_lookup_only_checks_known_program_names() {
        assert!(!wayland_capture_program_on_path(
            "grim definitely does not exist"
        ));
    }

    fn display_target(id: &str, x: i32, y: i32, width: u32, height: u32) -> TargetDescriptor {
        TargetDescriptor {
            id: id.to_owned(),
            title: None,
            kind: CaptureTargetKind::Display,
            name: id.to_owned(),
            bounds: crate::model::Bounds {
                x,
                y,
                width,
                height,
            },
            scale_factor: crate::model::ScaleFactor::identity(),
            capture_supported: true,
            input_supported: false,
            app_name: None,
            process_id: None,
        }
    }

    fn sample_png(
        width: u32,
        height: u32,
        left: image::Rgba<u8>,
        right: image::Rgba<u8>,
    ) -> Vec<u8> {
        let image =
            image::DynamicImage::ImageRgba8(image::ImageBuffer::from_fn(width, height, |x, _| {
                if x < width / 2 { left } else { right }
            }));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("sample png should encode");
        bytes
    }
}
